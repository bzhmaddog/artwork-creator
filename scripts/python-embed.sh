#!/bin/sh
# Downloads python.org's embeddable Python for Windows, at a pinned version checked against
# its SHA-256, and unpacks it into target/python-embed with pip's wheel (pip.whl, also
# pinned, run from the plugin manager to install packages): the Python the Windows packages
# (the .zip, the setup .exe and the MSIX) ship in a python folder next to the app, for the
# Python plugins. Used by `make dist-windows`, `make windows-setup`, `make msix` and the
# Release workflow. Does nothing when that version is already there.
#
#   sh scripts/python-embed.sh
#
# Keep PYTHON_VERSION the Linux one (scripts/python-linux.sh). Raise PYTHON_VERSION and
# PYTHON_SHA256 together, on purpose: the SHA-256 is
# sha256sum's of python-X.Y.Z-embed-amd64.zip from https://www.python.org/downloads/windows/
# (python.org publishes only .sigstore files). PIP_URL and PIP_SHA256 likewise, from
# https://pypi.org/project/pip/#files (the py3-none-any wheel).
set -eu

PYTHON_VERSION=3.14.10
PYTHON_SHA256=90905e64a840557d8c1bc4f5b74db3d7ee85e3ffb217611cac1c7813acea4159
PIP_URL=https://files.pythonhosted.org/packages/07/25/ee8c38ae72d66b844ce8eb9711ba905dba21ca1a8a0c960057c41cd23337/pip-26.3-py3-none-any.whl
PIP_SHA256=f3b60b20ffaadd4415ed7957212a38ac63fa4c5fa375bd0f4a797777940fad13

die() {
    echo "python-embed: $*" >&2
    exit 1
}

[ -n "$PYTHON_SHA256" ] || die "set PYTHON_SHA256 in $0"

dir=target/python-embed
zip=python-$PYTHON_VERSION-embed-amd64.zip

stamp="$PYTHON_VERSION $PIP_SHA256"
if [ -f "$dir/python.exe" ] && [ "$(cat "$dir/.version" 2>/dev/null)" = "$stamp" ]; then
    echo "$dir"
    exit 0
fi

rm -rf "$dir" "target/$zip"
mkdir -p "$dir"
curl --proto '=https' --tlsv1.2 -fsSL -o "target/$zip" "https://www.python.org/ftp/python/$PYTHON_VERSION/$zip"
echo "$PYTHON_SHA256  target/$zip" | sha256sum -c - >/dev/null || die "$zip checksum mismatch"
# Git Bash (the Release workflow) may have no unzip: PowerShell's then.
if command -v unzip >/dev/null; then
    unzip -q "target/$zip" -d "$dir"
else
    powershell -NoProfile -Command "Expand-Archive -Path 'target/$zip' -DestinationPath '$dir'"
fi
rm "target/$zip"
curl --proto '=https' --tlsv1.2 -fsSL -o "$dir/pip.whl" "$PIP_URL"
echo "$PIP_SHA256  $dir/pip.whl" | sha256sum -c - >/dev/null || die "pip.whl checksum mismatch"
echo "$stamp" >"$dir/.version"
echo "$dir"
