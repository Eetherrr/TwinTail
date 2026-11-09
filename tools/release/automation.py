#!/usr/bin/env python3
"""Offline release assembly, Python 3.11+ and standard library only.

Contract: artifacts contains exactly one directory per release.TARGETS key. Each
contains only twintail-VERSION-TARGET.tar.gz (Linux) or .zip (Windows) and
validation.json. The latter is the unmodified JSON result of release.py verify:
{"verified": true, "target": TARGET, "version": VERSION,
 "runtime_verified": false}. It is packaging evidence, not runtime certification.

collect emits three archives, TARGET-validation.json reports, SHA256SUMS (all six
inputs plus release-notes.md), and release-notes.md into a new directory. --commit
is a caller-supplied full Git SHA, not independently authenticated provenance.
No network, Git commands, environment secrets, or credential stores are accessed.
Inputs/output parents must remain protected from concurrent same-user mutation,
as with release.py. A failed assembly removes only its own newly-created output.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import shutil
import stat
import struct
import sys
import tarfile
import tomllib
import zipfile
import zlib

import release

# SemVer 2.0.0: numeric identifiers have no leading zeroes; build identifiers may.
_NUMBER = r"(?:0|[1-9][0-9]*)"
_PRE = rf"(?:{_NUMBER}|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)"
TAG = re.compile(rf"v({_NUMBER}\.{_NUMBER}\.{_NUMBER}(?:-{_PRE}(?:\.{_PRE})*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?)\Z", re.ASCII)
COMMIT = re.compile(r"[0-9a-f]{40}\Z", re.ASCII)
REPORT_LIMIT = 16 * 1024


def tag_version(tag):
    """Return the strict, path-safe SemVer portion of a v-prefixed tag."""
    release.require(isinstance(tag, str) and len(tag) <= 256, "invalid release tag")
    match = TAG.fullmatch(tag)
    release.require(match is not None, "invalid release tag")
    version = match.group(1)
    release.require(release.VERSION.fullmatch(version) is not None, "release packager does not support build metadata")
    return version


def regular(path, limit):
    """Reject symbolic links, junctions, special files and hard-linked inputs."""
    path = release.safe_path(path)
    info = path.stat()
    release.require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1, "input must be a singly-linked regular file")
    return release.read(path, limit)


def validate(repo, tag):
    """Match a strict tag against workspace and all four plugin manifests."""
    version = tag_version(tag)
    repo = release.safe_path(repo)
    workspace = tomllib.loads(regular(repo / "Cargo.toml", 1024 * 1024).decode("utf-8"))
    release.require(workspace["workspace"]["package"]["version"] == version, "tag differs from workspace version")
    for plugin, source in release.PLUGINS.items():
        manifest = tomllib.loads(regular(repo / source / "plugin.toml", 65536).decode("utf-8"))
        release.require(manifest.get("id") == plugin and manifest.get("version") == version, "tag differs from plugin identity/version")
    return {"validated": True, "tag": tag, "version": version, "plugins": sorted(release.PLUGINS)}


def archive_name(version, target):
    """Return the single permitted archive filename for a target."""
    extension = "zip" if target.startswith("windows-") else "tar.gz"
    return f"twintail-{version}-{target}.{extension}"


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        release.require(key not in result, "duplicate validation report key")
        result[key] = value
    return result


def validation_report(data, version, target):
    """Validate the exact release.verify report, preserving candidate semantics."""
    report = json.loads(data, object_pairs_hook=unique_object)
    release.require(isinstance(report, dict) and set(report) == {"verified", "target", "version", "runtime_verified"}, "unexpected validation report fields")
    release.require(report["verified"] is True and report["runtime_verified"] is False and report["version"] == version and report["target"] == target, "validation report mismatch")
    return report


def directory(path):
    path = release.safe_path(path)
    release.require(path.is_dir(), "artifact directory missing")
    return path


def collect(repo, artifacts, output, tag, commit):
    """Verify exactly three platform archives and assemble without overwriting."""
    version = validate(repo, tag)["version"]
    release.require(isinstance(commit, str) and COMMIT.fullmatch(commit), "source commit must be a full lowercase Git SHA")
    artifacts, output = directory(artifacts), release.safe_path(output)
    release.require(not output.exists(), "collection output already exists")
    release.require(output.parent.is_dir(), "collection output parent must exist")
    release.require(not output.is_relative_to(artifacts), "output must be outside artifacts")
    release.require({path.name for path in artifacts.iterdir()} == set(release.TARGETS), "expected exactly three platform artifact directories")
    inputs = []
    for target in release.TARGETS:
        source = directory(artifacts / target)
        name = archive_name(version, target)
        release.require({path.name for path in source.iterdir()} == {name, "validation.json"}, "unexpected or missing platform artifact files")
        report = regular(source / "validation.json", REPORT_LIMIT)
        validation_report(report, version, target)
        # Preflight metadata before creating any output; read/verify the copied
        # bytes again below so checksums and verification refer to published bytes.
        archive = release.safe_path(source / name)
        info = archive.stat()
        release.require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and info.st_size <= release.MAX_TOTAL, "invalid archive input")
        inputs.append((target, archive, report))

    # mkdir is exclusive even for an empty existing directory (rename is not).
    output.mkdir(mode=0o700)
    try:
        hashes = {}
        for target, archive, report in inputs:
            data = regular(archive, release.MAX_TOTAL)
            destination = output / archive.name
            with destination.open("xb") as stream:
                stream.write(data)
            hashes[archive.name] = release.sha(data)
            del data
            verified = release.verify(destination)
            release.require(verified == validation_report(report, version, target), "archive differs from validation report")
            report_name = f"{target}-validation.json"
            with (output / report_name).open("xb") as stream:
                stream.write(report)
            hashes[report_name] = release.sha(report)
        notes = (
            f"# TwinTail {tag}\n\n"
            f"Source commit (caller-supplied SHA): `{commit}`\n\n"
            f"变更与升级说明：[CHANGELOG](https://github.com/Eetherrr/TwinTail/blob/{commit}/CHANGELOG.md) · "
            "[入门文档](https://eetherrr.github.io/TwinTail/plugin-api/quickstart/)\n\n"
            "下载对应平台的归档，先用 SHA256SUMS 校验。升级前停止旧宿主及配置写入进程并备份数据；"
            "不原地覆盖旧插件版本。此版本未扩大真实 SnowLuma 验证范围。\n\n"
            "## Target scope\n\n"
            + "".join(f"- `{target}` / `{release.TARGETS[target][0]}`\n" for target in release.TARGETS)
            + "\n## Candidate and validation scope\n\n"
            "These archives retain channel=candidate, public_release=false and "
            "runtime_verified=false. Archive structure, manifest identities and "
            "integrity were checked offline with the existing release verifier. "
            "Native workflow tests/builds are separate evidence; these reports "
            "do not certify runtime compatibility, real SnowLuma integration, "
            "publisher identity, or source-to-binary provenance.\n\n"
            "SHA256SUMS covers the three archives, three validation reports and "
            "this release note; SHA256 detects corruption, not authenticity.\n"
        ).encode("utf-8")
        with (output / "release-notes.md").open("xb") as stream:
            stream.write(notes)
        hashes["release-notes.md"] = release.sha(notes)
        with (output / "SHA256SUMS").open("xb") as stream:
            stream.write("".join(f"{digest}  {name}\n" for name, digest in sorted(hashes.items())).encode("ascii"))
        return {"collected": True, "tag": tag, "version": version, "commit": commit, "targets": list(release.TARGETS), "runtime_verified": False}
    except BaseException:
        shutil.rmtree(output)
        raise


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    commands = parser.add_subparsers(dest="command", required=True)
    check = commands.add_parser("validate")
    check.add_argument("--tag", required=True)
    assembly = commands.add_parser("collect")
    assembly.add_argument("--artifacts", type=Path, required=True)
    assembly.add_argument("--output", type=Path, required=True)
    assembly.add_argument("--tag", required=True)
    assembly.add_argument("--commit", required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "validate":
            result = validate(args.repo, args.tag)
        else:
            result = collect(args.repo, args.artifacts, args.output, args.tag, args.commit)
        print(json.dumps(result, sort_keys=True))
        return 0
    except (release.ReleaseError, OSError, ValueError, KeyError, TypeError, tarfile.TarError, zipfile.BadZipFile, EOFError, RuntimeError, struct.error, zlib.error):
        print("release automation failed: invalid, unsafe, inconsistent or unavailable input", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())