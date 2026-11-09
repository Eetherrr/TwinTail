"""Stdlib synthetic release automation contract tests; no runtime certification."""
import contextlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import unittest
from unittest.mock import patch

import automation
import release
import test_release


class AutomationTests(unittest.TestCase):
    def setUp(self):
        # Reuse the existing synthetic repository and ELF/PE fixture machinery,
        # without inheriting (and accidentally duplicating) its test methods.
        self.fixture = test_release.ReleaseTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.repo = self.fixture.repo
        self.artifacts = self.root / "artifacts"
        self.output = self.root / "output"
        self.commit = "a" * 40

    def artifacts_fixture(self):
        self.artifacts.mkdir()
        for target in release.TARGETS:
            directory = self.artifacts / target
            directory.mkdir()
            stage = self.fixture.stage(target, "stage-" + target)
            archive = directory / automation.archive_name("0.1.0", target)
            release.archive(stage, archive)
            (directory / "validation.json").write_bytes(release.json_bytes(release.verify(archive)))

    def collect(self):
        return automation.collect(self.repo, self.artifacts, self.output, "v0.1.0", self.commit)

    def assert_collection_fails(self):
        with self.assertRaises((release.ReleaseError, OSError, ValueError)):
            self.collect()
        self.assertFalse(self.output.exists())

    def test_validate_current_workspace_and_four_manifests(self):
        result = automation.validate(self.repo, "v0.1.0")
        self.assertTrue(result["validated"])
        self.assertEqual(set(result["plugins"]), set(release.PLUGINS))

    def test_strict_semver_valid_cases(self):
        for version in ("0.0.0", "1.2.3", "1.0.0-0", "1.0.0-alpha.1", "1.0.0-01a", "1.0.0-x-y.z"):
            with self.subTest(version=version):
                self.assertEqual(automation.tag_version("v" + version), version)

    def test_invalid_semver_and_path_injection(self):
        cases = ("0.1.0", "V0.1.0", "v01.1.0", "v1.01.0", "v1.0.00", "v1.0", "v1.0.0-01", "v1.0.0-rc.01", "v1.0.0-", "v1.0.0+", "v1.0.0-a..b", "v1.0.0+build..x", "v1.0.0/../x", "../v0.1.0", "v1.0.0\\x", "v1.0.0\n", "v1.0.0\x00", "v1.0.0;id", "v١.0.0", "v1.0.0-å", "v" + "1" * 257)
        for tag in cases:
            with self.subTest(tag=tag), self.assertRaises(release.ReleaseError):
                automation.tag_version(tag)
        for value in (None, 1, []):
            with self.subTest(value=value), self.assertRaises(release.ReleaseError):
                automation.tag_version(value)

    def test_build_metadata_rejected_before_build(self):
        for tag in ("v1.0.0+001.build", "v1.0.0-rc.1+build.009"):
            with self.subTest(tag=tag), self.assertRaises(release.ReleaseError):
                automation.tag_version(tag)

    def test_workspace_version_mismatch(self):
        with self.assertRaises(release.ReleaseError):
            automation.validate(self.repo, "v0.1.1")

    def test_every_plugin_version_and_identity_checked(self):
        for plugin, source in release.PLUGINS.items():
            path = self.repo / source / "plugin.toml"
            original = path.read_text(encoding="utf-8")
            for old, new in [('version = "0.1.0"', 'version = "0.1.1"'), (f'id = "{plugin}"', 'id = "wrong"')]:
                path.write_text(original.replace(old, new), encoding="utf-8")
                with self.subTest(plugin=plugin, field=old), self.assertRaises(release.ReleaseError):
                    automation.validate(self.repo, "v0.1.0")
            path.write_text(original, encoding="utf-8")

    def test_validate_prerelease(self):
        for version in ("0.1.0-rc.1", "0.1.0-alpha.2"):
            for path in [self.repo / "Cargo.toml", *(self.repo / source / "plugin.toml" for source in release.PLUGINS.values())]:
                text = path.read_text(encoding="utf-8")
                # Replace either previous fixture version with the new one.
                path.write_text(re.sub(r'(?m)^version = ".*"$', f'version = "{version}"', text), encoding="utf-8")
            self.assertEqual(automation.validate(self.repo, "v" + version)["version"], version)

    def test_collect_exact_flat_outputs_hashes_and_candidate_notes(self):
        self.artifacts_fixture()
        with patch.object(release, "verify", wraps=release.verify) as verifier:
            result = self.collect()
        self.assertEqual(verifier.call_count, 3)
        self.assertFalse(result["runtime_verified"])
        names = {automation.archive_name("0.1.0", target) for target in release.TARGETS}
        names |= {f"{target}-validation.json" for target in release.TARGETS}
        self.assertEqual({path.name for path in self.output.iterdir()}, names | {"release-notes.md", "SHA256SUMS"})
        checksums = (self.output / "SHA256SUMS").read_text(encoding="ascii").splitlines()
        self.assertEqual(len(checksums), 7)
        self.assertEqual(checksums, sorted(checksums, key=lambda line: line.split("  ")[1]))
        for line in checksums:
            digest, name = line.split("  ")
            self.assertEqual(digest, release.sha((self.output / name).read_bytes()))
        for target in release.TARGETS:
            self.assertEqual((self.output / f"{target}-validation.json").read_bytes(), (self.artifacts / target / "validation.json").read_bytes())
            self.assertEqual((self.output / automation.archive_name("0.1.0", target)).read_bytes(), (self.artifacts / target / automation.archive_name("0.1.0", target)).read_bytes())
        notes = (self.output / "release-notes.md").read_text(encoding="utf-8")
        for text in (self.commit, "caller-supplied", "candidate", "runtime_verified=false", "do not certify", *release.TARGETS):
            self.assertIn(text, notes)

    def test_missing_platform_rejected(self):
        self.artifacts_fixture()
        shutil.rmtree(self.artifacts / "linux-aarch64")
        self.assert_collection_fails()

    def test_extra_platform_and_nested_directory_rejected(self):
        self.artifacts_fixture()
        for path in (self.artifacts / "extra", self.artifacts / "linux-x86_64" / "nested"):
            path.mkdir()
            self.assert_collection_fails()
            path.rmdir()

    def test_missing_report_and_archive_rejected(self):
        self.artifacts_fixture()
        for filename in ("validation.json", automation.archive_name("0.1.0", "linux-x86_64")):
            path = self.artifacts / "linux-x86_64" / filename
            data = path.read_bytes()
            path.unlink()
            self.assert_collection_fails()
            path.write_bytes(data)

    def test_extra_duplicate_archive_and_wrong_name_rejected(self):
        self.artifacts_fixture()
        directory = self.artifacts / "linux-x86_64"
        source = directory / automation.archive_name("0.1.0", "linux-x86_64")
        for name in ("duplicate.tar.gz", "twintail-0.1.1-linux-x86_64.tar.gz", "extra.json"):
            path = directory / name
            path.write_bytes(source.read_bytes())
            self.assert_collection_fails()
            path.unlink()
        source.rename(directory / "wrong.tar.gz")
        self.assert_collection_fails()

    def test_report_exact_schema_types_identity_and_no_duplicate_keys(self):
        self.artifacts_fixture()
        path = self.artifacts / "linux-x86_64" / "validation.json"
        original = json.loads(path.read_bytes())
        cases = [b"not json", b"[]", b"null", b'{"verified":true,"verified":true}']
        for key, value in (("verified", False), ("verified", 1), ("runtime_verified", True), ("runtime_verified", 0), ("target", "windows-x86_64"), ("version", "0.1.1"), ("extra", "field")):
            cases.append(release.json_bytes({**original, key: value}))
        cases.append(release.json_bytes({key: value for key, value in original.items() if key != "verified"}))
        cases.append(b" " * (automation.REPORT_LIMIT + 1))
        for data in cases:
            with self.subTest(data=data[:100]):
                path.write_bytes(data)
                self.assert_collection_fails()

    def test_corrupt_archive_cleanup(self):
        self.artifacts_fixture()
        archive = self.artifacts / "linux-x86_64" / automation.archive_name("0.1.0", "linux-x86_64")
        archive.write_bytes(b"not an archive")
        self.assert_collection_fails()

    def test_valid_archive_wrong_target_rejected(self):
        self.artifacts_fixture()
        source = self.artifacts / "linux-aarch64" / automation.archive_name("0.1.0", "linux-aarch64")
        destination = self.artifacts / "linux-x86_64" / automation.archive_name("0.1.0", "linux-x86_64")
        destination.write_bytes(source.read_bytes())
        self.assert_collection_fails()

    def test_valid_archive_wrong_version_rejected(self):
        self.artifacts_fixture()
        for path in [self.repo / "Cargo.toml", *(self.repo / source / "plugin.toml" for source in release.PLUGINS.values())]:
            path.write_text(path.read_text(encoding="utf-8").replace('version = "0.1.0"', 'version = "0.1.1"'), encoding="utf-8")
        stage = self.fixture.stage("linux-x86_64", "other-version")
        wrong = self.root / "wrong-version.tar.gz"
        release.archive(stage, wrong)
        for path in [self.repo / "Cargo.toml", *(self.repo / source / "plugin.toml" for source in release.PLUGINS.values())]:
            path.write_text(path.read_text(encoding="utf-8").replace('version = "0.1.1"', 'version = "0.1.0"'), encoding="utf-8")
        destination = self.artifacts / "linux-x86_64" / automation.archive_name("0.1.0", "linux-x86_64")
        destination.write_bytes(wrong.read_bytes())
        self.assert_collection_fails()

    def test_competing_output_is_not_removed(self):
        self.artifacts_fixture()
        original_mkdir = Path.mkdir
        def competing_mkdir(path, *args, **kwargs):
            if path == self.output:
                original_mkdir(path)
                (path / "keep").write_bytes(b"competitor")
            return original_mkdir(path, *args, **kwargs)
        with patch.object(Path, "mkdir", competing_mkdir), self.assertRaises(FileExistsError):
            self.collect()
        self.assertEqual((self.output / "keep").read_bytes(), b"competitor")

    def test_post_creation_failure_cleans_only_new_output(self):
        self.artifacts_fixture()
        with patch.object(release, "verify", side_effect=OSError("synthetic I/O failure")), self.assertRaises(OSError):
            self.collect()
        self.assertFalse(self.output.exists())
        self.assertEqual({path.name for path in self.artifacts.iterdir()}, set(release.TARGETS))

    def test_existing_output_never_overwritten_even_empty(self):
        self.artifacts_fixture()
        self.output.mkdir()
        for populated in (False, True):
            if populated:
                (self.output / "keep").write_text("original", encoding="utf-8")
            with self.assertRaises(release.ReleaseError):
                self.collect()
            self.assertTrue(self.output.is_dir())
        self.assertEqual((self.output / "keep").read_text(encoding="utf-8"), "original")

    def test_invalid_commits_and_output_parent(self):
        self.artifacts_fixture()
        for commit in ("", "a" * 39, "A" * 40, "g" * 40, "a" * 40 + "\n", "../" + "a" * 40, "$(id)"):
            self.commit = commit
            self.assert_collection_fails()
        self.commit = "b" * 40
        self.output = self.root / "missing" / "output"
        self.assert_collection_fails()

    def test_parent_components_in_artifact_and_output_paths(self):
        self.artifacts_fixture()
        original = self.output
        self.output = self.root / "extra" / ".." / "output"
        self.assert_collection_fails()
        self.output = original
        self.artifacts = self.artifacts / ".." / "artifacts"
        self.assert_collection_fails()

    def test_symlink_inputs_directories_and_output_ancestors(self):
        self.artifacts_fixture()
        target = self.artifacts / "linux-x86_64"
        original = self.root / "original"
        target.rename(original)
        try:
            target.symlink_to(original, target_is_directory=True)
        except OSError:
            original.rename(target)
            self.skipTest("symlink creation unavailable")
        self.assert_collection_fails()
        target.unlink()
        original.rename(target)
        for name in ("validation.json", automation.archive_name("0.1.0", "linux-x86_64")):
            path = target / name
            saved = self.root / (name + ".saved")
            path.rename(saved)
            path.symlink_to(saved)
            self.assert_collection_fails()
            path.unlink()
            saved.rename(path)
        link = self.root / "linked-parent"
        link.symlink_to(self.root, target_is_directory=True)
        self.output = link / "output"
        self.assert_collection_fails()

    def test_hardlinked_archive_and_report_rejected(self):
        self.artifacts_fixture()
        for name in ("validation.json", automation.archive_name("0.1.0", "linux-x86_64")):
            path = self.artifacts / "linux-x86_64" / name
            link = self.root / "hardlink"
            try:
                os.link(path, link)
            except OSError:
                self.skipTest("hardlinks unavailable")
            self.assert_collection_fails()
            link.unlink()

    @unittest.skipUnless(hasattr(os, "mkfifo"), "FIFO requires Unix")
    def test_special_file_rejected(self):
        self.artifacts_fixture()
        path = self.artifacts / "linux-x86_64" / "validation.json"
        path.unlink()
        os.mkfifo(path)
        self.assert_collection_fails()

    def test_cli_validate_collect_success_and_failure(self):
        self.artifacts_fixture()
        for arguments, expected in [(["validate", "--tag", "v0.1.0"], 0), (["validate", "--tag", "v0.1.0/../x"], 1), (["collect", "--tag", "v0.1.0", "--commit", self.commit, "--artifacts", str(self.artifacts), "--output", str(self.output)], 0)]:
            with self.subTest(arguments=arguments):
                result = subprocess.run([sys.executable, str(Path(automation.__file__)), "--repo", str(self.repo), *arguments], capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, expected, result.stderr)
                if expected == 0:
                    self.assertIsInstance(json.loads(result.stdout), dict)
                else:
                    self.assertNotIn(str(self.root), result.stderr)

    def test_cli_error_redacted_and_no_credentials_or_network(self):
        output, error = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(error), patch.dict(os.environ, {"GH_TOKEN": "never-read-this-secret"}), patch("subprocess.run", side_effect=AssertionError("no subprocess")):
            self.assertEqual(automation.main(["--repo", str(self.repo), "validate", "--tag", "v0.1.0"]), 0)
            self.assertEqual(automation.main(["--repo", str(self.repo), "validate", "--tag", "v0.1.0/../../secret"]), 1)
        self.assertNotIn("never-read-this-secret", output.getvalue() + error.getvalue())
        self.assertNotIn(str(self.root), error.getvalue())


if __name__ == "__main__":
    unittest.main()
