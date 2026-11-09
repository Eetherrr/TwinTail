"""Exercise the actual inline publication program without GitHub mutations."""
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest
from unittest.mock import patch

WORKFLOW = Path(__file__).resolve().parents[2] / ".github/workflows/release.yml"
SOURCE = "a" * 40


class PublicationTests(unittest.TestCase):
    def run_publication(self, *, wrong_tag=False, tamper=False, duplicate=False,
                        change_id=False, extra_asset=False, wrong_approval=False):
        text = WORKFLOW.read_text(encoding="utf-8")
        marker = "      - name: Verify uploaded draft bytes and publish approved tag\n"
        program = text.split(marker, 1)[1].split("          python - <<'PY'\n", 1)[1]
        program = textwrap.dedent(program.rsplit("          PY", 1)[0])
        self.mutations = []
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "release-output"
            output.mkdir()
            files = {"release-notes.md": b"reviewed notes\n"}
            for target in ("linux-x86_64", "linux-aarch64", "windows-x86_64"):
                ext = ".zip" if target.startswith("windows") else ".tar.gz"
                files["twintail-0.1.1-" + target + ext] = target.encode()
                files[target + "-validation.json"] = b"{}\n"
            files["SHA256SUMS"] = "".join(
                hashlib.sha256(data).hexdigest() + "  " + name + "\n"
                for name, data in sorted(files.items())
            ).encode("ascii")
            for name, data in files.items():
                (output / name).write_bytes(data)
            assets = [{"id": i, "name": name, "size": len(data), "state": "uploaded"}
                      for i, (name, data) in enumerate(files.items(), 1)]
            if extra_asset:
                assets.append({"id": 99, "name": "unexpected.txt", "size": 0, "state": "uploaded"})
            release = {"id": 123, "tag_name": "v0.1.1", "draft": True,
                       "target_commitish": "main", "assets": assets}
            lookups = 0

            def urlopen(request, timeout):
                nonlocal lookups
                url = request.full_url
                if "/releases?" in url:
                    lookups += 1
                    value = dict(release)
                    if change_id and lookups > 1:
                        value["id"] = 124
                    data = [value, value] if duplicate else [value]
                elif "/git/ref/tags/" in url:
                    data = {"object": {"type": "tag", "sha": "b" * 40}}
                elif "/git/tags/" in url:
                    data = {"message": "not approved" if wrong_approval else "TwinTail-Publish: approved",
                            "object": {"type": "commit", "sha": "c" * 40 if wrong_tag else SOURCE}}
                else:
                    raise AssertionError("Unexpected API path: " + url)
                return contextlib.closing(io.BytesIO(json.dumps(data).encode()))

            def run(command, **kwargs):
                if command[0] == "sha256sum":
                    # The actual digest validation runs locally; no network or write API.
                    for line in (Path(kwargs["cwd"]) / "SHA256SUMS").read_text().splitlines():
                        digest, name = line.split("  ")
                        if hashlib.sha256((Path(kwargs["cwd"]) / name).read_bytes()).hexdigest() != digest:
                            raise subprocess.CalledProcessError(1, command)
                elif "--method" in command:
                    self.assertEqual(command, ["gh", "api", "--method", "PATCH",
                                               "repos/OWNER/REPO/releases/123", "-F", "draft=false",
                                               "-f", "make_latest=true"])
                    self.mutations.append(command)
                else:
                    asset_id = int(command[2].rsplit("/", 1)[1])
                    name = next(asset["name"] for asset in assets if asset["id"] == asset_id)
                    kwargs["stdout"].write(files[name] + (b"tampered" if tamper else b""))
                return subprocess.CompletedProcess(command, 0)

            old = Path.cwd()
            try:
                os.chdir(root)
                env = {"RELEASE_TAG": "v0.1.1", "SOURCE_COMMIT": SOURCE,
                       "GITHUB_API_URL": "https://api.github.com", "GITHUB_REPOSITORY": "OWNER/REPO",
                       "GH_REPO": "OWNER/REPO", "GH_TOKEN": "test-only"}
                with patch.dict(os.environ, env), patch("urllib.request.urlopen", urlopen), patch("subprocess.run", run):
                    exec(compile(program, str(WORKFLOW), "exec"), {})
            finally:
                os.chdir(old)

    def test_branch_metadata_uses_tag_identity_and_release_id(self):
        self.run_publication()
        self.assertEqual(len(self.mutations), 1)

    def test_reject_wrong_tag(self):
        with self.assertRaises(SystemExit):
            self.run_publication(wrong_tag=True)
        self.assertEqual(self.mutations, [])

    def test_reject_tampered_download(self):
        with self.assertRaises(SystemExit):
            self.run_publication(tamper=True)
        self.assertEqual(self.mutations, [])

    def test_reject_duplicate_release(self):
        with self.assertRaises(SystemExit):
            self.run_publication(duplicate=True)
        self.assertEqual(self.mutations, [])

    def test_reject_changed_draft_id(self):
        with self.assertRaises(SystemExit):
            self.run_publication(change_id=True)
        self.assertEqual(self.mutations, [])

    def test_reject_extra_asset(self):
        with self.assertRaises(SystemExit):
            self.run_publication(extra_asset=True)
        self.assertEqual(self.mutations, [])

    def test_reject_removed_approval(self):
        with self.assertRaises(SystemExit):
            self.run_publication(wrong_approval=True)
        self.assertEqual(self.mutations, [])


if __name__ == "__main__":
    unittest.main()
