use super::*;
use crate::config::SpaceAgentsMode;

fn agent(pane: &str, workspace: &str, tab: &str, status: AgentStatus) -> ClientShellAgent {
    ClientShellAgent {
        subagents: Default::default(),
        subagent_history: Default::default(),
        pane_id: pane.into(),
        workspace_id: workspace.into(),
        tab_id: tab.into(),
        name: None,
        display_agent: None,
        agent: Some("claude".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: status,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        token_links: Default::default(),
        focused: false,
    }
}

fn tab(id: &str, workspace: &str, number: usize, label: &str) -> ClientShellTab {
    ClientShellTab {
        tab_id: id.into(),
        workspace_id: workspace.into(),
        number,
        label: label.into(),
        custom_label: true,
        zoomed: false,
        focused: false,
        agent_status: AgentStatus::Idle,
    }
}

fn workspace(id: &str, number: usize, label: &str) -> ClientShellWorkspace {
    let mut workspace = snapshot().workspaces.remove(0);
    workspace.workspace_id = id.into();
    workspace.active_tab_id = format!("tab_{id}");
    workspace.number = number;
    workspace.label = label.into();
    workspace.branch = None;
    workspace.focused = false;
    workspace
}

/// ws_1 has agents in its first tab ("web", pane_a and pane_c) and its second
/// ("api", pane_b; listed first among the agents), ws_2 has one agent, ws_3 has
/// none.
fn agents_snapshot() -> ClientShellSnapshot {
    let mut snapshot = snapshot();
    snapshot.workspaces[0].agent_status = AgentStatus::Working;
    snapshot.workspaces[0].branch = None;
    snapshot.workspaces.push(workspace("ws_2", 2, "docs"));
    snapshot.workspaces.push(workspace("ws_3", 3, "empty"));
    snapshot.tabs = vec![
        tab("tab_a", "ws_1", 1, "web"),
        tab("tab_b", "ws_1", 2, "api"),
        tab("tab_d", "ws_2", 1, "notes"),
    ];
    snapshot.agents = vec![
        agent("pane_b", "ws_1", "tab_b", AgentStatus::Blocked),
        agent("pane_a", "ws_1", "tab_a", AgentStatus::Working),
        agent("pane_c", "ws_1", "tab_a", AgentStatus::Idle),
        agent("pane_d", "ws_2", "tab_d", AgentStatus::Idle),
    ];
    snapshot
}

fn config_with_agents(mode: SpaceAgentsMode) -> ClientShellConfig {
    let mut config = Config::default();
    config.ui.sidebar.spaces.agents = mode;
    ClientShellConfig::from_config(&config)
}

fn state_with(config: ClientShellConfig, snapshot: ClientShellSnapshot) -> ClientShellState {
    let mut state = ClientShellState::new(config);
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state
}

fn compose_rows(state: &mut ClientShellState) -> Vec<String> {
    let frame = state.compose(106, 30).expect("composed frame");
    // Only the spaces section: it ends at the rule above the agents panel.
    frame_rows(&frame)
        .into_iter()
        .map(|row| row.chars().take(30).collect::<String>())
        .take_while(|row| !row.starts_with('─'))
        .collect()
}

fn row_index(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not found in {rows:#?}"))
}

fn left_click(state: &mut ClientShellState, x: u16, y: u16) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::empty(),
    })])
}

#[test]
fn agents_group_by_workspace_in_tab_position_then_snapshot_order_ignoring_agent_view() {
    let mut snapshot = agents_snapshot();
    snapshot.agent_view_label = Some("view".into());
    snapshot.agent_order = vec!["pane_c".into(), "pane_b".into()];
    let index = crate::client::shell::space_agents::SpaceAgentIndex::new(&snapshot);
    let ids = |workspace: &str| {
        index
            .agents(workspace)
            .iter()
            .map(|agent| agent.pane_id.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids("ws_1"), ["pane_a", "pane_c", "pane_b"]);
    assert_eq!(ids("ws_2"), ["pane_d"]);
    assert!(ids("ws_3").is_empty());

    // Tab position, not the creation number, orders the agents: a moved tab
    // keeps its number.
    snapshot.tabs.swap(0, 1);
    let index = crate::client::shell::space_agents::SpaceAgentIndex::new(&snapshot);
    let ids = index
        .agents("ws_1")
        .iter()
        .map(|agent| agent.pane_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["pane_b", "pane_a", "pane_c"]);
}

#[test]
fn effective_expansion_combines_mode_and_per_space_override() {
    let ws_1 = "ws_1";
    for (mode, default) in [
        (SpaceAgentsMode::Off, false),
        (SpaceAgentsMode::Collapsed, false),
        (SpaceAgentsMode::Expanded, true),
    ] {
        let mut state = state_with(config_with_agents(mode), agents_snapshot());
        let local = ClientEndpointId::Local;
        assert_eq!(state.space_agents_expanded(&local, ws_1), default);
        state.toggle_space_agents(&local, ws_1);
        assert_eq!(
            state.space_agents_expanded(&local, ws_1),
            mode != SpaceAgentsMode::Off && !default
        );
        assert_eq!(state.space_agents_expanded(&local, "ws_2"), default);
        state.toggle_space_agents(&local, ws_1);
        assert_eq!(state.space_agents_expanded(&local, ws_1), default);
    }
}

#[test]
fn off_mode_renders_no_children_and_no_toggle() {
    let mut state = state_with(config_with_agents(SpaceAgentsMode::Off), agents_snapshot());
    let rows = compose_rows(&mut state);
    assert!(!rows.iter().any(|row| row.contains("web")), "{rows:#?}");
    assert!(state.hits.space_agents.is_empty());
    assert!(state
        .hits
        .workspaces
        .iter()
        .all(|hit| hit.agent_toggle.is_none() && hit.child_rows == 0));
    assert!(!rows
        .iter()
        .any(|row| row.contains('▾') || row.contains('▸')));
}

#[test]
fn expanded_mode_renders_indented_children_with_their_own_status() {
    let mut state = state_with(
        config_with_agents(SpaceAgentsMode::Expanded),
        agents_snapshot(),
    );
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    let working = status_icon(AgentStatus::Working, &state.config.status_indicators).to_owned();
    let idle = status_icon(AgentStatus::Idle, &state.config.status_indicators).to_owned();
    // The space keeps its aggregate (working) status; children follow directly,
    // ordered by tab number then snapshot order, each with its own icon.
    assert!(rows[space].contains(&working), "{:?}", rows[space]);
    assert!(rows[space + 1].contains("web") && rows[space + 1].contains(&working));
    assert!(rows[space + 2].contains("web") && rows[space + 2].contains(&idle));
    assert!(rows[space + 3].contains("api"));
    assert!(rows[space + 3].contains(status_icon(
        AgentStatus::Blocked,
        &state.config.status_indicators
    )));
    // Children hang off tree connectors under the space's status icon, with
    // their content three columns beyond the space's first-row content.
    let content = rows[space].find(&working).unwrap();
    let child_icon = rows[space + 1].find(&working).unwrap();
    assert_eq!(child_icon, content + "├─ ".len(), "{rows:#?}");
    assert!(rows[space + 1].starts_with(" ├─ "), "{rows:#?}");
    assert!(rows[space + 2].starts_with(" ├─ "), "{rows:#?}");
    assert!(rows[space + 3].starts_with(" └─ "), "{rows:#?}");
    // The next space starts below ws_1's children and shows its own child.
    let docs = row_index(&rows, "docs");
    assert_eq!(docs, space + 4);
    assert!(rows[docs + 1].contains("notes"));
    // A space without agents has no toggle and no children.
    let empty = row_index(&rows, "empty");
    assert!(!rows[empty].contains('▾') && !rows[empty].contains('▸'));

    let ws_1 = &state.hits.workspaces[0];
    assert_eq!(ws_1.child_rows, 3);
    assert_eq!(state.hits.space_agents.len(), 4);
    assert_eq!(state.hits.space_agents[0].1, "pane_a");
}

#[test]
fn collapsed_mode_hides_children_until_a_space_is_expanded() {
    let mut state = state_with(
        config_with_agents(SpaceAgentsMode::Collapsed),
        agents_snapshot(),
    );
    let rows = compose_rows(&mut state);
    assert!(!rows.iter().any(|row| row.contains("web")), "{rows:#?}");
    assert!(state.hits.space_agents.is_empty());
    assert!(rows[row_index(&rows, "client-shell")].contains('▸'));
    assert!(rows[row_index(&rows, "docs")].contains('▸'));

    let toggle = state.hits.workspaces[0].agent_toggle.expect("toggle");
    left_click(&mut state, toggle.x, toggle.y);
    let rows = compose_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("web")));
    assert!(!rows.iter().any(|row| row.contains("notes")));
    assert!(rows[row_index(&rows, "client-shell")].contains('▾'));
}

#[test]
fn custom_child_rows_use_agent_tokens_and_elide_missing_values() {
    let mut config = Config::default();
    config.ui.sidebar.spaces.agents = SpaceAgentsMode::Expanded;
    config.ui.sidebar.spaces.agent_rows = toml::from_str::<crate::config::SpacesSidebarConfig>(
        "agent_rows = [[\"state_text\", \"tab\", \"agent\"], [\"$task\"], [\"$missing\"]]",
    )
    .unwrap()
    .agent_rows;
    let mut snapshot = agents_snapshot();
    snapshot.agents[1].tokens = vec![("task".into(), "ship it".into())];
    let mut state = state_with(ClientShellConfig::from_config(&config), snapshot);
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    // First child: pane_a (working). Two rows, the empty third row is elided.
    assert!(
        rows[space + 1].contains("working") && rows[space + 1].contains("web"),
        "{rows:#?}"
    );
    assert!(rows[space + 2].contains("ship it"), "{rows:#?}");
    assert!(rows[space + 3].contains("idle"), "{rows:#?}");
    assert_eq!(state.hits.workspaces[0].child_rows, 4);
}

#[test]
fn toggle_glyph_and_hit_sit_on_the_first_row_and_shift_left_of_a_group_toggle() {
    let mut state = state_with(
        config_with_agents(SpaceAgentsMode::Expanded),
        agents_snapshot(),
    );
    let rows = compose_rows(&mut state);
    let hit = &state.hits.workspaces[0];
    let toggle = hit.agent_toggle.expect("agent toggle");
    assert_eq!((toggle.y, toggle.x), (hit.rect.y, hit.rect.right() - 1));
    assert_eq!(
        rows[toggle.y as usize].chars().nth(toggle.x as usize),
        Some('▾')
    );
    assert!(state.hits.workspaces[2].agent_toggle.is_none());

    // A worktree group parent also has the group toggle at the edge.
    let mut snapshot = agents_snapshot();
    snapshot.workspaces[0].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: false,
    });
    snapshot.workspaces[1].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: true,
    });
    let mut state = state_with(config_with_agents(SpaceAgentsMode::Expanded), snapshot);
    let rows = compose_rows(&mut state);
    let hit = &state.hits.workspaces[0];
    let (group, _) = hit.group_toggle.as_ref().expect("group toggle");
    let toggle = hit.agent_toggle.expect("agent toggle");
    assert_eq!(group.x, hit.rect.right() - 1);
    assert_eq!(toggle.x, group.x - 1);
    assert_eq!(
        rows[toggle.y as usize].chars().nth(toggle.x as usize),
        Some('▾')
    );
    assert_eq!(
        rows[group.y as usize].chars().nth(group.x as usize),
        Some('▾')
    );
}

#[test]
fn toggle_click_flips_only_that_space_and_child_click_focuses_the_pane() {
    let mut state = state_with(
        config_with_agents(SpaceAgentsMode::Expanded),
        agents_snapshot(),
    );
    compose_rows(&mut state);
    let toggle = state.hits.workspaces[0].agent_toggle.expect("toggle");
    let focused_before = state
        .snapshot
        .as_deref()
        .unwrap()
        .focused_workspace_id
        .clone();
    let outcome = left_click(&mut state, toggle.x, toggle.y);
    assert!(outcome.actions.is_empty(), "toggle must not focus anything");
    assert_eq!(
        state.snapshot.as_deref().unwrap().focused_workspace_id,
        focused_before
    );
    let rows = compose_rows(&mut state);
    assert!(!rows.iter().any(|row| row.contains("web")), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains("notes")));
    assert!(rows[row_index(&rows, "client-shell")].contains('▸'));

    // Clicking a child of the other space requests focus for that pane.
    let (rect, pane_id) = state.hits.space_agents[0].clone();
    assert_eq!(pane_id, "pane_d");
    let click = left_click(&mut state, rect.x + 3, rect.y);
    let [ClientShellAction::Endpoint { request, .. }] = &click.actions[..] else {
        panic!("child click should focus through the endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_d"
    ));
}

