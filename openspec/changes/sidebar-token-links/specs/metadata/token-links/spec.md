# Spec Delta

## Purpose

Lets a reporter attach a web URL to a workspace or pane metadata token, so clients can open the resource that the token names.

## ADDED Requirements

### Requirement: Report a link with a token
`workspace report-metadata` and `pane report-metadata` SHALL accept an optional `links` map from token name to URL. The CLI SHALL accept it as repeated `--link NAME=URL`. A link SHALL be stored for a token only when the same request sets that token's value.

#### Scenario: Token with link
- **WHEN** a plugin reports `--token pr=#7009 --link pr=https://github.com/o/r/pull/7009`
- **THEN** the workspace token `pr` SHALL have value `#7009` and link `https://github.com/o/r/pull/7009`

#### Scenario: Link without a value in the request
- **WHEN** a request contains `--link pr=https://...` but does not set token `pr`
- **THEN** the request SHALL be rejected with an invalid-params error and no token SHALL change

### Requirement: Only web URLs are accepted
A link SHALL be an absolute `http` or `https` URL of at most 2048 bytes. A request with any other link SHALL be rejected as a whole with an invalid-params error naming the token.

#### Scenario: Non-web scheme
- **WHEN** a request sets `--link pr=file:///etc/passwd`
- **THEN** the request SHALL be rejected and no token SHALL change

### Requirement: A link follows its token's lifecycle
Setting a token's value SHALL replace its link with the link given in the same request, or remove it when none is given. Clearing a token, or the token expiring through its TTL, SHALL remove its link. Sequence and source rules SHALL apply to links exactly as to values.

#### Scenario: Value updated without link
- **WHEN** token `pr` has a link and a later request sets `--token pr=#7010` with no link
- **THEN** token `pr` SHALL have value `#7010` and no link

#### Scenario: Token cleared
- **WHEN** token `pr` has a link and is cleared
- **THEN** both its value and its link SHALL be gone

#### Scenario: TTL expiry
- **WHEN** token `pr` was reported with a link and a TTL that has elapsed
- **THEN** both its value and its link SHALL be gone

### Requirement: Links are exposed next to tokens
Workspace info, pane info, metadata-updated events and the client snapshot SHALL expose links as an optional map next to `tokens`, containing only tokens that have a link. The field SHALL be omitted when no token has a link, so existing payloads stay unchanged.

#### Scenario: No links
- **WHEN** no token of a workspace has a link
- **THEN** its info, events and snapshot entry SHALL serialize exactly as before this change

#### Scenario: Older client
- **WHEN** a client that does not know links receives a snapshot containing links
- **THEN** it SHALL decode the snapshot and render tokens as plain text
