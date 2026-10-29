#!/usr/bin/env python3
"""Deterministic local release candidates; Python 3.11+, standard library only."""
from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile
import zlib

TARGETS = {
    "linux-x86_64": ("x86_64-unknown-linux-gnu", "", 62),
    "linux-aarch64": ("aarch64-unknown-linux-gnu", "", 183),
    "windows-x86_64": ("x86_64-pc-windows-msvc", ".exe", 0x8664),
}
PLUGINS = {
    "adapter-snowluma": "adapters/adapter-snowluma",
    "web-admin": "plugins/web-admin",
    "echo-plugin": "examples/echo-plugin",
    "mock-adapter": "examples/mock-adapter",
}
API_DOCS = "contract-tools README manifest errors events configuration storage quickstart actions web-admin migration hot-reload lifecycle adapters protocol permissions troubleshooting services".split()
ARCH_DOCS = "README 0001-p0-baseline 0002-p1-plugin-host 0003-p2-sdk-routing 0004-p3-management-config-reload 0005-p4-onebot-snowluma 0006-p5-web-admin 0007-p6-release-hardening security".split()
SCHEMAS = "protocol initialize plugin-manifest config-prepare action-call service-call hello service-registration config-commit bot-ref delivery-report register-bot management-call host-config config-snapshot event-envelope subscribe publish-event".split()
DOCUMENTS = ["README.md", "AGENT.md", "SKILL.md", "CHANGELOG.md", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "web/package.json", "web/package-lock.json", "plugins/web-admin/openapi.json"] + [f"docs/plugin-api/{name}.md" for name in API_DOCS] + [f"docs/architecture/{name}.md" for name in ARCH_DOCS] + ["docs/compatibility/README.md", "docs/compatibility/snowluma.md"] + [f"schemas/{name}-v1.json" for name in SCHEMAS]
MAX_FILE = 128 * 1024 * 1024
MAX_TOTAL = 512 * 1024 * 1024
MAX_FILES = 256
RELEASE_DOCS = "README operations security-updates platforms plugins dependency-audit".split()
DOCUMENTS += [f"docs/release/{name}.md" for name in RELEASE_DOCS]
DOCUMENTS += ["LICENSE", "THIRD_PARTY_NOTICES.md", "third-party-licenses/rust-dependencies.txt", "third-party-licenses/web-dependencies.txt", "SECURITY.md", "tools/state/state.py", "tools/state/README.md", "tools/release/release.py", "tools/release/README.md"]
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?\Z")


class ReleaseError(Exception):
    """Safe, path-independent packaging error."""


def require(condition, message):
    if not condition:
        raise ReleaseError(message)


def safe_path(path):
    """Reject links, junctions and parent components, including existing ancestors."""
    path = Path(path)
    require(".." not in path.parts, "parent components are forbidden")
    path = path.absolute()
    for ancestor in [path, *path.parents]:
        if ancestor.exists() or ancestor.is_symlink():
            require(not ancestor.is_symlink() and not getattr(ancestor, "is_junction", lambda: False)() and not getattr(ancestor.lstat(), "st_file_attributes", 0) & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400), "links and junctions are forbidden")
    return path


def relative(name):
    require(isinstance(name, str) and name and "\\" not in name and ":" not in name and "\x00" not in name, "invalid archive path")
    path = PurePosixPath(name)
    require(not path.is_absolute() and all(part not in ("", ".", "..") for part in name.split("/")), "unsafe archive path")
    return name


def read(path, limit=MAX_FILE):
    path = safe_path(path)
    require(stat.S_ISREG(path.stat().st_mode), "only regular input files are allowed")
    require(path.stat().st_size <= limit, "input file exceeds limit")
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    require(len(data) <= limit, "input file exceeds limit")
    return data


def sha(data):
    return hashlib.sha256(data).hexdigest()


def json_bytes(value):
    return (json.dumps(value, ensure_ascii=True, sort_keys=True, indent=2) + "\n").encode()


