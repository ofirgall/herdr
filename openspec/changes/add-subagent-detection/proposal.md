# Proposal

## Why

Coding agents increasingly fan work out to subagents (Claude Code's Task/Agent tool, background agents), but Herdr only shows one row per pane. A pane can look "idle" or show a generic "working" while several background subagents are doing the real work, and the user has no way to see what they are, how many are running, or when they finish without switching into the pane.

## What Changes

- Herdr's server tracks the subagents of agent sessions it already knows about, starting with Claude Code. It reads the per-session `subagents/*.meta.json` files that Claude writes next to the session transcript that Herdr already receives through `pane.report_agent_session`.
- A filesystem watcher (new `notify` dependency) observes those directories and re-scans on change. No filesystem I/O happens in view computation or rendering.
- A subagent counts as running only when its metadata is not marked finished, the owning pane still has a live agent with the same session, and the subagent was spawned after Herdr observed the current session (re)start. Subagents from crashed or exited sessions disappear immediately. Finished subagents stay visible as "done" for a short linger window.
- Subagents are modeled as a tree (each has an optional parent subagent, resolved from the spawning `toolUseId`). A subagent whose owner can't be resolved attaches to the pane root.
- Subagents are exposed as shared runtime data: included in pane info returned by the JSON API, with a change event subscribers can listen to.
- The sidebar agents panel shows a count and expand/collapse chevron on agent rows that have subagents. Expanding shows nested subagent rows (type, description, running/done indicator). Clicking a subagent row focuses the parent pane. Expand/collapse state is TUI-only.
- A per-agent sidebar toggle switches the subagent list between the current view (default) and the full history of the session (including subagents finished before Herdr saw them and `interrupted` leftovers of earlier runs). History is also available through a `pane.subagents` API request with `scope: "history"`.
- Subagents are display-only. They never change the parent pane's detected agent state, which remains screen-detected.
- Claude Code's background shells (Bash tool calls with `run_in_background`) are tracked the same way, from the session transcript: running, done, failed, killed, or `interrupted`, with current and history lists, a separate API field and event, and their own "shells" group under the agent in the spaces sidebar.

## Capabilities

### New Capabilities
- `agents/subagent-tracking`: Server-side discovery and lifecycle of an agent session's subagents (source files, running/done/dropped rules, tree structure, linger window, API exposure).
- `sidebar/subagent-rows`: How the sidebar agents panel presents subagents under their parent agent row (count, collapse/expand, nested rows, click behavior).

### Modified Capabilities
<!-- None: no existing spec covers agent detection or the sidebar. -->

## Impact

- **Code**: new server-side subagent tracker module (watcher plus meta parsing); `TerminalState` (or adjacent runtime state) gains a subagent list; API schema (`PaneInfo`, new event kind); `src/ui/sidebar.rs` agent panel layout, row heights, scrolling and hit-testing; TUI input handling for the chevron.
- **Dependencies**: adds the `notify` crate (cross-platform filesystem events; tokio has no equivalent).
- **Protocol**: if subagent data crosses the server/client wire for the TUI, `PROTOCOL_VERSION` may need a bump under the project's protocol rules.
- **Performance**: the agent panel renders a pre-computed list only. The watcher's cost scales with the number of Claude panes, not with render frequency.
- **Coupling risk**: depends on Claude Code's undocumented on-disk format. A format change degrades to "no subagents shown" and must never cause errors or wrong parent state.
