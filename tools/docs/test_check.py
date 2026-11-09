"""Offline regression checks for generated-site validation (stdlib only)."""
import contextlib
import importlib.util
import io
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

spec = importlib.util.spec_from_file_location("docs_check", Path(__file__).with_name("check.py"))
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


class SiteChecks(unittest.TestCase):
    def validate(self, files):
        with TemporaryDirectory() as directory:
            root = Path(directory)
            for name, text in files.items():
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(text, encoding="utf-8")
            with contextlib.redirect_stdout(io.StringIO()):
                check.validate(root.resolve())

    def test_directory_base_unicode_and_external_links(self):
        self.validate({
            "index.html": '<a href="guide/#中文">Guide</a><a href="/TwinTail/guide/">Base</a><a href="https://example.invalid/">External</a>',
            "guide/index.html": '<h1 id="中文">Title</h1><a href="../">Home</a><img src="../asset.svg">',
            "asset.svg": "<svg/>",
        })

    def test_missing_anchor(self):
        with self.assertRaisesRegex(SystemExit, "missing anchor"):
            self.validate({"index.html": '<a href="#absent">Missing</a>'})

    def test_missing_target(self):
        with self.assertRaisesRegex(SystemExit, "missing target"):
            self.validate({"index.html": '<img src="absent.png">'})

    def test_outside_base(self):
        with self.assertRaisesRegex(SystemExit, "outside site base"):
            self.validate({"index.html": '<a href="/other/">Wrong base</a>'})

    def test_empty_or_wrong_site(self):
        for files in ({}, {"other.html": "Other"}):
            with self.subTest(files=files), self.assertRaisesRegex(SystemExit, "root index.html"):
                self.validate(files)

    def test_path_escape(self):
        with self.assertRaisesRegex(SystemExit, "missing target"):
            self.validate({"index.html": '<a href="../../outside.html">Escape</a>'})


if __name__ == "__main__":
    unittest.main()
