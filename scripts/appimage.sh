#!/bin/sh
# Packages a Linux release build as an AppImage: one file that runs on most distributions
# without installing. Used by `make appimage` and the Release workflow.
#
#   sh scripts/appimage.sh <executable> <output.AppImage>
#
# The AppDir has the executable in usr/bin/ and the templates in
# usr/share/artwork-creator/, where the app looks for them (`shipped_dir`). appimagetool
# and the AppImage runtime are downloaded once into $APPIMAGE_TOOLS (default
# ~/.cache/appimage), at pinned versions checked against their SHA-256; raise them on
# purpose.
set -eu

APPIMAGETOOL_VERSION=1.9.1
APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
RUNTIME_VERSION=20251108
RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d

APP=artwork-creator
exe=$1
out=$2
tools=${APPIMAGE_TOOLS:-$HOME/.cache/appimage}

# fetch <url> <file> <sha256>
fetch() {
    if [ ! -f "$2" ] || ! echo "$3  $2" | sha256sum -c --status; then
        curl -fsSL -o "$2.part" "$1"
        echo "$3  $2.part" | sha256sum -c --quiet
        mv "$2.part" "$2"
    fi
}

mkdir -p "$tools"
tool=$tools/appimagetool-$APPIMAGETOOL_VERSION
runtime=$tools/runtime-$RUNTIME_VERSION
fetch "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-x86_64.AppImage" \
    "$tool" "$APPIMAGETOOL_SHA256"
fetch "https://github.com/AppImage/type2-runtime/releases/download/$RUNTIME_VERSION/runtime-x86_64" \
    "$runtime" "$RUNTIME_SHA256"
chmod +x "$tool"

appdir=$(mktemp -d)
trap 'rm -rf "$appdir"' EXIT
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/$APP"
cp "$exe" "$appdir/usr/bin/$APP"
cp -r templates "$appdir/usr/share/$APP/"
cp assets/$APP.desktop "$appdir/"
cp assets/icon/png/$APP-256.png "$appdir/$APP.png"
ln -s usr/bin/$APP "$appdir/AppRun"

mkdir -p "$(dirname "$out")"
# Extracting instead of mounting: appimagetool is an AppImage itself, and CI runners have
# no FUSE.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$tool" --no-appstream --runtime-file "$runtime" \
    "$appdir" "$out"
