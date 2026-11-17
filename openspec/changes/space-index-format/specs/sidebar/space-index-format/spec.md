# Spec Delta

## Purpose

Defines how users format the space jump number drawn by the `index` token.

## ADDED Requirements

### Requirement: The index token accepts a format
A styled `index` token in space rows SHALL accept a `format` value in which `{}` stands for the space's jump digit. The token SHALL render the format with `{}` replaced by the digit. A plain `index` token, or one without `format`, SHALL render the bare digit as before.

#### Scenario: Bracketed numbers
- **WHEN** `rows = [[{token = "index", format = "[{}]"}, "workspace"]]` and the spaces are A, B, C with none skipped
- **THEN** the rows SHALL read `[1] A`, `[2] B`, `[3] C`

#### Scenario: Tenth space
- **WHEN** the format is `{}.` and a space has jump number `0`
- **THEN** its number SHALL render as `0.`

#### Scenario: Plain token unchanged
- **WHEN** the row uses `"index"` without a format
- **THEN** the number SHALL render as the bare digit

### Requirement: Spaces without a number render nothing
When a space has no jump number (skipped, or after the tenth), the `index` token SHALL be elided entirely, including the text around `{}` in its format.

#### Scenario: Skipped space
- **WHEN** the format is `[{}]` and B of A, B, C is skipped
- **THEN** the rows SHALL read `[1] A`, `B`, `[2] C`

### Requirement: Formats are validated
A `format` SHALL contain exactly one `{}`, no other `{` or `}`, no control characters, and at most 16 characters. Tokens other than `index` SHALL reject `format`. An invalid format SHALL be a config error, handled like any other invalid sidebar token: config check reports it and a reload with it is rejected.

#### Scenario: Missing placeholder
- **WHEN** the format is `#`
- **THEN** config check SHALL report an error naming the `index` format

#### Scenario: Format on another token
- **WHEN** a row contains `{token = "workspace", format = "{}:"}`
- **THEN** config check SHALL report an error that `format` is only supported on `index`

### Requirement: Styles and rules still apply
A formatted `index` token SHALL keep supporting `fg`, `bold`, `dim` and rules, applied to the whole formatted text. Rules SHALL match against the bare digit, not the formatted text.

#### Scenario: Rule on the tenth space
- **WHEN** the token has `format = "[{}]"` and a rule `equals = "0"` with `fg = "red"`
- **THEN** the tenth space's `[0]` SHALL render red
