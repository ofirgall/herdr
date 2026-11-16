# Spec Delta

## Purpose

Lets users attach static key/value metadata to spaces from config, so `$name` tokens in space rows can show stable labels without scripts setting them through the API every session.

## ADDED Requirements

### Requirement: Metadata entries are declared in config
The sidebar SHALL read a list `[[ui.sidebar.spaces.metadata]]`. Each entry has a `match` table and a `tokens` table of string values. `match` SHALL hold exactly one of `label` (a glob against the space label) or `path` (a directory prefix against the space's working directory). The list SHALL hold at most 64 entries, and each entry at most 16 tokens.

#### Scenario: Match by label
- **WHEN** an entry has `match = { label = "api-*" }` and `tokens = { env = "staging" }`
- **THEN** a space labelled `api-gateway` SHALL have `$env` = `staging`

#### Scenario: Match by path
- **WHEN** an entry has `match = { path = "~/work/herdr" }`
- **THEN** a space whose working directory is `~/work/herdr` or below it SHALL receive the entry's tokens
- **AND** a space at `~/work/herdr-old` SHALL NOT match

### Requirement: Metadata keys and values are validated
Static token keys SHALL follow the same rules as API metadata token keys: non-empty, bounded length, ASCII alphanumeric, `_` or `-`. Values SHALL be trimmed, stripped of control characters and truncated to the API value length limit. An empty value SHALL be ignored.

#### Scenario: Invalid key
- **WHEN** an entry declares the token key `my env`
- **THEN** config loading SHALL report a diagnostic naming the key, and that entry SHALL be ignored

#### Scenario: Invalid match
- **WHEN** an entry sets both `label` and `path`, or neither
- **THEN** config loading SHALL report a diagnostic, and that entry SHALL be ignored

### Requirement: Later entries and API values take precedence
All entries that match a space SHALL apply in config order, and a later entry SHALL override an earlier one for the same key. A value set at runtime through the API for that space and key SHALL override every static value.

#### Scenario: Two matching entries
- **WHEN** entry 1 sets `env = "dev"` and entry 2 sets `env = "prod"`, and both match a space
- **THEN** that space's `$env` SHALL be `prod`

#### Scenario: API override
- **WHEN** config sets `env = "dev"` for a space and the API sets `env = "hotfix"` on it
- **THEN** `$env` SHALL render `hotfix`
- **AND** after the API value is cleared or expires, `$env` SHALL render `dev`

### Requirement: Static metadata renders through space tokens
Static metadata SHALL be available to every `$name` token in `[ui.sidebar.spaces] rows`, including token styles, conditional rules and hiding. Rows SHALL elide missing values exactly as they do for API tokens.

#### Scenario: Styled static token
- **WHEN** space rows contain `{ token = "$env", rules = [{ equals = "prod", fg = "#ff0000" }] }` and a space has static `env = "prod"`
- **THEN** that space SHALL render `prod` in red

### Requirement: Static metadata stays in the client
Static metadata SHALL affect only how the sidebar renders. It SHALL NOT be stored by the server or returned by workspace API queries, and it SHALL apply to spaces from every connected machine.

#### Scenario: API query unaffected
- **WHEN** a space has static `env = "prod"` and no API-set tokens
- **THEN** a workspace API query SHALL NOT report an `env` token

### Requirement: Metadata follows config reload
Changing `[[ui.sidebar.spaces.metadata]]` SHALL take effect on config reload without restarting the client.

#### Scenario: Reload
- **WHEN** the user edits an entry's value and reloads config
- **THEN** the next frame SHALL render the new value
