> **Port note (upstream 143bf5fd):** mouse dispatch now lives in the client shell
> (`src/client/shell/mouse.rs`), and upstream already forwards `Down(MouseButton::Middle)`
> to a mouse-reporting pane there. On this branch the change only adds the pane focus:
> a forwarded middle-click down on an unfocused pane pushes a `pane.focus` endpoint
> request. References to `src/app/input/mouse.rs` below describe the original fork base.

## Why

Herdr forwards middle mouse **up** and **drag** events to the terminal pane, but drops the middle mouse **down** event. The `Down(MouseButton::Middle)` is used only to focus the pane (in `focus_pane_before_mouse_press`), but no arm in the main `handle_mouse` match dispatches it to the terminal. This means programs with mouse reporting enabled (vim, tmux, etc.) never receive the middle-click press, breaking middle-click paste (X11 primary selection) and any application behavior triggered by middle mouse down.

The popup pane path (`handle_popup_pane_mouse`) already forwards all `Down(_)` including middle — this is only a gap in the main mouse dispatch.

## What Changes

- Add a `Down(MouseButton::Middle)` arm in the main `handle_mouse` match in `src/app/input/mouse.rs` that forwards the event to the terminal pane, matching the existing pattern used for `Up(MouseButton::Middle)` and `Drag(MouseButton::Middle)`

## Capabilities

### New Capabilities
- `input/middle-mouse-forwarding`: Middle mouse down events are forwarded to the terminal pane when mouse reporting is active

### Modified Capabilities

## Impact

- `src/app/input/mouse.rs` — add match arm for `Down(MouseButton::Middle)` that forwards to the pane via `forward_pane_mouse_button`
