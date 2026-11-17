# Spec Delta

## Purpose

Defines the `tab_stripped` agent-row token, which shows a tab's name in the sidebar without the jump number a rename plugin wrote into it.

## ADDED Requirements

### Requirement: tab_stripped renders the tab name without its number prefix
Agent rows SHALL accept a `tab_stripped` token, in the agents panel and in agents nested under spaces. It SHALL render the same name as `tab`, with a leading jump-number prefix removed when one is present. It SHALL be shown or hidden under the same conditions as `tab`. Tab names, the tab bar, the API and the `tab` token SHALL keep the full name.

#### Scenario: Bar-separated number
- **WHEN** the second tab of a space is named `2| zsh` and an agent row uses `tab_stripped`
- **THEN** the row SHALL show `zsh`

#### Scenario: Bracketed number
- **WHEN** the first tab of a space is named `[1] build`
- **THEN** `tab_stripped` SHALL show `build`

#### Scenario: tab token unchanged
- **WHEN** the first tab of a space is named `1| zsh` and a row uses `tab`
- **THEN** the row SHALL show `1| zsh`

### Requirement: Only the tab's own number is stripped
The prefix SHALL be removed only when all of these hold:
- It starts with the tab's position among its space's tabs, written as decimal digits (the number `switch_tab` uses), optionally wrapped in `[]` or `()`.
- It is followed by at most one character that is neither alphanumeric nor whitespace.
- It is followed by at least one whitespace character, which is removed too.
- Text remains after it.

Otherwise the full name SHALL be shown.

#### Scenario: A year is not a number prefix
- **WHEN** the first tab of a space is named `2024 report`
- **THEN** `tab_stripped` SHALL show `2024 report`

#### Scenario: Another tab's number
- **WHEN** the first tab of a space is named `3| zsh`
- **THEN** `tab_stripped` SHALL show `3| zsh`

#### Scenario: Number without a separator gap
- **WHEN** the second tab of a space is named `2d plan`
- **THEN** `tab_stripped` SHALL show `2d plan`

#### Scenario: Nothing after the prefix
- **WHEN** the first tab of a space is named `1| `
- **THEN** `tab_stripped` SHALL show `1| `

### Requirement: An exact prefix can replace the built-in rule
A styled `tab_stripped` token SHALL accept a `prefix` value in which `{}` stands for the tab's position among its space's tabs. When `prefix` is set, the token SHALL remove the name's leading text only when it equals the prefix with `{}` replaced by the position's decimal digits and text remains after it, and SHALL NOT apply the built-in rule. Otherwise it SHALL show the full name.

#### Scenario: Multi-symbol plugin format
- **WHEN** the token is `{token = "tab_stripped", prefix = "{}    󰅂 │ "}` and the second tab is named `2    󰅂 │ zsh`
- **THEN** the row SHALL show `zsh`

#### Scenario: Prefix replaces the built-in rule
- **WHEN** the token has `prefix = "[{}] "` and the first tab is named `1| zsh`
- **THEN** the row SHALL show `1| zsh`

#### Scenario: Another tab's number
- **WHEN** the token has `prefix = "{}| "` and the first tab is named `2| zsh`
- **THEN** the row SHALL show `2| zsh`

### Requirement: Prefixes are validated
A `prefix` SHALL contain exactly one `{}`, no other `{` or `}`, no control characters, and at most 32 characters. Tokens other than `tab_stripped` SHALL reject `prefix`. An invalid prefix SHALL be a config error, handled like any other invalid sidebar token.

#### Scenario: Invalid prefix
- **WHEN** the token has `prefix = "| "`
- **THEN** config check SHALL report an error naming the `tab_stripped` prefix

#### Scenario: Prefix on another token
- **WHEN** a row contains `{token = "tab", prefix = "{}| "}`
- **THEN** config check SHALL report an error that `prefix` is only supported on `tab_stripped`

### Requirement: Styles and rules apply to the stripped name
`tab_stripped` SHALL support `fg`, `bold`, `dim` and rules like other text tokens. Rules SHALL match the stripped name.

#### Scenario: Rule on the stripped name
- **WHEN** the token has a rule `equals = "zsh"` and the first tab is named `1| zsh`
- **THEN** the rule SHALL apply
