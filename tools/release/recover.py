#!/usr/bin/env python3
"""Bounded v0.1.1 recovery: verify an existing draft, then publish it once.

No release creation, asset upload/deletion, tag write, rebuild or retry of PATCH.
Run only trusted recovery code, with GH_TOKEN and GH_REPO supplied by Actions.
The approved source/tag remain fixed even when selecting a different original run.
"""
from __future__ import annotations

import argparse
import copy
import filecmp
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request

import automation
import release

TAG = "v0.1.1"
SOURCE = "cf62e4381600a383d70fbba44b11811e75d5558f"
ORIGINAL_RUN = "42577373476"
PUBLISH_JOB = "Create draft and publish if explicitly approved (tag only)"
PUBLISH_STEP = "Verify uploaded draft bytes and publish approved tag"
CREATE_STEP = "Create new GitHub Release draft"
JSON_LIMIT = 16 * 1024 * 1024
LOG_LIMIT = 8 * 1024 * 1024
MAX_PAGES = 20


class RecoveryError(Exception):
    """A failed precondition; no publication is attempted after this error."""


def require(condition, message):
    if not condition:
        raise RecoveryError(message)


def redact(text, token=""):
    text = str(text)
    if token:
        text = text.replace(token, "[REDACTED]")
    text = re.sub(r"(?i)(bearer|token)\s+[\w.=-]+", r"\1 [REDACTED]", text)
    text = re.sub(r"\b(?:gh[pousr]_[A-Za-z0-9_]+|github_pat_[A-Za-z0-9_]+)\b", "[REDACTED]", text)
    text = re.sub(r"https?://[^\s<>]+", lambda m: m[0].split("?")[0] + ("?[REDACTED]" if "?" in m[0] else ""), text)
    return "".join(c for c in text if c >= " " or c in "\n\r\t")


