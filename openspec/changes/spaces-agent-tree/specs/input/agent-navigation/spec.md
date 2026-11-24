# Spec Delta

## Purpose

Makes `focus_agent(N)`, `next_agent` and `previous_agent` follow the sidebar's tree numbering when the space tree is the only agent list on screen, so each agent key reaches the agent showing that number.

## ADDED Requirements

### Requirement: Agent keys follow tree order

In tree numbering mode, `focus_agent(N)`, `next_agent` and `previous_agent` SHALL target agents by tree index (see `sidebar/space-agent-numbering`). Agents with no tree index SHALL NOT be targets. Agent-view filters and the agents-section sort SHALL NOT affect targets. Outside tree numbering mode, these keys SHALL behave as today.

#### Scenario: Focus by number
- **WHEN** tree numbering mode is active and the user presses `focus_agent(3)`
- **THEN** the agent with tree index 3 is focused, switching space and tab if needed

#### Scenario: Number out of range
- **WHEN** tree numbering mode is active, two agents have tree indexes, and the user presses `focus_agent(5)`
- **THEN** focus does not change

#### Scenario: Collapsed children are not targets
- **WHEN** tree numbering mode is active and space A's children are collapsed
- **THEN** `next_agent`, `previous_agent` and `focus_agent(N)` never target A's agents

#### Scenario: Filter and sort ignored
- **WHEN** tree numbering mode is active, the agents-section sort is priority, and an agent view is active
- **THEN** targets follow tree order and include agents the view would exclude

#### Scenario: Not in tree numbering mode
- **WHEN** the agents section is enabled
- **THEN** `focus_agent(N)` targets the Nth agent in the agents section's order, as before

### Requirement: Cycling between agents

In tree numbering mode, `next_agent` and `previous_agent` SHALL step between agents only, never stopping on a space row, and SHALL wrap at both ends. When the focused pane has no tree index, `next_agent` SHALL go to tree index 1 and `previous_agent` to the last tree index. With no tree indexes, they SHALL do nothing.

#### Scenario: Wrap forward
- **WHEN** the focused pane has the last tree index
- **THEN** `next_agent` focuses tree index 1

#### Scenario: Space without agents
- **WHEN** agent 2 is in space A, agent 3 is in space C, and space B between them has no agents
- **THEN** `next_agent` from agent 2 focuses agent 3

#### Scenario: Focus outside the tree
- **WHEN** the focused pane is a shell pane
- **THEN** `next_agent` focuses tree index 1 and `previous_agent` focuses the last tree index

#### Scenario: No agents
- **WHEN** no agent has a tree index
- **THEN** `next_agent` and `previous_agent` do not change focus

### Requirement: Target revealed in the space list

After an agent key focuses an agent in tree numbering mode, the space list SHALL scroll, if needed, so that the agent's child row is visible.

#### Scenario: Off-screen child
- **WHEN** the target child row is scrolled out of the space list
- **THEN** after focusing, the list scrolls so the row is visible