def binary_check(data, target):
    machine = TARGETS[target][2]
    if target.startswith("linux-"):
        require(len(data) >= 64 and data[:6] == b"\x7fELF\x02\x01" and struct.unpack_from("<H", data, 18)[0] == machine, "binary does not match requested ELF architecture")
    else:
        require(len(data) >= 64 and data[:2] == b"MZ", "binary is not Windows PE")
        offset = struct.unpack_from("<I", data, 60)[0]
        require(offset + 6 <= len(data) and data[offset:offset+4] == b"PE\0\0" and struct.unpack_from("<H", data, offset + 4)[0] == machine, "binary does not match requested PE architecture")


def executable(name, target):
    suffix = TARGETS[target][1]
    return name == f"bin/twintail-host{suffix}" or any(name.endswith(f"/{plugin}{suffix}") for plugin in PLUGINS)


def mode(name, target):
    return 0o755 if executable(name, target) else 0o644


def base_files(version, target):
    files = set(DOCUMENTS)
    suffix = TARGETS[target][1]
    files.add(f"bin/twintail-host{suffix}")
    for plugin in PLUGINS:
        prefix = f"packages/{plugin}/{version}"
        files.update(f"{prefix}/{name}" for name in [plugin + suffix, "plugin.toml", "checksums.json"])
        if plugin in ("adapter-snowluma", "web-admin"):
            files.add(f"{prefix}/config.schema.json")
    return files


def manifest_bytes(data, plugin, version, target):
    text = data.decode("utf-8")
    parsed = tomllib.loads(text)
    require(parsed.get("id") == plugin and parsed.get("version") == version and parsed.get("entrypoint") == plugin, "manifest identity/version/entrypoint differs from release")
    require(parsed.get("config_schema") in (None, "config.schema.json"), "unexpected config schema path")
    # Rewrite only top-level fields before any table; preserve original human docs.
    first_table = re.search(r"(?m)^\s*\[", text)
    head, tail = (text[:first_table.start()], text[first_table.start():]) if first_table else (text, "")
    head = re.sub(r'(?m)^entrypoint\s*=.*$', f'entrypoint = "{plugin}{TARGETS[target][1]}"', head)
    head = re.sub(r'(?m)^target\s*=.*\n?', '', head)
    return (head.rstrip() + f'\ntarget = "{target}"\n\n' + tail).encode()


def stage(repo, binaries, target, output):
    repo, binaries, output = map(safe_path, (repo, binaries, output))
    require(not output.exists(), "stage output already exists")
    workspace = tomllib.loads(read(repo / "Cargo.toml", 1024 * 1024).decode())
    require(workspace["workspace"]["package"].get("license") == "MIT", "workspace license must match MIT authorization")
    version = workspace["workspace"]["package"]["version"]
    require(isinstance(version, str) and VERSION.fullmatch(version), "unsupported release version")
    files = {name: read(repo / name, 8 * 1024 * 1024) for name in DOCUMENTS}
    require(b"MIT License" in files["LICENSE"] and b"Permission is hereby granted, free of charge" in files["LICENSE"], "MIT license text missing")
    require(all(files[name].strip() for name in ("THIRD_PARTY_NOTICES.md", "third-party-licenses/rust-dependencies.txt", "third-party-licenses/web-dependencies.txt")), "third-party license documents missing")
    suffix = TARGETS[target][1]
    for binary in ["twintail-host", *PLUGINS]:
        data = read(binaries / (binary + suffix))
        binary_check(data, target)
        destination = f"bin/{binary}{suffix}" if binary == "twintail-host" else f"packages/{binary}/{version}/{binary}{suffix}"
        files[destination] = data
    for plugin, source in PLUGINS.items():
        prefix = f"packages/{plugin}/{version}"
        files[f"{prefix}/plugin.toml"] = manifest_bytes(read(repo / source / "plugin.toml", 65536), plugin, version, target)
        if plugin in ("adapter-snowluma", "web-admin"):
            schema = read(repo / source / "config.schema.json", 1024 * 1024)
            json.loads(schema)
            files[f"{prefix}/config.schema.json"] = schema
        receipt = {name.removeprefix(prefix + "/"): sha(data) for name, data in files.items() if name.startswith(prefix + "/")}
        files[f"{prefix}/checksums.json"] = json_bytes(receipt)
    files["release.json"] = json_bytes({"format": 1, "channel": "candidate", "public_release": False, "license_gate": "MIT", "license": "MIT", "version": version, "target": target, "rust_target": TARGETS[target][0], "rust_version": workspace["workspace"]["package"]["rust-version"], "profile": "release", "runtime_verified": False, "lock_sha256": {name: sha(files[name]) for name in ("Cargo.lock", "web/package-lock.json")}})
    files["SHA256SUMS"] = "".join(f"{sha(data)}  {name}\n" for name, data in sorted(files.items())).encode()
    validate(files)
    require(output.parent.is_dir(), "stage output parent must exist")
    # Private same-parent staging, followed by rename; never overwrite an output.
    with tempfile.TemporaryDirectory(prefix=".twintail-stage-", dir=output.parent) as temporary:
        temporary = Path(temporary)
        for name, data in sorted(files.items()):
            path = temporary / name
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("xb") as stream:
                stream.write(data)
            path.chmod(mode(name, target))
            os.utime(path, (0, 0))
        require(not output.exists(), "stage output appeared during staging")
        # Python stdlib rename cannot guard hostile same-user races; private output
        # parents must be protected by the invoking administrator (see README).
        temporary.rename(output)
    return {"stage": str(output), "version": version, "target": target, "files": len(files)}


