# Tasks

## 1. Subagent model and Claude source

- [x] 1.1 Add the `notify` dependency to `Cargo.toml` (with a comment saying why) and verify `cargo build` succeeds on macOS and Linux targets
- [x] 1.2 Add a subagent module with the `Subagent` type (`id`, `kind`, `description`, `parent`, `background`, `state: Running | Done { done_at }`) and a `SubagentSource` trait; verify it compiles with no `#[cfg(target_os)]` in the module
- [x] 1.3 Implement the Claude source: resolve `<session_id>/subagents/` from an `AgentSessionRef` path, parse `agent-*.meta.json` (missing `finished` = unfinished, `requestShape == "background"` = background), skip malformed or partial files; verify with unit tests using fixture JSON for finished, unfinished, malformed, truncated, and unknown-extra-field records

## 2. Lifecycle rules

- [x] 2.1 Implement the pure reconcile function from design D3 (finished → done, liveness + session-ref match, `session_observed_at` scoping, finished-at-first-sight hidden, 30 s done linger pruning); verify unit tests cover every scenario in `specs/agents/subagent-tracking` for "done", "linger", "live parent", and "session run" requirements
- [x] 2.2 Implement `toolUseId` parent attribution for `spawnDepth > 1` (bounded byte search of sibling `agent-*.jsonl`, cached per subagent id, root fallback); verify unit tests with a fixture directory containing a nested spawn, an unattributable spawn, and depth-1 records (which must not trigger any file search)

## 3. Runtime state and watcher

- [x] 3.1 Add `subagents`, `session_observed_at`, and `agent_session_transcript` to `TerminalState`; thread the reported `agent_session_path` through `AgentSessionReported` (without changing `session_ref_from_report`) and set it plus `session_observed_at` when handling the event; clear subagents when `session_ref_changed`, `agent_released`, or the pane closes; verify with `AppState::test_new()` tests that a session change or release empties the list right away
- [x] 3.2 Implement the background watcher (design D2): one non-recursive watch per tracked Claude pane, upgrading from the session dir to `subagents/` when it appears, `.meta.json`-only filtering, ~200 ms debounce, full dir re-scan, 30 s safety-net re-scan, and results sent as `AppEvent::SubagentsScanned`; verify with a temp-dir integration test that creating, finishing, and deleting meta files produces the expected events, including when `subagents/` is created after watching starts
- [x] 3.3 Wire watch registration and removal to session reported / changed / released / pane close and handle `SubagentsScanned` through the reconcile function (dropping results whose `session_ref` no longer matches); verify tests confirm a stale scan for an old session is ignored and that watch registration failure logs and leaves the list empty
- [x] 3.4 Run linger pruning from an existing app timer tick (not render); verify a test that advances time past the linger window removes `done` entries and emits a change

## 4. JSON API

- [x] 4.1 Add `subagents` to `PaneInfo` (id, kind, description, parent, background, state) and a `pane.subagents_changed` event through the event hub/subscriptions; regenerate or update the API schema and verify API tests for pane read and subscription delivery
- [x] 4.2 Document the new `PaneInfo.subagents` field and event in `docs/next/website/src/content/docs/` and add a `docs/next/CHANGELOG.md` entry; verify `node website/scripts/docs-preview.mjs check` (or the relevant docs check) passes

## 5. Sidebar rendering and input

- [x] 5.1 Add `expanded_subagent_panes` presentation state to `AppState` (pruned on pane close, not persisted) and extend `AgentPanelEntry` with the running count and flattened subagent rows (display order + depth) built without I/O; verify unit tests for flattening a tree
- [x] 5.2 Extend the agent-panel geometry functions (`agent_entry_height_in_body`, visible count, scroll metrics, bottom start) to include expanded subagent rows; verify unit tests for heights and scroll metrics with collapsed and expanded entries
- [x] 5.3 Render the chevron + running count on the first row of entries that have subagents, and render subagent rows (indent per depth, kind, truncated description, running/done state icon); verify render tests on a test buffer for collapsed, expanded, nested, done, and long-description cases, and that entries without subagents render byte-identical to before
- [x] 5.4 Extend hit-testing in `src/app/input/sidebar.rs` with `Chevron(pane)` (toggle expand) and `SubagentRow(pane)` (focus parent pane), using the shared geometry; verify tests that click the chevron and every rendered subagent row at several scroll offsets and after a subagent is added/removed while expanded

## 6. Integration checks

- [x] 6.1 Profile the agent-panel change with `just bench-render-scale` at 1 and 15+ panes and record the scaling delta in the PR description
- [x] 6.2 Manual end-to-end: in a throwaway Herdr session (`env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH cargo run -- ...`) run Claude Code, spawn foreground and background subagents, and confirm rows appear, show done, disappear after linger, and vanish right away when Claude is killed; confirm `pane.get` shows the same data
- [ ] 6.3 Run `just check` and verify it passes

## 7. Session history mode

