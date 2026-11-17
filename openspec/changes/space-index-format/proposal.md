# Proposal

## Why

Herdr now draws space jump numbers itself with the `index` token, so they follow skipped spaces and the tenth-space `0`. Users who got their numbers from a rename plugin are used to styles like `1.` or `[1]`, but `index` renders only a bare digit in the generic secondary color. To replace the plugin's prefix, the token needs a configurable format, and themes need a way to color the number on their own.

## What Changes

- **`format` on the `index` token.** A styled `index` token in space rows accepts `format`, a short template where `{}` is the jump digit, for example `{token = "index", format = "[{}]"}` renders `[1]`. A plain `"index"` keeps rendering the bare digit. A space without a jump number still renders nothing, including the surrounding format text.
- **Validation.** `format` must contain exactly one `{}`, no other braces and no control characters, and be at most 16 characters. Other tokens reject `format` for now. An invalid value is a config error, like any other invalid sidebar token.
- **Theme color `index`.** The palette gains an optional `index` color, settable in `[theme.custom]` and `[theme.custom.light]` / `[theme.custom.dark]`. No built-in theme sets it, so the number keeps today's colors (mauve on the focused space, overlay0 otherwise) until a user sets it. When it is set, the number uses it on every row. A token's own `fg`, including from rules, still wins.

No breaking changes: existing configs and themes render exactly as before.

## Capabilities

### New Capabilities
- `sidebar/space-index-format`: the `format` key on the `index` space token, its validation, and how the formatted number is laid out and elided.
- `theme/index-color`: the optional `index` palette color, where it can be set, and its precedence against the default number color and token styles.

### Modified Capabilities
<!-- none -->

## Impact

- **Config:** sidebar token parsing and validation in `src/config/sidebar.rs` (the styled-token form gains `format`); theme overrides in `src/config/theme.rs` (`CustomThemeColors` and `ModeThemeColors` gain `index`).
- **Palette:** `src/app/state.rs` gains `Palette.index` and applies it in `with_overrides` / `with_mode_overrides`.
- **Rendering:** token resolution in `src/ui/sidebar/tokens.rs`, width and span drawing in `src/ui/sidebar.rs`, and the secondary-style choice in `src/client/shell/sidebar.rs`.
- **Docs:** `configuration.mdx` (the `index` token and the Theme section) and `config-reference.json`.
- **Not affected:** the server, API, wire protocol and snapshot. Format and color are TUI presentation only.
- **User setup (outside the repo):** to replace the rename plugin's numbers, turn off its workspace prefix and add a formatted `index` token to the space rows.