#[test]
fn context_menu_offers_show_or_hide_agents_only_for_spaces_with_agents() {
    let items = |state: &ClientShellState| match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu.items(),
        _ => panic!("workspace context menu"),
    };
    let label_of = |state: &ClientShellState| {
        items(state)
            .into_iter()
            .find(|item| item.action == ClientContextMenuAction::ToggleAgents)
            .map(|item| item.label)
    };

    let mut state = state_with(
        config_with_agents(SpaceAgentsMode::Expanded),
        agents_snapshot(),
    );
    state.open_workspace_context_menu("ws_1".into(), 0, 0);
    assert_eq!(label_of(&state), Some("Hide agents"));
    let index = items(&state)
        .iter()
        .position(|item| item.action == ClientContextMenuAction::ToggleAgents)
        .unwrap();
    state.activate_context_menu_item(index, &mut ClientShellInput::default());
    assert!(!state.space_agents_expanded(&ClientEndpointId::Local, "ws_1"));
    assert!(state.space_agents_expanded(&ClientEndpointId::Local, "ws_2"));
    state.open_workspace_context_menu("ws_1".into(), 0, 0);
    assert_eq!(label_of(&state), Some("Show agents"));
    state.open_workspace_context_menu("ws_3".into(), 0, 0);
    assert_eq!(label_of(&state), None);

    let mut off = state_with(config_with_agents(SpaceAgentsMode::Off), agents_snapshot());
    off.open_workspace_context_menu("ws_1".into(), 0, 0);
    assert_eq!(label_of(&off), None);
}

#[test]
fn overrides_persist_and_are_pruned_for_missing_workspaces() {
    let path = std::env::temp_dir().join(format!(
        "herdr-space-agents-prefs-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let config = config_with_agents(SpaceAgentsMode::Expanded).with_preferences_path(path.clone());
    let mut state = state_with(config, agents_snapshot());
    let local = ClientEndpointId::Local;
    state.toggle_space_agents(&local, "ws_1");
    state.toggle_space_agents(&local, "gone");
    let profile = SavedSshEndpoint::new("Build", "dev@build.example", "agents").unwrap();
    let remote = ClientEndpointId::Ssh(profile.id.clone());
    state.toggle_space_agents(&remote, "remote_ws");
    state.persist_chrome_preferences(&mut ClientShellInput::default());

    let stored = std::fs::read_to_string(&path).unwrap();
    assert!(stored.contains("ws_1"));
    assert!(!stored.contains("gone"), "{stored}");
    // No snapshot is known for the remote endpoint, so its override is kept.
    assert!(stored.contains("remote_ws"));

    let reloaded = ClientShellState::new(
        config_with_agents(SpaceAgentsMode::Expanded).with_preferences_path(path.clone()),
    );
    assert!(!reloaded.space_agents_expanded(&local, "ws_1"));
    assert!(reloaded.space_agents_expanded(&local, "ws_2"));
    assert!(!reloaded.space_agents_expanded(&remote, "remote_ws"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn children_never_split_from_their_space_while_scrolling() {
    let mut snapshot = agents_snapshot();
    for (index, id) in ["ws_4", "ws_5", "ws_6"].into_iter().enumerate() {
        snapshot
            .workspaces
            .push(workspace(id, 4 + index, &format!("space-{id}")));
        let tab_id = format!("tab_{id}");
        snapshot.tabs.push(tab(&tab_id, id, 1, "t"));
        for n in 0..3 {
            snapshot.agents.push(agent(
                &format!("pane_{id}_{n}"),
                id,
                &tab_id,
                AgentStatus::Idle,
            ));
        }
    }
    let mut state = state_with(
        config_with_agents(SpaceAgentsMode::Expanded),
        snapshot.clone(),
    );
    let frame_height = 18;
    for scroll in 0..snapshot.workspaces.len() {
        state.workspace_scroll = scroll;
        state.reveal_focused_workspace = false;
        state.compose(106, frame_height).expect("frame");
        assert!(
            state.hits.workspace_max_scroll > 0,
            "list should overflow the section"
        );
        let hits = &state.hits;
        assert!(!hits.workspaces.is_empty());
        for hit in &hits.workspaces {
            let expected = snapshot
                .agents
                .iter()
                .filter(|agent| agent.workspace_id == hit.workspace_id)
                .count() as u16;
            // Every drawn space carries all of its children (one row each).
            let drawn = hits
                .space_agents
                .iter()
                .filter(|(rect, _)| {
                    rect.y >= hit.rect.bottom() && rect.y < hit.rect.bottom() + hit.child_rows
                })
                .count() as u16;
            assert_eq!(hit.child_rows, expected, "{}", hit.workspace_id);
            assert_eq!(drawn, expected, "{}", hit.workspace_id);
            assert!(hit.rect.bottom() + hit.child_rows <= hits.workspace_body.bottom());
        }
    }

    // Reveal-on-focus brings the focused space and its children into view.
    let mut focused = snapshot.clone();
    focused.workspaces[0].focused = false;
    focused.workspaces[5].focused = true;
    focused.focused_workspace_id = Some("ws_6".into());
    state.set_snapshot(Box::new(focused));
    state.workspace_scroll = 0;
    state.reveal_focused_workspace = true;
    state.compose(106, frame_height).expect("frame");
    let hit = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.workspace_id == "ws_6")
        .expect("focused space revealed");
    assert_eq!(hit.child_rows, 3);
}

#[test]
fn collapsed_worktree_group_hides_the_children_of_hidden_members() {
    let mut snapshot = agents_snapshot();
    snapshot.workspaces[0].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: false,
    });
    snapshot.workspaces[1].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: true,
    });
    let mut state = state_with(config_with_agents(SpaceAgentsMode::Expanded), snapshot);
    compose_rows(&mut state);
    assert!(state
        .hits
        .space_agents
        .iter()
        .any(|(_, pane)| pane == "pane_d"));

    state.collapsed_groups.insert("repo".into());
    let rows = compose_rows(&mut state);
    assert!(
        !state
            .hits
            .space_agents
            .iter()
            .any(|(_, pane)| pane == "pane_d"),
        "{rows:#?}"
    );
    assert!(!rows.iter().any(|row| row.contains("notes")));
    // The visible parent keeps its own children.
    assert!(state
        .hits
        .space_agents
        .iter()
        .any(|(_, pane)| pane == "pane_a"));
}

#[test]
fn collapsed_sidebar_ignores_agent_children() {
    let mut state = state_with(
        config_with_agents(SpaceAgentsMode::Expanded),
        agents_snapshot(),
    );
    state.sidebar_collapsed = true;
    state.compose(106, 30).expect("frame");
    assert!(state.hits.space_agents.is_empty());
    assert!(state
        .hits
        .workspaces
        .iter()
        .all(|hit| hit.agent_toggle.is_none()));
}

#[test]
fn children_of_a_non_last_linked_worktree_keep_the_tree_rail() {
    let mut snapshot = agents_snapshot();
    for (index, linked) in [(0, false), (1, true), (2, true)] {
        snapshot.workspaces[index].worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: linked,
        });
    }
    let mut state = state_with(config_with_agents(SpaceAgentsMode::Expanded), snapshot);
    let rows = compose_rows(&mut state);

    // ws_2 ("docs") has a sibling below it, so its child row continues the rail
    // in the same column as the `├` connectors, with its own connector under
    // the worktree's status icon.
    let child = &rows[row_index(&rows, "notes")];
    assert!(child.starts_with(" │  └─ "), "{rows:#?}");
    let docs = &rows[row_index(&rows, "docs")];
    assert!(docs.starts_with(" ├─ "), "{rows:#?}");
    let empty = &rows[row_index(&rows, "empty")];
    assert!(empty.starts_with(" └─ "), "{rows:#?}");
    // The parent's agents share the trunk with its worktrees, so even its last
    // agent keeps a `├─` connector down to the first worktree.
    let parent_child = &rows[row_index(&rows, "api")];
    assert!(parent_child.starts_with(" ├─ "), "{rows:#?}");
}

#[test]
fn children_of_the_last_linked_worktree_draw_no_tree_rail() {
    let mut snapshot = agents_snapshot();
    for (index, linked) in [(0, false), (1, true)] {
        snapshot.workspaces[index].worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: linked,
        });
    }
    let mut state = state_with(config_with_agents(SpaceAgentsMode::Expanded), snapshot);
    let rows = compose_rows(&mut state);
    let child = &rows[row_index(&rows, "notes")];
    assert!(!child.chars().take(6).any(|ch| ch == '│'), "{rows:#?}");
}

#[test]
fn focused_child_highlight_starts_at_its_tree_connector_in_a_tree() {
    let mut snapshot = agents_snapshot();
    for (index, linked) in [(0, false), (1, true), (2, true)] {
        snapshot.workspaces[index].worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: linked,
        });
    }
    // pane_d is the only agent of the linked worktree "docs".
    snapshot.agents[3].focused = true;
    let mut state = state_with(config_with_agents(SpaceAgentsMode::Expanded), snapshot);
    let frame = state.compose(106, 30).expect("composed frame");
    let child = state
        .hits
        .space_agents
        .iter()
        .find(|(_, pane)| pane == "pane_d")
        .expect("child hit")
        .0;
    let buffer = frame.to_ratatui_buffer().expect("frame should reconstruct");
    let highlight = state.config.palette.active_row_bg;
    // Tree rail at column 1, connector at 4: the highlight starts at the
    // connector.
    assert_eq!(buffer[(child.x + 1, child.y)].symbol(), "│");
    assert_eq!(buffer[(child.x + 4, child.y)].symbol(), "└");
    for x in child.x..child.x + 4 {
        assert_ne!(buffer[(x, child.y)].bg, highlight, "tree column {x}");
    }
    for x in child.x + 4..child.right() {
        assert_eq!(buffer[(x, child.y)].bg, highlight, "content column {x}");
    }
}

#[test]
fn focused_child_highlight_starts_at_its_tree_connector_at_top_level() {
    let mut snapshot = agents_snapshot();
    snapshot.agents[0].focused = true;
    let mut state = state_with(config_with_agents(SpaceAgentsMode::Expanded), snapshot);
    let frame = state.compose(106, 30).expect("composed frame");
    let child = state
        .hits
        .space_agents
        .iter()
        .find(|(_, pane)| pane == "pane_b")
        .expect("child hit")
        .0;
    let buffer = frame.to_ratatui_buffer().expect("frame should reconstruct");
    let highlight = state.config.palette.active_row_bg;
    // pane_b ("api") is the last child: the highlight starts at its connector
    // in column 1.
    assert_eq!(buffer[(child.x + 1, child.y)].symbol(), "└");
    for x in child.x..child.x + 1 {
        assert_ne!(buffer[(x, child.y)].bg, highlight, "tree column {x}");
    }
    for x in child.x + 1..child.right() {
        assert_eq!(buffer[(x, child.y)].bg, highlight, "content column {x}");
    }
}

/// Sidebar text (first 30 columns) of a frame with agent rows configured for
/// the agents panel and for agents nested under spaces.
fn sidebar_text_with_tab_tokens(token: &str, tabs: [&str; 2]) -> (String, String) {
    let mut config = Config::default();
    config.ui.sidebar.spaces.agents = SpaceAgentsMode::Expanded;
    config.ui.sidebar.agents.rows =
        toml::from_str::<crate::config::AgentsSidebarConfig>(&format!("rows = [[\"{token}\"]]"))
            .unwrap()
            .rows;
    config.ui.sidebar.spaces.agent_rows = toml::from_str::<crate::config::SpacesSidebarConfig>(
        &format!("agent_rows = [[\"{token}\"]]"),
    )
    .unwrap()
    .agent_rows;
    let mut snapshot = snapshot();
    snapshot.agents = vec![agent("pane_1", "ws_1", "tab_2", AgentStatus::Idle)];
    // Creation numbers differ from positions: the second tab is number 5.
    snapshot.tabs = vec![
        tab("tab_1", "ws_1", 1, tabs[0]),
        tab("tab_2", "ws_1", 5, tabs[1]),
    ];
    let mut state = state_with(ClientShellConfig::from_config(&config), snapshot);
    let frame = state.compose(106, 30).expect("composed frame");
    let rows = frame_rows(&frame)
        .into_iter()
        .map(|row| row.chars().take(30).collect::<String>())
        .collect::<Vec<_>>();
    // The spaces section ends at the first rule; the agents panel is below.
    let split = rows
        .iter()
        .position(|row| row.starts_with('─'))
        .unwrap_or(rows.len());
    (rows[..split].join("\n"), rows[split..].join("\n"))
}

