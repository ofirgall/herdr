## Why

Herdr parses underline color (SGR 58) from the PTY and stores it in the ratatui `Style`, but drops it during ANSI rendering because `CellData` has no field for it and `build_sgr()` never emits SGR 58. This means programs like neovim that use colored undercurls (e.g., red undercurl for diagnostics errors, yellow for warnings) render with unstyled/foreground-colored underlines instead.

## What Changes

- Add an in-memory `underline_color` field to `CellData`; the frozen generation-1 cell codec stays byte-identical
- Add the negotiated `surface_underline_color` endpoint capability so the color reaches the client shell without changing the v1 codec
- Emit SGR 58 (underline color) in `build_sgr()` during ANSI rendering
- Carry `underline_color` through `from_ratatui_cell` and any cell comparison/diff logic

## Capabilities

### New Capabilities
- `rendering/underline-color`: Underline color (SGR 58) is preserved through the wire protocol and emitted in ANSI output to the outer terminal

### Modified Capabilities

## Impact

- `src/protocol/wire.rs` — `CellData` gains an in-memory field (`#[serde(skip)]`, hand-written bincode `Decode`); v1 bytes unchanged
- `src/protocol/render_ansi.rs` — `build_sgr()` emits SGR 58; cell comparison includes the new field
- `src/pane/terminal.rs` — `cell_data_from_style` populates the new field
- `src/protocol/surface_underline.rs` (new), `surface_reuse::Decoder`, `endpoint.rs`, `client/` — negotiated `surface_underline_color` capability and `endpoint.surface-underline.v1` wrapper
- `src/server/render_stream.rs`, `src/server/headless/` — compute the color map per prepared render and wrap at the send sites
