"""Cold-state tests; all fixtures are synthetic and contain no real secrets."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import stat
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("state_tool", Path(__file__).with_name("state.py"))
state = importlib.util.module_from_spec(spec)
spec.loader.exec_module(state)


@unittest.skipUnless(os.name == "posix", "Unix endpoint exclusion only")
class StateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.source = self.base / "source"
        self.source.mkdir(mode=0o700)
        self.backup = self.base / "backup"
        self.target = self.base / "restored"
        (self.source / "plugins/snow/data").mkdir(parents=True)
        self.config = self.source / "plugins/snow/config.toml"
        self.config.write_text('config_version = 1\nrevision = 7\n[values]\nurl = "ws://127.0.0.1:3001"\nself_id = "10000"\n[values.access_token]\n"$secret" = "SNOW_TOKEN"\n')
        (self.source / "plugins/snow/config.applied.toml").write_text('revision = 6\n')
        (self.source / "plugins/snow/config.lock").write_bytes(b"")
        (self.source / "plugins/snow/data/private.bin").write_bytes(b"private-business-fixture")
        (self.source / "host.toml").write_text('[plugins]\n')

    def backup_source(self):
        return state.snapshot(self.source, self.backup)

    def test_roundtrip_preserves_legacy_forward_bytes_refs_and_mismatch(self):
        with patch.dict(os.environ, {"SNOW_TOKEN": "must-not-enter-backup"}):
            summary = self.backup_source()
        self.assertEqual(summary["desired_applied_mismatches"], 1)
        self.assertEqual(state.verify(self.backup), summary)
        self.assertFalse((self.backup / "tree/plugins/snow/config.lock").exists())
        state.restore(self.backup, self.target)
        self.assertEqual((self.target / "plugins/snow/config.toml").read_bytes(), self.config.read_bytes())
        self.assertNotIn(b"must-not-enter-backup", b"".join(p.read_bytes() for p in self.backup.rglob("*") if p.is_file()))
        self.assertEqual((self.target / "plugins/snow/config.applied.toml").read_text(), 'revision = 6\n')
        self.assertFalse((self.source / "management.sock").exists())
        self.assertFalse((self.target / "management.sock").exists())
        for root in [self.backup, self.target]:
            for path in [root, *root.rglob("*")]:
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o700 if path.is_dir() else 0o600)

    def test_endpoint_active_or_stale_is_never_automatically_removed(self):
        marker = self.source / "management.sock"
        server = socket.socket(socket.AF_UNIX)
        server.bind(str(marker))
        server.listen()
        with self.assertRaises(state.StateError): self.backup_source()
        server.close()
        with self.assertRaises(state.StateError): self.backup_source()
        self.assertTrue(marker.exists())
        marker.unlink()
        for name in ["run.lock", "host.lock", "host.run.lock"]:
            lock = self.source / name
            lock.touch()
            with self.assertRaises(state.StateError): self.backup_source()
            self.assertTrue(lock.exists())
            lock.unlink()

    def test_reserved_endpoint_prevents_competing_host_bind(self):
        with state.stopped(self.source):
            other = socket.socket(socket.AF_UNIX)
            with other, self.assertRaises(OSError): other.bind(str(self.source / "management.sock"))
        self.assertFalse((self.source / "management.sock").exists())

    def test_active_configuration_lock_rejected_without_trusting_file_age(self):
        import fcntl
        with (self.source / "plugins/snow/config.lock").open("rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(state.StateError): self.backup_source()
        self.backup_source()

    def test_never_overwrites_nonempty_live_or_existing_targets(self):
        self.backup_source()
        self.target.mkdir(mode=0o700)
        sentinel = self.target / "sentinel"
        sentinel.write_bytes(b"keep")
        with self.assertRaises(state.StateError): state.restore(self.backup, self.target)
        self.assertEqual(sentinel.read_bytes(), b"keep")
        with self.assertRaises(state.StateError): state.restore(self.backup, self.source)
        sentinel.unlink()
        state.restore(self.backup, self.target)

    def test_links_special_files_and_overlapping_roots_are_rejected(self):
        outside = self.base / "outside"
        outside.write_bytes(b"outside-private")
        candidate = self.source / "escape"
        candidate.symlink_to(outside)
        with self.assertRaises(state.StateError): self.backup_source()
        candidate.unlink()
        os.link(outside, candidate)
        with self.assertRaises(state.StateError): self.backup_source()
        candidate.unlink()
        os.mkfifo(candidate)
        with self.assertRaises(state.StateError): self.backup_source()
        candidate.unlink()
        with self.assertRaises(state.StateError): state.snapshot(self.source, self.source / "backup")
        link = self.base / "parent-link"
        link.symlink_to(self.source, target_is_directory=True)
        with self.assertRaises(state.StateError): state.snapshot(link, self.backup)
        self.assertEqual(outside.read_bytes(), b"outside-private")

    def test_corruption_extra_entries_and_permissions_are_detected(self):
        self.backup_source()
        target = self.backup / "tree/host.toml"
        original = target.read_bytes()
        target.write_bytes(b"corrupt")
        with self.assertRaises(state.StateError): state.verify(self.backup)
        with self.assertRaises(state.StateError): state.restore(self.backup, self.target)
        self.assertFalse(self.target.exists())
        target.write_bytes(original)
        target.chmod(0o644)
        with self.assertRaises(state.StateError): state.verify(self.backup)
        target.chmod(0o600)
        extra = self.backup / "tree/extra"
        extra.touch(mode=0o600)
        with self.assertRaises(state.StateError): state.verify(self.backup)
        extra.unlink()
        state.verify(self.backup)

    def test_malicious_manifest_path_rejected_without_outside_access(self):
        self.backup_source()
        manifest = self.backup / "manifest.json"
        original = manifest.read_bytes()
        for name in ["../escape", "/absolute", "a/../escape", "a\\escape", "C:/escape", "./host.toml"]:
            value = json.loads(original)
            value["entries"][0]["path"] = name
            manifest.write_text(json.dumps(value))
            with self.assertRaises(state.StateError): state.verify(self.backup)
        self.assertFalse((self.base / "escape").exists())

    def test_restore_failure_cleans_only_its_output_and_preserves_source(self):
        self.backup_source()
        original = state.write_file
        calls = 0
        def failing_write(path, data, on_created=lambda: None):
            nonlocal calls
            calls += 1
            original(path, data, on_created)
            if calls == 2: raise OSError("injected-private-error")
        with patch.object(state, "write_file", failing_write):
            with self.assertRaises(OSError): state.restore(self.backup, self.target)
        self.assertFalse(self.target.exists())
        state.verify(self.backup)
        state.restore(self.backup, self.target)

    def test_size_entry_limits_and_applied_bounds_fail_closed(self):
        with patch.object(state, "MAX_FILES", 2):
            with self.assertRaises(state.StateError): self.backup_source()
        with patch.object(state, "MAX_FILE", 2):
            with self.assertRaises(state.StateError): self.backup_source()
        (self.source / "plugins/snow/config.applied.toml").write_text('revision = 8\n')
        with self.assertRaises(state.StateError): self.backup_source()
        self.assertFalse(self.backup.exists())

    def test_errors_do_not_print_private_filesystem_or_toml_instances(self):
        self.config.write_text('config_version = "private-invalid-version"\n')
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            self.assertEqual(state.main(["snapshot", str(self.source), str(self.backup)]), 2)
        self.assertNotIn("private-invalid-version", stderr.getvalue())
        self.assertNotIn(str(self.source), stderr.getvalue())

    def test_snapshot_mutation_detected_and_failed_output_removed(self):
        original = state.read_file
        def changing(path, limit=state.MAX_FILE):
            data = original(path, limit)
            if path == self.source / "host.toml": path.write_bytes(data + b"\n")
            return data
        with patch.object(state, "read_file", changing):
            with self.assertRaises(state.StateError): self.backup_source()
        self.assertFalse(self.backup.exists())
        self.assertFalse((self.source / "management.sock").exists())


    def test_business_file_named_config_lock_is_not_excluded(self):
        business = self.source / "plugins/snow/data/config.lock"
        business.write_bytes(b"application-owned-state")
        self.backup_source()
        state.restore(self.backup, self.target)
        self.assertEqual((self.target / "plugins/snow/data/config.lock").read_bytes(), business.read_bytes())

    def test_snapshot_failed_partial_write_is_cleaned_and_source_unchanged(self):
        original_config = self.config.read_bytes()
        original = state.write_file
        def fail_after_create(path, data, on_created=lambda: None):
            original(path, data, on_created)
            raise OSError("private-disk-fixture")
        with patch.object(state, "write_file", fail_after_create):
            with self.assertRaises(OSError): self.backup_source()
        self.assertFalse(self.backup.exists())
        self.assertEqual(self.config.read_bytes(), original_config)
        self.assertFalse((self.source / "management.sock").exists())

    def test_restore_failure_preserves_preexisting_empty_target(self):
        self.backup_source()
        self.target.mkdir(mode=0o700)
        original = state.write_file
        def fail_after_create(path, data, on_created=lambda: None):
            original(path, data, on_created)
            raise OSError("private-disk-fixture")
        with patch.object(state, "write_file", fail_after_create):
            with self.assertRaises(OSError): state.restore(self.backup, self.target)
        self.assertTrue(self.target.is_dir())
        self.assertEqual(list(self.target.iterdir()), [])
        state.restore(self.backup, self.target)

    def test_private_root_and_redacted_reference_rules(self):
        self.source.chmod(0o755)
        with self.assertRaises(state.StateError): self.backup_source()
        self.source.chmod(0o700)
        for token in ['"***"', '{"$secret" = "bad-name"}', '{"$redacted" = true}', '{"$secret" = "TOKEN", "extra" = true}']:
            self.config.write_text('config_version = 1\nrevision = 7\n[values]\naccess_token = ' + token + '\n')
            with self.assertRaises(state.StateError): self.backup_source()
            self.assertFalse(self.backup.exists())


if __name__ == "__main__":
    unittest.main()
