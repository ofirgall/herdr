# Design

## Context

The sidebar is drawn entirely by the client from `ClientShellSnapshot` (`src/protocol/wire.rs`):

- `render_shell` (`src/client/shell/render.rs`) picks one of three paths: the multi-machine sidebar (`endpoint_sidebar.rs`), the collapsed sidebar (`render_collapsed_sidebar`), or the expanded sidebar (`render_sidebar`, `src/client/shell/sidebar.rs`).
- The expanded sidebar splits its height with `expanded_sidebar_sections` / `sidebar_section_heights` (`src/ui/sidebar.rs`). The spaces list sits on top, followed by the `new`/`menu` footer. `render_agent_panel` (`agent_sidebar.rs`) draws below that.
- Rows are built from token layouts (`src/config/sidebar.rs`). They are resolved by `agent_rows` / `space_rows` (`src/ui/sidebar/tokens.rs`) and fitted by `resolved_token_spans`.
- The spaces list computes per-entry heights and gaps and passes them to `scroll::list_scroll_metrics`. Mouse handling uses `ShellHitMap` entries (`WorkspaceHit` with an optional `group_toggle`, plus agent hits).
- Client chrome preferences (`preferences.rs`) already persist `collapsed_groups` per endpoint.

The snapshot already carries everything this change needs: `agents[]` (with `workspace_id`, `tab_id`, `agent_status`, `tokens`), `tabs[]` (with `label` and `number`), and `workspaces[]` (with `new_workspace_cwd`, `label` and `tokens`).

## Goals / Non-Goals

**Goals:**
- Every feature is client-only presentation, with no protocol, API or server changes.
- Reuse the existing token pipeline for child rows and static metadata rather than adding new rendering paths.
- With default config, rendered output stays byte-identical.

**Non-Goals:**
- Agent children in the collapsed or multi-machine sidebars.
- OSC 9;4 percentage progress. "Progress" here means agent status.
- Exposing static metadata through the API or persisting it server-side.
- A runtime keybinding to hide or show whole sections.

## Decisions

### 1. Section visibility as an `enabled` flag per existing section table
Add `enabled: bool` (default `true`) to `AgentsSidebarConfig` and `SpacesSidebarConfig`. Section geometry becomes a function of `(area, split, spaces_on, agents_on)`. With one section on, that section gets the full content rect and the divider rect is `Rect::default()`, so mouse dragging is disabled automatically. The persisted split is left untouched. When both flags are false, `ClientShellConfig` construction normalizes them back to true and reports a diagnostic through the existing sidebar-bounds diagnostic path.
- *Alternative:* a single `ui.sidebar.sections = ["spaces", "agents"]` list. It also allows reordering, but it adds a second way to express table-local settings and invites ordering requirements nobody asked for.

The `new`/`menu` footer moves into a helper that runs against whichever rect is the bottom of the sidebar. With spaces off, the agents list body loses one bottom row to the footer.

### 2. Children are part of a space's list item
The spaces loop already treats each entry as `rows.len()` rows. A space's item height becomes `space_rows + child_rows`, where `child_rows` is the sum of each expanded child's resolved row count. Scroll metrics, reveal-to-focused and drop indicators keep working without changes, because they operate on item heights. A new `WorkspaceAgentHit { rect, endpoint_id, pane_id }` is pushed per child. The mouse handler routes it to the same focus action the agents panel uses for its hits.
- *Alternative:* children as separate list items. This would let a scroll boundary separate a child from its space and break drag-reorder semantics. Rejected.

Child ordering is a client-side sort of `snapshot.agents.filter(workspace_id)` by (tab `number`, original snapshot order). The agent-view `agent_order` is deliberately ignored here.