def annotation(kind, text, token=""):
    text = redact(text, token).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::{kind}::{text}", flush=True)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class API:
    def __init__(self, repo, token, api_url="https://api.github.com"):
        require(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo) is not None, "Invalid repository identity")
        require(api_url == "https://api.github.com", "Recovery is restricted to the public GitHub API")
        require(bool(token), "GH_TOKEN is required")
        self.repo, self.token = repo, token
        self.base = api_url + "/repos/" + repo
        self.opener = urllib.request.build_opener(NoRedirect())

    def _open(self, path, accept, method="GET", payload=None):
        require(path.startswith("/") and not path.startswith("//"), "Invalid repository API path")
        headers = {"Authorization": "Bearer " + self.token, "Accept": accept,
                   "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "TwinTail-release-recovery"}
        data = None if payload is None else json.dumps(payload).encode("utf-8")
        if data is not None:
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(self.base + path, data=data, headers=headers, method=method)
        try:
            return self.opener.open(request, timeout=60)
        except urllib.error.HTTPError as error:
            is_download = accept == "application/octet-stream" or re.fullmatch(r"/actions/jobs/[0-9]+/logs", path) is not None
            if method == "GET" and is_download and error.code in (301, 302, 303, 307, 308):
                location = error.headers.get("Location", "")
                error.close()
                # Never forward the API Authorization header to a signed asset/log URL.
                for _ in range(5):
                    parsed = urllib.parse.urlsplit(location)
                    require(parsed.scheme == "https" and parsed.hostname and not parsed.username and not parsed.password,
                            "Asset/log redirect must use HTTPS without URL credentials")
                    redirected = urllib.request.Request(location, headers={"User-Agent": "TwinTail-release-recovery"})
                    try:
                        return self.opener.open(redirected, timeout=60)
                    except urllib.error.HTTPError as redirect_error:
                        if redirect_error.code not in (301, 302, 303, 307, 308):
                            raise RecoveryError(f"Asset/log redirect returned HTTP {redirect_error.code}") from None
                        location = urllib.parse.urljoin(location, redirect_error.headers.get("Location", ""))
                        redirect_error.close()
                raise RecoveryError("Too many asset/log redirects")
            raise RecoveryError(f"GitHub API {method} {path} returned HTTP {error.code}") from None
        except urllib.error.URLError:
            raise RecoveryError(f"GitHub API {method} {path} transport failed") from None

    def get(self, path):
        with self._open(path, "application/vnd.github+json") as response:
            data = response.read(JSON_LIMIT + 1)
        require(len(data) <= JSON_LIMIT, "API JSON response exceeds limit")
        return json.loads(data)

    def patch(self, path, payload):
        with self._open(path, "application/vnd.github+json", "PATCH", payload) as response:
            data = response.read(JSON_LIMIT + 1)
        require(len(data) <= JSON_LIMIT, "Publication response exceeds limit; inspect remote state before any retry")
        return json.loads(data)

    def binary(self, path, limit, destination=None):
        accept = "application/vnd.github+json" if re.fullmatch(r"/actions/jobs/[0-9]+/logs", path) else "application/octet-stream"
        with self._open(path, accept) as response:
            if destination is None:
                data = response.read(limit + 1)
                require(len(data) <= limit, "Log download exceeds limit")
                return data
            count = 0
            with destination.open("xb") as output:
                while True:
                    chunk = response.read(min(1024 * 1024, limit - count + 1))
                    if not chunk:
                        break
                    count += len(chunk)
                    require(count <= limit, "Asset download exceeds expected size")
                    output.write(chunk)
            require(count == limit, "Asset download size differs from review bundle")

    def pages(self, path, key=None):
        result = []
        separator = "&" if "?" in path else "?"
        for page in range(1, MAX_PAGES + 1):
            response = self.get(f"{path}{separator}per_page=100&page={page}")
            entries = response if key is None else response[key]
            require(isinstance(entries, list), "API inventory must be a list")
            result.extend(entries)
            if len(entries) < 100:
                return result
        raise RecoveryError("API inventory exceeds bounded pagination limit")


def original_jobs(api, run_id):
    return api.pages(f"/actions/runs/{run_id}/jobs?filter=latest", "jobs")


def publication_diagnostics(api, jobs):
    publishers = [job for job in jobs if job.get("name") == PUBLISH_JOB]
    require(len(publishers) == 1, "Original run must have exactly one publication job")
    job = publishers[0]
    data = api.binary(f"/actions/jobs/{int(job['id'])}/logs", LOG_LIMIT)
    lines = data.decode("utf-8", errors="replace").splitlines()
    relevant = [i for i, line in enumerate(lines) if re.search(r"(?i)(traceback|error|exception|failed|differs|mismatch|not found|404|403|SystemExit)", line)]
    require(bool(relevant), "Original publication logs contain no diagnostic errors")
    # Include final traceback context so the actual failing check is visible.
    indices = sorted({j for i in relevant[-8:] for j in range(max(0, i - 2), min(len(lines), i + 2))})[-24:]
    for index in indices:
        annotation("notice", "Original publication log: " + lines[index][:2000], api.token)
    return job


def validate_run(api, run_id, tag, jobs, publisher):
    run = api.get(f"/actions/runs/{run_id}")
    require(str(run.get("id")) == str(run_id) and run.get("head_sha") == SOURCE,
            "Original run source differs from approved source")
    require(run.get("path") == ".github/workflows/release.yml" and run.get("event") == "push"
            and run.get("head_branch") == tag, "Original run is not the original version-tag release workflow")
    require(run.get("status") == "completed" and run.get("conclusion") == "failure", "Original run is not a completed failed release")
    require(run.get("repository", {}).get("full_name", "").lower() == api.repo.lower(), "Original run repository mismatch")
    required = {"Validate release identity", "Collect and optionally create draft"}
    required.update("Package / " + target for target in release.TARGETS)
    checks = {"Rust 1.99.1 / " + os_name for os_name in ("ubuntu-latest", "ubuntu-24.04-arm", "windows-latest")}
    checks.add("WebUI / Node 26.7.0")
    required.update("Full source validation / " + name for name in checks)
    require(len(jobs) == 10 and len({job.get("name") for job in jobs}) == 10,
            "Original run must contain exactly ten distinct release jobs")
    require({job.get("name") for job in jobs} == required | {PUBLISH_JOB}, "Original run job inventory mismatch")
    for job in jobs:
        if job.get("name") in required:
            require(job.get("status") == "completed" and job.get("conclusion") == "success", "Original validation/package/collect job did not succeed: " + job["name"])
    require(publisher.get("status") == "completed" and publisher.get("conclusion") == "failure", "Original publication job was not failed")
    steps = publisher.get("steps", [])
    require(sum(step.get("name") == CREATE_STEP and step.get("conclusion") == "success" for step in steps) == 1,
            "Original draft creation did not succeed")
    require([step.get("name") for step in steps if step.get("conclusion") == "failure"] == [PUBLISH_STEP],
            "Original publication failed outside the final verify/publish step")
    return run


def approved_tag(api, tag):
    ref = api.get("/git/ref/tags/" + urllib.parse.quote(tag, safe=""))["object"]
    require(ref.get("type") == "tag", "Publication requires the original annotated version tag")
    tag_object = copy.deepcopy(ref)
    obj = ref
    for depth in range(16):
        if obj.get("type") != "tag":
            break
        require(re.fullmatch(r"[0-9a-f]{40}", obj.get("sha", "")) is not None, "Invalid tag object SHA")
        annotation_data = api.get("/git/tags/" + obj["sha"])
        if depth == 0:
            require(annotation_data.get("tag") == tag and "TwinTail-Publish: approved" in annotation_data.get("message", "").split("\n"),
                    "Original top-level tag lacks explicit publication approval")
        obj = annotation_data["object"]
    require(obj.get("type") == "commit" and obj.get("sha") == SOURCE, "Remote version tag does not pin the approved original source")
    return tag_object


def expected_files(tag):
    version = automation.tag_version(tag)
    return {"SHA256SUMS", "release-notes.md"} | {
        name for target in release.TARGETS
        for name in (automation.archive_name(version, target), target + "-validation.json")}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify_bundle(directory, tag):
    expected = expected_files(tag)
    entries = list(directory.iterdir())
    require({path.name for path in entries} == expected and len(entries) == 8, "Review bundle must contain exactly eight expected files")
    for path in entries:
        info = path.lstat()
        require(path.is_file() and not path.is_symlink() and info.st_nlink == 1 and info.st_size <= release.MAX_TOTAL,
                "Review bundle contains a non-regular/oversized file")
    require((directory / "SHA256SUMS").stat().st_size <= 16384, "Checksum inventory exceeds limit")
    sums = (directory / "SHA256SUMS").read_bytes()
    seen = set()
    for line in sums.decode("ascii").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        require(match is not None, "Invalid SHA256SUMS line")
        checksum, name = match.groups()
        require(name in expected - {"SHA256SUMS"} and name not in seen, "Unexpected/duplicate checksum entry")
        require(digest(directory / name) == checksum, "Review bundle checksum mismatch: " + name)
        seen.add(name)
    require(seen == expected - {"SHA256SUMS"}, "Checksum inventory must cover precisely seven files")
    version = automation.tag_version(tag)
    for target in release.TARGETS:
        report = automation.validation_report(automation.regular(directory / (target + "-validation.json"), automation.REPORT_LIMIT), version, target)
        require(release.verify(directory / automation.archive_name(version, target)) == report,
                "Offline archive verification differs from collected report: " + target)
    notes = automation.regular(directory / "release-notes.md", 128 * 1024).decode("utf-8")
    require(f"# TwinTail {tag}\n" in notes and f"Source commit (caller-supplied SHA): `{SOURCE}`" in notes,
            "Review bundle release notes do not identify original tag/source")
    return {name: (directory / name).stat().st_size for name in expected}


def existing_draft(api, tag, sizes):
    # GET /releases/tags/TAG may report 404 for a draft; authenticated LIST is required.
    matches = [entry for entry in api.pages("/releases") if entry.get("tag_name") == tag]
    require(len(matches) == 1, "Expected exactly one already-existing release for the approved tag")
    draft = matches[0]
    require(draft.get("draft") is True, "Release is already public; recovery makes no changes")
    draft = api.get(f"/releases/{int(draft['id'])}")
    require(draft.get("draft") is True and draft.get("tag_name") == tag and draft.get("prerelease") is False,
            "Existing release is not the expected non-prerelease draft")
    annotation("notice", "Observed draft target_commitish=" + str(draft.get("target_commitish")) + "; binding is actual annotated tag -> approved original run SHA", api.token)
    validate_assets(draft, sizes)
    return draft


def validate_assets(record, sizes):
    assets = record.get("assets", [])
    require(len(assets) == 8 and {asset.get("name") for asset in assets} == set(sizes), "Draft asset set differs from exact eight-file review bundle")
    require(len({asset.get("id") for asset in assets}) == 8, "Duplicate release asset IDs")
    for asset in assets:
        require(isinstance(asset.get("id"), int) and asset["id"] > 0 and asset.get("state") == "uploaded"
                and asset.get("size") == sizes[asset["name"]], "Draft asset size/state mismatch: " + str(asset.get("name")))


def stable_record(record):
    result = copy.deepcopy(record)
    for asset in result.get("assets", []):
        asset.pop("download_count", None)  # Our authenticated downloads change this counter.
    result["assets"] = sorted(result.get("assets", []), key=lambda asset: asset["id"])
    return result


def download_bundle(api, run_id, directory):
    result = subprocess.run(["gh", "run", "download", str(run_id), "--repo", api.repo,
                             "-n", "release-review-bundle", "-D", str(directory)],
                            capture_output=True, text=True, timeout=300, check=False)
    require(result.returncode == 0, "Original review bundle download failed: " + redact(result.stderr[-2000:], api.token))


def recover(api, tag, run_id, downloader=download_bundle):
    require(tag == TAG, "Recovery is bounded to the already-approved v0.1.1 release")
    require(re.fullmatch(r"[1-9][0-9]{0,19}", str(run_id)) is not None, "Invalid original workflow run ID")
    jobs = original_jobs(api, run_id)
    publisher = publication_diagnostics(api, jobs)  # Diagnose the original failure before recovery checks.
    validate_run(api, run_id, tag, jobs, publisher)
    original_tag = approved_tag(api, tag)
    with tempfile.TemporaryDirectory(prefix="twintail-recover-") as temporary:
        root = Path(temporary)
        local = root / "review"
        local.mkdir(mode=0o700)
        downloader(api, run_id, local)
        sizes = verify_bundle(local, tag)
        draft = existing_draft(api, tag, sizes)
        snapshot = stable_record(draft)
        downloaded = root / "uploaded"
        downloaded.mkdir(mode=0o700)
        for asset in draft["assets"]:
            name = asset["name"]
            destination = downloaded / name
            api.binary(f"/releases/assets/{asset['id']}", sizes[name], destination)
            require(filecmp.cmp(local / name, destination, shallow=False), "Uploaded asset bytes differ: " + name)
            require(digest(local / name) == digest(destination), "Uploaded asset SHA256 differs: " + name)
            remote_digest = asset.get("digest")
            require(remote_digest is None or remote_digest == "sha256:" + digest(destination), "GitHub asset digest differs: " + name)
        # Verify both checksum manifests, reports and all three uploaded archives offline.
        require(verify_bundle(downloaded, tag) == sizes, "Uploaded review bundle differs")
        require(approved_tag(api, tag) == original_tag, "Remote annotated tag object changed during recovery")
        current = api.get(f"/releases/{draft['id']}")
        require(stable_record(current) == snapshot, "Existing draft changed during verification; publication stopped")
        # The only remote mutation. Never retry automatically after an ambiguous result.
        api.patch(f"/releases/{draft['id']}", {"draft": False, "make_latest": "true"})
        public = api.get("/releases/tags/" + urllib.parse.quote(tag, safe=""))
        require(public.get("id") == draft["id"] and public.get("tag_name") == tag
                and public.get("draft") is False and bool(public.get("published_at")),
                "Post-publication public record mismatch; inspect remote state before retrying")
        validate_assets(public, sizes)
        public_snapshot = stable_record(public)
        # GitHub legitimately replaces draft URLs and source-archive links on publication.
        identity_fields = ("id", "node_id", "tag_name", "target_commitish", "name", "body",
                           "prerelease", "created_at", "assets")
        require(all(public_snapshot.get(field) == snapshot.get(field) for field in identity_fields),
                "Published record/assets unexpectedly changed; inspect remote state")
        require(api.get("/releases/latest").get("id") == draft["id"], "Published release is not latest; inspect remote state")
        require(approved_tag(api, tag) == original_tag, "Post-publication remote tag changed; inspect remote state")
        return {"published": True, "tag": tag, "source": SOURCE, "original_run": str(run_id), "release_id": draft["id"], "assets": 8}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--run", default=ORIGINAL_RUN, help="Original release.yml run, never the recovery run")
    args = parser.parse_args(argv)
    token = os.environ.get("GH_TOKEN", "")
    try:
        api = API(os.environ.get("GH_REPO", os.environ.get("GITHUB_REPOSITORY", "")), token,
                  os.environ.get("GITHUB_API_URL", "https://api.github.com"))
        print(json.dumps(recover(api, args.tag, args.run), sort_keys=True))
        return 0
    except Exception as error:
        # Also annotate malformed archive/transport failures; never dump a traceback or token.
        annotation("error", f"Release recovery stopped ({type(error).__name__}): {error}", token)
        return 1


if __name__ == "__main__":
    sys.exit(main())
