# Tasks

## 1. Server: skip-index flag

- [x] 1.1 Add `skip_index: bool` to the server `Workspace` and to the persisted session form (`serde(default)`), and carry it through restore and live handoff. Verify with persistence tests: a skipped space round-trips, and an old session without the field restores as false.
- [x] 1.2 Add the `WorkspaceSetSkipIndex { workspace_id, skip }` API method and the CLI command `herdr workspace set-skip-index <WORKSPACE_ID> <true|false>`. It emits `workspace.updated` only on change and returns not-found for unknown ids. Verify with API tests (set, no-op, unknown id), a CLI parse test, and the regenerated `docs/next/api/herdr-api.schema.json`.
- [x] 1.3 Expose `skip_index` (skipped when false) on `WorkspaceInfo` and `ClientShellWorkspace`, filled in `src/server/client_shell.rs`, and advertise the method for endpoint clients. Verify:
  - The generation-1 snapshot fixture, bincode digests and method-shape fixtures are untouched and still pass.
  - A new test shows the field present only when true.
- [x] 1.4 Document the CLI command and the field in `cli-reference.mdx` and `socket-api.mdx` under `docs/next`. Verify with `just docs-contract-test`.

## 2. Client: jump numbers and keys

- [x] 2.1 Add a single jump-number projection over `navigation_workspace_entries`: skip flagged spaces, number 1–10, and display 10 as `0`. Verify with unit tests for no skips, one skip, eleven spaces, and a collapsed worktree group.
- [x] 2.2 Make `SwitchWorkspace(n)`, its availability check, and navigate-mode digits use jump numbers, with navigate-mode `0` selecting number 10. Verify with shell tests:
  - The key passes over a skipped space.
  - A key with no matching space does nothing.
  - Navigate `0` selects the tenth space.
  - Previous/next still visit skipped spaces.
- [x] 2.3 Accept the `0..9` range and a single key `0` for `switch_workspace` only; `switch_tab` and `focus_agent` keep rejecting them with the existing diagnostic. Reserve direct `0` in navigate mode. Verify with keybinding config tests for `ctrl+0..9` matching index 9 on `ctrl+0`, `1..9` not binding `0`, and `switch_tab = "prefix+0..9"` being rejected.
- [x] 2.4 Document `0..9` in the "Indexed jumps" section of `configuration.mdx`, and verify with the docs contract test.

## 3. Client: toggle, token, collapsed sidebar

- [x] 3.1 Add the `toggle_space_index` keybinding (unbound default), mapped to `KeybindAction::ToggleSpaceIndex`, which sends the set-skip-index method with the negated current flag of the focused space. Verify with shell tests:
  - The method and value are pushed.
  - The unsupported-method path shows the notice.
  - The config field parses.
- [x] 3.2 Add the "Skip jump number" / "Use jump number" space context-menu item, following `ToggleAgents`. Verify with context-menu tests for both labels and the pushed method.
- [x] 3.3 Add the `index` space token, which resolves to the jump number and is elided when the space has none, and works with styles and rules. Verify with token and render tests for rows `[["index", "workspace"]]` with a skipped space and with eleven spaces.
- [x] 3.4 Make the collapsed sidebar draw jump numbers, and blanks for unnumbered spaces. Verify with a collapsed-sidebar render test.
- [x] 3.5 Document the `index` token and `toggle_space_index` in `configuration.mdx`, and add them to `config-reference.json`. Verify with the maintenance config-reference test.

## 4. Integration

- [x] 4.1 Run a live smoke test on an isolated dev server:
  - Bind `switch_workspace = "ctrl+0..9"` and `toggle_space_index`, and add `index` to the rows.
  - Skip a space and confirm the numbers shift and `ctrl+N` follows them.
  - Reach the tenth space with `ctrl+0`.
  - Restart the server and confirm the skip persists.
  - Record the results.
  - Results (2026-11-17, isolated XDG dirs, client in tmux, so `alt+0..9` / `alt+i` stood in for `ctrl`):
    - With 11 spaces the sidebar read `1`–`9`, `0`, then a bare eleventh.
    - `alt+0` focused the tenth space.
    - `alt+i` on the first space set `skip_index: true`; the numbers shifted (`1 B` … `0 K`), `alt+2` focused C and `alt+0` focused K.
    - After `server stop` and relaunch the flag was still true and the numbering unchanged.
    - `workspace set-skip-index w1 false` restored the original numbering and the field disappeared from `workspace get`.
    - Not checked live (covered by shell tests): the context-menu item and the collapsed sidebar.
- [ ] 4.2 Run `just check` (or the agreed narrower validation, naming the known pre-existing failures) and confirm nothing new fails.