### 3. Child rows reuse the agent token resolver
Add `agent_rows: AgentSidebarRows` (default `[[StateIcon, Tab]]`) to `SpacesSidebarConfig`, validated with `deserialize_sidebar_rows`. Children are resolved with the existing `tokens::agent_rows` by building an `AgentTokenContext` from the child's agent, exactly as `agent_sidebar.rs` does, with the state icon and text taken from that agent's own status. To avoid duplicating the per-agent token logic, the config passed in is a borrowed `AgentsSidebarConfig`-shaped view with `rows = agent_rows` and no `rows_by_agent`. The view is a small refactor that makes `agent_rows` take `&AgentSidebarRows` instead of the whole config. Children are rendered with a two-column indent (`"  "` plus the token spans). Their fitting budget is the space width minus that indent.
- *Alternative:* a new child-specific token set. It would duplicate the agent vocabulary for no gain.

### 4. Per-space toggle state
The config value `SpaceAgentsMode::{Off, Collapsed, Expanded}` is the default. Client state holds `space_agent_overrides: HashMap<ClientEndpointId, HashMap<String /*workspace_id*/, bool /*expanded*/>>`, which the toggle flips. Preferences gain `space_agent_overrides: Vec<{endpoint, workspace_id, expanded}>` with `#[serde(default)]`, so old preference files still load. Entries for workspace ids missing from the current snapshot are pruned when preferences are written.

The toggle glyph (`▾`/`▸`) is drawn at the right edge of the space's first row, the same slot `render_parent_group_toggle` uses for worktree groups. A space that is both a worktree group parent and has agents puts the agent toggle one column left of the group toggle. Its hit is added as `agent_toggle: Option<Rect>` on `WorkspaceHit`.

The context menu adds "Show agents" / "Hide agents" for spaces with agents when the mode is not `Off`.

### 5. Static metadata merged in the client before resolution
`SpacesSidebarConfig.metadata: Vec<SpaceMetadataEntry { matcher: Label(glob) | Path(PathBuf), tokens: Vec<(String, String)> }>`. It is validated at deserialize time; invalid entries are dropped and a diagnostic is reported (see Risks).

In `workspace_rows`, the token map is built as `static (in config order) ← snapshot workspace.tokens`, so API values win.
- Label glob: a minimal `*`/`?` matcher, implemented locally with no new dependency, case-sensitive, matched against `workspace.label`.
- Path prefix: `~` is expanded once at config load, then compared component-wise via `Path::starts_with` against `new_workspace_cwd`, which is the cwd the server already resolves for the workspace's active tab.

Key and value normalization reuses the API's rules. That function is private to `app::api_helpers`, so the rule is extracted into a shared helper (`crate::metadata_tokens` or `config::sidebar`) used by both callers.
- *Alternative:* push static metadata to the server as workspace tokens. That would leak presentation config into runtime state, contradicting the runtime/client boundary guardrail. Rejected.

### 6. Performance
Everything happens per rendered space and agent in client render, so cost scales with spaces × agents.
- The token map merge allocates once per visible space, as today.
- Glob and path matching per space per frame is bounded: at most 64 entries, and only for spaces actually iterated.
- Grouping agents by workspace builds one `HashMap` per frame rather than scanning `agents` once per space.

No server-side or per-PTY work is added.

## Risks / Trade-offs

- [Row counts change when agents appear or disappear, so the spaces list jumps] → Scroll state is already clamped by `list_scroll_metrics`, and reveal-on-focus keeps the focused space visible.
- [`new_workspace_cwd` follows the active tab, so a path match can change as the user switches tabs] → Documented as "matches the space's current working directory". This is acceptable for labels and avoids a wire change.
- [Diagnostics for invalid metadata entries: serde would reject the whole config on a bad entry] → Deserialize entries leniently into a raw form, then validate in `ClientShellConfig` construction, collecting per-entry diagnostics. This matches the requirement that only the bad entry is ignored.
- [Toggle glyph steals one column from the space's first row] → It is only drawn when the mode is not `Off` and the space has agents. Token fitting uses the reduced width.

## Migration Plan

No migration. All options default to current behavior. Preference files without `space_agent_overrides` load unchanged. Docs land in `docs/next/website/src/content/docs/configuration.mdx`.
