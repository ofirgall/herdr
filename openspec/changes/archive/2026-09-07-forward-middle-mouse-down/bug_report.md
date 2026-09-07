## Is this a reproducible bug?

- [x] I confirm this is a reproducible bug, not a feature request, idea, question, contribution proposal, or direction check.
- [x] I reproduced this bug on the version and environment reported below using the exact steps provided.

## Current behavior

Middle mouse down events are not forwarded to the terminal pane. The main `handle_mouse` match in `src/app/input/mouse.rs` has arms for `Up(MouseButton::Middle)` and `Drag(MouseButton::Middle)` but no arm for `Down(MouseButton::Middle)`. The down event only triggers pane focus via `focus_pane_before_mouse_press` and is then silently dropped.

Programs with mouse reporting enabled (vim, tmux, etc.) receive middle mouse up and drag but never the press event.

## Expected behavior

Middle mouse down events should be forwarded to the terminal pane the same way middle mouse up and drag already are, so applications with mouse reporting enabled receive the full middle-click sequence (down, optional drag, up).

## Reproduction

1. Open herdr with a pane running a program that enables mouse reporting (e.g. `vim`, `tmux`, or `cat -v` to see raw input)
2. Middle-click on the pane
3. Observe that the terminal application does not receive the middle mouse down event (only up is received)

## Impact

Middle-click paste (X11 primary selection) and any application behavior triggered by middle mouse press do not work inside herdr panes. The popup pane path (`handle_popup_pane_mouse`) already forwards all `Down(_)` correctly — this is only a gap in the main mouse dispatch.

## Environment

- Herdr version: master (commit 50033738)
- Update channel: n/a (source build)
- Operating system: macOS (Darwin 25.6.0)
- Terminal: n/a (herdr is the terminal)
