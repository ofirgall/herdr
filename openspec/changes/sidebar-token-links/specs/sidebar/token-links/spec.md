# Spec Delta

## Purpose

Makes sidebar tokens that carry a link clickable, so a token such as a PR number opens its page in the browser, with a setting for which click opens it.

## ADDED Requirements

### Requirement: Linked tokens are marked
In space rows, agent rows and agent child rows of the expanded single-machine sidebar, a custom `$name` token whose metadata has a link SHALL be drawn underlined, on top of its configured style. Tokens without a link SHALL render as today.

#### Scenario: Linked token
- **WHEN** space row `["workspace", "$pr"]` shows token `pr` that has a link
- **THEN** the `pr` value SHALL be underlined and the workspace label SHALL NOT

#### Scenario: Token without a link
- **WHEN** token `env` was reported without `--link`
- **THEN** it SHALL NOT be underlined or clickable

### Requirement: Click setting
The sidebar SHALL read `[ui.sidebar] token_link_click`, defaulting to `"click"`. Accepted values are `"click"`, `"off"`, or a modifier `ctrl`, `alt`, `cmd` (aliases `control`, `option`, `command`, `super`), `meta`, `hyper`, or a `+`-joined combination of them without `shift`. Any other value SHALL be a config error and the default SHALL be kept.

#### Scenario: Invalid value
- **WHEN** `token_link_click = "shift"`
- **THEN** config check SHALL report an error and links SHALL open on a plain click

### Requirement: Clicking a linked token opens its URL
When the click matches `token_link_click`, a left click on the visible part of a linked token SHALL open its URL with the client machine's default browser and SHALL NOT select the space, focus the agent, or start a drag. The link SHALL open on release, only when press and release are on the same token and no drag happened.

#### Scenario: Plain click by default
- **WHEN** `token_link_click` is not set and the user clicks the underlined `#7009` token
- **THEN** its URL SHALL open and the focused space SHALL NOT change

#### Scenario: Drag starting on a token
- **WHEN** the user presses on a linked token and drags the space to a new position
- **THEN** the space SHALL be reordered as today and no URL SHALL open

#### Scenario: Remote endpoint
- **WHEN** the active endpoint is an SSH server and the user clicks a linked token
- **THEN** the URL SHALL open on the machine running the client

### Requirement: Modifier click mode
When `token_link_click` is a modifier, a left click with exactly that modifier set on a linked token SHALL open the URL. A plain click on a linked token SHALL behave as a click anywhere else on the row.

#### Scenario: Ctrl mode, plain click
- **WHEN** `token_link_click = "ctrl"` and the user plain-clicks a linked token in an unfocused space
- **THEN** that space SHALL be focused and no URL SHALL open

#### Scenario: Ctrl mode, ctrl click
- **WHEN** `token_link_click = "ctrl"` and the user ctrl-clicks a linked token
- **THEN** its URL SHALL open

### Requirement: Links can be turned off
With `token_link_click = "off"`, linked tokens SHALL render without the underline and clicks on them SHALL behave as clicks anywhere else on the row.

#### Scenario: Off
- **WHEN** `token_link_click = "off"` and the user clicks a linked token
- **THEN** no URL SHALL open and the row SHALL handle the click as today

### Requirement: Only visible text is clickable
The click target SHALL be exactly the columns the token occupies after width fitting. An elided token SHALL have no target, and a truncated token SHALL be clickable only on its visible columns. Separators and the rest of the row SHALL NOT be part of the target.

#### Scenario: Elided token
- **WHEN** the sidebar is too narrow and token `$pr` is elided
- **THEN** no column of that row SHALL open the link

### Requirement: Scope of rendering
The collapsed sidebar and the multi-machine sidebar SHALL be unchanged by links.

#### Scenario: Collapsed sidebar
- **WHEN** the sidebar is collapsed
- **THEN** it SHALL render and handle clicks as today
