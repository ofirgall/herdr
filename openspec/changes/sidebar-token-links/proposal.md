# Proposal

## Why

Plugins report sidebar tokens such as a PR number, a CI state or a ticket id, but the tokens are plain text. To open the PR behind `#7009`, the user has to find it somewhere else. Tokens that already name a web resource should open it when clicked.

## What Changes

- **Links on metadata tokens.** `workspace report-metadata` and `pane report-metadata` accept an optional `links` map (CLI: `--link NAME=URL`) next to `tokens`. A link belongs to the token with the same name: it is set together with that token's value, and it is removed when the token is cleared, expires or is set again without a link. Only `http` and `https` URLs are accepted.
- **Links in the API and snapshot.** Workspace and pane info, metadata events, and the client snapshot carry the links as an optional field next to `tokens`. Older clients ignore it.
- **Clickable tokens in the sidebar.** A token with a link is drawn underlined in space rows, agent rows and agent child rows. Clicking it opens the URL in the browser on the machine running the client. It does not select the space or focus the agent. Clicking anywhere else on the row behaves as today.
- **Configurable click.** New `[ui.sidebar] token_link_click` (default `"click"`): `"click"` opens on a plain click, a modifier such as `"ctrl"`, `"alt"`, `"cmd"` or `"ctrl+alt"` requires that modifier (a plain click then selects the row as today), and `"off"` turns link opening and the underline off.

No breaking changes: tokens without links render and behave as today.

## Capabilities

### New Capabilities
- `metadata/token-links`: attaching an optional web URL to a reported workspace or pane metadata token, its validation and lifecycle, and how it is exposed through the API, events and client snapshot.
- `sidebar/token-links`: how the sidebar shows tokens that carry a link, which click opens the link, and the `token_link_click` setting.

### Modified Capabilities
<!-- none -->

## Impact

- API schema: `src/api/schema/workspaces.rs`, `src/api/schema/panes.rs` (`links` param and info field), JSON schema output, CLI bindings (`--link`, `--clear-link`) in `src/cli/workspace_bindings.rs` and `src/cli/pane/bindings.rs`.
- Server state: `src/metadata_tokens.rs` (a link stored with each token), `src/app/api/workspaces.rs`, `src/app/api/panes.rs`, snapshot building.
- Wire: optional `token_links` on `ClientShellWorkspace` and `ClientShellAgent` in `src/protocol/wire.rs`. It is skipped when empty, so the frozen generation-1 snapshot fixture stays valid.
- Client: token resolution and fitting (`src/ui/sidebar/tokens.rs`, `src/ui/sidebar.rs`), sidebar renderers (`sidebar.rs`, `agent_sidebar.rs`, `space_agents.rs`), hit map and mouse handling (`state.rs`, `mouse.rs`), config (`src/config/sidebar.rs`, `src/client/shell/config.rs`).
- Docs: `docs/next/website/src/content/docs/` (configuration and plugin/API metadata pages), `config-reference.json`.