/// Foreground and boldness of the first letter of every `needle` in the
/// sidebar's first 30 columns, top to bottom: the space child first, then the
/// agents panel row.
fn sidebar_word_styles(frame: &FrameData, needle: &str) -> Vec<(ratatui::style::Color, bool)> {
    let buffer = frame.to_ratatui_buffer().expect("frame should reconstruct");
    frame_rows(frame)
        .iter()
        .enumerate()
        .filter_map(|(y, row)| {
            let sidebar = row.chars().take(30).collect::<String>();
            let x = sidebar.find(needle)?;
            let cell = &buffer[(sidebar[..x].chars().count() as u16, y as u16)];
            Some((
                cell.fg,
                cell.modifier.contains(ratatui::style::Modifier::BOLD),
            ))
        })
        .collect()
}

#[test]
fn focused_agent_tab_names_use_tab_focused_and_bold() {
    use ratatui::style::Color;
    let tab_color = Color::Rgb(0x71, 0x4f, 0xb0);
    let focused_color = Color::Rgb(0xc6, 0xa6, 0xff);
    let styles = |token: &str, tab_focused: Option<Color>| {
        let mut config = Config::default();
        config.ui.sidebar.spaces.agents = SpaceAgentsMode::Expanded;
        config.ui.sidebar.agents.rows =
            toml::from_str::<crate::config::AgentsSidebarConfig>(&format!("rows = [[{token}]]"))
                .unwrap()
                .rows;
        config.ui.sidebar.spaces.agent_rows = toml::from_str::<crate::config::SpacesSidebarConfig>(
            &format!("agent_rows = [[{token}]]"),
        )
        .unwrap()
        .agent_rows;
        let mut config = ClientShellConfig::from_config(&config);
        config.palette.tab = Some(tab_color);
        config.palette.tab_focused = tab_focused;
        let mut snapshot = snapshot();
        snapshot.tabs = vec![
            tab("tab_1", "ws_1", 1, "alpha"),
            tab("tab_2", "ws_1", 2, "bravo"),
        ];
        snapshot.agents = vec![
            agent("pane_1", "ws_1", "tab_1", AgentStatus::Idle),
            agent("pane_2", "ws_1", "tab_2", AgentStatus::Idle),
        ];
        snapshot.agents[0].focused = true;
        let mut state = state_with(config, snapshot);
        let frame = state.compose(106, 30).expect("composed frame");
        (
            sidebar_word_styles(&frame, "alpha"),
            sidebar_word_styles(&frame, "bravo"),
        )
    };
    // Each agent shows as a space child and as an agents panel row.
    let (focused, other) = styles("\"tab\"", Some(focused_color));
    assert_eq!(focused, vec![(focused_color, true); 2]);
    assert_eq!(other, vec![(tab_color, false); 2]);
    let (focused, other) = styles("\"tab\"", None);
    assert_eq!(focused, vec![(tab_color, true); 2]);
    assert_eq!(other, vec![(tab_color, false); 2]);
    // A token's own `bold` wins over the focused weight.
    let (focused, _) = styles("{token = \"tab\", bold = false}", Some(focused_color));
    assert_eq!(focused, vec![(focused_color, false); 2]);
}

#[test]
fn focused_style_on_agent_index_applies_only_to_the_focused_agents_row() {
    use ratatui::style::Color;
    let base = Color::Rgb(0x71, 0x4f, 0xb0);
    let bright = Color::Rgb(0xc6, 0xa6, 0xff);
    let token =
        r##"{token = "agent_index", fg = "#714fb0", focused = {fg = "#c6a6ff", bold = true}}"##;
    let mut config = Config::default();
    config.ui.sidebar.spaces.agents = SpaceAgentsMode::Expanded;
    config.ui.sidebar.agents.rows = toml::from_str::<crate::config::AgentsSidebarConfig>(&format!(
        "rows = [[{token}, \"tab\"]]"
    ))
    .unwrap()
    .rows;
    config.ui.sidebar.spaces.agent_rows = toml::from_str::<crate::config::SpacesSidebarConfig>(
        &format!("agent_rows = [[{token}, \"tab\"]]"),
    )
    .unwrap()
    .agent_rows;
    let mut snapshot = snapshot();
    snapshot.tabs = vec![
        tab("tab_1", "ws_1", 1, "alpha"),
        tab("tab_2", "ws_1", 2, "bravo"),
    ];
    snapshot.agents = vec![
        agent("pane_1", "ws_1", "tab_1", AgentStatus::Idle),
        agent("pane_2", "ws_1", "tab_2", AgentStatus::Idle),
    ];
    snapshot.agents[0].focused = true;
    let mut state = state_with(ClientShellConfig::from_config(&config), snapshot);
    let frame = state.compose(106, 30).expect("composed frame");
    // Each agent shows as a space child and as an agents panel row.
    assert_eq!(
        sidebar_word_styles(&frame, "1 alpha"),
        vec![(bright, true); 2]
    );
    assert_eq!(
        sidebar_word_styles(&frame, "2 bravo"),
        vec![(base, false); 2]
    );
}

#[test]
fn tab_stripped_hides_the_tab_position_in_panel_and_space_children() {
    let tabs = ["1| web", "2| zsh"];
    let (spaces, panel) = sidebar_text_with_tab_tokens("tab_stripped", tabs);
    for text in [&spaces, &panel] {
        assert!(text.contains("zsh") && !text.contains("2| zsh"), "{text}");
    }
    let (spaces, panel) = sidebar_text_with_tab_tokens("tab", tabs);
    for text in [&spaces, &panel] {
        assert!(text.contains("2| zsh"), "{text}");
    }
    // The creation number (5) is not the position, so it is not stripped.
    let (spaces, panel) = sidebar_text_with_tab_tokens("tab_stripped", ["1| web", "5| zsh"]);
    for text in [&spaces, &panel] {
        assert!(text.contains("5| zsh"), "{text}");
    }
}

#[test]
fn subspace_row_gap_spaces_worktree_children_and_keeps_the_tree_rail() {
    let mut snapshot = agents_snapshot();
    for (index, linked) in [(0, false), (1, true), (2, true)] {
        snapshot.workspaces[index].worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: linked,
        });
    }
    snapshot.workspaces.push(workspace("ws_4", 4, "other"));
    let mut config = Config::default();
    config.ui.sidebar.spaces.agents = SpaceAgentsMode::Off;
    config.ui.sidebar.spaces.rows = vec![vec![crate::config::SpaceSidebarToken::Workspace]];
    config.ui.sidebar.spaces.row_gap = 1;
    config.ui.sidebar.spaces.subspace_row_gap = 1;
    let mut state = state_with(ClientShellConfig::from_config(&config), snapshot);
    let rows = compose_rows(&mut state);

    let parent = row_index(&rows, "client-shell");
    let docs = row_index(&rows, "docs");
    let empty = row_index(&rows, "empty");
    let other = row_index(&rows, "other");
    assert_eq!(docs, parent + 2, "{rows:#?}");
    assert_eq!(empty, docs + 2, "{rows:#?}");
    assert_eq!(other, empty + 2, "{rows:#?}");
    // Gaps inside the group continue the tree; the gap after the group does not.
    let rail = |row: &str| row.chars().nth(1) == Some('│');
    assert!(rail(&rows[parent + 1]), "{rows:#?}");
    assert!(rail(&rows[docs + 1]), "{rows:#?}");
    assert!(!rail(&rows[empty + 1]), "{rows:#?}");
}

#[test]
fn subspace_row_gap_defaults_to_packed_worktree_children() {
    let mut snapshot = agents_snapshot();
    for (index, linked) in [(0, false), (1, true)] {
        snapshot.workspaces[index].worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: linked,
        });
    }
    let mut config = Config::default();
    config.ui.sidebar.spaces.rows = vec![vec![crate::config::SpaceSidebarToken::Workspace]];
    config.ui.sidebar.spaces.row_gap = 1;
    let mut state = state_with(ClientShellConfig::from_config(&config), snapshot);
    let rows = compose_rows(&mut state);
    assert_eq!(
        row_index(&rows, "docs"),
        row_index(&rows, "client-shell") + 1,
        "{rows:#?}"
    );
}

#[test]
fn row_gap_after_agents_only_spaces_only_entries_with_shown_agents() {
    // ws_1 (parent) and ws_2 ("docs") have agents; ws_3 ("empty") has none.
    let mut snapshot = agents_snapshot();
    for (index, linked) in [(0, false), (1, true), (2, true)] {
        snapshot.workspaces[index].worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: linked,
        });
    }
    snapshot.workspaces.push(workspace("ws_4", 4, "other"));
    snapshot.workspaces.push(workspace("ws_5", 5, "quiet"));
    let mut config = Config::default();
    config.ui.sidebar.spaces.agents = SpaceAgentsMode::Expanded;
    config.ui.sidebar.spaces.rows = vec![vec![crate::config::SpaceSidebarToken::Workspace]];
    config.ui.sidebar.spaces.row_gap = 1;
    config.ui.sidebar.spaces.subspace_row_gap = 1;
    config.ui.sidebar.spaces.row_gap_after_agents_only = true;
    let mut state = state_with(ClientShellConfig::from_config(&config), snapshot);
    let rows = compose_rows(&mut state);

    let docs = row_index(&rows, "docs");
    let notes = row_index(&rows, "notes");
    let empty = row_index(&rows, "empty");
    let other = row_index(&rows, "other");
    // docs shows an agent, so a gap (continuing the rail) follows it.
    assert_eq!(empty, notes + 2, "{rows:#?}");
    assert_eq!(rows[notes + 1].chars().nth(1), Some('│'), "{rows:#?}");
    // empty shows no agents but closes the worktree group, so a plain gap
    // (no rail) separates it from "other".
    assert_eq!(other, empty + 2, "{rows:#?}");
    assert_eq!(
        rows[empty + 1].trim_matches(['│', '▕', ' ']),
        "",
        "{rows:#?}"
    );
    assert_ne!(rows[empty + 1].chars().nth(1), Some('│'), "{rows:#?}");
    // Top-level spaces without agents stay packed.
    assert_eq!(row_index(&rows, "quiet"), other + 1, "{rows:#?}");
    // The parent shows agents, so a gap precedes docs.
    assert_eq!(rows[docs - 1].chars().nth(1), Some('│'), "{rows:#?}");
    assert!(!rows[docs - 1].contains("api"), "{rows:#?}");
}

#[test]
fn multi_row_children_continue_the_connector_until_the_last_child() {
    let mut config = Config::default();
    config.ui.sidebar.spaces.agents = SpaceAgentsMode::Expanded;
    config.ui.sidebar.spaces.agent_rows =
        toml::from_str::<std::collections::HashMap<String, crate::config::SpacesSidebarConfig>>(
            "[x]\nagent_rows = [[\"state_icon\", \"tab\"], [\"agent\"]]\n",
        )
        .unwrap()
        .remove("x")
        .unwrap()
        .agent_rows;
    let mut state = state_with(ClientShellConfig::from_config(&config), agents_snapshot());
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");

    // web, web, api: each child is two rows; continuation rows carry `│`
    // until the last child, whose continuation row has no connector.
    assert!(rows[space + 1].starts_with(" ├─ "), "{rows:#?}");
    assert!(rows[space + 2].starts_with(" │    claude"), "{rows:#?}");
    assert!(rows[space + 5].starts_with(" └─ "), "{rows:#?}");
    assert!(rows[space + 6].starts_with("      claude"), "{rows:#?}");
}

#[test]
fn configured_tree_glyphs_draw_each_tree_with_its_own_glyphs() {
    let mut config: Config = toml::from_str(
        "[ui.sidebar.spaces]\nagents = \"expanded\"\n\
         [ui.sidebar.tree.worktrees]\nlast = \"╰─\"\n\
         [ui.sidebar.tree.agents]\nbranch = \"├╌\"\nlast = \"╰╌\"\nrail = \"┆\"\n",
    )
    .unwrap();
    config.ui.sidebar.spaces.agent_rows =
        toml::from_str::<std::collections::HashMap<String, crate::config::SpacesSidebarConfig>>(
            "[x]\nagent_rows = [[\"state_icon\", \"tab\"], [\"agent\"]]\n",
        )
        .unwrap()
        .remove("x")
        .unwrap()
        .agent_rows;
    let mut snapshot = agents_snapshot();
    for (index, linked) in [(0, false), (1, true), (2, true)] {
        snapshot.workspaces[index].worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: linked,
        });
    }
    let mut state = state_with(ClientShellConfig::from_config(&config), snapshot);
    let rows = compose_rows(&mut state);

    // The parent's agents branch off the worktree trunk, which keeps its rail.
    let api = row_index(&rows, "api");
    assert!(rows[api].starts_with(" ├╌ "), "{rows:#?}");
    assert!(rows[api + 1].starts_with(" │    claude"), "{rows:#?}");
    // A worktree's own agents use the agent glyphs throughout.
    let notes = row_index(&rows, "notes");
    assert!(rows[notes].starts_with(" │  ╰╌ "), "{rows:#?}");
    assert!(
        rows[row_index(&rows, "docs")].starts_with(" ├─ "),
        "{rows:#?}"
    );
    assert!(
        rows[row_index(&rows, "empty")].starts_with(" ╰─ "),
        "{rows:#?}"
    );
}

