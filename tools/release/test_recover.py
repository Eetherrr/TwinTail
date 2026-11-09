"""Mocked network and synthetic offline archives; never publishes a real release."""
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch
import urllib.error

import automation
import recover
import release
import test_release


class FakeAPI:
    repo = "Eetherrr/TwinTail"
    token = "ghp_SYNTHETIC_TEST_TOKEN"

    def __init__(self, bundle):
        self.calls = []
        self.mutations = []
        self.bundle = dict(bundle)
        self.jobs = []
        names = ["Validate release identity", "Collect and optionally create draft"]
        names += ["Package / " + target for target in release.TARGETS]
        names += ["Full source validation / Rust 1.99.1 / " + os_name for os_name in ("ubuntu-latest", "ubuntu-24.04-arm", "windows-latest")]
        names += ["Full source validation / WebUI / Node 26.7.0", recover.PUBLISH_JOB]
        for index, name in enumerate(names, 1):
            self.jobs.append({"id": index, "name": name, "status": "completed", "conclusion": "success"})
        self.jobs[-1].update(conclusion="failure", steps=[
            {"name": recover.CREATE_STEP, "conclusion": "success"},
            {"name": recover.PUBLISH_STEP, "conclusion": "failure"}])
        self.run = {"id": int(recover.ORIGINAL_RUN), "head_sha": recover.SOURCE,
                    "path": ".github/workflows/release.yml", "event": "push", "head_branch": recover.TAG,
                    "status": "completed", "conclusion": "failure", "repository": {"full_name": self.repo}}
        self.ref = {"type": "tag", "sha": "a" * 40}
        self.tag = {"tag": recover.TAG, "message": "Release\nTwinTail-Publish: approved\n",
                    "object": {"type": "commit", "sha": recover.SOURCE}}
        self.record = {"id": 101, "node_id": "release101", "tag_name": recover.TAG,
                       "target_commitish": "main", "draft": True, "prerelease": False,
                       "name": "TwinTail v0.1.1", "body": "original notes", "created_at": "2026-11-09T07:19:20Z",
                       "updated_at": "2026-11-09T07:19:20Z", "published_at": None,
                       "html_url": "https://github.com/Eetherrr/TwinTail/releases/tag/untagged-test",
                       "tarball_url": None, "zipball_url": None,
                       "assets": [{"id": index, "name": name, "state": "uploaded", "size": len(data),
                                   "digest": "sha256:" + release.sha(data), "download_count": 0}
                                  for index, (name, data) in enumerate(sorted(bundle.items()), 201)]}
        self.logs = b'Traceback (most recent call last):\n  in verify_draft\nurllib.error.HTTPError: HTTP Error 404: Not Found\n'
        self.on_last_download = None
        self.on_tag_recheck = None
        self.ref_reads = 0
        self.last_download = len(self.record["assets"]) - 1
        self.asset_downloads = 0
        self.override_public = None

    def pages(self, path, key=None):
        self.calls.append(("LIST", path))
        if path.startswith("/actions/runs/"):
            return copy.deepcopy(self.jobs)
        if path == "/releases":
            return [copy.deepcopy(self.record)]
        raise AssertionError(path)

    def get(self, path):
        self.calls.append(("GET", path))
        if path.startswith("/actions/runs/"):
            return copy.deepcopy(self.run)
        if path.startswith("/git/ref/"):
            self.ref_reads += 1
            if self.ref_reads == 2 and self.on_tag_recheck:
                self.on_tag_recheck(self)
            return {"object": copy.deepcopy(self.ref)}
        if path.startswith("/git/tags/"):
            return copy.deepcopy(self.tag)
        if path in ("/releases/tags/" + recover.TAG, "/releases/latest"):
            # Model the draft-by-tag 404 behavior that motivated recovery.
            if self.record["draft"]:
                raise recover.RecoveryError("GET tags returns 404 for draft")
            return copy.deepcopy(self.override_public or self.record)
        if path == "/releases/101":
            return copy.deepcopy(self.record)
        raise AssertionError(path)

    def binary(self, path, limit, destination=None):
        self.calls.append(("BINARY", path))
        if path.startswith("/actions/jobs/"):
            return self.logs
        asset_id = int(path.rsplit("/", 1)[1])
        name = next(asset["name"] for asset in self.record["assets"] if asset["id"] == asset_id)
        destination.write_bytes(self.bundle[name])
        self.asset_downloads += 1
        for asset in self.record["assets"]:
            if asset["id"] == asset_id:
                asset["download_count"] += 1
        if self.asset_downloads == 8 and self.on_last_download:
            self.on_last_download(self)

    def patch(self, path, payload):
        self.calls.append(("PATCH", path))
        self.mutations.append((path, payload))
        self.record.update(draft=False, published_at="2026-11-09T08:00:00Z", updated_at="2026-11-09T08:00:00Z",
                           html_url="https://github.com/Eetherrr/TwinTail/releases/tag/v0.1.1",
                           tarball_url="https://api.github.com/repos/Eetherrr/TwinTail/tarball/v0.1.1",
                           zipball_url="https://api.github.com/repos/Eetherrr/TwinTail/zipball/v0.1.1")
        return copy.deepcopy(self.record)


class RecoveryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Generate actual synthetic ELF/PE platform archives with the real packager.
        fixture = test_release.ReleaseTests()
        fixture.setUp()
        try:
            paths = [fixture.repo / "Cargo.toml", *(fixture.repo / source / "plugin.toml" for source in release.PLUGINS.values())]
            for path in paths:
                path.write_text(path.read_text(encoding="utf-8").replace('version = "0.1.0"', 'version = "0.1.1"'), encoding="utf-8")
            artifacts = fixture.root / "artifacts"
            artifacts.mkdir()
            for target in release.TARGETS:
                directory = artifacts / target
                directory.mkdir()
                archive = directory / automation.archive_name("0.1.1", target)
                release.archive(fixture.stage(target, "stage-" + target), archive)
                (directory / "validation.json").write_bytes(release.json_bytes(release.verify(archive)))
            output = fixture.root / "review"
            automation.collect(fixture.repo, artifacts, output, recover.TAG, recover.SOURCE)
            cls.bundle = {path.name: path.read_bytes() for path in output.iterdir()}
        finally:
            fixture.doCleanups()

    def setUp(self):
        self.api = FakeAPI(self.bundle)
        self.output = io.StringIO()

    def downloader(self, api, run_id, directory):
        api.calls.append(("DOWNLOAD_REVIEW", str(run_id)))
        for name, data in self.bundle.items():
            (directory / name).write_bytes(data)

    def recover(self):
        with contextlib.redirect_stdout(self.output):
            return recover.recover(self.api, recover.TAG, recover.ORIGINAL_RUN, self.downloader)

    def fails_without_mutation(self, pattern):
        with self.assertRaisesRegex((recover.RecoveryError, release.ReleaseError, ValueError), pattern):
            self.recover()
        self.assertEqual(self.api.mutations, [])

    def test_valid_publication_exactly_one_patch_actual_tag_not_target_commitish(self):
        with patch.object(release, "verify", wraps=release.verify) as verifier:
            result = self.recover()
        self.assertTrue(result["published"])
        self.assertEqual(verifier.call_count, 6)
        self.assertEqual(self.api.mutations, [("/releases/101", {"draft": False, "make_latest": "true"})])
        self.assertEqual(self.api.asset_downloads, 8)
        self.assertEqual(self.api.ref_reads, 3)
        self.assertIn("Observed draft target_commitish=main", self.output.getvalue())
        first_binary = next(i for i, call in enumerate(self.api.calls) if call[0] == "BINARY")
        self.assertLess(first_binary, self.api.calls.index(("GET", "/actions/runs/" + recover.ORIGINAL_RUN)))
        self.assertIn("HTTP Error 404: Not Found", self.output.getvalue())
        tag_lookup = self.api.calls.index(("GET", "/releases/tags/" + recover.TAG))
        self.assertGreater(tag_lookup, self.api.calls.index(("PATCH", "/releases/101")))

    def test_public_release_no_changes(self):
        self.api.record.update(draft=False, published_at="already")
        self.fails_without_mutation("already public")
        self.assertEqual(self.api.asset_downloads, 0)

    def test_missing_duplicate_and_prerelease_draft_fail_closed(self):
        original_pages = self.api.pages
        def duplicate_pages(path, key=None):
            records = original_pages(path, key)
            return records * 2 if path == "/releases" else records
        self.api.pages = duplicate_pages
        self.fails_without_mutation("exactly one")
        self.api = FakeAPI(self.bundle)
        original_pages = self.api.pages
        self.api.pages = lambda path, key=None: [] if path == "/releases" else original_pages(path, key)
        self.fails_without_mutation("exactly one")
        self.api = FakeAPI(self.bundle)
        self.api.record["prerelease"] = True
        self.fails_without_mutation("non-prerelease draft")

    def test_remote_asset_same_size_tamper(self):
        name = "release-notes.md"
        data = self.api.bundle[name]
        self.api.bundle[name] = bytes([data[0] ^ 1]) + data[1:]
        self.fails_without_mutation("bytes differ")

    def test_asset_inventory_state_size_and_digest_mismatch(self):
        for field, value in (("size", 1), ("state", "new"), ("name", "unexpected"), ("digest", "sha256:" + "0" * 64)):
            with self.subTest(field=field):
                self.api = FakeAPI(self.bundle)
                self.api.record["assets"][0][field] = value
                self.fails_without_mutation("asset|digest")
        self.api = FakeAPI(self.bundle)
        self.api.record["assets"].pop()
        self.fails_without_mutation("asset set")

    def test_run_source_path_event_tag_repo_and_status_mismatch(self):
        for field, value in (("head_sha", "0" * 40), ("path", ".github/workflows/other.yml"),
                             ("event", "workflow_dispatch"), ("head_branch", "main"),
                             ("repository", {"full_name": "other/repo"}), ("status", "in_progress")):
            with self.subTest(field=field):
                self.api = FakeAPI(self.bundle)
                self.api.run[field] = value
                self.fails_without_mutation("Original run")

    def test_every_required_job_must_succeed(self):
        for index in range(9):
            with self.subTest(index=index):
                self.api = FakeAPI(self.bundle)
                self.api.jobs[index]["conclusion"] = "failure"
                self.fails_without_mutation("did not succeed")

    def test_draft_creation_and_failed_step_must_match_original(self):
        self.api.jobs[-1]["steps"][0]["conclusion"] = "failure"
        self.fails_without_mutation("draft creation")
        self.api = FakeAPI(self.bundle)
        self.api.jobs[-1]["steps"][1]["name"] = "another failure"
        self.fails_without_mutation("outside the final")

    def test_extra_or_duplicate_jobs_rejected(self):
        self.api.jobs.append(copy.deepcopy(self.api.jobs[0]))
        self.fails_without_mutation("ten distinct")
        self.api = FakeAPI(self.bundle)
        self.api.jobs[0]["name"] = "unknown job"
        self.fails_without_mutation("inventory mismatch")

    def test_tag_mismatch_lightweight_unapproved_and_inner_approval_only(self):
        self.api.tag["object"]["sha"] = "b" * 40
        self.fails_without_mutation("approved original source")
        self.api = FakeAPI(self.bundle)
        self.api.ref["type"] = "commit"
        self.fails_without_mutation("annotated")
        self.api = FakeAPI(self.bundle)
        self.api.tag["message"] = "not approved\n"
        self.fails_without_mutation("explicit publication approval")
        self.api = FakeAPI(self.bundle)
        self.api.tag["message"] = "prefix TwinTail-Publish: approved\n"
        self.fails_without_mutation("explicit publication approval")

    def test_same_commit_retag_during_download_stops_publication(self):
        self.api.on_tag_recheck = lambda api: api.ref.update(sha="b" * 40)
        self.fails_without_mutation("tag object changed")

    def test_draft_body_target_or_asset_change_during_download(self):
        mutations = [lambda api: api.record.update(body="edited"),
                     lambda api: api.record.update(target_commitish="changed"),
                     lambda api: api.record["assets"][0].update(id=999),
                     lambda api: api.record.update(draft=False)]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.api = FakeAPI(self.bundle)
                self.api.on_last_download = mutation
                self.fails_without_mutation("draft changed")

    def test_wrong_review_bundle_checksum_report_extra_and_missing_files(self):
        original = self.bundle
        for case in ("checksum", "report", "extra", "missing", "symlink"):
            with self.subTest(case=case):
                self.api = FakeAPI(original)
                def wrong_download(api, run_id, directory):
                    self.downloader(api, run_id, directory)
                    if case == "checksum":
                        (directory / "release-notes.md").write_bytes(b"changed")
                    elif case == "report":
                        name = "linux-x86_64-validation.json"
                        (directory / name).write_bytes(b"{}")
                        self.rewrite_checksums(directory)
                    elif case == "extra":
                        (directory / "unexpected").write_bytes(b"extra")
                    elif case == "missing":
                        (directory / "release-notes.md").unlink()
                    else:
                        path = directory / "release-notes.md"
                        path.unlink()
                        path.symlink_to(directory / "SHA256SUMS")
                with contextlib.redirect_stdout(self.output), self.assertRaises((recover.RecoveryError, release.ReleaseError)):
                    recover.recover(self.api, recover.TAG, recover.ORIGINAL_RUN, wrong_download)
                self.assertEqual(self.api.mutations, [])

    @staticmethod
    def rewrite_checksums(directory):
        (directory / "SHA256SUMS").write_text("".join(f"{recover.digest(path)}  {path.name}\n" for path in sorted(directory.iterdir()) if path.name != "SHA256SUMS"), encoding="ascii")

    def test_corrupt_archive_with_consistent_outer_checksum_fails_offline(self):
        def downloader(api, run_id, directory):
            self.downloader(api, run_id, directory)
            name = automation.archive_name("0.1.1", "linux-x86_64")
            (directory / name).write_bytes(b"invalid archive")
            self.rewrite_checksums(directory)
        with contextlib.redirect_stdout(self.output), self.assertRaises(Exception):
            recover.recover(self.api, recover.TAG, recover.ORIGINAL_RUN, downloader)
        self.assertEqual(self.api.mutations, [])

    def test_malformed_checksum_duplicate_traversal_and_incomplete_inventory(self):
        for content in (b"bad\n", b"0" * 64 + b"  ../outside\n", b"", None):
            with self.subTest(content=content):
                self.api = FakeAPI(self.bundle)
                def downloader(api, run_id, directory):
                    self.downloader(api, run_id, directory)
                    sums = directory / "SHA256SUMS"
                    sums.write_bytes(content if content is not None else sums.read_bytes() * 2)
                with contextlib.redirect_stdout(self.output), self.assertRaises(recover.RecoveryError):
                    recover.recover(self.api, recover.TAG, recover.ORIGINAL_RUN, downloader)
                self.assertEqual(self.api.mutations, [])

    def test_diagnostics_redact_secrets_and_escape_annotations(self):
        self.api.logs = f"error Bearer {self.api.token}\nerror https://signed.example/log?sig=SECRET\n::error::injected%\n".encode()
        self.recover()
        output = self.output.getvalue()
        self.assertNotIn(self.api.token, output)
        self.assertNotIn("sig=SECRET", output)
        self.assertIn("[REDACTED]", output)
        self.assertIn("injected%25", output)

    def test_failed_log_retrieval_stops_before_bundle_or_patch(self):
        self.api.binary = Mock(side_effect=recover.RecoveryError("job log HTTP 403"))
        self.fails_without_mutation("HTTP 403")
        self.assertFalse(any(call[0] == "DOWNLOAD_REVIEW" for call in self.api.calls))

    def test_scope_and_run_id_are_bounded(self):
        for tag, run_id in (("v0.1.0", recover.ORIGINAL_RUN), (recover.TAG, "../run"), (recover.TAG, "-1")):
            with self.subTest(tag=tag, run=run_id), self.assertRaises(recover.RecoveryError):
                recover.recover(self.api, tag, run_id, self.downloader)
        self.assertEqual(self.api.calls, [])

    def test_post_publication_mismatch_does_not_retry_patch(self):
        self.api.override_public = {"id": 999, "draft": False, "tag_name": recover.TAG, "published_at": "now"}
        with self.assertRaisesRegex(recover.RecoveryError, "Post-publication"):
            self.recover()
        self.assertEqual(len(self.api.mutations), 1)


