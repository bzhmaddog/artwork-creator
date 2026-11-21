#!/bin/sh
# Lists the tools the Makefile targets need, what each is for, and whether it is installed,
# with how to install the missing ones. Used by `make check-tools` and `make dev-setup`.
#
#   sh scripts/check-tools.sh [--prerequisites]
#
# Fails when a tool for building and for the quality checks (make pre-commit) is missing;
# the packaging and dependency scan tools are only reported. With --prerequisites, only
# the tools make dev-setup can't install itself are checked: it runs that first, before
# installing anything.
set -u

prerequisites=0
[ "${1:-}" = --prerequisites ] && prerequisites=1

LINUX=x86_64-unknown-linux-gnu
WINDOWS=x86_64-pc-windows-gnu
missing=0

components=$(rustup component list --installed 2>/dev/null)
targets=$(rustup target list --installed 2>/dev/null)

# tool <required: 1|0> <name> <what for> <how to install> <check command...>
tool() {
    required=$1 name=$2 what=$3 how=$4
    shift 4
    if "$@" >/dev/null 2>&1; then
        printf '  ok       %-24s %s\n' "$name" "$what"
    else
        printf '  missing  %-24s %s\n           %-24s install: %s\n' "$name" "$what" "" "$how"
        [ "$required" = 1 ] && missing=$((missing + 1))
    fi
    return 0
}
has() { command -v "$1"; }
component() { printf '%s\n' "$components" | grep -q "^$1"; }
target() { printf '%s\n' "$targets" | grep -qx "$1"; }

echo "Required, to install before make dev-setup:"
tool 1 git "versioning, hooks, coverage of new code" "sudo apt install git" has git
tool 1 rustup "Rust toolchains, components, targets" "https://rustup.rs" has rustup
tool 1 cargo "builds, tests (make, make test)" "https://rustup.rs" has cargo
tool 1 mingw-w64 "Windows linker (make windows, pre-commit)" "sudo apt install mingw-w64" has x86_64-w64-mingw32-gcc
tool 1 python3 "script tests (make test)" "sudo apt install python3" has python3
tool 1 python3-venv "the .venv for Python's coverage" "sudo apt install python3-venv" python3 -c "import ensurepip"

if [ $prerequisites = 1 ]; then
    if [ $missing -gt 0 ]; then
        echo "check-tools: $missing required tool(s) missing; install them, then run make dev-setup again." >&2
        exit 1
    fi
    exit 0
fi

echo "Required, installed by make dev-setup:"
tool 1 rustfmt "formatting (make fmt, pre-commit)" "rustup component add rustfmt" component rustfmt
tool 1 clippy "lints (make clippy, pre-commit)" "rustup component add clippy" component clippy
tool 1 llvm-tools "coverage (make coverage)" "rustup component add llvm-tools-preview" component llvm-tools
tool 1 rust-analyzer "code completion in editors" "rustup component add rust-analyzer" component rust-analyzer
tool 1 "Linux target" "Linux builds (make linux)" "rustup target add $LINUX" target $LINUX
tool 1 "Windows target" "Windows builds and checks (make windows)" "rustup target add $WINDOWS" target $WINDOWS
tool 1 cargo-llvm-cov "coverage (make coverage, pre-commit)" "cargo install cargo-llvm-cov --locked" cargo llvm-cov --version
tool 1 "coverage in .venv" "script coverage (make coverage)" "make .venv/bin/coverage" .venv/bin/python3 -m coverage --version

echo "Optional, for packaging and the dependency scan:"
tool 0 zig "old-glibc Linux build (make linux-portable)" "https://ziglang.org/download/" has zig
tool 0 cargo-zigbuild "old-glibc Linux build (make linux-portable)" "cargo install cargo-zigbuild --locked" cargo zigbuild --version
tool 0 cargo-deb "Debian package (make deb)" "cargo install cargo-deb --locked" cargo deb --version
tool 0 zip "Windows .zip (make dist)" "sudo apt install zip" has zip
tool 0 curl "AppImage tools download (make appimage)" "sudo apt install curl" has curl
tool 0 "Docker or Podman" "make windows-setup, make check-dependencies" "install Docker or Podman" sh -c "command -v docker || command -v podman"

if [ $missing -gt 0 ]; then
    echo "check-tools: $missing required tool(s) missing." >&2
    exit 1
fi
echo "check-tools: all required tools are installed."
