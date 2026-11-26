# Spec Delta

## Purpose

Shows each agent's subagents in the sidebar agents panel as collapsible nested rows, so users can see delegated work without switching panes.

## ADDED Requirements

### Requirement: Agent rows indicate subagents

An agent row in the sidebar agents panel whose pane has one or more subagents in its session history SHALL show an expand/collapse chevron and the number of running subagents. Agent rows without subagents SHALL look exactly as they do today.

#### Scenario: Agent with running subagents
- **WHEN** a pane's agent has two running subagents
- **THEN** its agent row SHALL show a chevron and a count of 2

#### Scenario: Agent without subagents
- **WHEN** a pane's agent has no subagents
- **THEN** its agent row SHALL show no chevron and no count

#### Scenario: Only done subagents remain
- **WHEN** a pane's subagents are all `done` and still within the linger window
- **THEN** the agent row SHALL show a chevron and no running count

#### Scenario: Only historical subagents
- **WHEN** a pane's current list is empty but its session history is not
- **THEN** the agent row SHALL show a chevron and no running count

### Requirement: Subagent rows are collapsed by default

Subagent rows SHALL be collapsed by default. Clicking an agent row's chevron SHALL toggle between expanded and collapsed for that agent row only. Expanded state SHALL be sidebar presentation state, like the panel's scroll position. It SHALL NOT appear in the JSON API or in persisted session state.

#### Scenario: Expand an agent's subagents
- **WHEN** the user clicks the chevron of a collapsed agent row with subagents
- **THEN** that agent's subagent rows SHALL be shown beneath it

#### Scenario: Collapse again
- **WHEN** the user clicks the chevron of an expanded agent row
- **THEN** its subagent rows SHALL be hidden

#### Scenario: Expand state not exposed or persisted
- **WHEN** the user expands an agent row and Herdr later restarts or restores the session
- **THEN** the agent row SHALL be collapsed
- **AND** the JSON API SHALL NOT report expand state

#### Scenario: Pane closes while expanded
- **WHEN** an expanded agent row's pane is closed
- **THEN** its expand state SHALL be discarded

### Requirement: Subagent rows show type, description, and state

Each expanded subagent row SHALL show the subagent's type, its description (truncated to fit the sidebar width), and a running or done indicator using the sidebar's existing state styling. Nested subagents SHALL be indented beneath their parent subagent.

#### Scenario: Running subagent row
- **WHEN** an expanded agent has a running subagent of type "Explore" with description "find auth code"
- **THEN** a row SHALL show "Explore", the description, and a working-style indicator

#### Scenario: Done subagent row
- **WHEN** a subagent is `done` and within the linger window
- **THEN** its row SHALL show a done-style indicator

#### Scenario: Nested subagent
- **WHEN** a subagent has a parent subagent
- **THEN** its row SHALL be indented one level deeper than its parent's row

#### Scenario: Long description
- **WHEN** a description is wider than the available row width
- **THEN** it SHALL be truncated to fit without wrapping onto additional rows

### Requirement: Clicking a subagent row focuses the parent pane

Clicking a subagent row SHALL behave the same as clicking its parent agent row: it SHALL focus the pane running the parent agent.

#### Scenario: Click subagent row
- **WHEN** the user clicks a subagent row
- **THEN** the pane of its parent agent SHALL become focused

### Requirement: Agent panel layout accounts for subagent rows

Expanded subagent rows SHALL take part in the agents panel's height, scrolling, scrollbar, and hit-testing, so that every visible row's click target matches what's drawn.

#### Scenario: Expanded rows overflow the panel
- **WHEN** expanded subagent rows make the panel content taller than its area
- **THEN** the panel SHALL scroll and show a scrollbar as it does for agent rows

#### Scenario: Subagent list changes while expanded
- **WHEN** a subagent is added or removed while its agent row is expanded
- **THEN** the rows below SHALL reflow and their click targets SHALL match their new positions

### Requirement: Per-agent subagent list mode

Each agent row SHALL have a subagent list mode, `current` (default) or `history`. When expanded, the entry SHALL show a mode row above its subagent rows indicating the active mode; clicking it SHALL toggle that agent row's mode only. In `current` mode the rows SHALL show the current list; in `history` mode they SHALL show the session history, with `interrupted` entries using a distinct indicator. The mode SHALL be sidebar presentation state: not in the JSON API, not persisted, and discarded when the pane closes.

#### Scenario: Switch to history
- **WHEN** the user clicks the mode row of an expanded agent in `current` mode
- **THEN** that agent's rows SHALL show its session history
- **AND** other agents' modes SHALL be unchanged

#### Scenario: Switch back
- **WHEN** the user clicks the mode row of an agent in `history` mode
- **THEN** that agent's rows SHALL show the current list

#### Scenario: Current mode with empty current list
- **WHEN** an expanded agent in `current` mode has history but no current subagents
- **THEN** only the mode row SHALL be shown beneath it

#### Scenario: Mode not persisted
- **WHEN** Herdr restarts or the pane closes
- **THEN** the agent row's mode SHALL be `current`