#[test]
fn tree_glyphs_are_fitted_to_their_columns() {
    let glyphs = crate::client::shell::space_agents::TreeGlyphs::from_config(
        &crate::config::TreeGlyphsConfig {
            branch: "+".into(),
            last: "╰──>".into(),
            rail: "".into(),
        },
    );
    assert_eq!(glyphs.branch, "+ ");
    assert_eq!(glyphs.last, "╰─");
    assert_eq!(glyphs.rail, " ");
}

/// Presses a previous/next sidebar entry binding and names the focus request
/// it sends: `space:<workspace>` or `agent:<pane>`.
fn step_sidebar(state: &mut ClientShellState, next: bool) -> String {
    let action = if next {
        crate::input::KeybindAction::NextSidebarEntry
    } else {
        crate::input::KeybindAction::PreviousSidebarEntry
    };
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    match &outcome.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => match &request.method {
            crate::api::schema::Method::WorkspaceFocus(target) => {
                format!("space:{}", target.workspace_id)
            }
            crate::api::schema::Method::PaneFocus(target) => format!("agent:{}", target.pane_id),
            method => panic!("unexpected method {method:?}"),
        },
        actions => panic!("expected one focus request, got {actions:?}"),
    }
}

/// Applies the focus a step requested, as the server would. Focusing ws_1
/// lands on pane_a, one of the agents shown under it.
fn land(state: &mut ClientShellState, target: &str) {
    let mut snapshot = state.snapshot.as_deref().expect("snapshot").clone();
    let (workspace, pane) = match target {
        "space:ws_1" | "agent:pane_a" => ("ws_1", "pane_a"),
        "agent:pane_b" => ("ws_1", "pane_b"),
        "agent:pane_c" => ("ws_1", "pane_c"),
        "space:ws_2" | "agent:pane_d" => ("ws_2", "pane_d"),
        "space:ws_3" => ("ws_3", "pane_e"),
        other => panic!("unknown target {other}"),
    };
    snapshot.focused_workspace_id = Some(workspace.into());
    snapshot.focused_pane_id = Some(pane.into());
    state.set_snapshot(Box::new(snapshot));
}

fn walk_sidebar(state: &mut ClientShellState, next: bool, steps: usize) -> Vec<String> {
    (0..steps)
        .map(|_| {
            let target = step_sidebar(state, next);
            land(state, &target);
            target
        })
        .collect()
}

#[test]
fn next_sidebar_entry_walks_spaces_and_their_shown_agents_in_order() {
    let mut snapshot = agents_snapshot();
    snapshot.focused_pane_id = Some("pane_a".into());
    let mut state = state_with(config_with_agents(SpaceAgentsMode::Expanded), snapshot);
    // From the focused agent's row; agents follow their space in tab order.
    assert_eq!(
        walk_sidebar(&mut state, true, 7),
        [
            "agent:pane_c",
            "agent:pane_b",
            // Focusing ws_2 lands on its agent pane_d, the row the sidebar
            // then highlights, so one more step reaches ws_3.
            "space:ws_2",
            "space:ws_3",
            // Likewise ws_1 lands on pane_a, so the next step is pane_c.
            "space:ws_1",
            "agent:pane_c",
            "agent:pane_b",
        ]
    );
}

#[test]
fn previous_sidebar_entry_passes_the_space_of_the_focused_agent() {
    let mut snapshot = agents_snapshot();
    snapshot.focused_pane_id = Some("pane_a".into());
    let mut state = state_with(config_with_agents(SpaceAgentsMode::Expanded), snapshot);
    // From pane_a its own space ws_1 is already focused, so the step passes
    // it; the same holds for ws_2 from pane_d.
    assert_eq!(
        walk_sidebar(&mut state, false, 4),
        ["space:ws_3", "agent:pane_d", "agent:pane_b", "agent:pane_c"]
    );
}

#[test]
fn sidebar_entry_steps_advance_before_the_focus_arrives() {
    let mut snapshot = agents_snapshot();
    snapshot.focused_pane_id = Some("pane_a".into());
    let mut state = state_with(config_with_agents(SpaceAgentsMode::Expanded), snapshot);
    assert_eq!(step_sidebar(&mut state, true), "agent:pane_c");
    assert_eq!(step_sidebar(&mut state, true), "agent:pane_b");
    assert_eq!(step_sidebar(&mut state, true), "space:ws_2");
}

#[test]
fn sidebar_entries_skip_hidden_agents_and_restart_from_focus_after_a_click() {
    let mut snapshot = agents_snapshot();
    snapshot.focused_pane_id = Some("pane_a".into());
    // Collapsed: only spaces, so the walk starts from the focused space.
    let mut state = state_with(
        config_with_agents(SpaceAgentsMode::Collapsed),
        snapshot.clone(),
    );
    assert_eq!(
        walk_sidebar(&mut state, true, 3),
        ["space:ws_2", "space:ws_3", "space:ws_1"]
    );
    // Expanding one space shows only its agents; focusing ws_2 lands on its
    // agent pane_d.
    state.toggle_space_agents(&ClientEndpointId::Local, "ws_2");
    assert_eq!(
        walk_sidebar(&mut state, true, 3),
        ["space:ws_2", "space:ws_3", "space:ws_1"]
    );

    let mut state = state_with(config_with_agents(SpaceAgentsMode::Expanded), snapshot);
    assert_eq!(walk_sidebar(&mut state, true, 1), ["agent:pane_c"]);
    // A click elsewhere drops the cursor; the walk restarts from the focus.
    state.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        },
        &mut ClientShellInput::default(),
    );
    land(&mut state, "agent:pane_d");
    assert_eq!(step_sidebar(&mut state, true), "space:ws_3");
}

/// Presses a move sidebar entry binding and returns the request it sends.
fn move_sidebar(state: &mut ClientShellState, down: bool) -> Option<crate::api::schema::Method> {
    let action = if down {
        crate::input::KeybindAction::MoveSidebarEntryNext
    } else {
        crate::input::KeybindAction::MoveSidebarEntryPrevious
    };
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    match &outcome.actions[..] {
        // Nothing to move: the binding passes through unhandled, as other
        // shell actions with no target do.
        [ClientShellAction::Keybind(passed)] if *passed == action => None,
        [ClientShellAction::Endpoint { request, .. }] => Some(request.method.clone()),
        actions => panic!("expected at most one request, got {actions:?}"),
    }
}

/// A state focused on `workspace`'s non-agent pane, with agents collapsed so
/// the current entry is the space's row.
fn focused_space_state(snapshot: &ClientShellSnapshot, workspace: &str) -> ClientShellState {
    let mut snapshot = snapshot.clone();
    snapshot.focused_workspace_id = Some(workspace.into());
    snapshot.focused_pane_id = Some("pane_shell".into());
    state_with(config_with_agents(SpaceAgentsMode::Collapsed), snapshot)
}

#[test]
fn move_sidebar_entry_moves_a_space_past_its_neighbour_without_wrapping() {
    use crate::api::schema::{Method, WorkspaceMoveParams};
    let snapshot = agents_snapshot();
    let moved = |workspace: &str, insert_index| {
        Some(Method::WorkspaceMove(WorkspaceMoveParams {
            workspace_id: workspace.into(),
            insert_index,
        }))
    };
    let mut state = focused_space_state(&snapshot, "ws_2");
    assert_eq!(move_sidebar(&mut state, false), moved("ws_2", 0));
    assert_eq!(move_sidebar(&mut state, true), moved("ws_2", 3));
    let mut state = focused_space_state(&snapshot, "ws_1");
    assert_eq!(move_sidebar(&mut state, false), None);
    let mut state = focused_space_state(&snapshot, "ws_3");
    assert_eq!(move_sidebar(&mut state, true), None);
}

#[test]
fn move_sidebar_entry_moves_a_worktree_group_as_one_block() {
    use crate::api::schema::{Method, WorkspaceMoveBlockParams};
    let mut snapshot = agents_snapshot();
    for (index, linked) in [(0, false), (1, true)] {
        snapshot.workspaces[index].worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: linked,
        });
    }
    // From the linked worktree, its group (led by ws_1) passes ws_3.
    let mut state = focused_space_state(&snapshot, "ws_2");
    assert_eq!(
        move_sidebar(&mut state, true),
        Some(Method::WorkspaceMoveBlock(WorkspaceMoveBlockParams {
            workspace_ids: vec!["ws_1".into(), "ws_2".into()],
            before_workspace_id: None,
        }))
    );
    assert_eq!(move_sidebar(&mut state, false), None);
}

#[test]
fn move_sidebar_entry_moves_an_agent_tab_past_the_next_agent_tab() {
    use crate::api::schema::{Method, TabMoveParams};
    let mut snapshot = agents_snapshot();
    // A tab without agents sits between ws_1's two agent tabs.
    snapshot.tabs.insert(1, tab("tab_x", "ws_1", 3, "shell"));
    let moved = |tab: &str, insert_index| {
        Some(Method::TabMove(TabMoveParams {
            tab_id: tab.into(),
            insert_index,
        }))
    };
    let focused_on = |pane: &str| {
        let mut snapshot = snapshot.clone();
        snapshot.focused_workspace_id = Some(
            snapshot
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane)
                .expect("agent")
                .workspace_id
                .clone(),
        );
        snapshot.focused_pane_id = Some(pane.into());
        state_with(config_with_agents(SpaceAgentsMode::Expanded), snapshot)
    };
    // tab_a goes after tab_b, skipping the agentless tab: [tab_x, tab_b, tab_a].
    assert_eq!(
        move_sidebar(&mut focused_on("pane_a"), true),
        moved("tab_a", 3)
    );
    assert_eq!(move_sidebar(&mut focused_on("pane_a"), false), None);
    assert_eq!(
        move_sidebar(&mut focused_on("pane_b"), false),
        moved("tab_b", 0)
    );
    assert_eq!(move_sidebar(&mut focused_on("pane_b"), true), None);
    // ws_2 has one agent tab, so its agent has nothing to pass.
    assert_eq!(move_sidebar(&mut focused_on("pane_d"), true), None);
}

fn tree_config(spaces: SpaceAgentsMode, agents_enabled: bool) -> ClientShellConfig {
    let mut config = Config::default();
    config.ui.sidebar.spaces.agents = spaces;
    config.ui.sidebar.agents.enabled = agents_enabled;
    ClientShellConfig::from_config(&config)
}

fn tree_config_with_rows(rows: &str, agents_enabled: bool) -> ClientShellConfig {
    let mut config = Config::default();
    config.ui.sidebar.spaces.agents = SpaceAgentsMode::Expanded;
    config.ui.sidebar.agents.enabled = agents_enabled;
    config.ui.sidebar.spaces.agent_rows =
        toml::from_str::<crate::config::SpacesSidebarConfig>(&format!("agent_rows = {rows}"))
            .unwrap()
            .agent_rows;
    ClientShellConfig::from_config(&config)
}

/// The tree order the agent keys will use.
fn tree_order(state: &ClientShellState) -> Vec<String> {
    state.tree_agent_targets(state.snapshot.as_deref().expect("snapshot"))
}

#[test]
fn tree_numbering_mode_needs_children_on_and_the_agents_section_off() {
    for (spaces, agents_enabled, expected) in [
        (SpaceAgentsMode::Expanded, false, true),
        (SpaceAgentsMode::Collapsed, false, true),
        (SpaceAgentsMode::Expanded, true, false),
        (SpaceAgentsMode::Off, false, false),
        (SpaceAgentsMode::Off, true, false),
    ] {
        assert_eq!(
            tree_config(spaces, agents_enabled).tree_agent_numbering(),
            expected,
            "spaces {spaces:?}, agents enabled {agents_enabled}"
        );
    }
}

