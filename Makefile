ifeq ($(OS),Windows_NT)
ZIG ?= zig
CONFIG_ROOT ?= $(or $(XDG_CONFIG_HOME),$(APPDATA))
else
ZIG ?= /opt/homebrew/opt/zig@0.16/bin/zig
CONFIG_ROOT ?= $(or $(XDG_CONFIG_HOME),$(HOME)/.config)
endif

# Debug builds keep their config, plugins, and server sockets in herdr-dev, so a
# debug server runs alongside the installed one.
STABLE_CONFIG_DIR ?= $(CONFIG_ROOT)/herdr
DEV_CONFIG_DIR ?= $(CONFIG_ROOT)/herdr-dev
# Kept short: server socket paths must fit in sockaddr_un (104 bytes on macOS).
CLEAN_RUN_DIR ?= $(CONFIG_ROOT)/herdr-clean-run
SHARED_PLUGIN_ENTRIES := plugins plugins.json .plugins.lock
RUN_ENV := env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_CONFIG_PATH ZIG=$(ZIG)
# `make run` reads the installed config, or config-dev.generated.toml next to it when
# present (installed config plus dev-only overrides, generated outside this repo).
DEV_CONFIG_FILE ?= $(firstword $(wildcard $(STABLE_CONFIG_DIR)/config-dev.generated.toml) $(STABLE_CONFIG_DIR)/config.toml)
RUN_SHARED := $(RUN_ENV) HERDR_CONFIG_PATH="$(DEV_CONFIG_FILE)" cargo run --

.PHONY: setup build install run run-restart run-dev-plugins run-clean-config share-plugins check test clean

setup:
ifeq ($(OS),Windows_NT)
	winget install zig.zig --version 0.16.0 --accept-source-agreements --accept-package-agreements
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

# Debug build using the installed config and plugins.
run: share-plugins
	$(RUN_SHARED) $(ARGS)

# Rebuild, stop the running dev server (if any), and start `make run` again.
# The build finishes before the old server stops.
run-restart:
	-$(RUN_SHARED) server stop
	@$(MAKE) --no-print-directory run

# `make run` with a dev-only plugins.json: plugin checkouts stay shared, but the
# registry is left alone, so dev can point a plugin at a local checkout.
run-dev-plugins:
	@$(MAKE) --no-print-directory run SHARED_PLUGIN_ENTRIES="plugins .plugins.lock"

# Debug build with default config and no plugins, in its own config/state dirs and
# server. Panes inherit the XDG overrides. Reset by deleting $(CLEAN_RUN_DIR).
run-clean-config:
	$(RUN_ENV) XDG_CONFIG_HOME="$(CLEAN_RUN_DIR)/config" XDG_STATE_HOME="$(CLEAN_RUN_DIR)/state" cargo run -- $(ARGS)

# Link the dev plugin registry, checkouts, and registry lock to the installed ones.
# Existing dev entries are moved aside once. Takes effect when the dev server starts.
share-plugins:
	@mkdir -p "$(DEV_CONFIG_DIR)"
	@linked=; \
	for entry in $(SHARED_PLUGIN_ENTRIES); do \
		link="$(DEV_CONFIG_DIR)/$$entry"; \
		[ -L "$$link" ] && continue; \
		if [ -e "$$link" ]; then \
			if [ -e "$$link.dev-backup" ]; then \
				echo "$$link and $$link.dev-backup both exist; remove one to share plugins" >&2; \
				exit 1; \
			fi; \
			mv "$$link" "$$link.dev-backup"; \
		fi; \
		MSYS=winsymlinks:nativestrict ln -s "$(STABLE_CONFIG_DIR)/$$entry" "$$link" || exit 1; \
		linked=1; \
	done; \
	if [ -n "$$linked" ] && [ -S "$(DEV_CONFIG_DIR)/herdr.sock" ]; then \
		echo "Plugins are now shared, but a running dev server still uses its old config and plugins." >&2; \
		echo "Restart it: make run-restart" >&2; \
		exit 1; \
	fi

check:
	ZIG=$(ZIG) cargo fmt --check
	ZIG=$(ZIG) cargo nextest run --locked --status-level fail --final-status-level fail --failure-output final --success-output never

test:
	ZIG=$(ZIG) cargo nextest run --locked --status-level fail --final-status-level fail --failure-output final --success-output never

clean:
	cargo clean
