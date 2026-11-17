# Tasks

## 1. Section visibility

- [x] 1.1 Add `enabled: bool` (default `true`) to `AgentsSidebarConfig` and `SpacesSidebarConfig` in `src/config/sidebar.rs`, with serde round-trip; verify with unit tests for the defaults and for parsing `enabled = false`
- [x] 1.2 Normalize "both disabled" back to both enabled and surface a config diagnostic during `ClientShellConfig` construction / reload (`src/client/shell/config.rs`); verify with a test that asserts the diagnostic text and that both sections render
- [x] 1.3 Extend section geometry in `src/ui/sidebar.rs` (`expanded_sidebar_sections`, `sidebar_section_divider_rect`) to take section visibility, giving the full rect to the single enabled section and an empty divider rect; verify with geometry unit tests for each combination
- [x] 1.4 Update `render_sidebar` to skip the disabled section, move the `new`/`menu` footer into a helper drawn at the bottom of the sidebar, and keep the collapse toggle; verify with client shell render tests (agents-only and spaces-only) that check rows and hit rects
- [x] 1.5 Apply visibility to `render_collapsed_sidebar` (hide the workspace numbers or the agent area, drop the divider); verify with a collapsed-mode render test
- [x] 1.6 Confirm a saved section split survives disable/re-enable and that divider dragging is inert with one section; verify with a mouse test in `src/client/shell/tests/`
- [x] 1.7 Document `[ui.sidebar.spaces] enabled` and `[ui.sidebar.agents] enabled` in `docs/next/website/src/content/docs/configuration.mdx`; verify the TOML example parses in a config test

## 2. Agent children under spaces

- [x] 2.1 Add `SpaceAgentsMode { Off, Collapsed, Expanded }` as `[ui.sidebar.spaces] agents` (default `off`) and `agent_rows: AgentSidebarRows` (default `[["state_icon", "tab"]]`, same row/token limits) to `SpacesSidebarConfig`; verify with parse tests, including rejection of unknown modes and too many rows
- [x] 2.2 Refactor `tokens::agent_rows` to take a row set (`&AgentSidebarRows`) rather than the whole `AgentsSidebarConfig`, keeping `rows_by_agent` selection at the call site; verify that existing token tests pass unchanged
- [x] 2.3 Build a per-frame `workspace_id -> ordered agents` grouping (by tab number, then snapshot order, ignoring agent-view order) in `src/client/shell/sidebar.rs`; verify with a unit test using a snapshot whose agents are out of order
- [x] 2.4 Add per-space toggle state (`space_agent_overrides` keyed by endpoint + workspace id) to client state, and an effective-expanded helper combining config mode and override; verify with state unit tests
- [x] 2.5 Include child rows in each space's item height and render the children indented below the space rows, with each child's own state icon and text; verify with render tests showing a space with two agents in expanded mode and none in collapsed mode, and the space keeping its aggregate status
- [x] 2.6 Draw the `▾`/`▸` toggle on the first row of spaces with agents (offset when a worktree group toggle is present) and record `agent_toggle` on `WorkspaceHit`; verify with a render test asserting the glyph cell and hit rect
- [x] 2.7 Handle clicks: the toggle flips the override for that space only, and a child click focuses workspace/tab/pane via the existing agent-focus action; verify with mouse tests in `src/client/shell/tests/`
- [x] 2.8 Add "Show agents" / "Hide agents" to the space context menu when the mode is not `off` and the space has agents; verify with a context menu test
- [x] 2.9 Persist overrides in `ClientChromePreferences` (`#[serde(default)]`), prune ids missing from the snapshot when saving, and restore them on load; verify with preference round-trip tests, including loading an old preference JSON without the field
- [x] 2.10 Verify that scrolling and reveal-on-focus never split a space from its children, and that a collapsed worktree group hides members' children; verify with list scroll tests using tall children
- [x] 2.11 Document `agents` and `agent_rows` under `[ui.sidebar.spaces]` in `configuration.mdx`, including the toggle and context menu behavior; verify the documented TOML parses in a config test
- [x] 2.12 Add `[ui.sidebar.spaces] show_status` (default `true`) that hides space-row `state_icon`/`state_text` while children keep their own status; verify with `show_status_false_hides_space_status_tokens_only`, the config reference check and the docs entry

## 4. Integration

- [ ] 4.1 Confirm that default config renders byte-identically to before (existing sidebar render snapshot tests unchanged) and run `just check`
- [x] 4.2 Manually smoke-test with `env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH cargo run`: hide each section, enable agent children with two agents across tabs, toggle them, restart the client to confirm persistence, and record the results in the PR