#[test]
fn tree_order_continues_across_spaces_and_ignores_agent_view_and_sort() {
    let mut snapshot = agents_snapshot();
    snapshot.agent_view_label = Some("view".into());
    snapshot.agent_order = vec!["pane_d".into(), "pane_b".into()];
    let state = state_with(tree_config(SpaceAgentsMode::Expanded, false), snapshot);
    assert_eq!(tree_order(&state), ["pane_a", "pane_c", "pane_b", "pane_d"]);
}

#[test]
fn tree_order_skips_spaces_with_collapsed_children() {
    let mut state = state_with(
        tree_config(SpaceAgentsMode::Expanded, false),
        agents_snapshot(),
    );
    state.toggle_space_agents(&ClientEndpointId::Local, "ws_1");
    assert_eq!(tree_order(&state), ["pane_d"]);

    // In collapsed mode only spaces the user expanded contribute.
    let mut state = state_with(
        tree_config(SpaceAgentsMode::Collapsed, false),
        agents_snapshot(),
    );
    assert!(tree_order(&state).is_empty());
    state.toggle_space_agents(&ClientEndpointId::Local, "ws_2");
    assert_eq!(tree_order(&state), ["pane_d"]);
}

#[test]
fn tree_order_skips_agents_of_a_collapsed_worktree_group() {
    let mut snapshot = agents_snapshot();
    snapshot.workspaces[0].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: false,
    });
    snapshot.workspaces[1].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: true,
    });
    let mut state = state_with(tree_config(SpaceAgentsMode::Expanded, false), snapshot);
    assert_eq!(tree_order(&state), ["pane_a", "pane_c", "pane_b", "pane_d"]);
    state.collapsed_groups.insert("repo".into());
    assert_eq!(tree_order(&state), ["pane_a", "pane_c", "pane_b"]);

    // On mobile the layout lists every group expanded, but the order still
    // follows the desktop tree.
    state.compose(44, 30).expect("mobile frame");
    assert!(state.mobile_layout_active());
    assert_eq!(tree_order(&state), ["pane_a", "pane_c", "pane_b"]);
}

#[test]
fn tree_order_counts_agents_of_a_space_that_skips_its_jump_number() {
    let mut snapshot = agents_snapshot();
    snapshot.workspaces[0].skip_index = true;
    let state = state_with(tree_config(SpaceAgentsMode::Expanded, false), snapshot);
    assert_eq!(tree_order(&state), ["pane_a", "pane_c", "pane_b", "pane_d"]);
}

/// Two spaces: ws_1 with two agents (pane_a, pane_c) and ws_2 with one.
fn two_space_numbering_snapshot() -> ClientShellSnapshot {
    let mut snapshot = agents_snapshot();
    snapshot.agents.remove(0);
    snapshot
}

fn child_rows_with(rows: &[String], needle: &str) -> Vec<String> {
    rows.iter()
        .filter(|row| row.contains(needle))
        .map(|row| {
            row.trim()
                .trim_start_matches(['├', '└', '─', ' '])
                .to_owned()
        })
        .collect()
}

#[test]
fn tree_numbering_continues_child_indexes_across_spaces() {
    let mut state = state_with(
        tree_config_with_rows("[[\"agent_index\", \"tab\"]]", false),
        two_space_numbering_snapshot(),
    );
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    let docs = row_index(&rows, "docs");
    assert!(rows[space + 1].contains("1 web"), "{rows:#?}");
    assert!(rows[space + 2].contains("2 web"), "{rows:#?}");
    assert!(rows[docs + 1].contains("3 notes"), "{rows:#?}");

    // Collapsing the first space renumbers the next one from 1.
    state.toggle_space_agents(&ClientEndpointId::Local, "ws_1");
    let rows = compose_rows(&mut state);
    assert!(
        child_rows_with(&rows, "notes")
            .iter()
            .any(|row| row.starts_with("1 notes")),
        "{rows:#?}"
    );
}

#[test]
fn child_indexes_stay_per_space_outside_tree_numbering_mode() {
    // The agents section is enabled, so children keep per-space numbers.
    let mut state = state_with(
        tree_config_with_rows("[[\"agent_index\", \"tab\"]]", true),
        two_space_numbering_snapshot(),
    );
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    let docs = row_index(&rows, "docs");
    assert!(rows[space + 1].contains("1 web"), "{rows:#?}");
    assert!(rows[space + 2].contains("2 web"), "{rows:#?}");
    assert!(rows[docs + 1].contains("1 notes"), "{rows:#?}");
}

#[test]
fn agent_index_focused_marks_only_the_focused_child() {
    let mut snapshot = two_space_numbering_snapshot();
    snapshot.agents[2].focused = true; // pane_d, the third child
    let mut state = state_with(
        tree_config_with_rows("[[\"agent_index_focused\", \"tab\"]]", false),
        snapshot,
    );
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    let docs = row_index(&rows, "docs");
    assert!(rows[docs + 1].contains("3 notes"), "{rows:#?}");
    for row in &rows[space + 1..space + 3] {
        assert!(!row.contains(|c: char| c.is_ascii_digit()), "{rows:#?}");
        assert!(row.contains("web"), "{rows:#?}");
    }
}

/// Presses an agent key and names the pane it asks to focus, or `None` when
/// the key sends nothing.
fn press_agent_key(
    state: &mut ClientShellState,
    action: crate::input::KeybindAction,
) -> Option<String> {
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    match &outcome.actions[..] {
        [] => None,
        [ClientShellAction::Endpoint { request, .. }] => match &request.method {
            crate::api::schema::Method::PaneFocus(target) => Some(target.pane_id.clone()),
            method => panic!("unexpected method {method:?}"),
        },
        actions => panic!("expected at most one focus request, got {actions:?}"),
    }
}

fn focus_pane(state: &mut ClientShellState, workspace: &str, pane: &str) {
    let mut snapshot = state.snapshot.as_deref().expect("snapshot").clone();
    snapshot.focused_workspace_id = Some(workspace.into());
    snapshot.focused_pane_id = Some(pane.into());
    for candidate in &mut snapshot.workspaces {
        candidate.focused = candidate.workspace_id == workspace;
    }
    for candidate in &mut snapshot.agents {
        candidate.focused = candidate.pane_id == pane;
    }
    state.set_snapshot(Box::new(snapshot));
}

fn tree_state() -> ClientShellState {
    state_with(
        tree_config(SpaceAgentsMode::Expanded, false),
        agents_snapshot(),
    )
}

#[test]
fn focus_agent_by_number_follows_tree_order_across_spaces_and_tabs() {
    use crate::input::KeybindAction::FocusAgent;
    let mut state = tree_state();
    // Tree order is pane_a, pane_c, pane_b (another tab), pane_d (another space).
    let picked = (0..4)
        .map(|index| press_agent_key(&mut state, FocusAgent(index)))
        .collect::<Vec<_>>();
    assert_eq!(
        picked,
        [
            Some("pane_a".to_owned()),
            Some("pane_c".to_owned()),
            Some("pane_b".to_owned()),
            Some("pane_d".to_owned()),
        ]
    );
}

#[test]
fn focus_agent_out_of_range_does_nothing() {
    use crate::input::KeybindAction::FocusAgent;
    let mut state = tree_state();
    assert_eq!(press_agent_key(&mut state, FocusAgent(4)), None);
    assert_eq!(press_agent_key(&mut state, FocusAgent(9)), None);
}

#[test]
fn collapsed_children_are_not_agent_key_targets() {
    use crate::input::KeybindAction::{FocusAgent, NextAgent, PreviousAgent};
    let mut state = tree_state();
    state.toggle_space_agents(&ClientEndpointId::Local, "ws_1");
    assert_eq!(
        press_agent_key(&mut state, FocusAgent(0)),
        Some("pane_d".into())
    );
    assert_eq!(press_agent_key(&mut state, FocusAgent(1)), None);
    // From a shell pane next/previous reach only the shown agent.
    assert_eq!(
        press_agent_key(&mut state, NextAgent),
        Some("pane_d".into())
    );
    assert_eq!(
        press_agent_key(&mut state, PreviousAgent),
        Some("pane_d".into())
    );
}

#[test]
fn next_and_previous_agent_wrap_and_skip_spaces_without_agents() {
    use crate::input::KeybindAction::{NextAgent, PreviousAgent};
    // ws_3 has no agents; add ws_4 with one so ws_3 sits between two agents.
    let mut snapshot = agents_snapshot();
    snapshot.workspaces.push(workspace("ws_4", 4, "last"));
    snapshot.tabs.push(tab("tab_e", "ws_4", 1, "tail"));
    snapshot
        .agents
        .push(agent("pane_e", "ws_4", "tab_e", AgentStatus::Idle));
    let mut state = state_with(tree_config(SpaceAgentsMode::Expanded, false), snapshot);
    assert_eq!(
        tree_order(&state),
        ["pane_a", "pane_c", "pane_b", "pane_d", "pane_e"]
    );

    // Forward, skipping empty ws_3 and wrapping past the end.
    focus_pane(&mut state, "ws_2", "pane_d");
    assert_eq!(
        press_agent_key(&mut state, NextAgent),
        Some("pane_e".into())
    );
    focus_pane(&mut state, "ws_4", "pane_e");
    assert_eq!(
        press_agent_key(&mut state, NextAgent),
        Some("pane_a".into())
    );
    // Backward, wrapping past the start and skipping empty ws_3.
    focus_pane(&mut state, "ws_1", "pane_a");
    assert_eq!(
        press_agent_key(&mut state, PreviousAgent),
        Some("pane_e".into())
    );
    focus_pane(&mut state, "ws_4", "pane_e");
    assert_eq!(
        press_agent_key(&mut state, PreviousAgent),
        Some("pane_d".into())
    );
}

#[test]
fn agent_keys_from_a_shell_pane_go_to_the_first_or_last_target() {
    use crate::input::KeybindAction::{NextAgent, PreviousAgent};
    let mut state = tree_state();
    // The snapshot focuses pane_1, which is not an agent.
    assert_eq!(
        press_agent_key(&mut state, NextAgent),
        Some("pane_a".into())
    );
    assert_eq!(
        press_agent_key(&mut state, PreviousAgent),
        Some("pane_d".into())
    );
}

#[test]
fn agent_keys_do_nothing_without_agents() {
    use crate::input::KeybindAction::{FocusAgent, NextAgent, PreviousAgent};
    let mut snapshot = agents_snapshot();
    snapshot.agents.clear();
    let mut state = state_with(tree_config(SpaceAgentsMode::Expanded, false), snapshot);
    for action in [FocusAgent(0), NextAgent, PreviousAgent] {
        assert_eq!(press_agent_key(&mut state, action), None);
    }
}

#[test]
fn agent_keys_ignore_the_agent_view_and_priority_sort() {
    use crate::input::KeybindAction::{FocusAgent, NextAgent};
    let mut snapshot = agents_snapshot();
    // The view hides every agent but pane_d and the section order is reversed.
    snapshot.agent_view_label = Some("view".into());
    snapshot.agent_order = vec!["pane_d".into()];
    let mut config = Config::default();
    config.ui.sidebar.spaces.agents = SpaceAgentsMode::Expanded;
    config.ui.sidebar.agents.enabled = false;
    let mut config = ClientShellConfig::from_config(&config);
    config.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
    let mut state = state_with(config, snapshot);
    assert_eq!(
        press_agent_key(&mut state, FocusAgent(0)),
        Some("pane_a".into())
    );
    assert_eq!(
        press_agent_key(&mut state, FocusAgent(2)),
        Some("pane_b".into())
    );
    assert_eq!(
        press_agent_key(&mut state, NextAgent),
        Some("pane_a".into())
    );
}

#[test]
fn agent_keys_on_the_mobile_layout_follow_the_desktop_tree_order() {
    use crate::input::KeybindAction::FocusAgent;
    let mut snapshot = agents_snapshot();
    snapshot.workspaces[0].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: false,
    });
    snapshot.workspaces[1].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: true,
    });
    let mut state = state_with(tree_config(SpaceAgentsMode::Expanded, false), snapshot);
    state.collapsed_groups.insert("repo".into());
    state.compose(44, 30).expect("mobile frame");
    assert!(state.mobile_layout_active());
    // The mobile layout lists every group expanded, but the collapsed worktree
    // group still hides pane_d from the keys, as on desktop.
    assert_eq!(
        press_agent_key(&mut state, FocusAgent(2)),
        Some("pane_b".into())
    );
    assert_eq!(press_agent_key(&mut state, FocusAgent(3)), None);
}

