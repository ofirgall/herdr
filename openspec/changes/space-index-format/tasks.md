# Tasks

## 1. Index format

- [x] 1.1 Add `format` to the styled sidebar token config, validated as in design decision 1 and stored as `IndexFormat { before, after }` on `SpaceSidebarToken::Styled`. Verify with config tests:
  - `[{}]` and `{}.` parse.
  - `#`, `{}{}`, `{x}`, a 17-character format and a control character are rejected with their messages.
  - `format` on `workspace` is rejected.
  - A plain `"index"` has no format.
- [x] 1.2 Change `ResolvedTokenKind::Index` to carry the digit and the formatted text. Match rules on the digit, use the text's display width, and draw the text. Verify with token tests:
  - `[{}]` renders `[1]`, `[2]` and `[0]` for the tenth.
  - A skipped space elides the whole token.
  - A rule `equals = "0"` styles `[0]`.
  - Plain `index` is unchanged.
- [x] 1.3 Add a render test for rows `[[{token = "index", format = "[{}]"}, "workspace"]]` with B of A, B, C skipped, reading `[1] A`, `B`, `[2] C`.

## 2. Theme color

- [x] 2.1 Add `index` to `CustomThemeColors` and `ModeThemeColors`, add `Palette.index: Option<Color>` (`None` in every built-in theme), and apply it in `with_overrides` / `with_mode_overrides`. Verify with palette tests: unset by default, set by `[theme.custom]`, and replaced by `[theme.custom.light]` in light mode.
- [x] 2.2 Use `palette.index`, when set, as the number's base style in space rows and the collapsed sidebar, with the token's own style applied on top. Verify with render tests:
  - With no theme color, a focused row's number keeps mauve and an unfocused row's keeps overlay0.
  - A theme color applies on both rows.
  - A token `fg` beats the theme color.

## 3. Docs

- [x] 3.1 Document `format` on the `index` token, and the `index` theme color in the Theme section, in `docs/next/website/src/content/docs/configuration.mdx`. Add both to `config-reference.json`. Verify with the config-reference maintenance test and `just docs-contract-test`.

## 4. Integration

- [x] 4.1 Run a live smoke test on an isolated server (never with the onboarding's integrations step; dismiss it with Escape):
  - Rows with `{token = "index", format = "[{}]"}` and a skipped space.
  - `[theme.custom] index` set, then a token `fg`.
  - An invalid format rejected by `config check`.
  - Record the results.
  - Results (2026-11-17, isolated XDG dirs, client in tmux, read with ANSI capture):
    - Spaces A, B, C with B skipped read `[1] sidebar-rework`, `B`, `[2] C`.
    - `[theme.custom] index = "#ff0000"` drew `[1]` and `[2]` red on the focused and an unfocused row; labels kept their colors.
    - Adding `fg = "#00ff00"` to the token and reloading drew the number green.
    - `format = "#"` failed `config check` ("the `index` format must contain exactly one `{}`") and a reload reported it and kept the current ui settings.
    - `format` on `workspace` failed `config check` ("`format` is only supported on the `index` token").
    - The onboarding modal stayed open (Escape does not close it); the CLI drove everything and the integrations step was never reached.
- [x] 4.2 Run `cargo fmt --check`, `cargo clippy --all-targets` (only the two known warnings) and `cargo nextest run --no-fail-fast` (only the known failures), plus the maintenance and docs tests.
