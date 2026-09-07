## 1. Forward Middle Mouse Down

- [x] 1.1 Add a `Down(MouseButton::Middle)` arm in the main `match mouse.kind` block in `src/app/input/mouse.rs`, adjacent to the existing `Up(Middle) | Drag(Middle)` arm at line 930. Guard with `!in_sidebar`. Forward via `forward_pane_mouse_button` and return `mouse_pane_focus_action(info.id)` when the forward succeeds. Verify: compile succeeds, existing tests pass.

## 2. Testing

- [x] 2.1 Add a unit test in `src/app/input/mouse.rs` (in the existing `#[cfg(test)] mod tests`) that verifies `Down(MouseButton::Middle)` on a pane area is forwarded to the pane when mouse reporting is active. Use the existing test helpers (`AppState::test_new()`, mouse event construction). Verify: `just test` passes.

## 3. Validation

- [x] 3.1 Run `just check` and confirm all tests, formatting, and maintenance checks pass.
