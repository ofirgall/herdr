# Design

## Context

- **The `index` token** (added by `space-jump-index`):
  - `SpaceSidebarToken::Index` resolves to `ResolvedTokenKind::Index(char)` from `SpaceTokenContext.jump_number`, or `None` for unnumbered spaces (`src/ui/sidebar/tokens.rs`).
  - Rules are matched against the digit.
  - `src/ui/sidebar.rs` reserves width 1 for it and draws the digit with `apply_token_style(secondary_style, token.style)`.
- **Styled tokens.** `RawStyledSidebarToken` (`src/config/sidebar.rs`) carries `token`, `fg`, `bold`, `dim` and `rules`. `RawSidebarToken::parts` validates it, and the result is wrapped in `SpaceSidebarToken::Styled { token, style, rules }`. Any error is a serde error, so the whole config is rejected, as with the 16-token limit.
- **Number color today.** `secondary_style` in `src/client/shell/sidebar.rs` is `palette.mauve` on the focused space and `palette.overlay0` otherwise. The same style is used for branch and agent names, so the number has no color of its own.
- **Themes.**
  - `Palette` (`src/app/state.rs`) has one constructor per built-in theme.
  - `with_overrides` and `with_mode_overrides` apply `CustomThemeColors` / `ModeThemeColors` (`src/config/theme.rs`) in the order built-in, `[theme.custom]`, then mode table.

## Goals / Non-Goals

**Goals:**
- Existing configs and themes render pixel-identically.
- The format and the color are TUI presentation only: no server, API or wire changes.
- No per-frame allocation is added beyond the formatted number string for each numbered space (at most ten per frame).

**Non-Goals:**
- `format` on other tokens (`agent_index`, custom tokens). The key is rejected there, so it can be added later without changing meaning.
- Formatting the collapsed sidebar's numbers. It has one column per space and keeps the bare digit, though it uses the new theme color.
- Escape sequences in the format (`{{`, `}}`). Not needed for number styles.

## Decisions

### 1. `format` lives on the styled token and is stored on `SpaceSidebarToken::Styled`
`RawStyledSidebarToken` gains `format: Option<String>`. `parts` validates it:
- It is allowed only when the token name is `index`.
- It must contain exactly one `{}`, no other `{` or `}`, no control characters, and at most 16 characters.

The validated value is split once at parse time into `IndexFormat { before: String, after: String }`. `SpaceSidebarToken::Styled` gains `format: Option<IndexFormat>`; `AgentSidebarToken` is untouched.

The token-name check reuses the existing raw name before resolution, so `format` with `index` is accepted in space rows only (in agent rows, `index` is already an unknown token).
- *Alternative:* a sidebar-wide `index_format` setting. Rejected in favor of the per-token form the user picked; it also allows different formats in different rows.

### 2. The resolved token carries the formatted text
`ResolvedTokenKind::Index(char)` becomes `Index { digit: char, text: String }`. Resolution matches rules against `digit` (unchanged behavior) and builds `text` as `before + digit + after`, or just the digit without a format. Width is `display_width(text)` instead of 1, and drawing uses `text`. An unnumbered space still resolves to `None`, so the whole formatted token is elided.

### 3. `Palette.index: Option<Color>`, `None` in every built-in theme
- **Field.** `index: Option<Color>` instead of a `Color`, because the current number color depends on the row (mauve when focused, overlay0 otherwise). A single default `Color` would change focused rows.
- **Built-in themes.** Every constructor sets `index: None`.
- **Overrides.** `with_overrides` and `with_mode_overrides` set `Some(parse_color(..))` from the new `index` key in `CustomThemeColors` and `ModeThemeColors`. The existing precedence order is kept.
- **Space rows.** The space-row renderer computes the number's base style as `palette.index.map_or(secondary_style, |c| Style::default().fg(c))` before `apply_token_style`, so a token `fg` (direct or from a rule) still wins and `bold` / `dim` still combine.
- **Collapsed sidebar.** It uses `palette.index` when set, and its current color otherwise.
- *Alternative:* default `index` to `overlay0` in every theme. It changes the focused row's number from mauve, so it is rejected.

### 4. Validation errors follow the existing sidebar-token path
Errors are returned as strings from `parts`, like "sidebar rules require a text-valued token". `config check` and reload report them through the existing diagnostics.

The messages are:
- "the `index` format must contain exactly one `{}`".
- "`format` is only supported on the `index` token".
- One length message, and one message for invalid characters.

## Risks / Trade-offs

- **[A long format widens every numbered row.]** Capped at 16 characters, and the row's existing budget logic truncates other tokens first, as with any token.
- **[Unnumbered spaces lose the formatted gap, so labels don't line up across rows.]** This matches the spec, where skipped spaces read `B`, not `   B`. Users who want aligned columns can wait for a padding option; it is out of scope here.
- **[`Palette` derives `Eq` and is compared for theme changes.]** `Option<Color>` keeps that working.

## Migration Plan

Additive: the new keys are optional and their defaults are absent. Nothing to migrate.
