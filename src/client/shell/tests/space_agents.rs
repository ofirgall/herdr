use super::*;
use crate::config::SpaceAgentsMode;

fn agent(pane: &str, workspace: &str, tab: &str, status: AgentStatus) -> ClientShellAgent {
    ClientShellAgent {
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

/// ws_1 has agents in tab 2 and tab 1 (snapshot order is tab 2 first), ws_2
/// has one agent, ws_3 has none.
fn agents_snapshot() -> ClientShellSnapshot {
    let mut snapshot = snapshot();
    snapshot.workspaces[0].agent_status = AgentStatus::Working;
    snapshot.workspaces[0].branch = None;
    snapshot.workspaces.push(workspace("ws_2", 2, "docs"));
    snapshot.workspaces.push(workspace("ws_3", 3, "empty"));
    snapshot.tabs = vec![
        tab("tab_b", "ws_1", 2, "api"),
        tab("tab_a", "ws_1", 1, "web"),
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
fn agents_group_by_workspace_in_tab_then_snapshot_order_ignoring_agent_view() {
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
    // Children are indented two columns beyond the space's first-row content.
    let content = rows[space].find(&working).unwrap();
    let child_icon = rows[space + 1].find(&working).unwrap();
    assert_eq!(child_icon, content + 2, "{rows:#?}");
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
    // in the same column as the `├` connectors.
    let child = &rows[row_index(&rows, "notes")];
    assert!(child.starts_with(" │"), "{rows:#?}");
    let docs = &rows[row_index(&rows, "docs")];
    assert!(docs.starts_with(" ├ "), "{rows:#?}");
    let empty = &rows[row_index(&rows, "empty")];
    assert!(empty.starts_with(" └ "), "{rows:#?}");
    // The parent's own children carry the rail down to its first worktree.
    let parent_child = &rows[row_index(&rows, "api")];
    assert!(parent_child.starts_with(" │"), "{rows:#?}");
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
fn focused_child_highlight_starts_one_column_before_its_status_in_a_tree() {
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
    // Tree rail at column 1, status icon at column 5: the highlight starts at 4.
    assert_eq!(buffer[(child.x + 1, child.y)].symbol(), "│");
    for x in child.x..child.x + 4 {
        assert_ne!(buffer[(x, child.y)].bg, highlight, "tree column {x}");
    }
    for x in child.x + 4..child.right() {
        assert_eq!(buffer[(x, child.y)].bg, highlight, "content column {x}");
    }
}

#[test]
fn focused_child_highlight_starts_one_column_before_its_status_at_top_level() {
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
    // Status icon at column 3: the highlight starts at 2.
    for x in child.x..child.x + 2 {
        assert_ne!(buffer[(x, child.y)].bg, highlight, "indent column {x}");
    }
    for x in child.x + 2..child.right() {
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
