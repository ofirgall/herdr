# Proposal

## Why

Rename plugins write tab jump numbers into tab names (for example `1| zsh` or `[2] build`), so the numbers show up in the tab bar where they help with `switch_tab`. The sidebar's `tab` token prints the same name, so agent rows repeat the number where it is only noise, and tabs in different spaces show the same numbers side by side. Users want the number in the tab bar but not in the sidebar.

## What Changes

- **New agent-row token `tab_stripped`.** It renders the tab name with a leading jump-number prefix removed, for example `1| zsh` → `zsh` and `[2] build` → `build`. It follows the existing `terminal_title_stripped` naming and works in agent rows of the agents panel and of agents nested under spaces.
- **Only the tab's own number is stripped.** The prefix must be the tab's position among its space's tabs (the number `switch_tab` uses). It may be wrapped in `[]` or `()`, may be followed by at most one punctuation or symbol character, and must be followed by whitespace. Names like `2024 report`, `3d model` or a prefix that is not the tab's position are drawn unchanged.
- **Exact prefix option.** A styled token can give the exact prefix its plugin writes, with `{}` for the tab's position, for example `{token = "tab_stripped", prefix = "{}| "}`. Then only that text is removed (`2| ` on the second tab), and any plugin format works, including ones with several symbols and spaces. Without `prefix`, the built-in rule above applies.
- **Display only.** Tab names, the tab bar, the API and the `tab` token keep the full name.

No breaking changes: `tab_stripped` is a new token and is not in the default rows.

## Capabilities

### New Capabilities
- `sidebar/tab-stripped-token`: the `tab_stripped` agent-row token, its built-in strip rule, its `prefix` option, and where it is available.

### Modified Capabilities
<!-- none -->

## Impact

- **Config:** `src/config/sidebar.rs` adds `AgentSidebarToken::TabStripped`, its name, and the `prefix` key on styled tokens (validated like the `index` token's `format`).
- **Tokens:** `src/ui/sidebar/tokens.rs` adds a `tab_stripped` field to `AgentTokenContext` and resolves the token from it.
- **Callers:** `src/client/shell/agent_sidebar.rs` and `src/client/shell/space_agents.rs` pass the tab's position, so the stripped name can be computed when the context is built.
- **Docs:** `configuration.mdx` (agent-row tokens) and `config-reference.json`.
- **Not affected:** the server, API, wire protocol and snapshot. The tab's position is already derivable from the snapshot's tab order.
