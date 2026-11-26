# Design

## Context

- Claude Code's integration is session-only: the `SessionStart` hook calls `pane.report_agent_session` with `agent_session_id` and `agent_session_path` (the transcript `.jsonl`). For Claude, `session_ref_from_report` (`src/agent_resume.rs`) keeps only the session **id** in `AgentSessionRef` and discards the path (only pi/omp keep paths). So the path must be retained separately (see D5). `TerminalState` (`src/terminal/state.rs`) already tracks `session_ref_changed` / `agent_released`. Lifecycle hooks (`SubagentStop` and others) were removed on purpose because they were unreliable. Agent state comes from screen detection.
- Claude Code writes, next to the transcript:
  ```
  <dir>/<session_id>.jsonl                      <- agent_session_path
  <dir>/<session_id>/subagents/agent-<id>.jsonl  (subagent transcript)
  <dir>/<session_id>/subagents/agent-<id>.meta.json
      {agentType, description, toolUseId, spawnDepth, requestShape,
       isBuiltIn, model, finished?}
  ```
  We checked 26 recent records on this machine: `finished: true` appears on completion, and the key is **absent** while the subagent is running. Every observed record has `spawnDepth: 1`, so nested subagents haven't been seen in practice.
- The TUI is drawn on the server from a shared `AppState` (`render_virtual_with_runtime_registry` in `src/server/headless.rs`), and frames are streamed to clients. Sidebar presentation state (for example `agent_panel_scroll`) lives in `AppState`. So subagent data reaches the TUI without a wire-protocol change.
- The agents panel builds `AgentPanelEntry` values per render (`collect_agent_panel_entries_with_runtimes` in `src/ui/sidebar.rs`), and input hit-testing re-derives row geometry in `src/app/input/sidebar.rs` using `agent_entry_height_in_body`. Render and hit-test geometry have to stay in sync.

## Goals / Non-Goals

**Goals:**
- Tracking that's cheap, event-driven, and adds nothing to the render, layout, or PTY hot paths.
- A data model general enough for future non-Claude sources (a source trait), even though only Claude is implemented.
- Running/done decisions that can't show phantom "running" subagents after crashes or resumes.

**Non-Goals:**
- Parsing subagent transcripts for progress, tool calls, or token usage.
- Hook-based or screen-based subagent detection for other agents (possible follow-ups).
- Opening or focusing a subagent's transcript. Clicking only focuses the parent pane.
- Changing the parent's agent state, notifications, or sounds.

## Decisions