#[test]
fn agent_keys_are_unchanged_when_the_agents_section_is_enabled() {
    use crate::input::KeybindAction::FocusAgent;
    let mut snapshot = agents_snapshot();
    snapshot.agent_order = vec!["pane_d".into(), "pane_b".into()];
    let mut state = state_with(tree_config(SpaceAgentsMode::Expanded, true), snapshot);
    assert!(!state.tree_agent_keys_active());
    // The agents section order (snapshot order, here) decides, not the tree.
    assert_eq!(
        press_agent_key(&mut state, FocusAgent(0)),
        Some("pane_b".into())
    );
}

/// Eight spaces, each with two agents, in a short frame so the list overflows.
fn overflowing_tree_state() -> ClientShellState {
    let mut snapshot = agents_snapshot();
    snapshot.agents.clear();
    snapshot.workspaces.truncate(1);
    snapshot.tabs.truncate(1);
    snapshot
        .agents
        .push(agent("pane_a", "ws_1", "tab_a", AgentStatus::Idle));
    for number in 2..=8 {
        let id = format!("ws_{number}");
        let tab_id = format!("tab_{id}");
        snapshot
            .workspaces
            .push(workspace(&id, number, &format!("space-{number}")));
        snapshot.tabs.push(tab(&tab_id, &id, 1, "t"));
        for n in 0..2 {
            snapshot.agents.push(agent(
                &format!("pane_{number}_{n}"),
                &id,
                &tab_id,
                AgentStatus::Idle,
            ));
        }
    }
    state_with(tree_config(SpaceAgentsMode::Expanded, false), snapshot)
}

#[test]
fn agent_key_reveals_an_off_screen_target_child_row() {
    use crate::input::KeybindAction::FocusAgent;
    let mut state = overflowing_tree_state();
    let height = 18;
    state.compose(106, height).expect("frame");
    assert!(state.hits.workspace_max_scroll > 0, "list should overflow");
    assert!(
        !state
            .hits
            .space_agents
            .iter()
            .any(|(_, pane)| pane == "pane_8_1"),
        "target starts off-screen"
    );

    // pane_8_1 is the last agent of the tree: number 15.
    let pane = press_agent_key(&mut state, FocusAgent(14)).expect("target");
    assert_eq!(pane, "pane_8_1");
    // The server lands focus on the target.
    focus_pane(&mut state, "ws_8", "pane_8_1");
    state.compose(106, height).expect("frame");
    assert!(
        state
            .hits
            .space_agents
            .iter()
            .any(|(_, pane)| pane == "pane_8_1"),
        "{:#?}",
        state.hits.space_agents
    );
}

#[test]
fn agent_key_reveals_a_same_space_target_after_the_list_was_scrolled_away() {
    use crate::input::KeybindAction::FocusAgent;
    let mut state = overflowing_tree_state();
    let height = 18;
    // Focus sits in ws_8; the user then scrolls the list back to the top.
    focus_pane(&mut state, "ws_8", "pane_8_0");
    state.compose(106, height).expect("frame");
    state.workspace_scroll = 0;
    state.compose(106, height).expect("frame");
    assert!(
        !state
            .hits
            .space_agents
            .iter()
            .any(|(_, pane)| pane == "pane_8_1"),
        "target starts off-screen"
    );

    // The focused workspace does not change, so only the key's own reveal can
    // bring the row back.
    assert_eq!(
        press_agent_key(&mut state, FocusAgent(14)),
        Some("pane_8_1".into())
    );
    state.compose(106, height).expect("frame");
    assert!(
        state
            .hits
            .space_agents
            .iter()
            .any(|(_, pane)| pane == "pane_8_1"),
        "{:#?}",
        state.hits.space_agents
    );
}

#[test]
fn rendered_child_numbers_equal_one_plus_the_target_position() {
    let mut snapshot = agents_snapshot();
    snapshot.workspaces.push(workspace("ws_4", 4, "tail"));
    snapshot.tabs.push(tab("tab_e", "ws_4", 1, "tail-tab"));
    snapshot
        .agents
        .push(agent("pane_e", "ws_4", "tab_e", AgentStatus::Idle));
    // ws_1 and ws_2 form a worktree group; ws_2 is its linked member.
    snapshot.workspaces[0].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: false,
    });
    snapshot.workspaces[1].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: true,
    });
    let mut state = state_with(
        tree_config_with_rows("[[\"agent_index\", \"tab\"]]", false),
        snapshot,
    );

    let drawn = |state: &mut ClientShellState| -> Vec<(String, usize)> {
        // Child rows read `<n> <tab>`; pair each with its pane by position in
        // the hit map, which lists children in drawn order.
        let rows = compose_rows(state);
        let hits = state.hits.space_agents.clone();
        let frame_rows = rows;
        hits.iter()
            .map(|(rect, pane)| {
                let row = &frame_rows[usize::from(rect.y)];
                let number = row
                    .trim()
                    .trim_start_matches(['├', '└', '─', ' '])
                    .split_whitespace()
                    .next()
                    .and_then(|word| word.parse::<usize>().ok())
                    .unwrap_or_else(|| panic!("no number on {row:?} in {frame_rows:#?}"));
                (pane.clone(), number)
            })
            .collect()
    };

    let check = |state: &mut ClientShellState| {
        let targets = tree_order(state);
        let drawn = drawn(state);
        assert!(!targets.is_empty());
        assert_eq!(drawn.len(), targets.len(), "{drawn:?} vs {targets:?}");
        for (pane, number) in drawn {
            let position = targets
                .iter()
                .position(|target| *target == pane)
                .unwrap_or_else(|| panic!("{pane} drawn but not a target in {targets:?}"));
            assert_eq!(number, position + 1, "{pane}");
        }
    };

    check(&mut state);
    // A collapsed space renumbers everything after it.
    state.toggle_space_agents(&ClientEndpointId::Local, "ws_1");
    check(&mut state);
    // So does a collapsed worktree group (ws_2 hidden).
    state.collapsed_groups.insert("repo".into());
    check(&mut state);
}

fn sub(
    id: &str,
    parent: Option<&str>,
    state: crate::api::schema::SubagentStatus,
    kind: &str,
    description: &str,
) -> crate::api::schema::PaneSubagentInfo {
    crate::api::schema::PaneSubagentInfo {
        id: id.into(),
        kind: kind.into(),
        description: description.into(),
        parent_id: parent.map(str::to_owned),
        background: false,
        state,
        started_at_ms: None,
        ended_at_ms: None,
    }
}

fn set_subagents(
    snapshot: &mut ClientShellSnapshot,
    pane: &str,
    current: Vec<crate::api::schema::PaneSubagentInfo>,
    history: Vec<crate::api::schema::PaneSubagentInfo>,
) {
    let agent = snapshot
        .agents
        .iter_mut()
        .find(|agent| agent.pane_id == pane)
        .expect("agent");
    agent.subagents = current;
    agent.subagent_history = history;
}

fn subagent_state(snapshot: ClientShellSnapshot) -> ClientShellState {
    state_with(config_with_agents(SpaceAgentsMode::Expanded), snapshot)
}

fn click_hit(state: &mut ClientShellState, rect: Rect) -> ClientShellInput {
    left_click(state, rect.x, rect.y)
}

fn toggle_for(state: &ClientShellState, pane: &str) -> Rect {
    state
        .hits
        .subagent_toggles
        .iter()
        .rfind(|(_, id)| id == pane)
        .unwrap_or_else(|| panic!("no chevron for {pane}"))
        .0
}

fn mode_for(state: &ClientShellState, pane: &str) -> Rect {
    state
        .hits
        .subagent_modes
        .iter()
        .find(|(_, id)| id == pane)
        .unwrap_or_else(|| panic!("no history icon for {pane}"))
        .0
}

/// Foreground style of the cell at `rect`'s origin in the last composed frame.
fn cell_style(state: &mut ClientShellState, rect: Rect) -> ratatui::style::Style {
    let frame = state.compose(106, 30).expect("frame");
    let buffer = frame.frame.to_ratatui_buffer().expect("buffer");
    buffer[(rect.x, rect.y)].style()
}

use crate::api::schema::SubagentStatus as Sub;

/// A sidebar row without trailing padding and the divider.
fn body(row: &str) -> &str {
    row[..row.rfind('│').unwrap_or(row.len())].trim_end()
}

/// Display column (in chars) where `needle` starts in `row`.
fn col(row: &str, needle: &str) -> usize {
    row[..row
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in {row:?}"))]
        .chars()
        .count()
}

fn mixed_snapshot() -> ClientShellSnapshot {
    let mut snapshot = agents_snapshot();
    set_subagents(
        &mut snapshot,
        "pane_a",
        vec![
            sub("s1", None, Sub::Running, "Explore", "find auth code"),
            sub("s2", None, Sub::Running, "Plan", "outline"),
            sub("s3", None, Sub::Done, "Bash", "run tests"),
        ],
        vec![
            sub("s1", None, Sub::Done, "Explore", "find auth code"),
            sub("h1", None, Sub::Interrupted, "Old", "stale task"),
        ],
    );
    snapshot
}

#[test]
fn agents_without_subagents_render_without_chevron_or_extra_hits() {
    let mut state = subagent_state(agents_snapshot());
    let rows = compose_rows(&mut state);
    assert!(!rows.iter().any(|row| row.contains('▸')), "{rows:#?}");
    assert!(state.hits.subagent_toggles.is_empty());
    assert!(state.hits.subagent_modes.is_empty());
    assert_eq!(state.hits.workspaces[0].child_rows, 3);
}

#[test]
fn agent_with_history_shows_count_icon_and_chevron_on_its_first_row() {
    let mut state = subagent_state(mixed_snapshot());
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    // Expanded by default: `[count] ↺ ▾` at the right edge.
    assert!(body(&rows[space + 1]).ends_with("2 ↺ ▾"), "{rows:#?}");
    let toggle = toggle_for(&state, "pane_a");
    let mode = mode_for(&state, "pane_a");
    assert_eq!((toggle.y as usize, toggle.width), (space + 1, 1));
    assert_eq!((mode.y as usize, mode.width), (space + 1, 1));
    assert_eq!(mode.x + 2, toggle.x);
    let count = state
        .hits
        .subagent_toggles
        .iter()
        .find(|(rect, _)| rect.x + 2 == mode.x)
        .expect("count hit")
        .0;
    assert_eq!((count.y as usize, count.width), (space + 1, 1));
    // Other agents have neither.
    assert_eq!(state.hits.subagent_modes.len(), 1);
}

#[test]
fn chevron_and_icon_have_no_count_without_running_subagents() {
    let mut snapshot = agents_snapshot();
    set_subagents(
        &mut snapshot,
        "pane_a",
        vec![sub("s3", None, Sub::Done, "Bash", "run tests")],
        vec![sub("s3", None, Sub::Done, "Bash", "run tests")],
    );
    let mut state = subagent_state(snapshot);
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    assert!(body(&rows[space + 1]).ends_with(" ↺ ▾"), "{rows:#?}");
    assert!(!body(&rows[space + 1]).ends_with("0 ↺ ▾"));
    let toggle = toggle_for(&state, "pane_a");
    assert_eq!(mode_for(&state, "pane_a").x + 2, toggle.x);
    assert_eq!(state.hits.subagent_toggles.len(), 1);

    // Only historical subagents: still chevron and icon, no count.
    let mut snapshot = agents_snapshot();
    set_subagents(
        &mut snapshot,
        "pane_a",
        Vec::new(),
        vec![sub("h", None, Sub::Done, "Old", "x")],
    );
    let mut state = subagent_state(snapshot);
    compose_rows(&mut state);
    assert_eq!(state.hits.subagent_toggles.len(), 1);
    assert_eq!(state.hits.subagent_modes.len(), 1);
}

#[test]
fn subagent_rows_are_expanded_by_default_and_the_chevron_collapses_them() {
    let mut state = subagent_state(mixed_snapshot());
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    assert!(body(&rows[space + 1]).ends_with("2 ↺ ▾"), "{rows:#?}");
    assert!(!rows.iter().any(|row| row.contains("current · history")));
    let working = status_icon(AgentStatus::Working, &state.config.status_indicators).to_owned();
    let done = status_icon(AgentStatus::Done, &state.config.status_indicators).to_owned();
    // The first subagent row directly follows the agent's token row.
    assert!(rows[space + 2].contains(&working) && rows[space + 2].contains("find auth code"));
    assert!(rows[space + 3].contains("outline Plan"));
    assert!(rows[space + 4].contains(&done) && rows[space + 4].contains("run tests Bash"));
    // Subagent rows hang one tree level deeper than the agent's content.
    let agent_icon = col(&rows[space + 1], &working);
    assert_eq!(col(&rows[space + 2], "├─"), agent_icon, "{rows:#?}");
    assert!(rows[space + 2].starts_with(" │  ├─ "), "{rows:#?}");
    assert!(rows[space + 4].contains("└─"), "{rows:#?}");
    assert!(!rows[space + 2..space + 5]
        .iter()
        .any(|row| row.contains("stale task")));
    assert_eq!(state.hits.workspaces[0].child_rows, 3 + 3);
    assert_eq!(
        rows[space + 5]
            .trim_start_matches([' ', '│'])
            .chars()
            .next(),
        Some('├')
    );

    // The chevron collapses, and clicking it again re-expands.
    let toggle = toggle_for(&state, "pane_a");
    let outcome = click_hit(&mut state, toggle);
    assert!(outcome.actions.is_empty(), "chevron must not focus");
    let rows = compose_rows(&mut state);
    assert!(body(&rows[space + 1]).ends_with("2 ↺ ▸"), "{rows:#?}");
    assert!(!rows.iter().any(|row| row.contains("find auth code")));
    assert_eq!(state.hits.workspaces[0].child_rows, 3);
    let toggle = toggle_for(&state, "pane_a");
    click_hit(&mut state, toggle);
    let rows = compose_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("find auth code")));
    assert_eq!(state.hits.workspaces[0].child_rows, 3 + 3);
}

