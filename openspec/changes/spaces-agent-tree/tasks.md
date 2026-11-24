# Tasks

## 1. Tokens and mode predicate

- [x] 1.1 Add `AgentSidebarToken::AgentIndexFocused` ("agent_index_focused") with parse and serialize support. Add `focused: bool` to `AgentTokenContext`, passing `agent.focused` from every caller. Verify with token tests: the token renders the index only when focused, is elided with no stray separator otherwise, and a misspelled token is still a config error.
- [x] 1.2 Add `ClientShellConfig::tree_agent_numbering()` (`spaces.agents != Off && !agents.enabled`). Verify with unit tests covering all four on/off combinations.
- [x] 1.3 Accept `focused` on agent tokens (`AgentSidebarToken::Styled`), applied over `style` and rules on the focused agent's row, in the agents section and in space children. Verify with parse and round-trip tests, an unknown-key rejection test, and a render test where only the focused agent's `agent_index` gets the focused fg and bold in both lists.

## 2. Tree order and child numbering

- [x] 2.1 Implement the shared `tree_agent_order` in `space_agents.rs` (D2). Verify with unit tests: numbering continues across spaces; a space with collapsed children is skipped; agents in a collapsed worktree group are skipped; a space with a skipped jump number still has its agents counted; agent-view filters and priority sort are ignored.
- [x] 2.2 Give `SpaceAgentIndex::children` a `first_index`, and make `render_spaces` pass the running count in tree numbering mode or 0 otherwise (D3). Verify with a render buffer test: with `agent_rows` containing `agent_index`, two spaces render children 1, 2, then 3. Outside the mode, the existing per-space numbering test stays unchanged.

## 3. Agent keys

- [x] 3.1 Route `FocusAgent`, `NextAgent` and `PreviousAgent` through `tree_agent_order` in tree numbering mode on the single-machine sidebar (D5). Verify with action tests: focus by number; an out-of-range number is a no-op; collapsed children are not targets; wrap in both directions; skip a space without agents; start from a shell pane; no agents; a filter or priority sort is ignored. Also verify that the existing agents-section tests and multi-machine `focus_agent` tests pass unchanged.
- [x] 3.2 Verify, or implement, that the target's child row scrolls into view after an agent key in tree numbering mode. Verify with a test where the target child starts off-screen.
- [x] 3.3 Add a consistency test: for the same state, the agent-index numbers drawn on the space list equal the positions of the agent-key targets.

## 4. Docs and checks

- [x] 4.1 Document tree numbering mode and `agent_index_focused` in `docs/next/website/src/content/docs/configuration.mdx`, add `agent_index_focused` to `config-reference.json` if tokens are listed there, and add a changelog line in `docs/next/CHANGELOG.md`. Verify that the config-reference check script passes.
- [x] 4.2 Run `just check`. Verify that every failure also fails on a clean fork-v0.9.x checkout (record the baseline first).
- [ ] 4.3 Manual: set `spaces.agents = "expanded"` and `agents.enabled = false`, then run `make run`. Check that numbers continue across spaces, that collapsing a space's children or a worktree group renumbers, that `focus_agent(N)` and next/prev agent land on the shown numbers, and that `agent_index_focused` marks the focused agent.

## Workflow follow-up

- Archive the change once it is implemented and verified.
