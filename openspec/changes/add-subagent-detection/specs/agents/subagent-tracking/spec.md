# Spec Delta

## Purpose

Lets Herdr discover the subagents that a pane's coding agent spawns, track whether each is running or done, and expose them as shared runtime data for any client.

## ADDED Requirements

### Requirement: Subagents discovered from the agent session's on-disk records

For a pane whose Claude Code session has a reported transcript path, Herdr SHALL discover subagents from the subagent metadata records Claude Code stores alongside that transcript. Panes without a reported session path, or running other agents, SHALL have no subagents.

#### Scenario: Claude session spawns a subagent
- **WHEN** a pane runs Claude Code with a reported session transcript path
- **AND** Claude Code spawns a subagent during the session
- **THEN** the pane's subagent list SHALL contain that subagent with its type and description

#### Scenario: No session path reported
- **WHEN** a pane runs Claude Code but no session transcript path was reported
- **THEN** the pane's subagent list SHALL be empty

#### Scenario: Non-Claude agent
- **WHEN** a pane runs an agent other than Claude Code
- **THEN** the pane's subagent list SHALL be empty

### Requirement: Subagent discovery is change-driven

Herdr SHALL update a pane's subagent list when the subagent records change on disk, without waiting for user input or a redraw. Subagent discovery SHALL NOT perform filesystem I/O during view computation or rendering.

#### Scenario: New subagent appears without interaction
- **WHEN** Claude Code writes a new subagent record for a tracked session
- **THEN** the pane's subagent list SHALL include it within a few seconds, without any user input

#### Scenario: Subagents directory created after session start
- **WHEN** a tracked session has no subagent records yet
- **AND** Claude Code later creates the first subagent record
- **THEN** Herdr SHALL discover it

### Requirement: Finished subagents are marked done

A subagent whose record marks it as finished SHALL have the state `done`. A subagent whose record does not mark it as finished SHALL have the state `running`, subject to the liveness and session-scope requirements.

#### Scenario: Subagent completes
- **WHEN** a running subagent's record becomes marked as finished
- **THEN** the subagent's state SHALL change from `running` to `done`

### Requirement: Done subagents linger briefly

A subagent that transitions to `done` while Herdr is observing it SHALL remain in the list for a short linger window (about 30 seconds) and SHALL then be removed. Subagents that were already finished when Herdr first observed them SHALL NOT be listed.

#### Scenario: Done subagent disappears after linger
- **WHEN** a subagent transitions to `done`
- **THEN** it SHALL remain listed as `done` during the linger window
- **AND** it SHALL be removed from the list after the linger window elapses

#### Scenario: Historical finished subagents are not shown
- **WHEN** Herdr starts tracking a session that already has finished subagent records
- **THEN** those subagents SHALL NOT appear in the list

### Requirement: Subagents require a live parent agent

Subagents run inside the parent agent process. When the pane's agent exits, is released, or its session changes, Herdr SHALL remove all of that session's subagents right away, without a `done` linger, whatever their records say.

#### Scenario: Parent agent crashes with unfinished subagents
- **WHEN** a pane's Claude Code process exits while a subagent record is not marked finished
- **THEN** the pane's subagent list SHALL become empty right away

#### Scenario: Pane switches to a different session
- **WHEN** a pane's reported agent session changes to a different session
- **THEN** subagents of the previous session SHALL be removed from the pane's list

### Requirement: Subagents scoped to the current session run

When a session is resumed, Herdr SHALL ignore unfinished subagent records that were created before Herdr observed the current session start, so leftovers from an earlier crashed run are never shown as running.

#### Scenario: Resumed session with stale unfinished record
- **WHEN** a session is resumed whose directory holds an unfinished subagent record from a previous run
- **THEN** that subagent SHALL NOT appear in the pane's list

#### Scenario: New subagent in resumed session
- **WHEN** a resumed session spawns a new subagent
- **THEN** the new subagent SHALL appear in the pane's list as `running`

### Requirement: Subagents form a tree

Each subagent SHALL have an optional parent subagent. When the spawning record of a subagent can be attributed to another tracked subagent of the same session, that subagent SHALL be its parent. Otherwise the subagent SHALL be a direct child of the pane's agent.

#### Scenario: Subagent spawned by the main agent
- **WHEN** the main agent spawns a subagent
- **THEN** the subagent SHALL have no parent subagent

#### Scenario: Subagent spawned by another subagent
- **WHEN** a subagent spawns a nested subagent and the spawn can be attributed
- **THEN** the nested subagent's parent SHALL be the spawning subagent

#### Scenario: Unattributable nested subagent
- **WHEN** a subagent's spawner can't be determined
- **THEN** the subagent SHALL be a direct child of the pane's agent

### Requirement: Subagents never change parent agent state

