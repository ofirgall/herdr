ifeq ($(OS),Windows_NT)
ZIG ?= zig
else
ZIG ?= /opt/homebrew/opt/zig@0.15/bin/zig
endif

.PHONY: setup build install run check test clean

setup:
ifeq ($(OS),Windows_NT)
	winget install zig.zig --version 0.15.2 --accept-source-agreements --accept-package-agreements
	rustup set default-host x86_64-pc-windows-msvc
	rustup toolchain install 1.96.1-x86_64-pc-windows-msvc
	rustup default 1.96.1-x86_64-pc-windows-msvc
else
	rustup toolchain install 1.96.1
endif

build:
	ZIG=$(ZIG) cargo build --release

install:
	ZIG=$(ZIG) cargo install --locked --path .

run:
	env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH ZIG=$(ZIG) cargo run -- $(ARGS)

check:
	ZIG=$(ZIG) cargo fmt --check
	ZIG=$(ZIG) cargo nextest run --locked --status-level fail --final-status-level fail --failure-output final --success-output never

test:
	ZIG=$(ZIG) cargo nextest run --locked --status-level fail --final-status-level fail --failure-output final --success-output never

clean:
	cargo clean
