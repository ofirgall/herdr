# Proposal

## Why

Space jump keys (`switch_workspace`, e.g. `ctrl+1..9`) go to the Nth space in sidebar order, and only nine spaces can be reached. Long-lived spaces you rarely jump to (a parent repo, a scratch space) take up low numbers, pushing the spaces you actually switch between out of reach. Herdr also draws no jump number in the expanded sidebar, so users rely on a rename plugin that guesses the numbers and can drift from what the keys do.

## What Changes

- **Skip a space from jump numbering.** Each space has a "skip index" flag. A skipped space gets no jump number, and the numbers of the spaces after it shift down, so the keys reach the next spaces instead. It stays visible, clickable, and reachable with previous/next space keys.
- **Toggle it.** New keybinding `toggle_space_index` (unbound by default) toggles the flag for the focused space. The space context menu gets "Skip jump number" / "Use jump number". The flag is stored on the server with the session, so it survives restarts and is the same for every attached client.
- **API.** New `workspace set-skip-index` method and CLI command. `WorkspaceInfo`, workspace events and the client snapshot expose the flag as an optional field.
- **A tenth jump key.** Indexed `switch_workspace` bindings accept the range `0..9`: keys 1–9 reach numbered spaces 1–9 and key `0` reaches the tenth. Navigate mode accepts `0` the same way. `1..9` keeps its meaning, and tab and agent bindings still use `1..9` only.
- **Built-in number token.** New space row token `index` renders the space's jump number (`1`–`9`, `0` for the tenth), and nothing when the space is skipped or has no number. The collapsed sidebar's numbers follow the same numbering.

No breaking changes: with no skipped spaces and `1..9` bindings, numbering and keys behave as today.

## Capabilities

### New Capabilities
- `workspace/skip-index`: the per-space skip-index flag as shared session state, its persistence, and how it is set and exposed through the API, events and client snapshot.
- `sidebar/space-jump-index`: how jump numbers are assigned to spaces, the keys that reach them (including the tenth on `0`), the toggle keybinding and menu item, and the `index` token and collapsed-sidebar numbers that display them.

### Modified Capabilities
<!-- none -->

## Impact

- Server state and persistence:
  - `Workspace` gains a skip-index flag; session save/restore in `src/persist/` (serde default).
  - `src/app/api/workspaces.rs` gets the new method.
- API: `src/api/schema/workspaces.rs` (params, `WorkspaceInfo.skip_index`), the events schema, CLI bindings, and the regenerated `docs/next/api/herdr-api.schema.json`.
- Wire: optional `skip_index` on `ClientShellWorkspace` (`src/protocol/wire.rs`, `src/server/client_shell.rs`). The new endpoint method is advertised so older servers disable only the toggle.
- Client:
  - Keybinding range parsing and indexed matching (`src/config/keybinds.rs`, `src/config/model.rs`, `src/input/keybindings.rs`).
  - Jump-number assignment and use in `src/client/shell/actions.rs`, `input.rs` and `state.rs`.
  - Context menu (`context_menu.rs`).
  - The `index` token (`src/config/sidebar.rs`, `src/ui/sidebar/tokens.rs`).
  - The collapsed sidebar (`src/client/shell/sidebar.rs`).
- Docs: `configuration.mdx` (indexed jumps, `index` token, `toggle_space_index`), `cli-reference.mdx`, `socket-api.mdx`, `config-reference.json`.
