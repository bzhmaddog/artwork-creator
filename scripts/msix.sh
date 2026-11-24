#!/bin/sh
# Builds the Microsoft Store package (installer/msix/AppxManifest.xml) with makeappx and
# makepri from the Windows SDK: from WSL2 (the Windows SDK installed on the Windows side)
# or Git Bash. Used by `make msix` and the Release workflow, after scripts/python-embed.sh
# (the Python the package ships for the Python plugins, in target/python-embed).
#
#   sh scripts/msix.sh <artwork-creator.exe>
#
# Writes dist/artwork-creator.msix, unsigned: the Store signs the packages submitted to
# it, so it is for Partner Center only (Windows won't install it as it is).
# resources.pri tells Windows which icon of Assets/ to use at each size.
set -eu

die() {
    echo "msix: $*" >&2
    exit 1
}

exe=$1
manifest=installer/msix/AppxManifest.xml
stage=dist/msix
pri=dist/msix-pri
out=dist/artwork-creator.msix
python=target/python-embed

[ -f "$python/python.exe" ] || die "no $python: run sh scripts/python-embed.sh first"

# The newest Windows SDK's x64 tool, else the one in the PATH.
sdk_tool() {
    found=$(ls -d "/mnt/c/Program Files (x86)/Windows Kits/10/bin/"*/x64/"$1" \
        "/c/Program Files (x86)/Windows Kits/10/bin/"*/x64/"$1" 2>/dev/null | sort -V | tail -n 1)
    [ -n "$found" ] || found=$(command -v "$1") || die "$1 not found: install the Windows SDK"
    echo "$found"
}

# Paths as the Windows tools take them.
winpath() {
    if command -v wslpath >/dev/null; then
        wslpath -w "$1"
    elif command -v cygpath >/dev/null; then
        cygpath -w "$1"
    else
        echo "$1"
    fi
}

makeappx=$(sdk_tool makeappx.exe)
makepri=$(sdk_tool makepri.exe)
# Git Bash would turn the tools' /options into paths.
export MSYS2_ARG_CONV_EXCL='*'

# The Store wants 4 numbers, the last one 0.
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1).0

rm -rf "$stage" "$pri"
mkdir -p "$stage" "$pri"
cp "$exe" "$stage/artwork-creator.exe"
cp -r templates installer/msix/Assets "$stage/"
cp -r "$python" "$stage/python"
rm "$stage/python/.version"
sed "s/@VERSION@/$version/" "$manifest" >"$stage/AppxManifest.xml"

# The resources index is made from the icons only (not the templates).
cp -r installer/msix/Assets "$pri/"
"$makepri" createconfig /cf "$(winpath "$pri/priconfig.xml")" /dq en-US_fr-FR /o >/dev/null
"$makepri" new /pr "$(winpath "$pri")" /cf "$(winpath "$pri/priconfig.xml")" \
    /mn "$(winpath "$stage/AppxManifest.xml")" /of "$(winpath "$stage/resources.pri")" /o >/dev/null

"$makeappx" pack /d "$(winpath "$stage")" /p "$(winpath "$out")" /o >/dev/null
rm -rf "$stage" "$pri"
echo "$out"
