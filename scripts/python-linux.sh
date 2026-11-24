#!/bin/sh
# Downloads a relocatable CPython for Linux from python-build-standalone (the builds uv
# uses; they run on glibc 2.17 and later), at a pinned release checked against its
# SHA-256, and unpacks into target/python-linux what the Python plugins need: bin/python3
# (built with libpython in it), lib/python3.14 with pip, which the plugin manager runs to
# install the packages plugins need. The Linux packages ship it: the .tar.gz in a python
# folder next to the app, the .deb and the AppImage in usr/lib/artwork-creator/python.
# Used by `make dist-linux`, `make deb`, `make appimage` and the Release workflow. Does
# nothing when that version is already there.
#
#   sh scripts/python-linux.sh
#
# Keep PYTHON_VERSION the Windows one (scripts/python-embed.sh), so plugins see the same
# Python everywhere. Raise RELEASE, PYTHON_VERSION and PYTHON_SHA256 together, on
# purpose, from https://github.com/astral-sh/python-build-standalone/releases (the
# x86_64-unknown-linux-gnu-install_only_stripped .tar.gz; GitHub shows its SHA-256).
set -eu

RELEASE=20261110
PYTHON_VERSION=3.14.10
PYTHON_SHA256=c2ff2e97422f5d3896a16e0317fd212ba977bbf2688d93a6f7a03d817506bd30

die() {
    echo "python-linux: $*" >&2
    exit 1
}

dir=target/python-linux
tgz=cpython-$PYTHON_VERSION+$RELEASE-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz

stamp="$PYTHON_VERSION $RELEASE"
if [ -f "$dir/bin/python3" ] && [ "$(cat "$dir/.version" 2>/dev/null)" = "$stamp" ]; then
    echo "$dir"
    exit 0
fi

rm -rf "$dir" "target/$tgz" target/python-linux.part
mkdir -p target/python-linux.part
curl --proto '=https' --tlsv1.2 -fsSL -o "target/$tgz" \
    "https://github.com/astral-sh/python-build-standalone/releases/download/$RELEASE/$(echo "$tgz" | sed 's/+/%2B/')"
echo "$PYTHON_SHA256  target/$tgz" | sha256sum -c - >/dev/null || die "$tgz checksum mismatch"
tar -xzf "target/$tgz" -C target/python-linux.part
rm "target/$tgz"
# Only the interpreter and its library: no headers, terminfo, Tcl/Tk (no tkinter), shared
# libpython (python3 has it built in) or scripts, and no symlinks (cargo-deb copies files).
src=target/python-linux.part/python
minor=python$(echo "$PYTHON_VERSION" | cut -d. -f1-2)
mkdir -p "$dir/bin" "$dir/lib"
cp "$src/bin/$minor" "$dir/bin/python3"
mv "$src/lib/$minor" "$dir/lib/"
rm -rf target/python-linux.part
[ -z "$(find "$dir" -type l)" ] || die "symlinks left in $dir"
"$dir/bin/python3" -I -c "import pip, ssl, sqlite3, ctypes" || die "the Python in $dir does not run"
echo "$stamp" >"$dir/.version"
echo "$dir"
