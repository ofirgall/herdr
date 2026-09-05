## Context

See proposal.md for motivation. The current pipeline:

1. **Ghostty parser** (`ghostty/mod.rs`) captures `underline_color` from the PTY into a `CellColor`
2. **terminal.rs:2992** converts it to ratatui `Style.underline_color`
3. **wire.rs `CellData`** drops it — struct has only `symbol`, `fg`, `bg`, `modifier`, `skip`, `hyperlink`
4. **render_ansi.rs `build_sgr(fg, bg, modifier)`** never emits SGR 58

The underline color is already parsed and available in the ratatui `Cell` — it just needs to flow through `CellData` and into the ANSI output.

## Goals / Non-Goals

**Goals:**
- Underline color flows end-to-end from PTY parse to outer terminal
- Uses the same `u32` packing scheme as fg/bg for consistency

**Non-Goals:**
- Changing the Ghostty parser or terminal.rs (already works correctly)
- Supporting underline color independent of underline style (no underline = no underline color)

## Decisions

### Use `Option<u32>` for the underline_color field in CellData

Pack the color using the existing `color_to_u32()` function, same as fg/bg. Use `Option` because most cells have no underline color — `None` means "default/no override" and avoids emitting SGR 58 unnecessarily.

**Alternative: bare `u32` with sentinel value (e.g., 0 = no color).** Rejected because fg/bg already use 0x00000000 for `Color::Reset`, so 0 isn't a clean sentinel. `Option` is explicit and serializes cleanly with serde.

### Emit SGR 58 as a colon-separated sub-parameter

The underline color uses the modern SGR 58 colon syntax:
- RGB: `58:2::R:G:B`
- Indexed: `58:5:INDEX`
- Reset: `59`

This matches what terminals (Ghostty, kitty, WezTerm) expect and what programs (neovim) emit.

**Alternative: semicolon-separated `58;2;R;G;B`.** Rejected — the colon form is the standard per ECMA-48 sub-parameters and matches the undercurl style format (`4:3`).

### Add underline_color to `build_sgr` signature

Extend `build_sgr(fg, bg, modifier)` → `build_sgr(fg, bg, modifier, underline_color)`. The SGR 58 fragment is appended after modifiers and colors. When `underline_color` is `None`, nothing is emitted.

**Alternative: emit SGR 58 separately outside `build_sgr`.** Rejected — keeping all style in one SGR sequence avoids extra escape sequences and keeps the "has the style changed" comparison (`sgr != *last_sgr`) working correctly.

### Include underline_color in cell equality checks

Both `cells_equal()` and `cells_visually_equal()` must compare the new field so diff-based rendering re-draws cells when only the underline color changes.

## Risks / Trade-offs

- **Wire protocol size increase** → Each `CellData` gains an `Option<u32>` field. With serde serialization, `None` is compact. Most cells won't have underline color, so the size impact is minimal.
- **Wire protocol compatibility** → Adding a field is a breaking change for the serialization format. Server and client must be the same version. This is the existing constraint for herdr — no separate versioning needed.