Subagent tracking SHALL be informational only. It SHALL NOT change the pane's detected agent state (working, idle, blocked, unknown), its notifications, or its attention state.

#### Scenario: Background subagent running while parent is idle
- **WHEN** the pane's agent is detected as idle from the screen
- **AND** a background subagent is `running`
- **THEN** the pane's agent state SHALL remain idle

### Requirement: Unreadable records degrade safely

Subagent records that are missing, partially written, malformed, or in an unrecognized format SHALL be skipped without errors shown to the user and without affecting other subagents or pane state.

#### Scenario: Malformed record
- **WHEN** a subagent record can't be parsed
- **THEN** that subagent SHALL be omitted
- **AND** other subagents of the session SHALL still be listed

#### Scenario: Record written mid-read
- **WHEN** a record is read while partially written
- **THEN** Herdr SHALL retry on the next change and list the subagent once the record is complete

### Requirement: Subagents exposed through the JSON API

Pane information returned by the JSON API SHALL include the pane's current subagents, with id, type, description, parent subagent id, whether it runs in the background, state (`running` or `done`), and start and end timestamps when known. Herdr SHALL emit an event when a pane's subagent list changes.

#### Scenario: Reading a pane with subagents
- **WHEN** a client requests pane information for a pane with running subagents
- **THEN** the response SHALL include each subagent's id, type, description, parent id, background flag, and state

#### Scenario: Subscribing to subagent changes
- **WHEN** a client subscribes to subagent change events
- **AND** a subagent is added, changes state, or is removed
- **THEN** the client SHALL receive an event identifying the pane

### Requirement: Session subagent history

Herdr SHALL keep, alongside the current list, a history of all subagents recorded for the pane's current agent session, including subagents that finished before Herdr observed them and without the done linger. A history entry SHALL be `done` when its record is finished, `running` when unfinished and spawned during the current run, and `interrupted` when unfinished and spawned before Herdr observed the current session start. History SHALL be cleared under the same conditions as the current list (parent exit, release, or session change).

#### Scenario: Historical finished subagents appear in history
- **WHEN** Herdr starts tracking a session that already has finished subagent records
- **THEN** those subagents SHALL appear in the history as `done`
- **AND** they SHALL NOT appear in the current list

#### Scenario: Resume leftover appears as interrupted
- **WHEN** a resumed session's directory holds an unfinished record from a previous run
- **THEN** the history SHALL list it as `interrupted`, never as `running`

#### Scenario: Done subagents stay in history after linger
- **WHEN** a done subagent's linger window elapses
- **THEN** it SHALL be removed from the current list but remain in the history

### Requirement: Subagent history exposed through the JSON API

The JSON API SHALL provide a pane subagents request with a scope of `current` (default) or `history` returning the corresponding list. The subagent change event SHALL be emitted when either list changes.

#### Scenario: Request history
- **WHEN** a client requests a pane's subagents with scope `history`
- **THEN** the response SHALL include every history entry with its state `running`, `done`, or `interrupted`

#### Scenario: Default scope
- **WHEN** a client requests a pane's subagents without a scope
- **THEN** the response SHALL equal the pane info's `subagents` field

### Requirement: Background shells tracked from the session transcript

For a pane whose Claude Code session has a reported transcript path, Herdr SHALL track the session's background shells (shell commands started to run in the background) from the transcript, with id, command, description, state (`running`, `done`, `failed`, `killed`, or, in history, `interrupted`), and start and end times. Tracking SHALL read only new transcript content after the first read, SHALL NOT perform filesystem I/O during view computation or rendering, and SHALL follow the same liveness, session-run, linger, and history rules as subagents.

#### Scenario: Background shell starts
- **WHEN** Claude Code starts a background shell in a tracked session
- **THEN** the pane's current background shell list SHALL contain it as `running`

#### Scenario: Background shell ends
- **WHEN** the transcript records that a running background shell completed or failed
- **THEN** its state SHALL become `done` or `failed`, it SHALL linger in the current list for about 30 seconds, and it SHALL remain in the history

#### Scenario: Resume leftover shell
- **WHEN** a resumed session's transcript holds a background shell that never ended in an earlier run
- **THEN** it SHALL appear in the history as `interrupted` and SHALL NOT appear in the current list

#### Scenario: Parent exits
- **WHEN** the pane's agent exits or its session changes
- **THEN** both background shell lists SHALL become empty right away

### Requirement: Background shells exposed through the JSON API

Pane information SHALL include the pane's current background shells. A pane background shells request SHALL return the `current` (default) or `history` list. Herdr SHALL emit a background shells change event when either list changes.

#### Scenario: Request background shell history
- **WHEN** a client requests a pane's background shells with scope `history`
- **THEN** the response SHALL include every recorded background shell of the session with its state
