## Why

Herdr parses underline color (SGR 58) from the PTY and stores it in the ratatui `Style`, but drops it during ANSI rendering because `CellData` has no field for it and `build_sgr()` never emits SGR 58. This means programs like neovim that use colored undercurls (e.g., red undercurl for diagnostics errors, yellow for warnings) render with unstyled/foreground-colored underlines instead.

## What Changes

- Add an `underline_color` field to `CellData` in the wire protocol so the color survives serialization between server and client
- Emit SGR 58 (underline color) in `build_sgr()` during ANSI rendering
- Carry `underline_color` through `from_ratatui_cell` and any cell comparison/diff logic

## Capabilities

### New Capabilities
- `rendering/underline-color`: Underline color (SGR 58) is preserved through the wire protocol and emitted in ANSI output to the outer terminal

### Modified Capabilities

## Impact

- `src/protocol/wire.rs` — `CellData` struct gains a new field (wire protocol change)
- `src/protocol/render_ansi.rs` — `build_sgr()` emits SGR 58; cell comparison may need updating
- `src/pane/terminal.rs` — `from_ratatui_cell` (or equivalent) must populate the new field
- Wire protocol serialization format changes (existing clients/servers need matching versions)