#[test]
fn collapsed_agent_keeps_chevron_and_icon() {
    let mut state = subagent_state(mixed_snapshot());
    compose_rows(&mut state);
    let toggle = toggle_for(&state, "pane_a");
    click_hit(&mut state, toggle);
    compose_rows(&mut state);
    // The icon still shows, and still works, while collapsed.
    let mode = mode_for(&state, "pane_a");
    click_hit(&mut state, mode);
    let view = &state.subagent_views[&state.active_endpoint_id];
    assert!(view.history.contains("pane_a") && view.collapsed.contains("pane_a"));
}

#[test]
fn history_icon_toggles_history_and_changes_style() {
    let mut state = subagent_state(mixed_snapshot());
    compose_rows(&mut state);
    let mode = mode_for(&state, "pane_a");
    let palette = state.config.palette.clone();
    let style = cell_style(&mut state, mode);
    assert_eq!(style.fg, Some(palette.overlay0));
    let outcome = click_hit(&mut state, mode);
    assert!(outcome.actions.is_empty(), "icon must not focus");
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    assert!(rows[space + 2].contains("find auth code"), "{rows:#?}");
    assert!(rows[space + 3].contains('!') && rows[space + 3].contains("stale task Old"));
    assert!(!rows.iter().any(|row| row.contains("run tests Bash")));
    // The running count still comes from the current list.
    assert!(body(&rows[space + 1]).ends_with("2 ↺ ▾"));
    assert_eq!(state.hits.workspaces[0].child_rows, 3 + 2);
    let style = cell_style(&mut state, mode);
    assert_eq!(style.fg, Some(palette.accent));
    assert!(style.add_modifier.contains(ratatui::style::Modifier::BOLD));
    // Back to current.
    click_hit(&mut state, mode);
    let rows = compose_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("run tests Bash")));
}

#[test]
fn empty_current_list_shows_no_rows_but_chevron_and_icon() {
    let mut snapshot = agents_snapshot();
    set_subagents(
        &mut snapshot,
        "pane_a",
        Vec::new(),
        vec![sub("h", None, Sub::Done, "Old", "x")],
    );
    let mut state = subagent_state(snapshot);
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    assert!(body(&rows[space + 1]).ends_with("↺ ▾"), "{rows:#?}");
    assert_eq!(state.hits.workspaces[0].child_rows, 3);
    assert!(!rows.iter().any(|row| row.contains("Old x")));
    // Switching to history reveals the rows.
    let mode = mode_for(&state, "pane_a");
    click_hit(&mut state, mode);
    let rows = compose_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("x Old")), "{rows:#?}");
    assert_eq!(state.hits.workspaces[0].child_rows, 3 + 1);
}

#[test]
fn narrow_rows_drop_the_icon_before_the_chevron() {
    let mut saw_chevron_only = false;
    let mut saw_both = false;
    for width in 4..32 {
        let mut state = subagent_state(mixed_snapshot());
        state.config.sidebar_min_width = 4;
        state.sidebar_width = width;
        state.sidebar_width_manual = true;
        state.compose(106, 30).expect("frame");
        let icon = !state.hits.subagent_modes.is_empty();
        let chevron = !state.hits.subagent_toggles.is_empty();
        assert!(chevron || !icon, "icon without chevron at width {width}");
        saw_both |= icon && chevron;
        saw_chevron_only |= chevron && !icon;
    }
    assert!(saw_both && saw_chevron_only);
}

#[test]
fn nested_subagents_indent_under_their_parent() {
    let mut snapshot = agents_snapshot();
    let list = vec![
        sub("c", Some("p"), Sub::Running, "Child", "nested"),
        sub("p", None, Sub::Running, "Parent", "top"),
        sub("g", Some("c"), Sub::Done, "Grand", "dp"),
        sub("z", None, Sub::Running, "Zed", "last"),
    ];
    set_subagents(&mut snapshot, "pane_a", list.clone(), list);
    let mut state = subagent_state(snapshot);
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    // Tree order: parent, its child, the grandchild, then the next root.
    let column = col;
    assert!(rows[space + 2].contains("top Parent"), "{rows:#?}");
    assert!(rows[space + 3].contains("nested Child"));
    assert!(rows[space + 4].contains("dp Grand"));
    assert!(rows[space + 5].contains("last Zed"));
    let parent = column(&rows[space + 2], "top");
    let child = column(&rows[space + 3], "nested");
    let grand = column(&rows[space + 4], "dp");
    assert_eq!(child, parent + 3, "{rows:#?}");
    assert_eq!(grand, child + 3, "{rows:#?}");
    assert_eq!(column(&rows[space + 5], "last"), parent);
    // The parent's rail continues past its descendants because Zed follows.
    let rail = col(&rows[space + 2], "├─");
    assert_eq!(rows[space + 3].chars().nth(rail), Some('│'), "{rows:#?}");
}

#[test]
fn long_descriptions_truncate_and_tokens_truncate_before_the_chevron() {
    let mut snapshot = agents_snapshot();
    set_subagents(
        &mut snapshot,
        "pane_a",
        vec![sub(
            "s",
            None,
            Sub::Running,
            "Explore",
            "a very long description that cannot possibly fit in the sidebar width",
        )],
        vec![sub("s", None, Sub::Running, "Explore", "x")],
    );
    snapshot.agents[1].name = Some("an-extremely-long-agent-name-here".into());
    let mut state = subagent_state(snapshot);
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    // One row, ellipsized, never wrapped.
    assert!(rows[space + 2].contains('…'), "{rows:#?}");
    assert!(rows[space + 2].contains("a very long"), "{rows:#?}");
    // The first row keeps its chevron at the right edge.
    assert!(body(&rows[space + 1]).ends_with("1 ↺ ▾"), "{rows:#?}");
    assert_eq!(state.hits.workspaces[0].child_rows, 3 + 1);
}

#[test]
fn subagent_hits_toggle_one_pane_and_subagent_rows_focus_the_parent() {
    let mut snapshot = mixed_snapshot();
    set_subagents(
        &mut snapshot,
        "pane_c",
        vec![sub("c1", None, Sub::Running, "Other", "second")],
        vec![sub("c1", None, Sub::Running, "Other", "second")],
    );
    let mut state = subagent_state(snapshot);
    let rows = compose_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("second Other")));
    assert!(rows.iter().any(|row| row.contains("find auth code")));

    // Collapsing pane_a leaves pane_c expanded.
    let toggle_a = toggle_for(&state, "pane_a");
    click_hit(&mut state, toggle_a);
    let rows = compose_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("second Other")));
    assert!(!rows.iter().any(|row| row.contains("find auth code")));
    let view = &state.subagent_views[&state.active_endpoint_id];
    assert!(view.collapsed.contains("pane_a") && !view.collapsed.contains("pane_c"));
    let toggle_a = toggle_for(&state, "pane_a");
    click_hit(&mut state, toggle_a);
    let rows = compose_rows(&mut state);

    // The icon flips only pane_a and never focuses.
    let mode = mode_for(&state, "pane_a");
    let outcome = click_hit(&mut state, mode);
    assert!(outcome.actions.is_empty(), "icon must not focus");
    let view = &state.subagent_views[&state.active_endpoint_id];
    assert!(view.history.contains("pane_a") && view.history.len() == 1);
    let mode = mode_for(&state, "pane_a");
    click_hit(&mut state, mode);
    let rows2 = compose_rows(&mut state);
    assert_eq!(rows, rows2);

    // A subagent row of pane_a focuses pane_a, wherever it was drawn.
    let space = row_index(&rows, "client-shell");
    let y = (space + 2) as u16;
    let click = left_click(&mut state, 12, y);
    let [ClientShellAction::Endpoint { request, .. }] = &click.actions[..] else {
        panic!("subagent row click should focus the parent pane");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_a"
    ));
    // ... and a row of pane_c focuses pane_c.
    let y = row_index(&rows, "second Other") as u16;
    let click = left_click(&mut state, 12, y);
    let [ClientShellAction::Endpoint { request, .. }] = &click.actions[..] else {
        panic!("subagent row click should focus the parent pane");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_c"
    ));
}

#[test]
fn subagent_hits_follow_list_changes_and_pane_removal() {
    let mut state = subagent_state(mixed_snapshot());
    compose_rows(&mut state);
    let before = state.hits.space_agents.clone();

    // The current list grows: rows below move down and hits follow.
    let mut grown = mixed_snapshot();
    let mut current = grown.agents[1].subagents.clone();
    current.push(sub("s4", None, Sub::Running, "Extra", "new one"));
    grown.agents[1].subagents = current;
    state.set_snapshot(Box::new(grown));
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    assert!(rows[space + 5].contains("new one Extra"), "{rows:#?}");
    assert_eq!(state.hits.workspaces[0].child_rows, 3 + 4);
    let after = state.hits.space_agents.clone();
    assert_ne!(before, after);
    // The next agent child (pane_c) starts after the new rows; clicking it focuses it.
    let (rect, pane) = after.iter().find(|(_, pane)| pane == "pane_c").unwrap();
    let click = left_click(&mut state, rect.x + 5, rect.y);
    let [ClientShellAction::Endpoint { request, .. }] = &click.actions[..] else {
        panic!("expected focus");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneFocus(target) if &target.pane_id == pane
    ));

    // A collapsed pane that disappears is pruned: a re-added pane is expanded.
    let toggle = toggle_for(&state, "pane_a");
    click_hit(&mut state, toggle);
    assert!(state.subagent_views.contains_key(&state.active_endpoint_id));
    let mut without = mixed_snapshot();
    without.agents.retain(|agent| agent.pane_id != "pane_a");
    state.set_snapshot(Box::new(without));
    assert!(!state.subagent_views.contains_key(&state.active_endpoint_id));
    state.set_snapshot(Box::new(mixed_snapshot()));
    let rows = compose_rows(&mut state);
    assert!(
        rows.iter().any(|row| row.contains("find auth code")),
        "{rows:#?}"
    );
}

#[test]
fn subagent_rows_survive_sidebar_scrolling_and_hits_match_drawn_rows() {
    let mut snapshot = mixed_snapshot();
    for (index, id) in ["ws_4", "ws_5", "ws_6"].into_iter().enumerate() {
        snapshot
            .workspaces
            .push(workspace(id, 4 + index, &format!("space-{id}")));
        let tab_id = format!("tab_{id}");
        snapshot.tabs.push(tab(&tab_id, id, 1, "t"));
        for n in 0..3 {
            snapshot.agents.push(agent(
                &format!("pane_{id}_{n}"),
                id,
                &tab_id,
                AgentStatus::Idle,
            ));
        }
    }
    let mut state = subagent_state(snapshot);
    state.compose(106, 18).expect("frame");
    for scroll in 0..state.snapshot.as_deref().unwrap().workspaces.len() {
        state.workspace_scroll = scroll;
        state.reveal_focused_workspace = false;
        let frame = state.compose(106, 18).expect("frame");
        let rows = frame_rows(&frame);
        for hit in &state.hits.workspaces {
            if hit.workspace_id == "ws_1" {
                // All of ws_1's rows are drawn with it, so icon and chevron hits are inside.
                let (chevron, _) = toggle_for_pair(&state, "pane_a");
                assert!(rows[chevron.y as usize].contains('▾'));
                let mode = mode_for(&state, "pane_a");
                assert_eq!(mode.y, chevron.y);
                assert!(rows[mode.y as usize].contains('↺'));
                let (agent, _) = state
                    .hits
                    .space_agents
                    .iter()
                    .find(|(_, pane)| pane == "pane_a")
                    .expect("agent hit");
                assert_eq!(agent.height, 1 + 3);
                assert!(rows[(agent.y + 1) as usize].contains("find auth code"));
            }
        }
    }
}

