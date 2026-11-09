#!/usr/bin/env python3
"""Strict-build docs in a temporary directory and validate local HTML links.

Usage: python tools/docs/check.py [--site-dir /path/to/already-built/site]
No network or credentials: external URLs are deliberately not fetched.
"""
import argparse
from html.parser import HTMLParser
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[2]
BASE = "/TwinTail/"


class Page(HTMLParser):
    def __init__(self, text):
        super().__init__(convert_charrefs=True)
        self.ids = set()
        self.links = []
        self.feed(text)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            self.ids.add(attrs["id"])
        if tag == "a" and "name" in attrs:
            self.ids.add(attrs["name"])
        for key in ("href", "src"):
            if key in attrs:
                self.links.append(attrs[key])


def validate(site):
    pages = {path.resolve(): Page(path.read_text(encoding="utf-8")) for path in site.rglob("*.html")}
    if not pages or not (site / "index.html").is_file():
        raise SystemExit("Site must contain HTML pages and a root index.html")
    errors = []
    checked = 0
    for source, page in pages.items():
        for link in page.links:
            parts = urlsplit(link)
            if parts.scheme or parts.netloc:
                continue
            path = unquote(parts.path)
            if path.startswith("/"):
                if not path.startswith(BASE):
                    errors.append(f"{source.relative_to(site)}: outside site base: {link}")
                    continue
                target = site / path[len(BASE):]
            else:
                target = source.parent / path if path else source
            target = target.resolve()
            if target.is_dir():
                target /= "index.html"
            checked += 1
            if not target.is_relative_to(site) or not target.is_file():
                errors.append(f"{source.relative_to(site)}: missing target: {link}")
            elif parts.fragment and target in pages and unquote(parts.fragment) not in pages[target].ids:
                errors.append(f"{source.relative_to(site)}: missing anchor: {link}")
    if errors:
        raise SystemExit("\n".join(errors))
    print(f"Validated {len(pages)} HTML pages and {checked} local links/assets; no broken targets or anchors.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--site-dir", type=Path, help="validate an existing build instead of building")
    args = parser.parse_args()
    if args.site_dir:
        validate(args.site_dir.resolve())
    else:
        with TemporaryDirectory(prefix="twintail-docs-") as directory:
            subprocess.run([sys.executable, "-m", "mkdocs", "build", "--strict", "--site-dir", directory], cwd=ROOT, check=True)
            validate(Path(directory).resolve())


if __name__ == "__main__":
    main()
