## 1. Wire Protocol

- [x] 1.1 Add an in-memory `underline_color: Option<u32>` field to `CellData` in `src/protocol/wire.rs` (`#[serde(skip)]`, hand-written bincode `Decode`). Populate it in `from_ratatui_cell()` and `cell_data_from_style()` using `color_to_u32()`. Verify: an RGB underline color round-trips through `color_to_u32`, and the v1 codec bytes are unchanged for colored cells.
- [x] 1.2 Add the negotiated `surface_underline_color` capability and the `endpoint.surface-underline.v1` wrapper (`src/protocol/surface_underline.rs`), wrap prepared renders at the server send sites, and apply the colors in `surface_reuse::Decoder`. Verify: colors survive full, reuse, delta, and scroll encodings; no wrapper without negotiation.

## 2. ANSI Rendering

- [x] 2.1 Add a helper `underline_color_to_sgr(val: u32) -> String` in `src/protocol/render_ansi.rs` that returns the SGR 58 fragment: `58:2::R:G:B` for RGB, `58:5:INDEX` for indexed, and the appropriate named-color form for named colors. Verify: unit test covers RGB, indexed, and named color inputs.
- [x] 2.2 Extend `build_sgr()` to accept `underline_color: Option<u32>` and append the SGR 58 fragment when `Some`. Verify: `build_sgr_produces_valid_sequence` and a new test confirm SGR 58 appears when underline_color is set and is absent when `None`.
- [x] 2.3 Update the `build_sgr` call site in `write_cell()` to pass `cell.underline_color`. Verify: the existing `blit_frame` tests pass.

## 3. Cell Diff

- [x] 3.1 Add `underline_color` comparison to `cells_equal()` and `cells_visually_equal()` in `src/protocol/render_ansi.rs`. Verify: a new test confirms cells with different underline colors are not considered equal.

## 4. Integration Verification

- [x] 4.1 Run the full test suite (`cargo nextest run`) and confirm all tests pass including the existing `render_preserves_underline_color` and `full_frame_preserves_curly_underline_style` tests in terminal.rs.