def directory_files(path):
    path = safe_path(path)
    require(path.is_dir(), "stage must be a directory")
    files = {}
    found_directories = set()
    entries = 0
    for parent, directories, names in os.walk(path, followlinks=False):
        for name in [*directories, *names]:
            entries += 1
            require(entries <= MAX_FILES * 2, "too many staged entries")
            item = safe_path(Path(parent) / name)
            require(item.is_dir() or item.is_file(), "special staged file forbidden")
            if item.is_dir():
                found_directories.add(item.relative_to(path).as_posix())
        for name in names:
            item = Path(parent) / name
            key = relative(item.relative_to(path).as_posix())
            files[key] = read(item)
            require(sum(map(len, files.values())) <= MAX_TOTAL, "stage exceeds total limit")
            require(len(files) <= MAX_FILES, "too many staged files")
    expected_directories = {str(parent) for name in files for parent in PurePosixPath(name).parents if str(parent) != "."}
    require(found_directories == expected_directories, "unexpected empty staged directory")
    if os.name != "nt":
        metadata = json.loads(files.get("release.json", b"{}"))
        target = metadata.get("target")
        require(target in TARGETS, "release target missing")
        for name in files:
            require(stat.S_IMODE((path / name).stat().st_mode) == mode(name, target), "nonnormalized staged permissions")
    return files


