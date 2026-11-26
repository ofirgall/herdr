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
- Base is `fork-v0.9.x`, where the shell is rendered in the client (`src/client/shell/*`). The client is fed a JSON wire snapshot (`ClientShellSnapshot`, `src/protocol/wire.rs`) built by `src/server/client_shell.rs`, and owns presentation state (expand overrides, scroll, hit map) in `ClientShellState`. Agents appear in the spaces sidebar as children of their space (`src/client/shell/space_agents.rs`), one entry per tab agent.
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

### D6. Spaces sidebar rendering and input (client shell)
- Wire: `ClientShellAgent` gains `subagents` and `subagent_history` (`Vec<ClientShellSubagent { id, kind, description, parent_id, background, state }>`), both `#[serde(default, skip_serializing_if = "Vec::is_empty")]` like `token_links`, filled by `src/server/client_shell.rs` from `TerminalState` (no I/O). A subagent list change must mark the client-shell snapshot as changed.
- Placement: the spaces tree (`space_agents.rs`). Each agent child whose history is non-empty gets a `▸`/`▾` chevron plus running count at the right edge of its first row (like `render_agent_toggle` for spaces). Expanded, a mode row (`current · history`) and the active list's subagent rows are drawn beneath the agent child, one tree level deeper, using the existing tree glyphs/connectors, a state icon (running = working style, done = done style, interrupted = distinct `!` style), then the truncated description followed by the dimmed kind (description first because the sidebar is narrow). `SpaceAgentChild::height()` includes these rows so layout, scrolling, and hit-testing stay consistent. The flat agents panel is unchanged.
- State: `ClientShellState` keeps per-endpoint sets of expanded panes and history-mode panes (not persisted in preferences, pruned when the pane disappears from the snapshot).
- Hits: `ShellHitMap` gets subagent chevron and mode-row hit lists; subagent rows push the parent pane into `space_agents` so clicking focuses the parent pane. Subagent rows are not keyboard-navigable sidebar entries and don't change agent numbering.

### D7. Protocol
The client-shell snapshot is JSON; the new `ClientShellAgent` fields are optional with serde defaults (the established `token_links` pattern), so old/new peers interoperate and `PROTOCOL_VERSION` is not bumped.

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