fn toggle_for_pair(state: &ClientShellState, pane: &str) -> (Rect, String) {
    (toggle_for(state, pane), pane.to_owned())
}

#[test]
fn subagent_rows_do_not_join_keyboard_entries_or_tree_numbering() {
    let plain = subagent_state(agents_snapshot());
    let mut with = subagent_state(mixed_snapshot());
    compose_rows(&mut with);
    let snapshot = with.snapshot.clone().unwrap();
    assert_eq!(
        with.sidebar_entries(&snapshot),
        plain.sidebar_entries(plain.snapshot.as_deref().unwrap())
    );
    assert_eq!(tree_order(&with), tree_order(&plain));
}

fn timed_sub(
    id: &str,
    state: Sub,
    description: &str,
    started_s: Option<u64>,
    ended_s: Option<u64>,
) -> crate::api::schema::PaneSubagentInfo {
    let mut info = sub(id, None, state, "Explore", description);
    info.started_at_ms = started_s.map(|s| s * 1000);
    info.ended_at_ms = ended_s.map(|s| s * 1000);
    info
}

fn timed_state(now_s: u64) -> ClientShellState {
    let mut snapshot = agents_snapshot();
    set_subagents(
        &mut snapshot,
        "pane_a",
        vec![
            timed_sub("r", Sub::Running, "build", Some(1_000), None),
            timed_sub("d", Sub::Done, "tests", Some(1_100), Some(1_112)),
        ],
        vec![
            timed_sub("r", Sub::Running, "build", Some(1_000), None),
            timed_sub("i", Sub::Interrupted, "stale", Some(500), Some(560)),
        ],
    );
    let mut state = subagent_state(snapshot);
    state.config.subagent_clock.fixed_ms = Some(now_s * 1000);
    state
}

#[test]
fn subagent_rows_show_right_aligned_timing_from_the_frame_clock() {
    // Running 2m14s.
    let mut state = timed_state(1_134);
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    let running = &rows[space + 2];
    assert!(
        body(running).contains("build") && body(running).ends_with("2m14s"),
        "{rows:#?}"
    );
    // Right-aligned: the text ends against the sidebar divider's inner edge.
    assert_eq!(
        body(running).chars().count(),
        body(&rows[space + 1]).chars().count()
    );

    // Done: ran 12s, finished 3m ago, from the pinned frame time.
    state.config.subagent_clock.fixed_ms = Some((1_112 + 180) * 1000);
    let rows = compose_rows(&mut state);
    assert!(body(&rows[space + 3]).contains("tests"), "{rows:#?}");
    assert!(body(&rows[space + 3]).ends_with("12s·3m ago"), "{rows:#?}");

    // History mode: the interrupted row uses its last activity.
    let mode = mode_for(&state, "pane_a");
    click_hit(&mut state, mode);
    state.config.subagent_clock.fixed_ms = Some((560 + 60) * 1000);
    let rows = compose_rows(&mut state);
    assert!(body(&rows[space + 3]).ends_with("1m0s·1m ago"), "{rows:#?}");
}

#[test]
fn timing_is_kept_by_truncating_the_description_and_dropping_the_kind() {
    let mut snapshot = agents_snapshot();
    set_subagents(
        &mut snapshot,
        "pane_a",
        vec![timed_sub(
            "r",
            Sub::Running,
            "running one",
            Some(1_000),
            None,
        )],
        vec![],
    );
    let mut state = subagent_state(snapshot);
    state.config.subagent_clock.fixed_ms = Some(1_134_000);
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    let running = body(&rows[space + 2]);
    assert!(running.ends_with("2m14s"), "{running:?}");
    assert!(running.contains("running o…"), "{running:?}");
    assert!(!running.contains("Expl"), "{running:?}");

    // Unknown times show no timing, so the kind has room again.
    let mut snapshot = agents_snapshot();
    set_subagents(
        &mut snapshot,
        "pane_a",
        vec![timed_sub("r", Sub::Running, "running one", None, None)],
        vec![],
    );
    let mut state = subagent_state(snapshot);
    let rows = compose_rows(&mut state);
    assert!(
        body(&rows[space + 2]).contains("running one Exp"),
        "{rows:#?}"
    );
}

#[test]
fn subagent_timer_ticks_only_while_a_running_row_is_visible() {
    let now = std::time::Instant::now();
    let mut state = timed_state(1_134);
    compose_rows(&mut state);
    assert!(state.config.subagent_clock.next_deadline(now).is_some());
    assert!(!state.tick_subagent_timing(now));
    assert!(state.tick_subagent_timing(now + std::time::Duration::from_secs(1)));
    assert!(state.timer_delay(now) <= std::time::Duration::from_millis(100));

    // Collapsed rows: nothing running is drawn, so no deadline or tick.
    let toggle = toggle_for(&state, "pane_a");
    click_hit(&mut state, toggle);
    compose_rows(&mut state);
    assert!(state.config.subagent_clock.next_deadline(now).is_none());
    assert!(!state.tick_subagent_timing(now + std::time::Duration::from_secs(9)));

    // Only finished rows visible: also no ticks.
    let mut snapshot = agents_snapshot();
    set_subagents(
        &mut snapshot,
        "pane_a",
        vec![timed_sub("d", Sub::Done, "tests", Some(1_100), Some(1_112))],
        vec![],
    );
    let mut state = subagent_state(snapshot);
    state.config.subagent_clock.fixed_ms = Some(1_200_000);
    compose_rows(&mut state);
    assert!(state.config.subagent_clock.next_deadline(now).is_none());
}

fn style_of(state: &mut ClientShellState, rows_y: u16, needle_col: u16) -> ratatui::style::Style {
    cell_style(state, Rect::new(needle_col, rows_y, 1, 1))
}

/// Foreground of the first character of `text` on the sidebar row below the space.
fn subagent_text_style(state: &mut ClientShellState, text: &str) -> ratatui::style::Style {
    let rows = compose_rows(state);
    let y = row_index(&rows, text);
    let x = col(&rows[y], text);
    style_of(state, y as u16, x as u16)
}

#[test]
fn subagent_rows_use_tab_color_by_default_and_subagent_token_overrides() {
    use ratatui::style::Color;
    let snapshot = || {
        let mut snapshot = agents_snapshot();
        set_subagents(
            &mut snapshot,
            "pane_a",
            vec![timed_sub("r", Sub::Running, "build", Some(1_000), None)],
            vec![],
        );
        snapshot
    };
    let tab = Color::Rgb(0x5c, 0x6f, 0xae);
    let own = Color::Rgb(0x11, 0x99, 0x55);

    // No theme tokens: the secondary color an unthemed tab name uses.
    let mut state = subagent_state(snapshot());
    state.config.subagent_clock.fixed_ms = Some(1_134_000);
    let default_palette = state.config.palette.overlay0;
    assert_eq!(
        subagent_text_style(&mut state, "build").fg,
        Some(default_palette)
    );

    // `tab` alone colors subagent rows too.
    let mut state = subagent_state(snapshot());
    state.config.subagent_clock.fixed_ms = Some(1_134_000);
    state.config.palette.tab = Some(tab);
    assert_eq!(subagent_text_style(&mut state, "build").fg, Some(tab));

    // `subagent` wins over `tab`.
    state.config.palette.subagent = Some(own);
    assert_eq!(subagent_text_style(&mut state, "build").fg, Some(own));
    // The timing uses the dimmed variant of the same color.
    let rows = compose_rows(&mut state);
    let y = row_index(&rows, "build");
    let timing_col = col(&rows[y], "2m14s");
    let timing = style_of(&mut state, y as u16, timing_col as u16);
    assert_eq!(timing.fg, Some(own));
    assert!(timing.add_modifier.contains(ratatui::style::Modifier::DIM));
}

#[test]
fn subagent_focused_color_applies_only_while_the_parent_is_focused() {
    use ratatui::style::Color;
    let focused = Color::Rgb(0xa9, 0x7c, 0xfa);
    let tab_focused = Color::Rgb(0x44, 0x44, 0x44);
    let tab = Color::Rgb(0x5c, 0x6f, 0xae);
    let build = |focus: &str| {
        let mut snapshot = agents_snapshot();
        snapshot.focused_pane_id = Some(focus.into());
        for agent in &mut snapshot.agents {
            agent.focused = agent.pane_id == focus;
        }
        set_subagents(
            &mut snapshot,
            "pane_a",
            vec![timed_sub("r", Sub::Running, "build", Some(1_000), None)],
            vec![],
        );
        let mut state = subagent_state(snapshot);
        state.config.subagent_clock.fixed_ms = Some(1_134_000);
        state.config.palette.tab = Some(tab);
        state.config.palette.tab_focused = Some(tab_focused);
        state
    };
    // Parent focused: tab_focused (no subagent tokens), then subagent_focused.
    let mut state = build("pane_a");
    assert_eq!(
        subagent_text_style(&mut state, "build").fg,
        Some(tab_focused)
    );
    state.config.palette.subagent_focused = Some(focused);
    assert_eq!(subagent_text_style(&mut state, "build").fg, Some(focused));
    // Parent not focused: the plain color.
    let mut state = build("pane_c");
    state.config.palette.subagent_focused = Some(focused);
    assert_eq!(subagent_text_style(&mut state, "build").fg, Some(tab));
}

fn press_toggle_subagents(state: &mut ClientShellState) -> ClientShellInput {
    let mut outcome = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::ToggleSubagents),
        &mut outcome,
    );
    outcome
}

#[test]
fn toggle_subagents_key_flips_the_focused_agents_rows() {
    let mut snapshot = mixed_snapshot();
    snapshot.focused_pane_id = Some("pane_a".into());
    let mut state = subagent_state(snapshot);
    let rows = compose_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("find auth code")));

    let outcome = press_toggle_subagents(&mut state);
    assert!(outcome.repaint && outcome.actions.is_empty());
    let rows = compose_rows(&mut state);
    assert!(
        !rows.iter().any(|row| row.contains("find auth code")),
        "{rows:#?}"
    );
    assert!(state.subagent_views[&state.active_endpoint_id]
        .collapsed
        .contains("pane_a"));

    press_toggle_subagents(&mut state);
    let rows = compose_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("find auth code")));

    // A focused agent without subagents (or no agent) does nothing.
    let mut snapshot = mixed_snapshot();
    snapshot.focused_pane_id = Some("pane_c".into());
    let mut state = subagent_state(snapshot);
    compose_rows(&mut state);
    let outcome = press_toggle_subagents(&mut state);
    assert!(!outcome.repaint && outcome.actions.is_empty());
    assert!(state.subagent_views.is_empty());
}

fn right_click(state: &mut ClientShellState, x: u16, y: u16) {
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: x,
        row: y,
        modifiers: KeyModifiers::empty(),
    })]);
}

fn menu_labels(state: &ClientShellState) -> Vec<&'static str> {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => {
            menu.items().iter().map(|item| item.label).collect()
        }
        _ => Vec::new(),
    }
}

#[test]
fn agent_context_menu_toggles_subagent_rows_and_history() {
    let mut state = subagent_state(mixed_snapshot());
    let rows = compose_rows(&mut state);
    let space = row_index(&rows, "client-shell");
    // Right-click the agent's own row.
    right_click(&mut state, 12, (space + 1) as u16);
    assert_eq!(
        menu_labels(&state),
        ["Collapse subagents", "Show subagent history"]
    );
    state.activate_context_menu_item(0, &mut ClientShellInput::default());
    let rows = compose_rows(&mut state);
    assert!(!rows.iter().any(|row| row.contains("find auth code")));
    right_click(&mut state, 12, (space + 1) as u16);
    assert_eq!(
        menu_labels(&state),
        ["Expand subagents", "Show subagent history"]
    );
    state.activate_context_menu_item(1, &mut ClientShellInput::default());
    assert!(state.subagent_views[&state.active_endpoint_id]
        .history
        .contains("pane_a"));
    compose_rows(&mut state);
    right_click(&mut state, 12, (space + 1) as u16);
    assert_eq!(
        menu_labels(&state),
        ["Expand subagents", "Show current subagents"]
    );

    // An agent without subagents keeps the previous behavior: no agent menu.
    state.overlay = None;
    let rows = compose_rows(&mut state);
    let other = row_index(&rows, "api");
    right_click(&mut state, 12, other as u16);
    assert!(!menu_labels(&state).contains(&"Collapse subagents"));
}
