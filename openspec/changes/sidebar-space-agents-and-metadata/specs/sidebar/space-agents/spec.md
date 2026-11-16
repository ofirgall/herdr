# Spec Delta

## Purpose

Shows each space's agents as nested child entries under that space in the sidebar, each with its tab title and its own status, so users can see what every space is doing without a separate agents list.

## ADDED Requirements

### Requirement: Agent children mode is configurable
The sidebar SHALL read `[ui.sidebar.spaces] agents` with values `"off"`, `"collapsed"` or `"expanded"`, defaulting to `"off"`. An unknown value SHALL be a config error. With `"off"`, spaces SHALL render exactly as they do today, with no children and no toggles.

#### Scenario: Default is off
- **WHEN** the option is not set
- **THEN** no agent children and no per-space agent toggle SHALL be rendered

#### Scenario: Expanded by default
- **WHEN** `agents = "expanded"` and a space has two agents
- **THEN** the two agent children SHALL be rendered directly below that space's rows

#### Scenario: Collapsed by default
- **WHEN** `agents = "collapsed"` and a space has agents
- **THEN** no children SHALL be rendered for it until the user expands that space

### Requirement: One child per agent in the space
When a space's children are expanded, the sidebar SHALL render one child entry for each agent whose pane belongs to that space, ordered by tab order and then by pane order within the tab. Children SHALL be indented relative to their space. Children SHALL ignore any agent-view filter that applies to the agents section.

#### Scenario: Space without agents
- **WHEN** a space has no agents
- **THEN** it SHALL render no children and no agent toggle

#### Scenario: Agent view does not hide children
- **WHEN** an agent view filter excludes an agent from the agents section
- **THEN** that agent SHALL still appear as a child of its space

### Requirement: Child rows use configurable agent tokens
Each child SHALL be composed from `[ui.sidebar.spaces] agent_rows`, which accepts the same tokens, styles and rules as `[ui.sidebar.agents] rows` and has the same row and token limits. It SHALL default to `[["state_icon", "tab"]]`. The `state_icon` and `state_text` tokens SHALL reflect that agent's own status, not the space aggregate.

#### Scenario: Default child row
- **WHEN** an agent in tab "api" is blocked and `agent_rows` is not set
- **THEN** the child SHALL show the blocked status icon followed by `api`

#### Scenario: Individual status differs from space
- **WHEN** a space has one working and one idle agent
- **THEN** each child SHALL show its own status icon while the space row keeps its aggregate status

#### Scenario: Custom child layout
- **WHEN** `agent_rows = [["state_icon", "tab", "agent"], ["$task"]]`
- **THEN** each child SHALL render those rows, eliding tokens without values and rows left empty

### Requirement: Per-space toggle overrides the default
Each space that has agents SHALL show a toggle when the mode is not `"off"`. Clicking the toggle, or choosing the matching space context-menu item, SHALL flip that space between expanded and collapsed. This SHALL NOT change any other space or the selection.

#### Scenario: Collapse one space
- **WHEN** the mode is `"expanded"` and the user clicks the toggle of space A
- **THEN** space A's children SHALL be hidden and other spaces SHALL keep their children

#### Scenario: Context menu
- **WHEN** the user opens the context menu of a space with agents
- **THEN** it SHALL offer "Show agents" or "Hide agents" according to that space's current state

### Requirement: Per-space toggles persist
A per-space override SHALL be saved in the client chrome preferences, keyed by endpoint and workspace id, and restored on the next client start. Overrides for workspaces that no longer exist SHALL be dropped when preferences are saved.

#### Scenario: Restart keeps choice
- **WHEN** the user collapses space A and restarts the client
- **THEN** space A SHALL still be collapsed

#### Scenario: Override equal to default
- **WHEN** the config default later changes to match a saved override
- **THEN** the space SHALL follow the saved override, which now has the same result

### Requirement: Clicking a child focuses its agent
Clicking a child SHALL focus that agent's workspace, tab and pane, as clicking the same agent in the agents section does.

#### Scenario: Focus from child
- **WHEN** the user clicks the child for an agent in an unfocused space
- **THEN** that space, tab and pane SHALL become focused

### Requirement: Children participate in list layout
Child rows SHALL count toward the space list's height, scrolling and reveal-on-focus. A space together with its children SHALL be treated as one list item, and `row_gap` SHALL apply between list items. Worktree grouping SHALL be unchanged: a space hidden by a collapsed worktree group SHALL hide its children too.

#### Scenario: Scrolling with children
- **WHEN** expanded children make the space list taller than the section
- **THEN** the scrollbar SHALL appear and scrolling SHALL never cut a space apart from its children

#### Scenario: Collapsed worktree group
- **WHEN** a worktree group is collapsed
- **THEN** the children of its hidden linked worktrees SHALL NOT be rendered

### Requirement: Space status can be hidden
The sidebar SHALL read `[ui.sidebar.spaces] show_status`, defaulting to `true`. When `false`, the `state_icon` and `state_text` tokens in space rows SHALL resolve to no value and be elided like any missing token, in every renderer that uses space rows. Agent child rows SHALL keep showing each agent's own status.

#### Scenario: Default shows status
- **WHEN** `show_status` is not set
- **THEN** space rows SHALL render `state_icon` and `state_text` as before

#### Scenario: Status hidden
- **WHEN** `show_status = false` and a space row is `["state_icon", "workspace"]`
- **THEN** the row SHALL render only the workspace label
- **AND** that space's expanded children SHALL still show their status icons

### Requirement: Scope of rendering
Agent children SHALL render in the expanded single-machine sidebar. The collapsed sidebar and the multi-machine sidebar SHALL be unchanged by this option.

#### Scenario: Collapsed sidebar
- **WHEN** the sidebar is collapsed and `agents = "expanded"`
- **THEN** the collapsed sidebar SHALL render as it does today
