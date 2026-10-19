## 1. Forward Middle Mouse Down

- [x] 1.1 In the client shell's `Down(MouseButton::Middle)` arm (`src/client/shell/mouse.rs`), which already forwards the press to a mouse-reporting pane, push a `pane.focus` endpoint request when the hit pane is not the focused pane. Verify: compile succeeds, existing tests pass.

## 2. Testing

- [x] 2.1 Add `middle_click_down_is_forwarded_and_focuses_the_pane` in `src/client/shell/tests/mouse_selection.rs`: Down(Middle) on a mouse-reporting pane produces a targeted `ClientShellPaneInput`, an unfocused pane gets a PaneFocus request, and a focused pane does not. Verify: the test fails without 1.1 and passes with it.

## 3. Validation

- [x] 3.1 Run `just check` and confirm all tests, formatting, and maintenance checks pass.
