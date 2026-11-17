# Design

## Context

- **Jump keys today.**
  - `KeybindAction::SwitchWorkspace(index)` focuses `navigation_workspace_entries(snapshot)[index]` (`src/client/shell/actions.rs`).
  - Navigate mode maps digits `'1'..='9'` to the same entries (`src/client/shell/input.rs`).
  - The entries follow sidebar order, including worktree grouping and collapsed groups (`workspace_entries` in `sidebar.rs`).
- **Indexed bindings.**
  - They are written with a `1..9` range (`parse_range_modifiers`) and stored as `IndexedKeybind`.
  - `push_indexed_binding` rejects anything but `'1'..='9'`.
  - `IndexedKeybind::matched_index` maps the digit to a zero-based index.
  - Navigate mode also reserves the direct keys `1`–`9`.
- **Where numbers appear today.**
  - The collapsed sidebar draws `index + 1`.
  - The expanded sidebar draws no number.
  - The visible `N. name` labels come from a third-party rename plugin that writes custom names.
- **Space state.** It lives in the server `Workspace`, is persisted in the session, and reaches clients through `ClientShellWorkspace` (JSON snapshot, frozen generation-1 fixture). Workspace mutations from the TUI are endpoint methods (`push_endpoint_method`).

## Goals / Non-Goals

**Goals:**
- One function computes jump numbers, and keys, navigate mode, the `index` token and the collapsed sidebar all read it.
- Nothing changes when no space is skipped and bindings use `1..9`.
- The skip flag is shared session state, not a client preference.

**Non-Goals:**
- Tabs and agents (no skip flag, no `0` key).
- The server computing jump numbers. They depend on client-side collapsed groups, so the API exposes only the flag.
- Changing the multi-machine sidebar's numbers.
- Changing the third-party rename plugin. Users who want herdr's numbers turn its workspace prefix off and add `index` to their rows.

## Decisions

### 1. Jump numbers are a client projection over navigation entries
`fn space_jump_numbers(entries, snapshot) -> Vec<Option<JumpNumber>>` walks `navigation_workspace_entries` and assigns 1..=10 to non-skipped entries. `JumpNumber` displays `10` as `0`.

`SwitchWorkspace(n)` and the navigate digits look up the entry with number `n + 1` instead of `entries[n]`. The sidebar renderers compute the vector once per frame next to `workspace_entries`, so the cost is one pass over the visible entries, not per pane.
- *Alternative:* a server-side `jump_number` field. The server does not know which groups a client has collapsed, so the numbers would disagree with the keys. Rejected.

### 2. The skip flag is server state behind a new endpoint method
`Workspace.skip_index: bool` is set with `#[serde(default)]` in the persisted session. The new API method is `WorkspaceSetSkipIndex { workspace_id, skip: bool }` (CLI `workspace set-skip-index`). It emits the existing `workspace.updated` event, so no new event kind is added to the schema enum.

`WorkspaceInfo.skip_index` and `ClientShellWorkspace.skip_index` both use `#[serde(default, skip_serializing_if = "std::ops::Not::not")]`, so the generation-1 fixture is unchanged.

The TUI toggle sends the method with the negated current value, so two clients toggling at once converge on an explicit value instead of flipping twice. The method is advertised like other optional endpoint methods. On a server without it, the toggle surfaces the existing method-unsupported notice.
- *Alternative:* a client preference like the space-agent toggles. Other clients and plugins could not see it, and the user expects the numbering to be the same everywhere. Rejected.

### 3. Range syntax `0..9` for `switch_workspace` only
- **Parsing.** `parse_range_modifiers` accepts `0..9` as well as `1..9` and records which form was used. The expansion for `0..9` adds `0` after `1`–`9`.
- **Index mapping.** `matched_index` maps `'0'` to index 9.
- **Validation.** `push_indexed_binding` takes an `allow_zero` flag, which is true only for `switch_workspace`. `switch_tab` and `focus_agent` keep rejecting `0` with the existing diagnostic.
- **Navigate mode** additionally reserves direct `0` and maps it to number 10.
- *Alternative:* make `1..9` also bind `0`. That would silently claim a new key from existing configs and could collide with user bindings on `…+0`. Rejected.

### 4. `index` token and collapsed sidebar
- **Token.** `SpaceSidebarToken::Index` resolves to `ResolvedTokenKind::Index(char)` when the space has a number, and to `None` otherwise. It is fixed width 1, styled with the secondary style, and works with styles and rules like other built-in tokens.
- **Context.** `SpaceTokenContext` gains `jump_number: Option<char>`.
- **Collapsed sidebar.** It replaces `index + 1` with the jump number, or blanks for none.

### 5. Toggle keybinding and context menu
- **Keybinding.** `toggle_space_index` is an `ActionKeybinds` field (unbound by default) mapped to `KeybindAction::ToggleSpaceIndex`. It resolves the focused workspace and pushes the method.
- **Context menu.** The space context menu adds `ToggleSkipIndex` with the label chosen from the snapshot flag, following the existing `ToggleAgents` pattern.

## Risks / Trade-offs

- **[Collapsing a worktree group renumbers the spaces after it.]** This is already true today, since keys follow navigation entries. The `index` token makes the change visible instead of hidden.
- **[Rename plugin numbers disagree with herdr's once spaces are skipped.]** Documented: use the `index` token and disable the plugin's workspace prefix. The flag is in the API for plugins that want to adapt.
- **[`0..9` is unfamiliar syntax.]** It is documented next to `1..9`. A wrong field gets the existing indexed-binding diagnostic naming the field.

## Migration Plan

Additive. Old sessions restore with no skipped spaces. Old clients ignore `skip_index`. Old servers make the toggle show the unsupported notice.