### D1. Source: Claude's `subagents/*.meta.json`, behind a `SubagentSource` trait
The metadata file has exactly what the UI needs (type, description, finished, spawner tool-use id) in a small JSON document, so no transcript parsing is needed.
*Alternatives:* re-adding `SubagentStart/Stop` hooks (unreliable for Claude; already removed; needs users to reinstall the integration), screen manifest rules (count at best, depends on what's visible). A small trait (`fn subagent_dir(&AgentSessionRef) -> Option<PathBuf>`, `fn parse(...)`) keeps the Claude-specific path layout and JSON format in one module, so the rest of the system never depends on them.

### D2. Watching: `notify` crate, re-scan on any event
Use `notify` (recommended watcher; FSEvents on macOS, inotify on Linux, ReadDirectoryChangesW on Windows) on a dedicated background thread/task that sends results to the app event loop as `AppEvent::SubagentsScanned { pane_id, session_ref, records }`. Event payloads are treated only as "something changed in dir X". On each (debounced ~200 ms) event, re-read the directory's `*.meta.json` files. This tolerates merged, lost, or reordered events.
- Watch target: if `<session_id>/subagents/` exists, watch it. Otherwise watch `<session_id>/`, or the transcript's parent dir, non-recursively, and switch to the `subagents/` dir once it appears.
- Watches are registered and removed by the app reacting to session changes (`session_ref_changed`, `agent_released`, pane close). One watch per tracked Claude pane.
- Only `.meta.json` files are read. Subagent `.jsonl` writes don't trigger work: changes to those files are filtered out by extension before debouncing, so a busy subagent doesn't cause re-scans.
- Platform code: `notify` is cross-platform, so there's no `src/platform` split, unless Windows path quirks show up in testing.
*Alternative:* a tokio interval poll (no new dependency). Rejected by user preference for event-driven latency. A cheap safety-net re-scan (e.g. every 30 s for tracked dirs only) covers dropped events and is kept.

### D3. Running/done rules (pure function, unit-tested)
Reconciliation is a pure function over `(previous list, scanned records, pane facts, now)`:
1. `finished: true` → `done`.
2. No `finished` → `running`, only if:
   - the pane still has a live agent whose current `AgentSessionRef` matches the watched session (else the whole list is dropped immediately, with no linger), and
   - the record's file creation time (fallback: mtime) is ≥ `session_observed_at`, the time Herdr received the current `SessionStart` for this session ref. Older unfinished records are ignored (resume leftovers).
3. Records already `finished` the first time Herdr sees them are never listed. Only a `running → done` transition Herdr observes produces a lingering `done` entry.
4. `done` entries carry `done_at` and are pruned after 30 s. Pruning runs on a timer tick that already exists in the app loop, or on the next scan. It doesn't run in render.

`session_observed_at` comes from `AgentSessionReported` handling. With `session_start_source == "resume"` it still marks a new run, which is the whole point of rule 2's second condition.

*Considered:* a staleness timeout based on `.jsonl` mtime. It isn't needed because process liveness (rule 2) is authoritative: subagents run inside the parent process. Long-thinking subagents would also look stale with a timeout.
*Considered:* confirming completion via `<task-notification>` entries in the parent transcript. Deferred, since it means tailing a large `.jsonl`. Add it only if `finished` turns out to be written late in practice.

### D4. Tree via `toolUseId` attribution
Each record has `toolUseId` (the `tool_use` block that spawned it). On scan, for records with `spawnDepth > 1` only, search sibling `agent-*.jsonl` files for that id, using a bounded byte search with no JSON parsing and stopping at the first match. The containing file's agent id becomes the parent. Results are cached per `(subagent id)` because spawners never change. If there's no match, depth is 1, or the search fails, the subagent attaches to the root. Since depth ≥ 2 hasn't been seen, this path costs nothing today and only does I/O when nesting actually happens.

### D5. State placement and API
- Runtime: `TerminalState` gains `subagents: Vec<Subagent>`, `session_observed_at: Option<SystemTime>`, and `agent_session_transcript: Option<PathBuf>`. The last is the reported `agent_session_path`, threaded through `AgentSessionReported` and stored *separately* from `AgentSessionRef`, so resume/persist behavior is unchanged. It's runtime-only and not persisted, so after a Herdr restart subagents reappear only after the next `SessionStart` report. `session_observed_at == None` disables the resume-leftover filter (liveness still applies). `Subagent { id, kind, description, parent: Option<String>, background: bool, state: Running | Done { done_at } }`. This is shared runtime data, not UI data.
- API: `PaneInfo` gains `subagents` (omitted/empty when none). A new event kind `pane.subagents_changed { pane_id }` goes through the existing event hub and subscriptions. Names are neutral (no "sidebar"/"row"). API field and event names get added to `docs/next` API docs.
- Not persisted: subagents are re-derived after restore/handoff from the next scan, and the rules make stale data drop out.

### D6. Sidebar rendering and input
- `AgentPanelEntry` gains a borrowed or cloned summary: `subagent_running: usize`, `subagents: Vec<SubagentRow>` (flattened tree in display order with depth). These are built from already-computed state with no I/O. The clone is small and only happens for panes that have subagents.
- `AppState` gains `expanded_subagent_panes: HashSet<PaneId>` (presentation state, next to `agent_panel_scroll`), pruned when panes close.
- Row model: today every entry is one block of configured token rows. The panel's geometry functions (`agent_entry_height_in_body`, visible-count, scroll metrics, bottom-start) get extended so an entry's height = token rows + (expanded ? subagent rows : 0). Render and `src/app/input/sidebar.rs` hit-testing both call these same functions, so they stay in sync. Hit-testing returns either `AgentRow(pane)`, `Chevron(pane)`, or `SubagentRow(pane)`; `SubagentRow` focuses `pane`.
- Chevron + count go at the right edge of the entry's first row (styled like `workspace_group_chevron_rect`), reserving width before the configured tokens truncate.
- Subagent row: `"   ├ <kind>  <description…>  <state icon>"` using ASCII-safe tree glyphs consistent with existing sidebar glyph usage, `state_icon` styling (`Working` for running, idle/"done" style for done), indentation 2 cols per depth.

### D8. Session history view
Besides the current list (D3), the server keeps a per-pane **history** list: every parseable record of the watched session, rebuilt by a pure `reconcile_history` from the same scan (no extra I/O, no linger pruning). States: `finished` → `done`; unfinished and created at/after `session_observed_at` → `running`; unfinished and created before it (an earlier crashed/exited run of a resumed session) → `interrupted`. Liveness still applies: when the parent exits, is released, or switches session, history is cleared together with the current list. Ordered by creation time, tree attribution as in D4.
- API: `PaneInfo.subagents` stays the current list. A new request `pane.subagents { pane_id, scope: "current" | "history" }` (default `current`) returns either list; `interrupted` only appears in `history`. `pane.subagents_changed` fires when either list changes (payload unchanged: the current list).
- Sidebar: per-pane mode `current` (default) or `history`, stored in `AppState.subagent_history_panes: HashSet<PaneId>` next to `expanded_subagent_panes` (presentation state; not in the API, not persisted, pruned on pane close). The chevron appears when the history list is non-empty; the running count comes from the current list. An expanded entry renders a mode row (`current | history`, active one highlighted) above its subagent rows; clicking it flips that pane's mode. The mode row is part of the shared geometry (expanded height = 1 + rows of the active list). `interrupted` rows use a distinct muted/warning icon.

### D7. Protocol
No wire change is expected, because the TUI is drawn on the server. If any client-side code (e.g. mobile/remote) turns out to consume agent panel data over the wire, apply the `PROTOCOL_VERSION` rules then.

## Risks / Trade-offs

- [Claude changes its on-disk format or location] → Everything goes through `SubagentSource`. Parse failures yield "no subagents", never errors. A unit test pins the observed schema, so changes are caught by updating fixtures.
- [Running records without a live process (crash)] → Rule D3.2 drops them right away based on pane agent liveness, which Herdr already tracks.
- [File-creation time unavailable on some filesystems] → Fall back to mtime. A resume leftover with a new mtime could appear briefly. That's acceptable because liveness still applies.
- [FSEvents latency or merged events on macOS] → Re-scan the whole directory on each event, plus a 30 s safety-net re-scan.
- [Watch-handle limits (inotify `max_user_watches`) with many panes] → One non-recursive watch per Claude pane. Log and degrade (no subagents for that pane) if registration fails.
- [Render/hit-test geometry drifting apart] → Shared geometry functions, plus tests that click every rendered subagent row and the chevron at varied scroll offsets.
- [Hot-path cost] → Render only reads `Vec`s that were already computed. Profile with `just bench-render-scale` at 1 and 15+ panes, since this widens the agent-panel loop.
- [Many subagents in one pane (fan-out of 20+)] → Expanded rows scroll normally. Collapsed by default keeps the panel compact.

## Migration Plan

Additive only: no persisted-state or integration-version change, and existing Claude integrations already report `agent_session_path`. If something breaks, rollback is just removing the tracker. With no subagents, the sidebar falls back to today's rendering exactly.

## Open Questions

- Exact linger duration (30 s proposed) and whether it should be configurable. Tunable without spec changes.
- Whether to surface `model` / background flag in the subagent row, or only in the API.
