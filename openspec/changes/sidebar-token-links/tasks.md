# Tasks

## 1. Server: links on metadata tokens

- [x] 1.1 Add `link: Option<String>` to `MetadataToken` in `src/metadata_tokens.rs`. Make `patch` take value plus link per key, so that setting a value without a link clears the link, and clearing or expiring a token removes both. Verify with unit tests: set with link, set again without link, clear, TTL expiry, and a link-only change reporting `changed`.
- [x] 1.2 Add an optional `links` map to `WorkspaceReportMetadataParams` and `PaneReportMetadataParams`, plus the repeated `--link NAME=URL` CLI binding. Validate before mutation: each link key must be set in the same request, the URL must pass `safe_web_url`, and its length must be at most 2048. Verify with API tests for an accepted link, a link without a value, a `file://` URL and an oversized URL (all-or-nothing rejection), and with a CLI parse test.
- [x] 1.3 Expose `links` (skipped when empty) on `WorkspaceInfo`, `PaneInfo` and the metadata-updated events, and regenerate the JSON schema output. Verify that the existing schema and event tests are unchanged for token-only payloads, and that a new test shows links in info and events.
- [x] 1.4 Add `token_links: Vec<(String, String)>` (`serde(default, skip_serializing_if = "Vec::is_empty")`) to `ClientShellWorkspace` and `ClientShellAgent`, filled in `src/server/client_shell.rs`. Verify:
  - `endpoint-snapshot-v1.json`, the bincode digests and the method-shape fixtures are untouched and their tests pass.
  - A new test round-trips a snapshot with links.
  - A test decodes a links-bearing snapshot into a struct without the field.
- [x] 1.5 Document `--link` and the `links` field in the unreleased plugin/API metadata docs under `docs/next/website/src/content/docs/`, including the older-server behavior. Verify that the docs build or check recipe passes.

## 2. Client: resolve, fit and draw linked tokens

- [x] 2.1 Add `link: Option<String>` to `ResolvedToken`. Set it in `space_rows` / `agent_rows` only for `Custom` tokens whose key has an API link. Make a static metadata token never carry one, and make an overriding API token bring its own link. Verify with token resolution unit tests.
- [x] 2.2 Split `resolved_token_spans` into a layout step (visible tokens, start column, width) and a span-emit step. Keep the output identical for existing callers. Verify that the current sidebar render tests pass unchanged, and that a new unit test reports correct columns for visible, truncated and elided tokens.
- [x] 2.3 Add `TokenLinkHit { rect, url }` to `ShellHitMap`. In space rows (`sidebar.rs`), agent rows (`agent_sidebar.rs`) and agent children (`space_agents.rs`), underline linked tokens and push one hit per visible linked token. Skip both when there are no links or the mode is `"off"`. Verify with render tests: underline cells, hit rects matching the token columns, and no hits or underline in collapsed and multi-machine sidebars.

## 3. Client: click setting and click handling

- [x] 3.1 Add `[ui.sidebar] token_link_click` (default `"click"`), parsed into `click | off | modifiers` and reusing the `right_click_passthrough_modifier` grammar. Invalid values report a config diagnostic and keep the default. Thread it into `ClientShellConfig`. Verify with config parse tests for every accepted form, `"shift"` and an unknown string.
- [x] 3.2 Space rows: record the matching link on `ClientWorkspacePress`, and on an `Up` inside the token without a drag, emit `OpenSafeWebUrl` instead of selecting. Verify with shell mouse tests:
  - A plain click opens and does not select.
  - A press-drag reorders without opening.
  - In ctrl mode, a plain click selects and a ctrl click opens.
  - In `"off"` mode, a click selects.
- [x] 3.3 Agent rows and agent children: a matching `Down` over a link hit stores a pending press and skips focus. An `Up` on the same token opens it. Verify with shell mouse tests for open, a release elsewhere (no action), and a non-matching modifier (focus as today).
- [x] 3.4 Document `token_link_click` in `configuration.mdx`, including the terminal caveats for cmd and ctrl clicks, and add it to `config-reference.json`. Verify with the docs maintenance test.

## 4. Integration

- [ ] 4.1 Live smoke on the dev server:
  - `herdr workspace report-metadata --token pr=#1 --link pr=https://example.com` underlines `$pr` in a space row.
  - A click opens the browser without changing focus.
  - Re-reporting without `--link` removes the underline.
  - `token_link_click = "ctrl"` reloads and switches behavior.
  - Record the results.
- [x] 4.2 Run `just check` (or the agreed narrower validation, with known pre-existing failures named) and verify that nothing new fails.