class TransportTests(unittest.TestCase):
    def setUp(self):
        self.api = recover.API("Eetherrr/TwinTail", "ghp_SYNTHETIC_SECRET")

    def test_asset_redirect_strips_authorization_and_accept(self):
        self.api.opener = Mock()
        self.api.opener.open.side_effect = [urllib.error.HTTPError("https://api.github.com/asset", 302, "Found", {"Location": "https://release-assets.githubusercontent.com/blob?sig=secret"}, None), io.BytesIO(b"asset")]
        self.assertEqual(self.api.binary("/releases/assets/1", 10), b"asset")
        first, second = [call.args[0] for call in self.api.opener.open.call_args_list]
        self.assertEqual(first.get_header("Authorization"), "Bearer ghp_SYNTHETIC_SECRET")
        self.assertEqual(first.get_header("Accept"), "application/octet-stream")
        self.assertIsNone(second.get_header("Authorization"))
        self.assertIsNone(second.get_header("Accept"))
        self.assertEqual(first.full_url, self.api.base + "/releases/assets/1")

    def test_job_logs_request_json_media_then_follow_without_auth(self):
        self.api.opener = Mock()
        self.api.opener.open.side_effect = [
            urllib.error.HTTPError("original", 302, "Found", {"Location": "https://logs.example/log?sig=secret"}, None),
            io.BytesIO(b"original failure")]
        self.assertEqual(self.api.binary("/actions/jobs/129031731232/logs", 100), b"original failure")
        first, second = [call.args[0] for call in self.api.opener.open.call_args_list]
        self.assertEqual(first.get_header("Accept"), "application/vnd.github+json")
        self.assertEqual(first.get_header("Authorization"), "Bearer ghp_SYNTHETIC_SECRET")
        self.assertIsNone(second.get_header("Authorization"))
        self.assertIsNone(second.get_header("Accept"))

    def test_https_and_no_url_credentials_required_on_redirect(self):
        for url in ("http://example.com/asset", "https://user:password@example.com/asset", "file:///etc/passwd"):
            with self.subTest(url=url):
                self.api.opener = Mock()
                self.api.opener.open.side_effect = urllib.error.HTTPError("original", 302, "Found", {"Location": url}, None)
                with self.assertRaisesRegex(recover.RecoveryError, "HTTPS"):
                    self.api.binary("/releases/assets/1", 10)
                self.assertEqual(self.api.opener.open.call_count, 1)

    def test_api_binary_size_bounds_short_and_oversized(self):
        with tempfile.TemporaryDirectory() as temp:
            for data in (b"ab", b"abcdef"):
                with self.subTest(data=data):
                    self.api.opener = Mock()
                    self.api.opener.open.return_value = io.BytesIO(data)
                    path = Path(temp) / ("asset" + str(len(data)))
                    with self.assertRaisesRegex(recover.RecoveryError, "size"):
                        self.api.binary("/releases/assets/1", 4, path)

    def test_multiple_redirects_never_reintroduce_authorization(self):
        self.api.opener = Mock()
        self.api.opener.open.side_effect = [
            urllib.error.HTTPError("original", 302, "Found", {"Location": "https://assets.example/first"}, None),
            urllib.error.HTTPError("first", 307, "Moved", {"Location": "/second?sig=secret"}, None),
            io.BytesIO(b"asset")]
        self.assertEqual(self.api.binary("/releases/assets/1", 10), b"asset")
        requests = [call.args[0] for call in self.api.opener.open.call_args_list]
        self.assertEqual(requests[-1].full_url, "https://assets.example/second?sig=secret")
        self.assertTrue(all(request.get_header("Authorization") is None for request in requests[1:]))

    def test_patch_is_single_authenticated_json_request(self):
        self.api.opener = Mock()
        self.api.opener.open.return_value = io.BytesIO(b'{"id":101,"draft":false}')
        result = self.api.patch("/releases/101", {"draft": False, "make_latest": "true"})
        self.assertFalse(result["draft"])
        self.assertEqual(self.api.opener.open.call_count, 1)
        request = self.api.opener.open.call_args.args[0]
        self.assertEqual(request.method, "PATCH")
        self.assertEqual(request.get_header("Authorization"), "Bearer ghp_SYNTHETIC_SECRET")
        self.assertEqual(json.loads(request.data), {"draft": False, "make_latest": "true"})

    def test_json_error_descriptive_without_response_body_or_secret(self):
        self.api.opener = Mock()
        self.api.opener.open.side_effect = urllib.error.HTTPError("secret-url", 403, "secret", {}, io.BytesIO(b"secret"))
        with self.assertRaisesRegex(recover.RecoveryError, "GET /releases returned HTTP 403"):
            self.api.get("/releases")

    def test_bounded_pagination_including_drafts(self):
        self.api.get = Mock(side_effect=[[{"id": i} for i in range(100)], [{"id": 101, "draft": True}]])
        records = self.api.pages("/releases")
        self.assertEqual(len(records), 101)
        self.assertTrue(records[-1]["draft"])
        self.assertEqual(self.api.get.call_args_list[1].args[0], "/releases?per_page=100&page=2")
        self.api.get = Mock(return_value=[{}] * 100)
        with self.assertRaisesRegex(recover.RecoveryError, "pagination"):
            self.api.pages("/releases")
        self.assertEqual(self.api.get.call_count, recover.MAX_PAGES)

    def test_gh_download_uses_original_run_only_and_captures_output(self):
        completed = Mock(returncode=0, stderr="")
        with patch.object(recover.subprocess, "run", return_value=completed) as run:
            recover.download_bundle(self.api, recover.ORIGINAL_RUN, Path("/tmp/review"))
        self.assertEqual(run.call_args.args[0], ["gh", "run", "download", recover.ORIGINAL_RUN, "--repo", self.api.repo, "-n", "release-review-bundle", "-D", "/tmp/review"])
        self.assertTrue(run.call_args.kwargs["capture_output"])
        self.assertEqual(run.call_args.kwargs["timeout"], 300)

    def test_main_all_errors_annotated_and_redacted(self):
        with patch.dict(os.environ, {"GH_TOKEN": "ghp_SYNTHETIC_SECRET", "GH_REPO": self.api.repo}), patch.object(recover, "recover", side_effect=ValueError("invalid ghp_SYNTHETIC_SECRET\nsecond%line")):
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(recover.main(["--tag", recover.TAG]), 1)
        self.assertIn("::error::Release recovery stopped (ValueError)", output.getvalue())
        self.assertNotIn("ghp_SYNTHETIC_SECRET", output.getvalue())
        self.assertIn("%0Asecond%25line", output.getvalue())


if __name__ == "__main__":
    unittest.main()
