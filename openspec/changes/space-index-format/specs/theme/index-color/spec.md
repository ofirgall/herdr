# Spec Delta

## Purpose

Defines the theme color for space jump numbers drawn by the `index` token.

## ADDED Requirements

### Requirement: Themes can color the jump number
The theme palette SHALL have an optional `index` color, settable in `[theme.custom]` and in `[theme.custom.light]` / `[theme.custom.dark]`, using the same color syntax and precedence as the other custom theme colors. No built-in theme SHALL set it.

#### Scenario: Custom index color
- **WHEN** `[theme.custom] index = "#f8921a"` is set
- **THEN** every space's jump number SHALL render in `#f8921a`, on the focused space and on the others

#### Scenario: Mode override
- **WHEN** `[theme.custom] index = "#f8921a"` and `[theme.custom.light] index = "#444444"` are set and the light mode is active
- **THEN** jump numbers SHALL render in `#444444`

### Requirement: Unset color keeps today's look
When no `index` color is set, the jump number SHALL render in the same colors as before this change: the focused space's highlight color on the focused space and the muted secondary color elsewhere.

#### Scenario: Default theme
- **WHEN** no `index` color is configured
- **THEN** the sidebar SHALL render identically to before this change

### Requirement: Token styles win over the theme color
An `fg` set on the `index` token, directly or through a matching rule, SHALL take precedence over the theme `index` color. `bold` and `dim` on the token SHALL combine with it.

#### Scenario: Token fg
- **WHEN** `[theme.custom] index = "#f8921a"` and the token is `{token = "index", fg = "blue"}`
- **THEN** jump numbers SHALL render blue
