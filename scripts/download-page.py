#!/usr/bin/env python3
"""Fills in the download page (pages/download.html) from the project's GitHub releases.

Usage: download-page.py < RELEASES_JSON > download.html

Reads the GitHub API's list of releases on stdin (gh api --paginate --slurp
repos/OWNER/REPO/releases | jq 'add', or curl .../releases?per_page=100) and writes the
page to stdout. The template is the repository's pages/download.html. The newest published
release goes in the template's <!-- LATEST --> marker, one card per platform; the others
in <!-- OLDER -->, one row each. Drafts are left out, as are files the page doesn't know.
Run by .github/workflows/pages.yml.
"""

import json
import sys
from html import escape
from pathlib import Path

TEMPLATE = Path(__file__).resolve().parent.parent / "pages" / "download.html"

# Each known file: (platform, label, how to use it), by the end of its name. The first
# of a platform is its main download.
FILES = [
    ("-windows-setup.exe", "Windows", "Installer", "Installs the app with a Start menu entry; opens .artwork files."),
    ("-windows.zip", "Windows", "Portable .zip", "Unzip anywhere and run artwork-creator.exe."),
    (".AppImage", "Linux", "AppImage", "Make it executable (chmod +x) and run it."),
    ("_amd64.deb", "Linux", "Debian / Ubuntu .deb", "sudo apt install ./artwork-creator_*.deb"),
    ("-linux.tar.gz", "Linux", "Archive .tar.gz", "Extract and run artwork-creator."),
]
PLATFORMS = {
    "Windows": "64-bit",
    "Linux": "x86_64, glibc 2.28 or later (Debian 10, Ubuntu 20.04 and later)",
}


def kind(name):
    for end, platform, label, how in FILES:
        if name.endswith(end):
            return platform, label, how
    return None


def size(n):
    return f"{n / 1_000_000:.1f} MB"


def files(release):
    """The release's known files, in the order of FILES."""
    out = []
    for end, *_ in FILES:
        for a in release.get("assets", []):
            if a["name"].endswith(end):
                out.append((a, *kind(a["name"])))
    return out


def date(release):
    return (release.get("published_at") or "")[:10]


def latest_html(r):
    by_platform = {}
    for a, platform, label, how in files(r):
        by_platform.setdefault(platform, []).append((a, label, how))
    pre = ' · <span class="badge">Pre-release</span>' if r.get("prerelease") else ""
    out = [
        f'<p class="release">Version {escape(r["tag_name"].lstrip("v"))} · {date(r)} · '
        f'<a href="{escape(r["html_url"])}">Release notes</a>{pre}</p>',
        '<div class="cards">',
    ]
    for platform, needs in PLATFORMS.items():
        items = by_platform.get(platform)
        if not items:
            continue
        out.append(f'  <section class="card">\n    <h3>{platform}</h3>')
        out.append(f'    <p class="needs">{escape(needs)}</p>')
        for i, (a, label, how) in enumerate(items):
            cls = "button" if i == 0 else "button secondary"
            out.append(
                f'    <p class="file"><a class="{cls}" href="{escape(a["browser_download_url"])}">{escape(label)}</a>'
                f' <span class="size">{size(a["size"])}</span><br><span class="how">{escape(how)}</span></p>'
            )
        out.append("  </section>")
    out.append("</div>")
    return "\n".join(out)


def older_html(releases):
    if not releases:
        return '<p class="muted">No older releases yet.</p>'
    out = ["<table>", "  <thead><tr><th>Version</th><th>Date</th><th>Files</th></tr></thead>", "  <tbody>"]
    for r in releases:
        links = " · ".join(
            f'<a href="{escape(a["browser_download_url"])}">{escape(platform)} {escape(label)}</a>'
            for a, platform, label, _ in files(r)
        )
        out.append(
            f'    <tr><td><a href="{escape(r["html_url"])}">{escape(r["tag_name"])}</a></td>'
            f"<td>{date(r)}</td><td>{links}</td></tr>"
        )
    out += ["  </tbody>", "</table>"]
    return "\n".join(out)


def main():
    releases = [r for r in json.load(sys.stdin) if not r.get("draft") and files(r)]
    releases.sort(key=lambda r: r.get("published_at") or "", reverse=True)
    page = TEMPLATE.read_text(encoding="utf-8")
    if releases:
        latest = latest_html(releases[0])
    else:
        latest = '<p class="muted">No release yet.</p>'
    page = page.replace("<!-- LATEST -->", latest).replace("<!-- OLDER -->", older_html(releases[1:]))
    sys.stdout.write(page)


if __name__ == "__main__":
    main()
