# Design

## Context

- Metadata tokens live in `MetadataTokens` (`src/metadata_tokens.rs`), one map per workspace and per pane. Each entry holds a value and an optional expiry, and is patched by `handle_workspace_report_metadata` / the pane equivalent with source, `seq` and `ttl_ms` rules.
- The client snapshot (`ClientShellSnapshot`, JSON, `src/protocol/wire.rs`) carries them as `tokens: Vec<(String, String)>` on `ClientShellWorkspace` and `ClientShellAgent`. It is built in `src/server/client_shell.rs`. `tests/fixtures/endpoint-snapshot-v1.json` freezes the generation-1 shape.
- Sidebar rows resolve config tokens to `ResolvedToken { kind, style }` (`src/ui/sidebar/tokens.rs`). `resolved_token_spans` (`src/ui/sidebar.rs`) decides which tokens are visible and how wide each one is, then emits spans. Mouse hits are whole-row rects in `ShellHitMap`.
- Pane URL clicks already end in `ClientShellAction::OpenSafeWebUrl`, which re-checks `safe_web_url` and calls `platform::open_url` on the client machine.
- `right_click_passthrough_modifier` already parses modifier strings (`src/config/model.rs`).

## Goals / Non-Goals

**Goals:**
- One link per token, stored and expired with the token, so reporters do not manage two lifecycles.
- No new endpoint method, codec or protocol version: links travel as an optional JSON field.
- No extra work in the sidebar render loop for rows without linked tokens.

**Non-Goals:**
- Links on static `[[ui.sidebar.spaces.metadata]]` tokens (a later, client-only addition).
- Plugin actions as click targets. Only URLs.
- Links in the collapsed or multi-machine sidebars, or on built-in tokens (`branch`, `workspace`, ...).
- Hover feedback or tooltips showing the URL.

## Decisions

### 1. The link is a field of the token entry
`MetadataToken` gains `link: Option<String>`. `patch` receives `HashMap<String, Option<(String, Option<String>)>>`, i.e. value plus link per key, so "set value without link" naturally clears the link and clearing or expiring removes both. Equality checks include the link, so a link-only change still emits a metadata event.
- *Alternative:* a separate links map with its own patch semantics (`--clear-link`). It allows changing the URL without resending the value, but it creates orphan links and a second TTL story. Reporters already resend values every update. Rejected.

### 2. API shape: optional `links` param, optional `links` info field
`WorkspaceReportMetadataParams` and `PaneReportMetadataParams` gain `#[serde(default, skip_serializing_if = "HashMap::is_empty")] links: HashMap<String, String>`. Validation runs before any mutation: every link key must be set (`Some`) in `tokens`, every URL must pass `safe_web_url`, and each URL is at most 2048 bytes. `WorkspaceInfo` and `PaneInfo` gain `links: HashMap<String, String>`, skipped when empty. The JSON schema and CLI help are regenerated from the `cli_params!` definitions. The CLI binding is a repeated `--link NAME=URL`.

An older server ignores `links` and still reports success. This is the public JSON API, not the client endpoint contract, and the only effect is a token without an underline. Plugins that need certainty can check `herdr --version`. Documented, not negotiated.

### 3. Snapshot: `token_links` as an optional sibling field
`ClientShellWorkspace` and `ClientShellAgent` gain `#[serde(default, skip_serializing_if = "Vec::is_empty")] token_links: Vec<(String, String)>`, sorted like `tokens`. Existing fields are not reshaped (no `(name, value, link)` triples), so the generation-1 fixture, the bincode digests and older clients are unaffected. No `PROTOCOL_VERSION` bump: the field is additive and defaulted.

### 4. Resolution carries the link; fitting reports columns
`ResolvedToken` gains `link: Option<String>`. It is set only for `Custom` tokens whose key is in the row's API links. Static metadata never has one, and when an API token overrides a static one, the API link wins together with the API value. Space rows read the workspace's links. Agent rows and agent children read the agent's.

`resolved_token_spans` is split into a layout step that returns, per visible token, its start column and width, and an emit step that builds spans from that layout. When a row has no linked token, callers use the existing path unchanged. Otherwise the renderer pushes one `TokenLinkHit { rect, url }` per visible linked token into `ShellHitMap`. The underline is added to the span style there. The layout is computed once per row, so the cost is the same pass that already runs, plus one small allocation only for rows with links. This is per space and per agent row, never per pane or per byte.

### 5. Click handling in the shell mouse handler
`token_link_click` is parsed into `TokenLinkClick { Click, Modifier(KeyModifiers), Off }` by reusing the modifier grammar from `right_click_passthrough_modifier`, with `"click"` and `"off"` added.

Today a space row press arms `ClientWorkspacePress` on `Down` and selects on `Up`, or starts a reorder drag if the pointer moves. Agent rows and agent children focus on `Down` and have no drag. Links fit into both without changing those paths:

- **Space rows.** On `Down(Left)` with matching modifiers over a `TokenLinkHit`, `ClientWorkspacePress` is armed as usual and additionally records the hit's `link` (url and rect). If the gesture becomes a drag, the existing reorder path runs and the link is dropped. On an `Up` inside the recorded rect without a drag, the press emits `OpenSafeWebUrl(url)` instead of selecting. An `Up` elsewhere falls back to today's behavior.
- **Agent rows and children.** On a matching `Down` over a `TokenLinkHit`, the handler stores a pending `ClientTokenLinkPress` and returns before the focus logic. On an `Up` inside the same rect, it emits `OpenSafeWebUrl(url)`. On an `Up` elsewhere, it does nothing.
- A `Down` that does not match the modifiers, or `token_link_click = "off"`, ignores `TokenLinkHit`s, so the row behaves as today.

`OpenSafeWebUrl` already runs on the client. A remote (SSH) endpoint's links therefore open locally, which is what users expect for web pages.
- *Alternative:* resolve the click server-side as pane links do (`PaneLinkActivated`) so plugins could intercept it. That needs a new endpoint method and is only useful for plugin actions, which are a non-goal.

## Risks / Trade-offs

- [Terminals swallow some modifier clicks: macOS terminals keep cmd-click, and ctrl-click can become a right click] → The default is a plain click. Docs recommend `ctrl` or `alt` and note that terminal settings may need mouse reporting for the modifier.
- [A plain click on a token no longer selects the space] → Only the token's own columns are affected, and the underline signals it. Users who dislike it set a modifier or `"off"`.
- [A malicious or buggy reporter supplies a misleading URL] → Only `http`/`https` URLs are accepted on report, and they are re-checked by `safe_web_url` before opening. Reporters are local processes that already have API access.
- [Old server accepts `links` and silently drops them] → Documented minimum version. No incorrect state, just no underline.

## Migration Plan

Additive only. Rollback is reverting the change: snapshots without `token_links` and requests without `links` behave as before.
