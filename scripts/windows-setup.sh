#!/bin/sh
# Builds the Windows installer (installer/artwork-creator.iss) on Linux: Inno Setup's
# compiler (ISCC.exe) runs with Wine in the amake/innosetup container image, with Docker
# or Podman ($CONTAINER, as `make check-dependencies`). Used by `make windows-setup`; the
# Release workflow runs Inno Setup 6 on Windows.
#
#   sh scripts/windows-setup.sh <artwork-creator.exe>
#
# Writes dist/artwork-creator-windows-setup.exe. The image is pinned by digest, so a
# newly pushed (possibly compromised) one is never pulled on its own; change it on purpose.
set -eu

IMAGE=docker.io/amake/innosetup:innosetup6-bookworm@sha256:699b24aacd485f31899daf478e1b84eb06a1f966259bf6def146b46d7d3a3617

exe=$1
container=${CONTAINER:-$(command -v docker >/dev/null 2>&1 && echo docker || echo podman)}

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
# A Windows path relative to installer/, where the script is.
exe_rel=$(echo "../$exe" | tr / '\\')

# The image runs ISCC as its user 1000; rootless Podman would map that user to another
# one, which can't write into dist/.
userns=
case $container in *podman*) userns=--userns=keep-id ;; esac

mkdir -p dist
# The repository is mounted at /work, the image's working folder. WINEDEBUG=-all hides
# Wine's complaints about having no display, which the compiler doesn't need.
"$container" run --rm -i $userns -e WINEDEBUG=-all -v "$PWD":/work:z "$IMAGE" \
    /Q "/DAppVersion=$version" "/DExe=$exe_rel" installer/artwork-creator.iss
echo "dist/artwork-creator-windows-setup.exe"
