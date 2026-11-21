"""Tests of download-page.py. Run with: python3 -m unittest discover -s scripts"""

import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

_spec = importlib.util.spec_from_file_location("download_page", Path(__file__).with_name("download-page.py"))
dp = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(dp)


def asset(name, size=12_345_678):
    return {"name": name, "size": size, "browser_download_url": f"https://example.com/{name}"}


def release(tag, date, names, **extra):
    return {
        "tag_name": tag,
        "html_url": f"https://example.com/releases/{tag}",
        "published_at": f"{date}T10:00:00Z",
        "assets": [asset(n) for n in names],
        **extra,
    }


ALL = [
    "artwork-creator-linux.tar.gz",
    "artwork-creator-windows.zip",
    "artwork-creator_1.0.0_amd64.deb",
    "artwork-creator-windows-setup.exe",
    "artwork-creator.AppImage",
    "checksums.txt",
]


class Helpers(unittest.TestCase):
    def test_kind(self):
        self.assertEqual(dp.kind("x-windows-setup.exe")[:2], ("Windows", "Installer"))
        self.assertEqual(dp.kind("x.AppImage")[:2], ("Linux", "AppImage"))
        self.assertIsNone(dp.kind("checksums.txt"))

    def test_size(self):
        self.assertEqual(dp.size(12_345_678), "12.3 MB")

    def test_files_known_only_in_order(self):
        labels = [label for _, _, label, _ in dp.files(release("v1", "2026-01-01", ALL))]
        self.assertEqual(labels, ["Installer", "Portable .zip", "AppImage", "Debian / Ubuntu .deb", "Archive .tar.gz"])
        self.assertEqual(dp.files({}), [])

    def test_date(self):
        self.assertEqual(dp.date(release("v1", "2026-03-04", [])), "2026-03-04")
        self.assertEqual(dp.date({"published_at": None}), "")


class Latest(unittest.TestCase):
    def test_cards(self):
        html = dp.latest_html(release("v0.15.0", "2026-11-20", ALL))
        self.assertIn("Version 0.15.0 · 2026-11-20", html)
        self.assertIn('href="https://example.com/releases/v0.15.0"', html)
        self.assertIn("<h3>Windows</h3>", html)
        self.assertIn("<h3>Linux</h3>", html)
        self.assertNotIn("Pre-release", html)
        # The first file of a platform is its main button, the others secondary.
        self.assertIn('class="button" href="https://example.com/artwork-creator-windows-setup.exe"', html)
        self.assertIn('class="button secondary" href="https://example.com/artwork-creator-windows.zip"', html)
        self.assertIn("12.3 MB", html)

    def test_prerelease_and_missing_platform(self):
        html = dp.latest_html(release("v1.0.0-rc1", "2026-11-20", ["a.AppImage"], prerelease=True))
        self.assertIn("Pre-release", html)
        self.assertNotIn("Windows", html)
        self.assertIn("<h3>Linux</h3>", html)

    def test_escapes(self):
        r = release('v1"<x>', "2026-01-01", ["a.AppImage"])
        r["html_url"] = 'https://e.com/"><script>'
        html = dp.latest_html(r)
        self.assertNotIn("<script>", html)
        self.assertNotIn("<x>", html)


class Older(unittest.TestCase):
    def test_none(self):
        self.assertIn("No older releases yet.", dp.older_html([]))

    def test_rows(self):
        html = dp.older_html([release("v0.14.0", "2026-10-01", ["a-windows-setup.exe", "b.AppImage"])])
        self.assertIn('<a href="https://example.com/releases/v0.14.0">v0.14.0</a>', html)
        self.assertIn("<td>2026-10-01</td>", html)
        self.assertIn(">Windows Installer</a> · <a", html)
        self.assertIn(">Linux AppImage</a>", html)


class Main(unittest.TestCase):
    def run_main(self, releases):
        with tempfile.TemporaryDirectory() as tmp:
            template = Path(tmp) / "download.html"
            template.write_text("<main><!-- LATEST --><hr><!-- OLDER --></main>", encoding="utf-8")
            out = io.StringIO()
            with mock.patch.object(dp, "TEMPLATE", template), \
                    mock.patch("sys.stdin", io.StringIO(json.dumps(releases))), \
                    mock.patch("sys.stdout", out):
                dp.main()
        return out.getvalue()

    def test_no_release(self):
        page = self.run_main([])
        self.assertIn("No release yet.", page)
        self.assertIn("No older releases yet.", page)
        self.assertNotIn("<!--", page)

    def test_latest_and_older(self):
        page = self.run_main([
            release("v0.13.0", "2026-09-01", ALL),
            release("v0.15.0", "2026-11-20", ALL, draft=True),
            release("v0.14.0", "2026-10-01", ALL),
            release("v0.12.0", "2026-08-01", ["checksums.txt"]),
        ])
        latest, older = page.split("<hr>")
        # Drafts and releases without known files are left out; the newest is the latest.
        self.assertIn("Version 0.14.0", latest)
        self.assertIn("v0.13.0", older)
        self.assertNotIn("v0.15.0", page)
        self.assertNotIn("v0.12.0", page)

    def test_real_template_has_markers(self):
        text = dp.TEMPLATE.read_text(encoding="utf-8")
        self.assertIn("<!-- LATEST -->", text)
        self.assertIn("<!-- OLDER -->", text)


if __name__ == "__main__":
    unittest.main()
