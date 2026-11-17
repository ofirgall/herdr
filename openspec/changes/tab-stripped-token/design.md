# Design

## Context

- **The `tab` token.** `AgentSidebarToken::Tab` resolves from `AgentTokenContext.tab: Option<&str>` (`src/ui/sidebar/tokens.rs`). Two callers build the context through `resolve_agent_rows` (`src/client/shell/agent_sidebar.rs`):
  - **Agents panel** (`agent_sidebar.rs`, around line 280). It looks up the tab by `agent.tab_id` and hides the label when the space has one tab without a custom name.
  - **Space children** (`space_agents.rs::children`). It looks up the label in a `tab_id` map.
- **Existing precedent.** `terminal_title_stripped` arrives precomputed in the context (`terminal_title_stripped: Option<&str>`), and the token only selects it.
- **Tab position.** `ClientShellTab.number` is not the tab's position: tabs keep their creation number, so a space's fourth tab can be number 5. `switch_tab`, and rename plugins numbering tabs, use the position in the snapshot's tab order within the space (1-based).

## Goals / Non-Goals

**Goals:**
- `tab_stripped` hides the number only where it is certainly the tab's jump number.
- Rendering stays cheap: at most one small string slice per agent row, and no allocation when nothing is stripped.
- No server, API or wire change.

**Non-Goals:**
- Arbitrary user patterns (no regex). The `prefix` option is an exact template, not a pattern.
- Changing tab names, the tab bar or the `tab` token.
- A stripped variant for workspace names. Herdr draws space numbers itself with the `index` token.

## Decisions

### 1. Strip by a fixed grammar anchored to the tab's position
`fn strip_tab_number(label: &str, position: usize) -> &str` returns a subslice of `label`. It matches, in order:
1. an optional `[` or `(`;
2. exactly the decimal digits of `position`;
3. the matching `]` or `)` when one was opened;
4. at most one character that is neither alphanumeric nor whitespace (`|`, `.`, `:`, `▏`, …);
5. one or more whitespace characters.

It returns the rest only when the match succeeds and the rest is non-empty, and otherwise returns `label` unchanged. Anchoring to the position, and requiring whitespace after it, is what keeps `2024 report` and `2d plan` intact.
- *Alternative:* strip any leading number. Rejected because it eats real names.
- *Alternative:* a user-configured regex. Rejected as more surface than needed; decision 4 adds an exact template instead.

### 2. Compute the stripped label when building the context
`AgentTokenContext` gains `tab_stripped: Option<&'a str>`, and `resolve_agent_rows` takes a `tab_position: Option<usize>` and fills it with `strip_tab_number`. Both callers get the position from the snapshot's tab order:
- **Agents panel:** it already walks the space's tabs to count them, so it takes the index of the agent's tab in the same pass.
- **Space children:** they store the position next to the label in their `tab_id` map when the map is built, once per frame.

The resolver maps `AgentSidebarToken::TabStripped` to `context.tab_stripped` exactly as `Tab` maps to `context.tab`, so visibility, styles and rules behave the same, and rules match the stripped text.

### 3. Token name and docs
The token is named `tab_stripped`, after `terminal_title_stripped`. It is documented next to `tab` in the agent-row token list and in `config-reference.json`. It is not added to default rows.

### 4. `prefix` is an exact template, parsed like the `index` format
`RawStyledSidebarToken` already has `format` for `index` (from `space-index-format`). Add `prefix` beside it, allowed only when the token name is `tab_stripped`.
- **Validation:** the same rules as the index format (exactly one `{}`, no other braces, no control characters) with a 32-character limit, since plugin formats carry spacing and icons. It reuses the same parser and returns `(before, after)`.
- **Storage:** `AgentSidebarToken::Styled` gains `prefix: Option<(String, String)>`, or reuses the index format's type under a neutral name.
- **Context:** the context carries the tab label and position. The stripped text depends on the token's own prefix, so it is resolved per token: with a prefix, the token checks `label.strip_prefix(before)`, then the position's digits, then `after`, and requires a non-empty rest. Without one, it uses the precomputed `context.tab_stripped` from decision 2.
- **Cost:** a few `strip_prefix` calls on a short string, with no allocation.
- *Alternative:* a sidebar-wide setting. Rejected for the same reason as the index format: it is per token, so different rows can differ.

## Risks / Trade-offs

- **[A tab moves and the plugin has not renumbered it yet.]** The prefix briefly no longer matches the new position and is shown, until the plugin catches up. This is acceptable, and it is the price of never stripping a number that is not the tab's own.
- **[Plugin formats with two symbols, such as `1:: x`.]** The built-in rule leaves them alone; users set `prefix` to the plugin's exact format instead.

## Migration Plan

Additive. Nothing to migrate.
