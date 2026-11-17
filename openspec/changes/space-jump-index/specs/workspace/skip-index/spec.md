# Spec Delta

## Purpose

Keeps a per-space flag that excludes the space from jump numbering, stored with the session and shared by every client and API consumer.

## ADDED Requirements

### Requirement: Spaces carry a skip-index flag
Every space SHALL have a skip-index flag, false by default. New spaces SHALL start with it false.

#### Scenario: New space
- **WHEN** a space is created
- **THEN** its skip-index flag SHALL be false

### Requirement: The flag can be set through the API
The API SHALL provide a method that sets a space's skip-index flag to a given value, with a matching CLI command `herdr workspace set-skip-index <WORKSPACE_ID> <true|false>`. Setting the current value SHALL succeed without emitting an event. An unknown workspace SHALL return the usual not-found error.

#### Scenario: Skip a space
- **WHEN** a client calls the method for space `w2` with `true`
- **THEN** `w2`'s flag SHALL be true and a workspace-updated event SHALL be emitted

#### Scenario: Unknown space
- **WHEN** the method is called with a workspace id that does not exist
- **THEN** it SHALL return a not-found error and change nothing

### Requirement: The flag is persisted with the session
The flag SHALL be saved with the session and restored on server restart and live handoff. Sessions saved before this change SHALL restore with the flag false.

#### Scenario: Restart
- **WHEN** space `w2` is skipped and the server restarts
- **THEN** `w2` SHALL still be skipped

#### Scenario: Old session file
- **WHEN** a session saved by an older herdr is restored
- **THEN** every space SHALL have the flag false

### Requirement: The flag is exposed next to other space data
Workspace info, workspace events and the client snapshot SHALL expose the flag as an optional `skip_index` field. It SHALL be omitted when false, so payloads of spaces that are not skipped are unchanged. Clients that do not know the field SHALL ignore it.

#### Scenario: Not skipped
- **WHEN** no space is skipped
- **THEN** workspace info and snapshots SHALL serialize exactly as before this change

#### Scenario: Skipped space in info
- **WHEN** space `w2` is skipped
- **THEN** its workspace info SHALL contain `"skip_index": true`