def validate(files):
    require(len(files) <= MAX_FILES and sum(map(len, files.values())) <= MAX_TOTAL, "release exceeds bounds")
    for name, data in files.items():
        relative(name)
        require(len(data) <= MAX_FILE, "release file exceeds bounds")
    require("release.json" in files and "SHA256SUMS" in files, "release metadata missing")
    metadata = json.loads(files["release.json"])
    require(set(metadata) == {"format", "channel", "public_release", "license_gate", "license", "version", "target", "rust_target", "rust_version", "profile", "runtime_verified", "lock_sha256"}, "unexpected release metadata")
    require(metadata["channel"] == "candidate" and metadata["public_release"] is False and metadata["license_gate"] == "MIT" and metadata["license"] == "MIT", "invalid candidate gate")
    require(tomllib.loads(files["Cargo.toml"].decode())["workspace"]["package"].get("license") == "MIT", "workspace license mismatch")
    require(b"MIT License" in files["LICENSE"] and b"Permission is hereby granted, free of charge" in files["LICENSE"], "MIT license text missing")
    require(all(files[name].strip() for name in ("THIRD_PARTY_NOTICES.md", "third-party-licenses/rust-dependencies.txt", "third-party-licenses/web-dependencies.txt")), "third-party license documents missing")
    version, target = metadata["version"], metadata["target"]
    require(isinstance(version, str) and VERSION.fullmatch(version) and target in TARGETS, "invalid release identity")
    require(metadata["format"] == 1 and metadata["profile"] == "release" and metadata["runtime_verified"] is False and metadata["rust_target"] == TARGETS[target][0], "invalid release platform metadata")
    expected = base_files(version, target) | {"release.json", "SHA256SUMS"}
    require(set(files) == expected, "release has missing or unexpected files")
    expected_sums = "".join(f"{sha(data)}  {name}\n" for name, data in sorted(files.items()) if name != "SHA256SUMS").encode()
    require(files["SHA256SUMS"] == expected_sums, "release checksum mismatch")
    cargo = tomllib.loads(files["Cargo.toml"].decode())["workspace"]["package"]
    require(cargo["version"] == version and cargo["rust-version"] == metadata["rust_version"], "version metadata mismatch")
    require(metadata["lock_sha256"] == {name: sha(files[name]) for name in ("Cargo.lock", "web/package-lock.json")}, "lock metadata mismatch")
    for plugin in PLUGINS:
        prefix = f"packages/{plugin}/{version}"
        manifest = tomllib.loads(files[f"{prefix}/plugin.toml"].decode())
        require(manifest.get("id") == plugin and manifest.get("version") == version and manifest.get("target") == target and manifest.get("entrypoint") == plugin + TARGETS[target][1], "packaged manifest mismatch")
        receipt = {name.removeprefix(prefix + "/"): sha(data) for name, data in files.items() if name.startswith(prefix + "/") and not name.endswith("/checksums.json")}
        require(json.loads(files[f"{prefix}/checksums.json"]) == receipt, "plugin receipt mismatch")
        binary_check(files[f"{prefix}/{plugin}{TARGETS[target][1]}"], target)
    binary_check(files[f"bin/twintail-host{TARGETS[target][1]}"], target)
    return metadata


def archive(stage_path, output):
    files = directory_files(stage_path)
    metadata = validate(files)
    target = metadata["target"]
    output = safe_path(output)
    require(not output.exists() and output.parent.is_dir(), "archive output must be new with existing parent")
    require(not output.is_relative_to(safe_path(stage_path)), "archive output must be outside stage")
    prefix = f"twintail-{metadata['version']}-{target}"
    # A failed write is removed, never leaving an apparent candidate behind.
    created = False
    try:
        with output.open("xb") as stream:
            created = True
            if target.startswith("windows-"):
                with zipfile.ZipFile(stream, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive_file:
                    for name, data in sorted(files.items()):
                        info = zipfile.ZipInfo(prefix + "/" + name, (1980, 1, 1, 0, 0, 0))
                        info.create_system = 3
                        info.external_attr = (stat.S_IFREG | mode(name, target)) << 16
                        info.compress_type = zipfile.ZIP_DEFLATED
                        archive_file.writestr(info, data, compresslevel=9)
            else:
                with gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=0, compresslevel=9) as compressed:
                    with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive_file:
                        for name, data in sorted(files.items()):
                            info = tarfile.TarInfo(prefix + "/" + name)
                            info.size = len(data)
                            info.mode = mode(name, target)
                            info.mtime = 0
                            info.uid = info.gid = 0
                            info.uname = info.gname = ""
                            archive_file.addfile(info, io.BytesIO(data))
    except BaseException:
        if created:
            output.unlink(missing_ok=True)
        raise
    return {"archive": str(output), "sha256": sha(read(output, MAX_TOTAL)), "target": target, "version": metadata["version"]}


