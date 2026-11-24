# Design

## Context

On fork-v0.9.x the sidebar is drawn by the client shell from a `ClientShellSnapshot`.

- **Children.** `SpaceAgentIndex` (`src/client/shell/space_agents.rs`) groups agents by workspace in tab order. `children()` resolves child rows with a *per-space* `index` (from `enumerate()`).
- **Rendering.** `render_spaces` (`src/client/shell/sidebar.rs`) builds `workspace_entries(snapshot, collapsed_groups)` once per frame. Children appear for each entry whose space is expanded (`agent_mode.expanded(override)`).
- **Navigation.** `sidebar_entries()` already walks spaces and their shown agents in display order for `next/previous_sidebar_entry`. It uses `navigation_workspace_entries`, which expands every group on mobile, and it leaves children out on mobile.
- **Agent keys.** `FocusAgent` / `NextAgent` / `PreviousAgent` in `src/client/shell/actions.rs` use `agent_sidebar::ordered_agent_pane_ids(snapshot, agent_panel_sort)`, which is the agents-section order. They then adjust `agent_scroll` for the agents section.
- **Tokens.** Agent tokens resolve through `crate::ui::sidebar_agent_rows` with an `AgentTokenContext` that carries `agent_index` (`src/ui/sidebar/tokens.rs`).
- **Agent data.** `ClientShellAgent` has `focused`.

The requirements are in `specs/sidebar/space-agent-numbering` and `specs/input/agent-navigation`.

## Goals / Non-Goals

**Goals:**
- One function decides tree order. Rendering, `agent_index`, and the agent keys all use it.
- Everything is gated on tree numbering mode. Outside it, the existing code paths behave the same as today.

**Non-Goals:**
- New config options.
- Changes to `next/previous_sidebar_entry`.
- Changes to the multi-machine (endpoints) sidebar.
- Numbering on mobile.

## Decisions

### D1. Mode predicate
Add `ClientShellConfig::tree_agent_numbering(&self) -> bool`, true when `spaces.agents != Off && !agents.enabled`. Use the config fields as they exist on fork-v0.9.x. The single-machine condition comes from the call site: the multi-machine render path and the multi-machine action path (the code that uses `endpoint_agents` aggregate rows) never consult this predicate.

### D2. Shared tree order
Add `tree_agent_order(snapshot, entries: &[WorkspaceEntry], expanded: impl Fn(&str) -> bool, index: &SpaceAgentIndex) -> Vec<&str /* pane_id */>` in `space_agents.rs`. It walks `entries` in order and, for each expanded space, appends `index.agents(workspace_id)`.
- **Rendering** calls it with the same `entries` and `space_expanded` closure that `render_spaces` already builds, so the numbers match what is drawn by construction.
- **Actions** call it with `self.navigation_workspace_entries(snapshot)` on desktop. When mobile is active, actions must use the desktop collapse-aware entries (`workspace_entries` with the endpoint's collapsed groups), because the mobile layout shows no children but the keys should still follow the desktop tree. If that turns out to be awkward, follow the spec: targets are the agents the expanded desktop tree would show.
- Alternative considered: reuse `sidebar_entries()` and filter to `SidebarEntry::Agent`. That would be simpler, but it returns no agents on mobile and is coupled to the sidebar-entry cursor semantics. Factor both onto D2's function if it is clean to do so.

### D3. Child index offset
Change `SpaceAgentIndex::children(workspace, config)` to take the index of its first child, `first_index: usize`, and to use `first_index + i` for the `agent_index` it passes.
- In tree numbering mode, `render_spaces` passes the running count of children already emitted for earlier expanded entries.
- Otherwise it passes 0, which is the per-space numbering used today.

The running count follows from the existing `children` vector built in entry order. This needs no second pass.

### D4. Focused flag on tokens
Add `focused: bool` to `AgentTokenContext`. `AgentSidebarToken::AgentIndexFocused` resolves like `AgentIndex` when `focused` is true and to `None` otherwise, so it is elided with its separator as missing tokens are. Callers pass `agent.focused`. That covers `resolve_agent_rows`, and through it `children()`, `agent_row()` and `endpoint_agents`. Add the parse and serialize entries `"agent_index_focused"` next to `"agent_index"` in `src/config/sidebar.rs`.

### D5. Agent keys
In `actions.rs`, when `self.config.tree_agent_numbering()` holds and the sidebar is single-machine, build the targets with D2 instead of `ordered_agent_pane_ids`.
- `FocusAgent(i)` takes `targets.get(i)`.
- `Next`/`Previous` reuse the existing wrap logic over `targets`.
- Do not touch `agent_scroll`; the agents section is hidden in this mode. Reveal the target child through the existing space-list reveal-on-focus behavior, per the `sidebar/space-agents` requirement "Children participate in list layout". Verify it applies to child rows and not just space rows. If it does not, add a reveal for the target pane's child row through the same mechanism that `step_sidebar_entry` uses.

## Risks / Trade-offs

- [The render and action paths compute entries differently, for example mobile vs desktop, so the key number could mismatch the drawn number] → Both go through D2. A test asserts that the render-side indexes equal the action-side targets for the same state.
- [Tree index changes while a request is in flight] → Acceptable, because the targets are recomputed from the snapshot on each key press.
- [Adding `focused` to `AgentTokenContext` touches every caller] → It is a mechanical change, and the compiler finds every caller.

## Migration Plan

Opt-in by configuration that already exists. Users who keep the agents section enabled see no change.
