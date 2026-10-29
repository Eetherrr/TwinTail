#!/usr/bin/env python3
"""Bounded, offline TwinTail state copies. Python 3.11+, Unix only.

No environment lookup, plugin execution, schema conversion, or live overwrite.
Integrity hashes detect corruption; they do not authenticate an untrusted backup.
"""
from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import math
import os
from pathlib import Path
import socket
import stat
import sys
import tomllib

MAX_FILES = 10000
MAX_FILE = 64 * 1024 * 1024
MAX_TOTAL = 1024 * 1024 * 1024
MAX_MANIFEST = 4 * 1024 * 1024
MAX_DEPTH = 32
MAX_CONFIG = 2 * 1024 * 1024
RUNTIME = {"management.sock", "run.lock", "host.lock", "host.run.lock"}


class StateError(Exception):
    """Safe fixed-reason failure; never contains configuration or OS messages."""


def fail(reason):
    raise StateError(reason)


def platform():
    if os.name != "posix" or not hasattr(socket, "AF_UNIX"):
        fail("supported only on Unix; Windows named-pipe stopping is not verified")


def checked_path(value, exists=True):
    raw = Path(value).absolute()
    if ".." in raw.parts:
        fail("parent traversal is forbidden")
    for path in [*reversed(raw.parents), raw]:
        try:
            metadata = path.lstat()
        except FileNotFoundError:
            if exists or path != raw:
                fail("path or parent does not exist")
            continue
        if stat.S_ISLNK(metadata.st_mode):
            fail("symbolic links are forbidden")
    if exists and not raw.is_dir():
        fail("root must be a directory")
    return raw


def private_root(root):
    metadata = root.lstat()
    if metadata.st_uid != os.getuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
        fail("state and backup roots require current-user ownership and mode 0700")


def identity(metadata):
    return (metadata.st_dev, metadata.st_ino, metadata.st_size,
            metadata.st_mtime_ns, metadata.st_ctime_ns, metadata.st_mode)


def read_file(path, limit=MAX_FILE):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1:
            fail("only regular single-link files are accepted")
        if before.st_uid != os.getuid() or before.st_size > limit:
            fail("file ownership or size exceeds policy")
        data = stream.read(limit + 1)
        if len(data) > limit or identity(before) != identity(os.fstat(stream.fileno())):
            fail("file changed during bounded read")
        if identity(before) != identity(path.lstat()):
            fail("file path changed during read")
        return data


