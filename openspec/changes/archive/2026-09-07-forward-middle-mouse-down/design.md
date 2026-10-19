> **Port note (upstream 143bf5fd):** mouse dispatch now lives in the client shell
> (`src/client/shell/mouse.rs`), and upstream already forwards `Down(MouseButton::Middle)`
> to a mouse-reporting pane there. On this branch the change only adds the pane focus:
> a forwarded middle-click down on an unfocused pane pushes a `pane.focus` endpoint
> request. References to `src/app/input/mouse.rs` below describe the original fork base.

## Context

See proposal.md for motivation. The current mouse dispatch in `handle_mouse` (`src/app/input/mouse.rs:224`) has these relevant arms:

- `Down(MouseButton::Left)` — line 225: handles selection, chrome presses, and forwards to the pane via `forward_pane_mouse_button` when the app has mouse reporting
- `Up(MouseButton::Middle) | Drag(MouseButton::Middle)` — line 930: forwards to the pane unconditionally (when not in sidebar)
- `Down(MouseButton::Middle)` — **missing**: no arm, falls through the match

The pane-level encoding already handles middle mouse down correctly (`src/pane/input.rs:112` maps it to `MOUSE_ACTION_PRESS` + `MOUSE_BUTTON_MIDDLE`). The popup path (`handle_popup_pane_mouse` at line 75) also forwards all `Down(_)` events. The gap is only in the main dispatch.

## Goals / Non-Goals

**Goals:**
- Middle mouse down reaches the terminal pane when the click lands on a pane outside the sidebar

**Non-Goals:**
- Adding middle-click semantics to chrome (sidebar, tab bar, workspace bar) — middle-click on chrome is intentionally a no-op
- Changing the existing pane focus behavior for middle-click (already works via `focus_pane_before_mouse_press`)

## Decisions

### Add a `Down(MouseButton::Middle)` arm adjacent to the existing middle mouse arms

Place the new arm immediately before the existing `Up(Middle) | Drag(Middle)` arm at line 930, using the same pattern: guard with `!in_sidebar`, find the pane target, and call `forward_pane_mouse_button`. Return a focus action when the pane is not already focused.

The arm should mirror the structure of the Up/Drag arm rather than the Left-click arm, because middle-click does not interact with selection, chrome press tracking, or drag state — it just needs to reach the terminal.

**Alternative: combine all three middle events into one arm.** Acceptable and slightly cleaner. The Down/Up/Drag all do the same thing (forward to pane), so they can share a single arm body.

### Return a focus action from the Down arm

When `forward_pane_mouse_button` returns true (the app consumed the event), return `mouse_pane_focus_action(info.id)` so the focused pane switches. This matches the Left-click behavior at line 640-643. The pane is already focused by `focus_pane_before_mouse_press` before `handle_mouse` runs, but the action propagates the focus through the runtime API for multi-client consistency.

When `forward_pane_mouse_button` returns false (app didn't want mouse events), do nothing — middle-click has no chrome semantics.

## Risks / Trade-offs

- **None material.** This is a one-arm addition to an existing match block, using an existing forwarding function that already handles middle mouse encoding. The pane encoding layer already supports middle down. No wire protocol, state, or API changes.
