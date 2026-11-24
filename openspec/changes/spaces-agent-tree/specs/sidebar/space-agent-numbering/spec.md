# Spec Delta

## Purpose

Numbers the agents nested under spaces with one index across the whole sidebar tree when the space tree is the only agent list on screen. The number on each agent row then matches the key that reaches it.

## ADDED Requirements

### Requirement: Tree numbering mode

Tree numbering mode SHALL be active when `[ui.sidebar.spaces] agents` is not `"off"` and `[ui.sidebar.agents] enabled = false`. It SHALL apply to the single-machine sidebar only. In any other configuration, and in the multi-machine sidebar, agent numbering SHALL stay exactly as it is today.

#### Scenario: Both conditions met
- **WHEN** `spaces.agents = "expanded"` and `agents.enabled = false`
- **THEN** tree numbering mode is active

#### Scenario: Agents section visible
- **WHEN** `spaces.agents = "expanded"` and the agents section is enabled
- **THEN** tree numbering mode is not active, and child and agents-section numbering behave as before this change

#### Scenario: Children off
- **WHEN** `spaces.agents = "off"` and `agents.enabled = false`
- **THEN** tree numbering mode is not active

### Requirement: One index across the tree

In tree numbering mode, each agent child shown in the space list SHALL have a 1-based tree index, counted down the whole list in display order. Agents without a shown child SHALL have no tree index.

#### Scenario: Numbering continues across spaces
- **WHEN** space A shows children claude and pi, and space B below it shows child codex
- **THEN** their tree indexes are 1, 2 and 3

#### Scenario: Space children collapsed
- **WHEN** space A's children are collapsed with its per-space toggle, and space B below it shows two children
- **THEN** A's agents have no tree index and B's children are 1 and 2

#### Scenario: Worktree group collapsed
- **WHEN** a collapsed worktree group hides a linked worktree that has agents
- **THEN** those agents have no tree index and the remaining children are numbered without gaps

#### Scenario: Skipped space number
- **WHEN** a space is marked to skip its jump number and shows agent children
- **THEN** its children still get tree indexes

### Requirement: agent_index renders the tree index

In tree numbering mode, the `agent_index` token in agent child rows SHALL render the child's tree index. Outside tree numbering mode, it SHALL render what it renders today.

#### Scenario: Third child overall
- **WHEN** tree numbering mode is active and the first child of the second space is the third child shown
- **THEN** its `agent_index` renders `3`

#### Scenario: Not in tree numbering mode
- **WHEN** tree numbering mode is not active
- **THEN** child `agent_index` renders exactly as before this change

### Requirement: agent_index_focused token

An `agent_index_focused` agent token SHALL be accepted wherever agent tokens are accepted. It SHALL render the same value as `agent_index` on the row of the focused agent pane. On every other row it SHALL resolve to no value and be elided with its separator.

#### Scenario: Focused child
- **WHEN** tree numbering mode is active, the focused pane is the agent with tree index 2, and `agent_rows` contains `agent_index_focused`
- **THEN** that child shows `2` for the token, and no other child shows a value for it

#### Scenario: Agents section row
- **WHEN** the agents section is enabled and its rows contain `agent_index_focused`
- **THEN** only the focused agent's row shows its agents-section index for the token

#### Scenario: Focus not on an agent
- **WHEN** the focused pane is not an agent
- **THEN** no row renders a value for `agent_index_focused`

#### Scenario: Unknown token stays an error
- **WHEN** a config uses `agent_index_focussed`
- **THEN** config loading reports an unknown sidebar token

### Requirement: Focused style on agent tokens

An agent token SHALL accept `focused`, a table of `fg`, `bold` and `dim`, in both `[ui.sidebar.agents] rows` and `[ui.sidebar.spaces] agent_rows`. On the row of the focused agent, the token's style SHALL be its `style`, then a matching rule, then `focused` over them, the same order as space tokens. On every other row `focused` SHALL have no effect. An unknown key inside `focused` SHALL be a config error.

#### Scenario: Focused agent number
- **WHEN** `agent_rows` contains `{token = "agent_index", fg = "#714fb0", focused = {fg = "#c6a6ff", bold = true}}` and the second child is the focused agent
- **THEN** that child's number is `#c6a6ff` and bold, and every other child's number is `#714fb0` and not bold

#### Scenario: Agents section row
- **WHEN** the agents section rows contain the same token
- **THEN** only the focused agent's row shows the focused style

#### Scenario: Unknown key
- **WHEN** an agent token has `focused = {italic = true}`
- **THEN** config loading reports an error