- [x] 7.1 Add `reconcile_history` (pure; `done` / `running` / `interrupted` per D8) and a history list on `TerminalState` updated from the same scan and cleared with the current list; verify unit tests for historical finished, resume-leftover interrupted, linger-expired-still-in-history, and parent-dead-clears
- [x] 7.2 Add the `pane.subagents` request (`scope: current | history`, default current) with `interrupted` state in the schema; emit `pane.subagents_changed` when either list changes; regenerate the API schema, update `docs/next` socket API docs and changelog entry; verify API tests for both scopes and the default
- [x] 7.3 Add `subagent_history_panes` presentation state (pruned on pane close, not persisted); chevron shown when history is non-empty; expanded entries render a mode row plus the active list's rows; include the mode row in shared geometry; verify render tests (current/history, empty current with history, interrupted icon) and that panes without history render byte-identical
- [x] 7.4 Hit-test the mode row (`SubagentModeToggle(pane)`) to flip only that pane's mode; verify click tests at several scroll offsets and that subagent-row clicks still focus the parent
- [x] 7.5 Re-run the e2e check for history mode (resume a session with prior subagents, confirm `pane.subagents scope=history` lists them while `pane.get` does not) and the full test/lint comparison against baseline

## 8. Port to fork-v0.9.x (client-rendered shell)

- [x] 8.1 Rebase onto `fork-v0.9.x`, keeping the server/runtime/API work and dropping the server-rendered sidebar code (removed `AppState` expand/mode state); verify all subagent tests pass
- [x] 8.2 Add `subagents` / `subagent_history` to `ClientShellAgent` (optional serde fields), fill them in `src/server/client_shell.rs`, and make subagent changes refresh the client snapshot; verify wire round-trip tests (field omitted when empty, old payload decodes) and a server projection test
- [x] 8.3 Render chevron + running count, mode row, and subagent rows under agent children in the spaces tree with expand/mode state in `ClientShellState`; verify render tests (collapsed, expanded current/history, empty current with history, interrupted icon, nested, long description) and that agents without history render byte-identical
- [x] 8.4 Hit-test chevron, mode row, and subagent rows (focus parent pane) via `ShellHitMap`; verify click tests including after scrolling and after the list changes
- [x] 8.5 Live check in a throwaway session on the new base and full test/clippy comparison against plain `fork-v0.9.x`

## 9. Mode icon and default expanded

- [x] 9.1 Replace the mode row with a history-mode icon left of the chevron (hit-tested, toggles only that agent's mode) and make subagent rows expanded by default (track collapsed panes instead of expanded); verify render/click tests and a live client check

## 10. Timing, theme tokens, keybinding

- [x] 10.1 Record `started_at` / `ended_at` per D9 in the Claude source; carry them through `Subagent`, `PaneSubagentInfo` (`started_at_ms` / `ended_at_ms`), schema, docs; verify fixture tests (finished, unfinished, missing transcript fallback)
- [x] 10.2 Render right-aligned timing in subagent rows with a coarse client re-render tick only while running rows are visible; verify formatting unit tests and render tests with an injected clock
- [x] 10.3 Add `subagent` / `subagent_focused` theme tokens defaulting to `tab` / `tab_focused`; verify theme parsing and render style tests, update configuration docs
- [x] 10.4 Add the `toggle_subagents` keybinding (bound by default to a free prefix key) and context-menu items; verify keybind parsing/default tests and toggle tests, update keybinding docs
- [x] 10.5 Live client check of 9.1 and 10.x and full test/clippy comparison
- [x] 10.6 Add `shell` / `shell_focused` theme tokens for shell rows defaulting to the subagent colors; verify theme parsing and render style tests, update configuration docs

## 11. Background shells

- [x] 11.1 Incremental transcript parser for background shells (start from `backgroundTaskId` tool results + `run_in_background` Bash tool uses, end from `<task-notification>` status; prefilter, partial lines, rewrite reset) and the pure current/history reconcile per D10; verify unit tests with fixture lines (start, completed, failed, killed, unknown status, subagent notification ignored, split line across reads, resume leftover interrupted, linger, parent dead)
- [x] 11.2 Watch the transcript in the subagent watcher thread (ref-counted dir watch, path-filtered, debounced incremental tail) and deliver `AppEvent::BackgroundShellsScanned`; store current/history lists on `TerminalState` (cleared with subagents) with change detection; verify watcher temp-dir test (append start then end lines) and app tests
- [x] 11.3 API: `PaneInfo.background_shells`, `pane.background_shells` (scope), CLI `herdr pane background-shells`, event `pane.background_shells_changed`; schema + `docs/next` socket API/CLI docs; verify API tests for both scopes and the event
- [x] 11.4 Wire `ClientShellAgent.background_shells` / `background_shell_history` (optional serde) and client-shell projection, refreshing the snapshot on change; verify wire round-trip and projection tests
- [x] 11.5 Spaces sidebar `shells` group (header + rows, icons, timing, fold, count, collapse/history/keybinding/context menu, click focuses parent); verify render/click tests and a live client check with a real background shell
- [x] 11.6 Make the `shells` header a toggle (running count + chevron, click collapses only the shell rows; per-agent presentation state, pruned with the agent); verify render/click tests and a live client check
- [x] 11.7 Drop the `shells` header: show shell rows at the subagent level after the subagent rows, with the subagent state icons and a `$ ` prefix; verify render tests
