# Build Artwork Creator for Linux and Windows from Linux/WSL2.
#
# The Windows build is cross-compiled with mingw-w64 (linker set in .cargo/config.toml):
#   sudo apt install mingw-w64
#   rustup target add x86_64-pc-windows-gnu
#
# Common targets:
#   make              release builds for Linux and Windows
#   make linux        release build for Linux
#   make windows      release build for Windows (.exe)
#   make debug        debug builds for both
#   make run          run the Linux debug build
#   make run-windows  run the Windows debug build (from WSL)
#   make dev-setup    install what the quality checks need (rustfmt, clippy, the Windows
#                     target, llvm-tools and cargo-llvm-cov, rust-analyzer) and the hooks
#   make test         run the test suite
#   make coverage     run the tests with coverage; fails when new code (since main) is
#                     covered below MIN percent (80, the Sonar gate)
#   make hooks        run fmt, clippy, tests and coverage before each git commit
#   make pre-commit   run those checks now: clippy for Linux and Windows, tests and
#                     coverage on Linux;
#                     ARGS=--linux or ARGS=--windows checks one target only
#   make plugins      build the example plugins for Linux and Windows into plugins/
#   make linux-portable
#                     release build for Linux that runs on glibc GLIBC and later (older
#                     distributions too), with cargo-zigbuild: install zig
#                     (https://ziglang.org/download/) and cargo install cargo-zigbuild --locked
#   make dist         package each release build with its templates folder into dist/
#                     (the Linux one is linux-portable)
#   make deb          Linux .deb package into dist/, from linux-portable (needs cargo-deb:
#                     cargo install cargo-deb --locked)
#   make appimage     Linux AppImage into dist/, from linux-portable (see scripts/appimage.sh)
#   make windows-setup
#                     Windows installer into dist/, with Inno Setup in a Docker or
#                     Podman container (see scripts/windows-setup.sh)
#   make store-listing
#                     Microsoft Store listing import file installer/store/store-listing.csv,
#                     from the listing texts in installer/store/*.md (see scripts/store-listing.sh)
#   make version-check
#                     show the commits since the last release and suggest the next version
#   make version-bump-patch, version-bump-minor, version-bump-major
#                     set the version and commit "Bump version to X.Y.Z"; make version-bump
#                     APP=X.Y.Z sets an exact one (SDK=... sets the plugin SDK's too). A
#                     release is a vX.Y.Z tag pushed by hand: see doc/releasing.md
#   make check-dependencies
#                     scan Cargo.lock for known vulnerable or malicious crates with
#                     Trivy, run in a Docker or Podman container
#   make clean        remove build outputs

CARGO   ?= cargo
APP     := artwork-creator
LINUX   := x86_64-unknown-linux-gnu
WINDOWS := x86_64-pc-windows-gnu
DIST    := dist

# Oldest glibc the Linux packages run on (linux-portable); libc6 in Cargo.toml's deb
# depends must match. 2.28: Debian 10, Ubuntu 18.10, RHEL 8.
GLIBC   := 2.28

LINUX_BIN   := target/$(LINUX)/release/$(APP)
WINDOWS_BIN := target/$(WINDOWS)/release/$(APP).exe

# Example plugins (crates in plugin-examples/), built as .so and .dll into plugins/,
# where the app finds them when run from the repository.
PLUGINS     := halftone
PLUGINS_DIR := plugins

# Dependency scan (check-dependencies): Docker if installed, else Podman; CONTAINER=...
# picks one. The Trivy version is pinned so a freshly published (possibly compromised)
# image is never pulled on its own; raise TRIVY_VERSION on purpose. The vulnerability
# database is cached in TRIVY_CACHE between runs.
CONTAINER     ?= $(shell command -v docker >/dev/null 2>&1 && echo docker || echo podman)
TRIVY_VERSION ?= 0.76.1
TRIVY_IMAGE   ?= docker.io/aquasec/trivy:$(TRIVY_VERSION)
TRIVY_CACHE   ?= $(HOME)/.cache/trivy

# version-bump-patch, version-bump-minor, version-bump-major (see "Versioning" below).
VERSION_STEPS := $(foreach p,patch minor major,version-bump-$(p))

.PHONY: all linux linux-portable windows debug debug-linux debug-windows run run-windows \
        test coverage dev-setup check check-dependencies fmt clippy hooks pre-commit plugins dist version-check version-bump $(VERSION_STEPS) dist-linux dist-windows deb appimage windows-setup store-listing clean

all: linux windows

linux:
	$(CARGO) build --release --target $(LINUX)

# Same output path as linux, linked against an older glibc (see GLIBC).
linux-portable:
	$(CARGO) zigbuild --release --target $(LINUX).$(GLIBC)

windows:
	$(CARGO) build --release --target $(WINDOWS)

debug: debug-linux debug-windows

debug-linux:
	$(CARGO) build --target $(LINUX)

