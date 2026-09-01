ZIG ?= /opt/homebrew/opt/zig@0.15/bin/zig

.PHONY: build install check test clean

build:
	ZIG=$(ZIG) cargo build --release

install:
	ZIG=$(ZIG) cargo install --locked --path .

check:
	ZIG=$(ZIG) cargo fmt --check
	ZIG=$(ZIG) cargo nextest run --locked --status-level fail --final-status-level fail --failure-output final --success-output never

test:
	ZIG=$(ZIG) cargo nextest run --locked --status-level fail --final-status-level fail --failure-output final --success-output never

clean:
	cargo clean
