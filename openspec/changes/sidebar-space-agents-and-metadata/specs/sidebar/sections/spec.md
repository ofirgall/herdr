# Spec Delta

## Purpose

Lets users choose which sections of the sidebar (spaces, agents) are shown, so a single section can use the whole sidebar height.

## ADDED Requirements

### Requirement: Sections can be disabled in config
The sidebar SHALL read `[ui.sidebar.spaces] enabled` and `[ui.sidebar.agents] enabled`, both defaulting to `true`. A disabled section SHALL NOT be rendered in the expanded sidebar or in the collapsed sidebar, and SHALL NOT produce click targets.

#### Scenario: Defaults keep both sections
- **WHEN** the config sets neither option
- **THEN** the sidebar SHALL show the spaces section above the agents section, as it does today

#### Scenario: Agents section disabled
- **WHEN** `[ui.sidebar.agents] enabled = false`
- **THEN** the expanded sidebar SHALL NOT show the agents header or agent list
- **AND** the collapsed sidebar SHALL NOT show its agent area

#### Scenario: Spaces section disabled
- **WHEN** `[ui.sidebar.spaces] enabled = false`
- **THEN** the expanded sidebar SHALL NOT show the spaces header or space list
- **AND** the collapsed sidebar SHALL NOT show its workspace numbers

### Requirement: A single enabled section fills the sidebar
When exactly one section is enabled, it SHALL take the full sidebar content height. No section divider SHALL be drawn or draggable, and any saved section split SHALL be kept but ignored until both sections are enabled again.

#### Scenario: Only spaces enabled
- **WHEN** only the spaces section is enabled
- **THEN** the space list SHALL extend to the sidebar footer
- **AND** dragging at the former divider row SHALL NOT change the split

#### Scenario: Split restored when re-enabled
- **WHEN** the user had a saved section split, disabled one section, and then re-enabled it
- **THEN** the sidebar SHALL use the saved split again

### Requirement: Sidebar controls stay reachable
The `new`, `menu` and collapse-toggle controls SHALL remain visible and clickable whichever sections are enabled. When the spaces section is disabled, the `new` and `menu` footer SHALL be drawn at the bottom of the sidebar.

#### Scenario: Footer with agents only
- **WHEN** the spaces section is disabled and mouse capture is on
- **THEN** the `new` and `menu` controls SHALL appear on the bottom sidebar row and behave as before

### Requirement: Disabling every section is rejected
A config that disables both sections SHALL be reported as a config diagnostic, and both sections SHALL be shown, as if neither option were set.

#### Scenario: Both disabled
- **WHEN** the config sets both `enabled = false`
- **THEN** the user SHALL see a config warning naming the options
- **AND** both sections SHALL render

### Requirement: Section visibility follows config reload
Changing either `enabled` option SHALL take effect on config reload without restarting the client.

#### Scenario: Live reload
- **WHEN** the user sets `[ui.sidebar.agents] enabled = false` and reloads config
- **THEN** the next frame SHALL render without the agents section