def write_file(path, data, on_created=lambda: None):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        on_created()
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def sync_dir(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


@contextlib.contextmanager
def stopped(root):
    """Reserve the host's actual Unix startup endpoint, never infer stale death."""
    platform()
    private_root(root)
    for name in RUNTIME:
        if os.path.lexists(root / name):
            fail("runtime endpoint or run lock exists; stop host and investigate stale state manually")
    guard = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    marker = root / "management.sock"
    inode = None
    try:
        # Host binds this same name before spawning any plugin. No listening is
        # necessary: the reserved pathname makes competing startup fail closed.
        guard.bind(str(marker))
        inode = marker.lstat().st_ino
        marker.chmod(0o600)
        yield
    finally:
        guard.close()
        if inode is not None and os.path.lexists(marker) and marker.lstat().st_ino == inode:
            marker.unlink()


def is_config_lock(relative):
    parts = Path(relative).parts
    return len(parts) == 3 and parts[0] == "plugins" and parts[2] == "config.lock"


def inventory(root, skip_runtime=False):
    result = []
    total = 0
    def walk(directory, depth):
        nonlocal total
        if depth > MAX_DEPTH:
            fail("directory depth exceeds policy")
        # Bounded enumeration rather than sorting an unbounded directory first.
        with os.scandir(directory) as iterator:
            for entry in iterator:
                relative = Path(entry.path).relative_to(root).as_posix()
                if skip_runtime and relative == "management.sock":
                    continue
                if any(character in entry.name for character in ("\\", ":")):
                    fail("nonportable state path is forbidden")
                metadata = entry.stat(follow_symlinks=False)
                if metadata.st_uid != os.getuid():
                    fail("foreign-owned state entries are forbidden")
                if len(result) >= MAX_FILES:
                    fail("entry count exceeds policy")
                if stat.S_ISDIR(metadata.st_mode):
                    result.append((relative, "directory", identity(metadata)))
                    walk(Path(entry.path), depth + 1)
                elif stat.S_ISREG(metadata.st_mode) and metadata.st_nlink == 1:
                    if Path(relative).name in RUNTIME:
                        fail("nested runtime lock or endpoint is forbidden")
                    if metadata.st_size > MAX_FILE:
                        fail("file exceeds policy")
                    total += metadata.st_size
                    if total > MAX_TOTAL:
                        fail("total state size exceeds policy")
                    result.append((relative, "file", identity(metadata)))
                else:
                    fail("links, sockets and special files are forbidden")
    walk(root, 0)
    return sorted(result)


def check_config(relative, data):
    parts = Path(relative).parts
    if len(parts) != 3 or parts[0] != "plugins" or parts[2] not in {"config.toml", "config.applied.toml"}:
        return None
    if len(data) > MAX_CONFIG:
        fail("configuration envelope exceeds policy")
    try:
        value = tomllib.loads(data.decode("utf-8"))
    except (ValueError, UnicodeError, RecursionError):
        fail("invalid configuration TOML envelope")
    if parts[2] == "config.applied.toml":
        if set(value) != {"revision"} or type(value["revision"]) is not int or not 0 <= value["revision"] < 2**64:
            fail("invalid applied revision envelope")
        return (parts[1], "applied", value["revision"])
    if set(value) != {"config_version", "revision", "values"}:
        fail("invalid desired envelope fields")
    if type(value["config_version"]) is not int or not 1 <= value["config_version"] < 2**32:
        fail("unsupported configuration version envelope")
    if type(value["revision"]) is not int or not 0 <= value["revision"] < 2**64:
        fail("invalid desired revision envelope")
    nodes = 0
    def inspect(item, depth=0):
        nonlocal nodes
        nodes += 1
        if depth > 32 or nodes > 65536:
            fail("configuration structure exceeds policy")
        if isinstance(item, dict):
            if "$redacted" in item:
                fail("redacted sentinel cannot be restored as configuration")
            if "$secret" in item:
                name = item.get("$secret")
                if set(item) != {"$secret"} or not isinstance(name, str) or not name or len(name) > 256 or not name.isascii() or not (name[0].isalpha() or name[0] == "_") or not all(c.isalnum() or c == "_" for c in name):
                    fail("invalid environment secret reference")
            for child in item.values():
                inspect(child, depth + 1)
        elif isinstance(item, list):
            for child in item:
                inspect(child, depth + 1)
        elif isinstance(item, str) and item in {"[REDACTED]", "<redacted>", "***", "********"}:
            fail("redacted sentinel cannot be restored as configuration")
        elif item is not None and type(item) not in {str, int, float, bool}:
            fail("configuration values must be JSON-compatible")
        elif isinstance(item, float) and not math.isfinite(item):
            fail("configuration values must be finite")
    inspect(value["values"])
    if len(json.dumps(value["values"], ensure_ascii=False, separators=(",", ":")).encode()) > 1024 * 1024:
        fail("configuration JSON values exceed policy")
    return (parts[1], "desired", value["revision"], value["config_version"])


def summarize(records):
    plugins = {}
    for record in records:
        if record is not None:
            plugins.setdefault(record[0], {})[record[1]] = record[2]
    mismatch = 0
    for revisions in plugins.values():
        if "applied" in revisions and "desired" not in revisions:
            fail("applied marker without desired configuration")
        if revisions.get("applied", 0) > revisions["desired"]:
            fail("applied revision exceeds desired")
        mismatch += revisions.get("applied") != revisions["desired"]
    return {"configurations": len(plugins), "desired_applied_mismatches": mismatch}


@contextlib.contextmanager
def config_locks(root, entries):
    import fcntl
    streams = []
    try:
        for relative, kind, _ in entries:
            if kind == "file" and is_config_lock(relative):
                descriptor = os.open(root / relative, os.O_RDONLY | os.O_NOFOLLOW)
                streams.append(descriptor)
                if not stat.S_ISREG(os.fstat(descriptor).st_mode) or os.fstat(descriptor).st_nlink != 1:
                    fail("invalid configuration lock")
                try:
                    fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    fail("configuration transaction lock is active")
        yield
    finally:
        for descriptor in streams:
            os.close(descriptor)


def target_root(value, source):
    root = checked_path(value, exists=False)
    if root == source or source in root.parents or root in source.parents:
        fail("source and target roots must be disjoint")
    if root.exists():
        private_root(root)
        with os.scandir(root) as entries:
            if next(entries, None) is not None:
                fail("target must be absent or empty; existing state is never overwritten")
    return root


@contextlib.contextmanager
def output_root(root):
    # An absent target is claimed with mkdir(exclusive), never os.replace(). An
    # existing empty target is reserved by the actual host endpoint as well.
    created = not root.exists()
    if created:
        root.mkdir(mode=0o700)
    try:
        with stopped(root):
            yield root
        sync_dir(root)
        sync_dir(root.parent)
    except BaseException:
        # Only remove our own entries; an unexpected concurrent entry is left for
        # manual investigation. Same-user hostile writers are outside this model.
        if created:
            try:
                root.rmdir()
            except OSError:
                pass
        raise


def snapshot(source, destination):
    platform()
    source = checked_path(source)
    destination = target_root(destination, source)
    with stopped(source):
        before = inventory(source, skip_runtime=True)
        with config_locks(source, before), output_root(destination):
            records, entries = [], []
            try:
                (destination / "tree").mkdir(mode=0o700)
                for relative, kind, _ in before:
                    if kind == "file" and is_config_lock(relative):
                        continue
                    target = destination / "tree" / relative
                    if kind == "directory":
                        target.mkdir(mode=0o700)
                        entries.append({"path": relative, "kind": kind})
                    else:
                        data = read_file(source / relative)
                        records.append(check_config(relative, data))
                        metadata = {"path": relative, "kind": kind, "size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
                        write_file(target, data, lambda: entries.append(metadata))
                summary = summarize(records)
                if before != inventory(source, skip_runtime=True):
                    fail("source changed during cold snapshot")
                manifest = json.dumps({"format": 1, "entries": entries}, separators=(",", ":")).encode()
                if len(manifest) > MAX_MANIFEST:
                    fail("backup manifest exceeds policy")
                write_file(destination / "manifest.json", manifest)
                for directory in reversed([destination / "tree"] + [destination / "tree" / p for p, k, _ in before if k == "directory"]):
                    sync_dir(directory)
                return {"files": sum(e["kind"] == "file" for e in entries), **summary}
            except BaseException:
                cleanup(destination, entries, backup=True)
                raise


def cleanup(root, entries, backup=False):
    tree = root / "tree" if backup else root
    for entry in reversed(entries):
        path = tree / entry["path"]
        try:
            path.rmdir() if entry["kind"] == "directory" else path.unlink()
        except FileNotFoundError:
            pass
    if backup:
        (root / "manifest.json").unlink(missing_ok=True)
        (root / "tree").rmdir()


def verified(backup):
    private_root(backup)
    manifest_bytes = read_file(backup / "manifest.json", MAX_MANIFEST)
    try:
        manifest = json.loads(manifest_bytes)
    except (ValueError, RecursionError):
        fail("invalid backup manifest")
    if not isinstance(manifest, dict) or set(manifest) != {"format", "entries"} or manifest["format"] != 1 or not isinstance(manifest["entries"], list) or len(manifest["entries"]) > MAX_FILES:
        fail("unsupported backup format")
    tree = checked_path(backup / "tree")
    private_root(tree)
    actual = inventory(tree)
    for relative, kind, metadata in actual:
        if stat.S_IMODE(metadata[-1]) != (0o700 if kind == "directory" else 0o600):
            fail("backup directories require 0700 and files require 0600")
    if stat.S_IMODE((backup / "manifest.json").lstat().st_mode) != 0o600:
        fail("backup manifest requires mode 0600")
    expected = []
    records = []
    seen = set()
    for entry in manifest["entries"]:
        if not isinstance(entry, dict) or not isinstance(entry.get("path"), str):
            fail("invalid backup entry")
        relative = entry["path"]
        parts = relative.split("/")
        if relative in seen or any(part in {"", ".", ".."} for part in parts) or "\\" in relative or any(":" in part for part in parts):
            fail("unsafe or duplicate backup path")
        seen.add(relative)
        kind = entry.get("kind")
        if kind == "directory" and set(entry) == {"path", "kind"}:
            expected.append((relative, kind))
        elif kind == "file" and set(entry) == {"path", "kind", "size", "sha256"}:
            data = read_file(tree / relative)
            if type(entry["size"]) is not int or entry["size"] != len(data) or entry["sha256"] != hashlib.sha256(data).hexdigest():
                fail("backup size or checksum mismatch")
            records.append(check_config(relative, data))
            expected.append((relative, kind))
        else:
            fail("invalid backup entry fields")
    if sorted(expected) != [(p, k) for p, k, _ in actual]:
        fail("backup inventory differs from manifest")
    with os.scandir(backup) as entries:
        if {entry.name for entry in entries} != {"manifest.json", "tree"}:
            fail("unexpected backup root entries")
    return manifest["entries"], summarize(records)


def verify(backup):
    platform()
    backup = checked_path(backup)
    entries, summary = verified(backup)
    return {"files": sum(e["kind"] == "file" for e in entries), **summary}


def restore(backup, destination):
    platform()
    backup = checked_path(backup)
    destination = target_root(destination, backup)
    entries, summary = verified(backup)
    with output_root(destination):
        made = []
        try:
            for entry in sorted(entries, key=lambda e: e["path"]):
                target = destination / entry["path"]
                if entry["kind"] == "directory":
                    target.mkdir(mode=0o700)
                    made.append(entry)
                else:
                    data = read_file(backup / "tree" / entry["path"])
                    if hashlib.sha256(data).hexdigest() != entry["sha256"]:
                        fail("backup changed during restore")
                    write_file(target, data, lambda: made.append(entry))
            for directory in reversed([destination] + [destination / e["path"] for e in made if e["kind"] == "directory"]):
                sync_dir(directory)
        except BaseException:
            cleanup(destination, made)
            raise
    return {"files": sum(e["kind"] == "file" for e in entries), **summary}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ["snapshot", "restore"]:
        sub = commands.add_parser(name)
        sub.add_argument("source")
        sub.add_argument("destination")
    commands.add_parser("verify").add_argument("source")
    arguments = parser.parse_args(argv)
    try:
        result = globals()[arguments.command](arguments.source, *([] if arguments.command == "verify" else [arguments.destination]))
        print(json.dumps({"status": "ok", **result}, sort_keys=True))
        return 0
    except StateError as error:
        print("state: " + str(error), file=sys.stderr)
    except (OSError, ValueError, TypeError, KeyError, RecursionError):
        print("state: filesystem or format operation failed; inspect private roots locally", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