debug-windows:
	$(CARGO) build --target $(WINDOWS)

run:
	$(CARGO) run --target $(LINUX) -- $(ARGS)

run-windows: debug-windows
	./target/$(WINDOWS)/debug/$(APP).exe $(ARGS)

test:
	$(CARGO) test --workspace

# Coverage of new code, as the Sonar quality gate measures it (see scripts/coverage.sh).
MIN ?= 80
coverage:
	sh scripts/coverage.sh $(MIN)

# Everything the quality checks (make pre-commit) need. The Windows clippy check also
# needs mingw-w64 (sudo apt install mingw-w64), which this only checks for.
dev-setup: hooks
	rustup component add rustfmt clippy llvm-tools-preview rust-analyzer
	rustup target add $(WINDOWS)
	$(CARGO) install cargo-llvm-cov --locked
	@command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 || \
		echo "dev-setup: mingw-w64 is missing, needed for the Windows checks: sudo apt install mingw-w64"

plugins:
	$(CARGO) build --release --target $(LINUX) $(PLUGINS:%=-p %)
	$(CARGO) build --release --target $(WINDOWS) $(PLUGINS:%=-p %)
	mkdir -p $(PLUGINS_DIR)
	for p in $(PLUGINS); do \
		cp target/$(LINUX)/release/lib$$p.so target/$(WINDOWS)/release/$$p.dll $(PLUGINS_DIR)/; \
	done

# Git hooks are versioned in .githooks/; this points the local clone at them.
hooks:
	git config core.hooksPath .githooks

# The hook's checks, without committing. Options go through ARGS, e.g.
#   make pre-commit ARGS=--windows
pre-commit:
	sh .githooks/pre-commit $(ARGS)

check:
	$(CARGO) check --workspace --target $(LINUX)
	$(CARGO) check --workspace --target $(WINDOWS)

# Fails (exit code 1) when Trivy finds an advisory.
check-dependencies:
	$(CONTAINER) pull $(TRIVY_IMAGE)
	mkdir -p $(TRIVY_CACHE)
	$(CONTAINER) run --rm \
		-v "$(CURDIR)":/src:ro,z \
		-v "$(TRIVY_CACHE)":/root/.cache/trivy:z \
		$(TRIVY_IMAGE) fs --scanners vuln --skip-version-check --skip-dirs /src/target --exit-code 1 /src

fmt:
	$(CARGO) fmt

clippy:
	$(CARGO) clippy --workspace --all-targets --target $(LINUX) -- -D warnings
	$(CARGO) clippy --workspace --all-targets --target $(WINDOWS) -- -D warnings

# Versioning: see scripts/version.sh.
version-check:
	sh scripts/version.sh check

# version-bump-minor is version-bump APP=minor, and so on.
$(VERSION_STEPS): version-bump-%:
	sh scripts/version.sh bump $* $(if $(SDK),--sdk $(SDK))

version-bump:
	sh scripts/version.sh bump $(APP) $(if $(SDK),--sdk $(SDK))

# The app looks for templates/ next to the executable, so ship them together.
dist: dist-linux dist-windows

dist-linux: linux-portable
	rm -rf $(DIST)/$(APP)-linux
	mkdir -p $(DIST)/$(APP)-linux
	cp $(LINUX_BIN) $(DIST)/$(APP)-linux/
	cp -r templates $(DIST)/$(APP)-linux/
	# For desktop menus: see "Building" in README.md.
	cp assets/icon/png/$(APP)-256.png $(DIST)/$(APP)-linux/$(APP).png
	cp assets/$(APP).desktop $(DIST)/$(APP)-linux/
	tar -czf $(DIST)/$(APP)-linux.tar.gz -C $(DIST) $(APP)-linux

dist-windows: windows
	rm -rf $(DIST)/$(APP)-windows
	mkdir -p $(DIST)/$(APP)-windows
	cp $(WINDOWS_BIN) $(DIST)/$(APP)-windows/
	cp -r templates $(DIST)/$(APP)-windows/
	cd $(DIST) && rm -f $(APP)-windows.zip && \
		if command -v zip >/dev/null; then zip -qr $(APP)-windows.zip $(APP)-windows; \
		else echo "zip not installed; skipping $(APP)-windows.zip (folder is in $(DIST)/)"; fi

# Installers: the app finds its templates in ../share/artwork-creator/ from the executable.
deb: linux-portable
	mkdir -p $(DIST)
	$(CARGO) deb --no-build --target $(LINUX) -o $(DIST)/

appimage: linux-portable
	sh scripts/appimage.sh $(LINUX_BIN) $(DIST)/$(APP)-linux-x86_64.AppImage

windows-setup: windows
	sh scripts/windows-setup.sh $(WINDOWS_BIN)

store-listing:
	sh scripts/store-listing.sh

clean:
	$(CARGO) clean
	rm -rf $(DIST) $(PLUGINS_DIR)
