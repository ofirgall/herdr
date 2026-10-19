## Context

See proposal.md for motivation. The current pipeline:

1. **Ghostty parser** (`ghostty/mod.rs`) captures `underline_color` from the PTY into a `CellColor`
2. **terminal.rs:2992** converts it to ratatui `Style.underline_color`
3. **wire.rs `CellData`** drops it — struct has only `symbol`, `fg`, `bg`, `modifier`, `skip`, `hyperlink`, and that layout is the frozen generation-1 endpoint cell codec
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

The field is in-memory only: `#[serde(skip)]` plus a hand-written bincode `Decode` that reads exactly the six generation-1 fields. The frozen endpoint codec (digests, fixtures, wire tags) stays byte-identical.

### Carry the color to clients through a negotiated capability

The client hello sets `surface_underline_color` and the welcome advertises the capability. For a negotiated client, a prepared render whose cells carry underline colors is sent as an `endpoint.surface-underline.v1` control message: the framed canonical message (full, reuse, delta, or scroll encoded) plus a sparse `(x, y, color)` map. A full surface's map is authoritative; a patch carries the whole post-patch map because scroll-encoded patches expand on the client beyond the changed cells. `surface_reuse::Decoder` applies the colors after the canonical decode and copies them back into its baseline. Renders without colors, peers without the capability, and wrappers over the frame limit all get the unchanged canonical message.

**Alternative: add the field to the v1 cell codec.** Rejected — upstream froze the generation-1 endpoint contract, so a field change would break every pinned external client.

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

- **Wire size** → Only renders with underline colors are wrapped; the map is sparse, though a patch lists the whole surface's colors and the inner message is base64 encoded.
- **Wire protocol compatibility** → The v1 codec is unchanged and the wrapper is only sent to clients that negotiated it, so mixed-version peers keep working (with plain underlines).
