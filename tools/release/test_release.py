"""Portable synthetic packaging checks; no target-platform runtime claims."""
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import release


def binary(target):
    data = bytearray(128)
    if target.startswith("linux-"):
        data[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", data, 18, release.TARGETS[target][2])
    else:
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 60, 64)
        data[64:68] = b"PE\0\0"
        struct.pack_into("<H", data, 68, release.TARGETS[target][2])
    return bytes(data)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.binaries = self.root / "binaries"
        self.binaries.mkdir()
        for name in release.DOCUMENTS:
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("{}\n" if name.endswith(".json") else "fixture documentation\n", encoding="utf-8")
        (self.repo / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.0"\nrust-version = "1.99.1"\nlicense = "MIT"\n', encoding="utf-8")
        (self.repo / "LICENSE").write_text("MIT License\nPermission is hereby granted, free of charge\n", encoding="utf-8")
        for plugin, source in release.PLUGINS.items():
            path = self.repo / source
            path.mkdir(parents=True, exist_ok=True)
            schema = '\nconfig_schema = "config.schema.json"' if plugin in ("adapter-snowluma", "web-admin") else ""
            (path / "plugin.toml").write_text(f'id = "{plugin}"\nversion = "0.1.0"\nentrypoint = "{plugin}"{schema}\n\n[dependencies]\n', encoding="utf-8")
            if schema:
                (path / "config.schema.json").write_text("{}\n", encoding="utf-8")

    def stage(self, target="linux-x86_64", name="stage"):
        for plugin in ["twintail-host", *release.PLUGINS]:
            (self.binaries / (plugin + release.TARGETS[target][1])).write_bytes(binary(target))
        path = self.root / name
        release.stage(self.repo, self.binaries, target, path)
        return path

    def test_all_target_synthetic_roundtrip_and_manifest(self):
        for target in release.TARGETS:
            with self.subTest(target=target):
                stage = self.stage(target, target)
                suffix = ".zip" if target.startswith("windows") else ".tar.gz"
                archive = self.root / (target + suffix)
                release.archive(stage, archive)
                metadata = release.verify(archive)
                self.assertEqual(metadata["target"], target)
                self.assertFalse(metadata["runtime_verified"])
                manifest = (stage / "packages/web-admin/0.1.0/plugin.toml").read_text()
                self.assertIn(f'target = "{target}"', manifest)
                self.assertIn(f'entrypoint = "web-admin{release.TARGETS[target][1]}"', manifest)
                gate = json.loads((stage / "release.json").read_bytes())
                self.assertEqual(gate["channel"], "candidate")
                self.assertFalse(gate["public_release"])
                self.assertEqual(gate["license_gate"], "MIT")
                self.assertEqual(gate["license"], "MIT")
                self.assertTrue((stage / "LICENSE").is_file())
                self.assertTrue((stage / "THIRD_PARTY_NOTICES.md").is_file())

    def test_repeated_stage_archive_identical_digest(self):
        for target in release.TARGETS:
            first = self.stage(target, target + "-first")
            second = self.stage(target, target + "-second")
            outputs = [self.root / (target + str(i)) for i in range(2)]
            release.archive(first, outputs[0])
            release.archive(second, outputs[1])
            self.assertEqual(hashlib.sha256(outputs[0].read_bytes()).digest(), hashlib.sha256(outputs[1].read_bytes()).digest())

    def test_unknown_target_cli_rejected(self):
        with patch("sys.argv", ["release.py", "build", "--target", "darwin-arm64"]):
            with self.assertRaises(SystemExit) as error:
                release.main()
        self.assertEqual(error.exception.code, 2)

    def test_default_build_target_directory_contract(self):
        with patch("release.subprocess.run") as run:
            result = release.build(self.repo, "linux-aarch64", self.root / "target")
        self.assertEqual(result["binaries"], str(self.root / "target/aarch64-unknown-linux-gnu/release"))
        self.assertIn("--locked", run.call_args.args[0])
        self.assertIn("--release", run.call_args.args[0])

    def test_existing_outputs_never_overwritten(self):
        stage = self.stage()
        with self.assertRaises(release.ReleaseError):
            release.stage(self.repo, self.binaries, "linux-x86_64", stage)
        output = self.root / "existing"
        output.write_bytes(b"keep")
        with self.assertRaises(release.ReleaseError):
            release.archive(stage, output)
        self.assertEqual(output.read_bytes(), b"keep")

    def test_input_pollution_not_included_and_stage_pollution_rejected(self):
        for name in ["fixtures/secret", "config.toml", "target/token", "data/private", ".env"]:
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"synthetic credential must not ship")
        stage = self.stage()
        self.assertFalse(any(b"synthetic credential" in data for data in release.directory_files(stage).values()))
        (stage / "config.toml").write_bytes(b"synthetic secret")
        with self.assertRaises(release.ReleaseError):
            release.verify(stage)

    def test_empty_directory_pollution_rejected(self):
        stage = self.stage()
        (stage / "fixtures").mkdir()
        with self.assertRaises(release.ReleaseError):
            release.verify(stage)

    def test_tamper_detected(self):
        stage = self.stage()
        host = stage / "bin/twintail-host"
        host.write_bytes(host.read_bytes() + b"tampered")
        with self.assertRaises(release.ReleaseError):
            release.verify(stage)
        with self.assertRaises(release.ReleaseError):
            release.archive(stage, self.root / "bad.tar.gz")
        self.assertFalse((self.root / "bad.tar.gz").exists())

    def test_receipt_and_metadata_self_hash_are_bounded(self):
        stage = self.stage()
        sums = (stage / "SHA256SUMS").read_text()
        self.assertNotIn("  SHA256SUMS\n", sums)
        self.assertIn("  release.json\n", sums)
        receipt = json.loads((stage / "packages/web-admin/0.1.0/checksums.json").read_bytes())
        self.assertEqual(set(receipt), {"web-admin", "plugin.toml", "config.schema.json"})

    def test_wrong_architecture_and_manifest_identity_rejected(self):
        self.stage()
        (self.binaries / "twintail-host").write_bytes(binary("linux-aarch64"))
        with self.assertRaises(release.ReleaseError):
            release.stage(self.repo, self.binaries, "linux-x86_64", self.root / "bad-arch")
        (self.binaries / "twintail-host").write_bytes(binary("linux-x86_64"))
        (self.repo / "plugins/web-admin/plugin.toml").write_text('id="other"\nversion="0.1.0"\nentrypoint="web-admin"\n')
        with self.assertRaises(release.ReleaseError):
            release.stage(self.repo, self.binaries, "linux-x86_64", self.root / "bad-manifest")

    def test_path_traversal_and_archive_output_in_stage_rejected(self):
        for name in ["../x", "/x", "a/../../x", "a\\x", "C:/x", "a//x", "a/./x"]:
            with self.subTest(name=name), self.assertRaises(release.ReleaseError):
                release.relative(name)
        stage = self.stage()
        with self.assertRaises(release.ReleaseError):
            release.archive(stage, stage / "archive.tar.gz")
        with self.assertRaises(release.ReleaseError):
            release.safe_path(self.root / ".." / "escape")

    def test_symlink_source_ancestor_stage_and_output_rejected(self):
        stage = self.stage()
        link = self.root / "link"
        try:
            link.symlink_to(self.repo, target_is_directory=True)
        except (OSError, NotImplementedError):
            self.skipTest("symlink creation unavailable on this runner")
        with self.assertRaises(release.ReleaseError):
            release.stage(link, self.binaries, "linux-x86_64", self.root / "newstage")
        (stage / "badlink").symlink_to(self.repo / "Cargo.lock")
        with self.assertRaises(release.ReleaseError):
            release.verify(stage)
        with self.assertRaises(release.ReleaseError):
            release.archive(self.root / "stage", link / "archive")

    @unittest.skipIf(os.name == "nt", "POSIX file modes and FIFO")
    def test_special_files_and_executable_permission_loss_rejected(self):
        stage = self.stage()
        os.mkfifo(stage / "fifo")
        with self.assertRaises(release.ReleaseError):
            release.verify(stage)
        with self.assertRaises(release.ReleaseError):
            release.verify(stage / "fifo")
        (stage / "fifo").unlink()
        (stage / "bin/twintail-host").chmod(0o644)
        with self.assertRaises(release.ReleaseError):
            release.verify(stage)

    def test_malicious_archive_paths_links_duplicates_rejected(self):
        for case in ["traversal", "symlink", "hardlink", "duplicate", "special"]:
            archive = self.root / (case + ".tar.gz")
            with archive.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed, tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as packed:
                member = tarfile.TarInfo("../escape" if case == "traversal" else "root/file")
                if case in ("symlink", "hardlink"):
                    member.type = tarfile.SYMTYPE if case == "symlink" else tarfile.LNKTYPE
                    member.linkname = "../../escape"
                elif case == "special":
                    member.type = tarfile.FIFOTYPE
                packed.addfile(member, io.BytesIO())
                if case == "duplicate":
                    packed.addfile(member, io.BytesIO())
            with self.subTest(case=case), self.assertRaises(release.ReleaseError):
                release.verify(archive)
        self.assertFalse((self.root / "escape").exists())

    def test_create_new_race_never_deletes_competing_output(self):
        stage = self.stage()
        output = self.root / "competing.tar.gz"
        original = Path.open
        def racing_open(path, *args, **kwargs):
            if path == output and args and args[0] == "xb":
                with original(output, "wb") as stream:
                    stream.write(b"competing writer")
                raise FileExistsError()
            return original(path, *args, **kwargs)
        with patch.object(Path, "open", racing_open):
            with self.assertRaises(FileExistsError):
                release.archive(stage, output)
        self.assertEqual(output.read_bytes(), b"competing writer")

    def test_archive_permission_and_gzip_metadata_tampering(self):
        stage = self.stage()
        archive = self.root / "valid.tar.gz"
        release.archive(stage, archive)
        data = bytearray(archive.read_bytes())
        data[4] = 1
        archive.write_bytes(data)
        with self.assertRaises(release.ReleaseError):
            release.verify(archive)
        archive.unlink()
        release.archive(stage, archive)
        altered = self.root / "altered.tar.gz"
        with tarfile.open(archive, "r:gz") as source, altered.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed, tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as output:
            for member in source:
                if member.name.endswith("bin/twintail-host"):
                    member.mode = 0o644
                output.addfile(member, source.extractfile(member))
        with self.assertRaises(release.ReleaseError):
            release.verify(altered)

    def test_corrupt_archive_cli_returns_safe_error(self):
        archive = self.root / "private-value.tar.gz"
        archive.write_bytes(b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x02\xff")
        errors = io.StringIO()
        with patch("sys.argv", ["release.py", "verify", str(archive)]), patch("sys.stderr", errors):
            self.assertEqual(release.main(), 1)
        self.assertEqual(errors.getvalue(), "release operation failed: invalid, unsafe, inconsistent or unavailable input\n")

    def test_mit_license_and_notice_required(self):
        self.stage()
        license_path = self.repo / "LICENSE"
        license_bytes = license_path.read_bytes()
        license_path.write_text("not the authorized license")
        with self.assertRaises(release.ReleaseError):
            release.stage(self.repo, self.binaries, "linux-x86_64", self.root / "bad-license")
        license_path.write_bytes(license_bytes)
        (self.repo / "THIRD_PARTY_NOTICES.md").write_bytes(b"")
        with self.assertRaises(release.ReleaseError):
            release.stage(self.repo, self.binaries, "linux-x86_64", self.root / "missing-notices")
        (self.repo / "THIRD_PARTY_NOTICES.md").write_text("Third-party notice fixture")
        manifest = self.repo / "Cargo.toml"
        manifest.write_text(manifest.read_text().replace('license = "MIT"', 'license = "Apache-2.0"'))
        with self.assertRaises(release.ReleaseError):
            release.stage(self.repo, self.binaries, "linux-x86_64", self.root / "wrong-license")

    def test_bounds_enforced_before_staging(self):
        self.stage()
        with patch("release.MAX_FILE", 100):
            with self.assertRaises(release.ReleaseError):
                release.stage(self.repo, self.binaries, "linux-x86_64", self.root / "too-big")
        with patch("release.MAX_TOTAL", 10):
            with self.assertRaises(release.ReleaseError):
                release.directory_files(self.root / "stage")


if __name__ == "__main__":
    unittest.main()
