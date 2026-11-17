# Proposal

## Why

The expanded sidebar always shows two fixed sections (spaces and agents). Agents live in their own list, separate from the space they belong to. Users who think in spaces cannot see "which agents are running in this space, and in what state" at a glance. Users who only care about one section cannot reclaim the space taken by the other.

## What Changes

- **Hide sections.** New config switches `[ui.sidebar.spaces] enabled` and `[ui.sidebar.agents] enabled` (both default `true`). When one section is disabled, the other fills the whole sidebar height. The section divider and split drag go away. The footer (`new` / `menu`) and the collapse toggle stay available. Disabling both is rejected as a config error and the defaults are kept.
- **Agents as children of spaces.** New `[ui.sidebar.spaces] agents = "off" | "collapsed" | "expanded"` (default `"off"`, so today's sidebar is unchanged). When enabled, each space that has agents can show one indented child entry per agent. Child rows are configured with a new `[ui.sidebar.spaces] agent_rows` token layout. It reuses the existing agent token vocabulary and defaults to `[["state_icon", "tab"]]`, i.e. the tab title plus that agent's individual status. Clicking a child focuses that agent's pane.
- **Per-space toggle for agent children.** The config value sets the default. Each space with agents also gets a runtime toggle (a click target on the space entry and a space context-menu item) that expands or collapses its children. Toggles persist in the client chrome preferences, keyed by workspace id.
- **Hide the space's aggregate status.** New `[ui.sidebar.spaces] show_status` (default `true`). When `false`, space rows drop `state_icon` and `state_text`; agent children keep their own status.

No breaking changes: every new option defaults to the current behavior.

## Capabilities

### New Capabilities
- `sidebar/sections`: which sidebar sections are shown, and how the layout behaves when only one section is shown.
- `sidebar/space-agents`: agents shown as children nested under their space, including child row composition, the per-space expand/collapse toggle and its persistence, and child interaction.

### Modified Capabilities
<!-- none: no existing spec covers the sidebar -->

## Impact

- Config: `src/config/sidebar.rs` (new fields, validation, (de)serialization), `src/config/model.rs` defaults/tests, settings documentation in `docs/next/website/src/content/docs/`.
- Client shell rendering: `src/client/shell/sidebar.rs` (spaces list height/scroll accounting with children, child rendering, toggle glyph), `src/ui/sidebar.rs` (section geometry for one section), `src/client/shell/agent_sidebar.rs` (agents section filling the full height), collapsed sidebar.
- Client input/state: `src/client/shell/mouse.rs`, `state.rs` (new hit types, per-space toggle state), `preferences.rs` (persisted toggles), `context_menu.rs`.
- Token resolution: `src/ui/sidebar/tokens.rs` (child rows reuse the agent resolver).
- No server, wire protocol, or API changes. Everything needed (agents per workspace, agent status, tab labels, workspace cwd) is already in `ClientShellSnapshot`.
