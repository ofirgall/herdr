# Tasks

## 1. Strip rule

- [x] 1.1 Add `strip_tab_number(label, position)` as in design decision 1. Verify with unit tests:
  - `1| zsh`, `[1] build`, `(1) x`, `1. x` and `1 ▏ x` at position 1 strip.
  - `12| x` at position 12 strips.
  - `2024 report`, `2d plan`, `3| zsh` at position 1, `1|| x` and `1| ` stay unchanged.
  - A multi-byte name after the prefix is returned intact.

## 2. Token

- [x] 2.1 Add `AgentSidebarToken::TabStripped` (`tab_stripped`) to config parsing and serialization, and `tab_stripped` to `AgentTokenContext`. Resolve the token like `tab`. Verify with config and token tests:
  - The name round-trips.
  - It resolves to the stripped text.
  - It is elided when `tab` would be.
  - A rule `equals = "zsh"` matches `1| zsh`.
- [x] 2.2 Pass the tab's position from the agents panel and the space-children path into `resolve_agent_rows`, computed once per frame. Verify with shell render tests for both paths: a space whose second tab is `2| zsh` shows `zsh` with `tab_stripped` and `2| zsh` with `tab`.
- [x] 2.3 Add `prefix` to styled sidebar tokens, allowed only on `tab_stripped` and validated as in design decision 4, and resolve it per token. Verify with config and token tests:
  - `{}| ` strips `2| zsh` on the second tab.
  - `{}    󰅂 │ ` strips its multi-symbol form.
  - With `[{}] ` set, `1| zsh` stays whole (the built-in rule is not used).
  - `{}| ` keeps `2| zsh` on the first tab.
  - `| `, `{}{}`, a 33-character prefix and a control character are rejected.
  - `prefix` on `tab` is rejected.

## 3. Docs

- [x] 3.1 Document `tab_stripped` and its `prefix` option next to `tab` in `docs/next/website/src/content/docs/configuration.mdx`, and add it to `config-reference.json`. Verify with the config-reference maintenance test and `just docs-contract-test`.

## 4. Integration

- [x] 4.1 Run a live smoke test on an isolated server, driving it from the CLI and never reaching the onboarding's integrations step:
  - Agent rows `[["state_icon", "tab_stripped", "agent"]]` in the agents panel and under spaces.
  - Tabs renamed `1| zsh` and `2| build`, then one tab moved so its prefix no longer matches.
  - The same with `prefix = "{}| "` set, plus an invalid prefix rejected by `config check`.
  - Record the results.
  - Results (2026-11-17, isolated XDG dirs, client in tmux, agents faked with `pane report-agent`):
    - Tabs `1| zsh`, `2| build`, `2024 report` read `zsh`, `build`, `2024 report` in both the agents panel and the space's children.
    - Closing the first tab moved `2| build` to position 1; both paths then showed `2| build`.
    - With `prefix = "{}| "`, `1| build` read `build`, `[3] logs` stayed whole (built-in rule off), and `3| report` at position 2 stayed whole.
    - `prefix = "| "` failed `config check` ("the `tab_stripped` prefix must contain exactly one `{}`"); a reload reported it as partial and kept the current ui settings.
    - `prefix` on `tab` failed `config check` ("`prefix` is only supported on the `tab_stripped` token").
    - The onboarding's integrations step was never reached; the Claude config stayed untouched.
- [x] 4.2 Run `cargo fmt --check`, `cargo clippy --all-targets` (only the two known warnings), `cargo nextest run --no-fail-fast` (only the known failures), and the maintenance and docs tests.
