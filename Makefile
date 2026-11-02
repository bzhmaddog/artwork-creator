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
#   make test         run the test suite
#   make hooks        run fmt, clippy and tests before each git commit
#   make dist         package each release build with its templates folder into dist/
#   make clean        remove build outputs

CARGO   ?= cargo
APP     := artwork-creator
LINUX   := x86_64-unknown-linux-gnu
WINDOWS := x86_64-pc-windows-gnu
DIST    := dist

LINUX_BIN   := target/$(LINUX)/release/$(APP)
WINDOWS_BIN := target/$(WINDOWS)/release/$(APP).exe

.PHONY: all linux windows debug debug-linux debug-windows run run-windows \
        test check fmt clippy hooks dist dist-linux dist-windows clean

all: linux windows

linux:
	$(CARGO) build --release --target $(LINUX)

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
	$(CARGO) test

# Git hooks are versioned in .githooks/; this points the local clone at them.
hooks:
	git config core.hooksPath .githooks

check:
	$(CARGO) check --target $(LINUX)
	$(CARGO) check --target $(WINDOWS)

fmt:
	$(CARGO) fmt

clippy:
	$(CARGO) clippy --target $(LINUX) -- -D warnings
	$(CARGO) clippy --target $(WINDOWS) -- -D warnings

# The app looks for templates/ next to the executable, so ship them together.
dist: dist-linux dist-windows

dist-linux: linux
	rm -rf $(DIST)/$(APP)-linux
	mkdir -p $(DIST)/$(APP)-linux
	cp $(LINUX_BIN) $(DIST)/$(APP)-linux/
	cp -r templates $(DIST)/$(APP)-linux/
	tar -czf $(DIST)/$(APP)-linux.tar.gz -C $(DIST) $(APP)-linux

dist-windows: windows
	rm -rf $(DIST)/$(APP)-windows
	mkdir -p $(DIST)/$(APP)-windows
	cp $(WINDOWS_BIN) $(DIST)/$(APP)-windows/
	cp -r templates $(DIST)/$(APP)-windows/
	cd $(DIST) && rm -f $(APP)-windows.zip && \
		if command -v zip >/dev/null; then zip -qr $(APP)-windows.zip $(APP)-windows; \
		else echo "zip not installed; skipping $(APP)-windows.zip (folder is in $(DIST)/)"; fi

clean:
	$(CARGO) clean
	rm -rf $(DIST)
