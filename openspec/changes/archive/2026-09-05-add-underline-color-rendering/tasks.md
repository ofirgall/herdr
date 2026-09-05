## 1. Wire Protocol

- [x] 1.1 Add `underline_color: Option<u32>` field to `CellData` in `src/protocol/wire.rs`. Populate it in `from_ratatui_cell()` using the existing `color_to_u32()` on `cell.underline_color`. Verify: existing tests compile with the new field, and a new unit test confirms an RGB underline color round-trips through `color_to_u32`.

## 2. ANSI Rendering

- [x] 2.1 Add a helper `underline_color_to_sgr(val: u32) -> String` in `src/protocol/render_ansi.rs` that returns the SGR 58 fragment: `58:2::R:G:B` for RGB, `58:5:INDEX` for indexed, and the appropriate named-color form for named colors. Verify: unit test covers RGB, indexed, and named color inputs.
- [x] 2.2 Extend `build_sgr()` to accept `underline_color: Option<u32>` and append the SGR 58 fragment when `Some`. Verify: `build_sgr_produces_valid_sequence` and a new test confirm SGR 58 appears when underline_color is set and is absent when `None`.
- [x] 2.3 Update the `build_sgr` call site in `write_cell()` (~line 747) to pass `cell.underline_color`. Verify: the existing `blit_frame` tests pass.

## 3. Cell Diff

- [x] 3.1 Add `underline_color` comparison to `cells_equal()` and `cells_visually_equal()` in `src/protocol/render_ansi.rs`. Verify: a new test confirms cells with different underline colors are not considered equal.

## 4. Integration Verification

- [x] 4.1 Run the full test suite (`cargo test`) and confirm all tests pass including the existing `render_preserves_underline_color` and `dirty_patch_preserves_curly_underline_style` tests in terminal.rs.
