use super::*;

fn config_with_sections(spaces: bool, agents: bool) -> ClientShellConfig {
    let mut config = Config::default();
    config.ui.sidebar.spaces.enabled = spaces;
    config.ui.sidebar.agents.enabled = agents;
    ClientShellConfig::from_config(&config)
}

fn snapshot_with_agent() -> ClientShellSnapshot {
    let mut snapshot = snapshot();
    snapshot.agents.push(ClientShellAgent {
        pane_id: "pane_1".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: None,
        display_agent: None,
        agent: Some("claude".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Working,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: true,
    });
    snapshot
}

fn composed(config: ClientShellConfig, collapsed: bool) -> (ClientShellState, Vec<String>) {
    let mut state = ClientShellState::new(config);
    state.sidebar_collapsed = collapsed;
    state.set_snapshot(Box::new(snapshot_with_agent()));
    state.set_pane_surface(surface());
    let frame = state.compose(106, 20).expect("composed frame");
    let rows = frame_rows(&frame);
    (state, rows)
}

fn sidebar_text(rows: &[String], width: usize) -> String {
    rows.iter()
        .map(|row| row.chars().take(width).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn both_sections_disabled_reports_a_diagnostic_and_shows_both() {
    let mut config = Config::default();
    config.ui.sidebar.spaces.enabled = false;
    config.ui.sidebar.agents.enabled = false;
    let diagnostic = config
        .all_sidebar_sections_disabled_diagnostic()
        .expect("diagnostic");
    assert!(diagnostic.contains("ui.sidebar.spaces.enabled"));
    assert!(diagnostic.contains("ui.sidebar.agents.enabled"));
    assert!(config.collect_diagnostics().contains(&diagnostic));

    let shell = ClientShellConfig::from_config(&config);
    assert!(shell.spaces.enabled && shell.agents.enabled);
    let (state, rows) = composed(shell, false);
    let text = sidebar_text(&rows, 32);
    assert!(
        text.contains(" spaces") && text.contains(" agents"),
        "{text}"
    );
    assert!(!state.hits.sidebar_section_divider.is_empty());

    let mut live = ClientShellConfig::from_config(&Config::default());
    let diagnostics = live.apply_live_config(&config, &[], &[]);
    assert!(diagnostics.contains(&diagnostic), "{diagnostics:?}");
    assert!(live.spaces.enabled && live.agents.enabled);
}

#[test]
fn default_sections_render_the_divider_and_both_headers() {
    let (state, rows) = composed(config_with_sections(true, true), false);
    let text = sidebar_text(&rows, 32);
    assert!(text.contains(" spaces") && text.contains(" agents"));
    assert!(!state.hits.sidebar_section_divider.is_empty());
    assert!(!state.hits.workspaces.is_empty());
    assert!(!state.hits.agents.is_empty());
}

#[test]
fn agents_only_sidebar_fills_height_with_footer_and_toggle_on_the_bottom_row() {
    let (state, rows) = composed(config_with_sections(false, true), false);
    let text = sidebar_text(&rows, 32);
    assert!(!text.contains(" spaces"), "{text}");
    assert!(text.contains(" agents"), "{text}");
    assert!(state.hits.workspaces.is_empty());
    assert!(state.hits.workspace_body.is_empty());
    assert!(state.hits.sidebar_section_divider.is_empty());
    assert!(!state.hits.agents.is_empty());
    // The header starts on the first row: there is no divider rule above it.
    assert!(rows[0].starts_with(" agents"), "{:?}", rows[0]);
    let bottom = state.hits.sidebar_toggle.y;
    assert_eq!(state.hits.new_workspace.y, bottom);
    assert_eq!(state.hits.global_launcher.y, bottom);
    assert!(state.hits.global_launcher.right() <= state.hits.sidebar_toggle.x);
    assert!(rows[bottom as usize].contains(" new"));
    assert!(rows[bottom as usize].contains("menu"));
    assert!(rows[bottom as usize].contains('«'));
}

#[test]
fn spaces_only_sidebar_fills_height_with_footer_and_toggle_on_the_bottom_row() {
    let (state, rows) = composed(config_with_sections(true, false), false);
    let text = sidebar_text(&rows, 32);
    assert!(text.contains(" spaces"), "{text}");
    assert!(!text.contains(" agents"), "{text}");
    assert!(state.hits.agents.is_empty());
    assert!(state.hits.agent_body.is_empty());
    assert!(state.hits.agent_sort_toggle.is_empty());
    assert!(state.hits.sidebar_section_divider.is_empty());
    assert!(!state.hits.workspaces.is_empty());
    // The space list extends to the footer row.
    let bottom = state.hits.sidebar_toggle.y;
    assert_eq!(state.hits.workspace_body.bottom(), bottom);
    assert_eq!(state.hits.new_workspace.y, bottom);
    assert!(state.hits.global_launcher.right() <= state.hits.sidebar_toggle.x);
    assert!(rows[bottom as usize].contains(" new"));
    assert!(rows[bottom as usize].contains("menu"));
    assert!(rows[bottom as usize].contains('«'));
}

#[test]
fn collapsed_sidebar_hides_disabled_sections() {
    let (state, _) = composed(config_with_sections(true, true), true);
    assert!(!state.hits.workspaces.is_empty() && !state.hits.agents.is_empty());

    let (state, rows) = composed(config_with_sections(false, true), true);
    assert!(state.hits.workspaces.is_empty());
    assert!(!state.hits.agents.is_empty());
    assert!(!state.hits.sidebar_toggle.is_empty());
    assert!(
        rows.iter()
            .all(|row| !row.chars().take(7).any(|c| c == '─')),
        "no divider expected: {rows:?}"
    );

    let (state, rows) = composed(config_with_sections(true, false), true);
    assert!(!state.hits.workspaces.is_empty());
    assert!(state.hits.agents.is_empty());
    assert!(!state.hits.sidebar_toggle.is_empty());
    assert!(
        rows.iter()
            .all(|row| !row.chars().take(7).any(|c| c == '─')),
        "no divider expected: {rows:?}"
    );
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> RawInputEvent {
    RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })
}

#[test]
fn saved_split_survives_disabling_and_divider_drag_is_inert_with_one_section() {
    let (mut state, _) = composed(config_with_sections(true, true), false);
    let divider = state.hits.sidebar_section_divider;
    assert!(!divider.is_empty());
    state.handle_raw_events(vec![mouse(
        MouseEventKind::Down(MouseButton::Left),
        divider.x + 2,
        divider.y,
    )]);
    state.handle_raw_events(vec![mouse(
        MouseEventKind::Drag(MouseButton::Left),
        divider.x + 2,
        14,
    )]);
    state.handle_raw_events(vec![mouse(
        MouseEventKind::Up(MouseButton::Left),
        divider.x + 2,
        14,
    )]);
    let saved = state.sidebar_section_split;
    assert!(saved > 0.6, "split should have moved: {saved}");
    state.compose(106, 20).expect("frame");
    let divider = state.hits.sidebar_section_divider;

    let mut hidden = Config::default();
    hidden.ui.sidebar.agents.enabled = false;
    let diagnostics = state.config.apply_live_config(&hidden, &[], &[]);
    assert!(diagnostics.is_empty());
    state.compose(106, 20).expect("single section frame");
    assert!(state.hits.sidebar_section_divider.is_empty());

    // Pressing and dragging on the former divider row does nothing.
    for row in [divider.y, divider.y + 1] {
        state.handle_raw_events(vec![mouse(
            MouseEventKind::Down(MouseButton::Left),
            divider.x + 2,
            row,
        )]);
        assert!(!matches!(
            state.chrome_drag,
            Some(ClientChromeDrag::SidebarSection)
        ));
        state.handle_raw_events(vec![mouse(
            MouseEventKind::Drag(MouseButton::Left),
            divider.x + 2,
            5,
        )]);
        state.handle_raw_events(vec![mouse(
            MouseEventKind::Up(MouseButton::Left),
            divider.x + 2,
            5,
        )]);
    }
    assert_eq!(state.sidebar_section_split, saved);

    let applied = state.config.apply_live_config(&Config::default(), &[], &[]);
    assert!(applied.is_empty());
    state.compose(106, 20).expect("restored frame");
    assert_eq!(state.sidebar_section_split, saved);
    assert_eq!(state.hits.sidebar_section_divider, divider);
}

#[test]
fn live_reload_toggles_sections_on_the_next_frame() {
    let (mut state, rows) = composed(config_with_sections(true, true), false);
    assert!(sidebar_text(&rows, 32).contains(" agents"));
    let mut hidden = Config::default();
    hidden.ui.sidebar.agents.enabled = false;
    state.config.apply_live_config(&hidden, &[], &[]);
    let frame = state.compose(106, 20).expect("frame");
    assert!(!sidebar_text(&frame_rows(&frame), 32).contains(" agents"));
}
