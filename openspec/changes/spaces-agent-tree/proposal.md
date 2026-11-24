# Proposal

## Why

With `[ui.sidebar.spaces] agents` nesting agents under their spaces and `[ui.sidebar.agents] enabled = false` hiding the agents section, the space tree is the only agent list on screen. Agent numbers and agent keys do not follow it, though. A nested child's `agent_index` restarts at 1 in each space, and `focus_agent(N)`, `next_agent` and `previous_agent` still use the hidden agents section's order and sort. The number you see does not match the key that reaches it.

## What Changes

- **Tree numbering mode.** It is active when space agent children are enabled (`spaces.agents` is not `"off"`) and the agents section is disabled (`agents.enabled = false`). No new config option is added.
- **One numbering across the tree.** In tree numbering mode, every agent child shown in the space list gets a 1-based index counted down the whole list, in display order. Agents that are not shown get no index: a space whose children are collapsed, or a linked worktree hidden by a collapsed worktree group. A nested child's `agent_index` token renders this tree index instead of the per-space index.
- **Agent keys follow the tree.** In tree numbering mode, `focus_agent(N)`, `next_agent` and `previous_agent` target agents in tree order. They skip spaces, ignore agent-view filters and the agents-section sort, and wrap at both ends.
- **New `agent_index_focused` token.** It renders the agent's index only on the row of the focused agent, in both agent child rows and agents-section rows.
- **Unchanged outside tree numbering mode.** Outside that mode, and in the multi-machine sidebar, behavior is exactly as it is today.

## Capabilities

### New Capabilities
- `sidebar/space-agent-numbering`: when tree numbering mode is active, how agent children are numbered, and what the `agent_index` and `agent_index_focused` tokens render.
- `input/agent-navigation`: how `focus_agent(N)`, `next_agent` and `previous_agent` pick their target in tree numbering mode.

### Modified Capabilities
(none). The related `sidebar/space-agents` capability is still an unarchived change (`sidebar-space-agents-and-metadata`), so this change adds a new capability instead of modifying it.

## Impact

- `src/config/sidebar.rs`: add `AgentSidebarToken::AgentIndexFocused`.
- `src/ui/sidebar/tokens.rs` (or wherever agent tokens resolve on fork-v0.9.x): pass a focused flag to agent token resolution.
- `src/client/shell/space_agents.rs`: a shared tree-order function, child indexes from it, and agent-key targets from it.
- `src/client/shell/sidebar.rs`: use the tree order for rendering.
- `src/client/shell/agent_sidebar.rs`: pass the focused flag for agents-section rows.
- `src/client/shell/actions.rs`: `FocusAgent`, `NextAgent` and `PreviousAgent` in tree numbering mode.
- Docs in `docs/next` (configuration page, config reference, changelog).
- TUI client presentation only. No change to the server, API, wire protocol or persisted state.