def archive_files(path):
    path = safe_path(path)
    require(stat.S_ISREG(path.stat().st_mode), "archive must be a regular file")
    require(path.stat().st_size <= MAX_TOTAL, "archive exceeds limit")
    files, prefix, permissions, member_order = {}, None, {}, []
    def add(name, size, stream):
        nonlocal prefix
        relative(name)
        require("/" in name, "archive root prefix missing")
        root, name = name.split("/", 1)
        prefix = root if prefix is None else prefix
        require(prefix == root and name not in files and size <= MAX_FILE and len(files) < MAX_FILES, "invalid or duplicate archive member")
        require(sum(map(len, files.values())) + size <= MAX_TOTAL, "archive expands beyond limit")
        data = stream.read(size + 1)
        require(len(data) == size, "invalid archive member size")
        files[name] = data
        member_order.append(name)
    if zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as packed:
            require(not packed.comment, "nonnormalized ZIP comment")
            for member in packed.infolist():
                permissions[member.filename.split("/", 1)[-1]] = stat.S_IMODE(member.external_attr >> 16)
                require(not member.extra and not member.comment and member.compress_type == zipfile.ZIP_DEFLATED and member.create_system == 3 and not member.is_dir() and stat.S_IFMT(member.external_attr >> 16) == stat.S_IFREG and not member.flag_bits & 1 and member.date_time == (1980, 1, 1, 0, 0, 0), "unsafe or nonnormalized ZIP member")
                with packed.open(member) as stream:
                    add(member.filename, member.file_size, stream)
    else:
        with path.open("rb") as stream:
            require(stream.read(10) == b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x02\xff", "nonnormalized gzip header")
        with tarfile.open(path, "r:gz") as packed:
            for member in packed:
                permissions[member.name.split("/", 1)[-1]] = member.mode
                require(member.isfile() and not member.pax_headers and member.uid == member.gid == member.mtime == 0 and member.uname == member.gname == "", "unsafe or nonnormalized TAR member")
                with packed.extractfile(member) as stream:
                    add(member.name, member.size, stream)
    metadata = validate(files)
    require(member_order == sorted(member_order), "archive members not sorted")
    require(all(permissions[name] == mode(name, metadata["target"]) for name in files), "nonnormalized archive permissions")
    require(zipfile.is_zipfile(path) == metadata["target"].startswith("windows-"), "archive format differs from target")
    require(prefix == f"twintail-{metadata['version']}-{metadata['target']}", "archive identity prefix mismatch")
    return files


def verify(path):
    path = safe_path(path)
    metadata = validate(directory_files(path) if path.is_dir() else archive_files(path))
    return {"verified": True, "target": metadata["target"], "version": metadata["version"], "runtime_verified": False}


def build(repo, target, target_dir):
    repo, target_dir = safe_path(repo), safe_path(target_dir)
    command = ["cargo", "build", "--locked", "--release", "--target", TARGETS[target][0], "--target-dir", str(target_dir)]
    for package in ["twintail-host", *PLUGINS]:
        command += ["-p", package]
    subprocess.run(command, cwd=repo, check=True)
    return {"binaries": str(target_dir / TARGETS[target][0] / "release"), "target": target, "rust_target": TARGETS[target][0], "runtime_verified": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    commands = parser.add_subparsers(dest="command", required=True)
    build_parser = commands.add_parser("build")
    build_parser.add_argument("--target", choices=TARGETS, required=True)
    build_parser.add_argument("--target-dir", type=Path, default=Path("target"))
    stage_parser = commands.add_parser("stage")
    stage_parser.add_argument("--target", choices=TARGETS, required=True)
    stage_parser.add_argument("--binaries", type=Path, required=True)
    stage_parser.add_argument("--output", type=Path, required=True)
    archive_parser = commands.add_parser("archive")
    archive_parser.add_argument("--stage", type=Path, required=True)
    archive_parser.add_argument("--output", type=Path, required=True)
    verify_parser = commands.add_parser("verify")
    verify_parser.add_argument("path", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "build": result = build(args.repo, args.target, args.target_dir)
        elif args.command == "stage": result = stage(args.repo, args.binaries, args.target, args.output)
        elif args.command == "archive": result = archive(args.stage, args.output)
        else: result = verify(args.path)
        print(json.dumps(result, sort_keys=True))
        return 0
    except (ReleaseError, OSError, ValueError, KeyError, TypeError, tarfile.TarError, zipfile.BadZipFile, subprocess.CalledProcessError, EOFError, RuntimeError, struct.error, zlib.error):
        print("release operation failed: invalid, unsafe, inconsistent or unavailable input", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
