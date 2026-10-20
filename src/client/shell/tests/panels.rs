use super::*;
use crate::client::endpoint::ClientEndpointId;
use crate::client::shell::panels::PanelBodyState;
use crate::protocol::endpoint::{EndpointPanelStatus, EndpointPanelSurface};

fn panel_config(sections: &[&str]) -> ClientShellConfig {
    let mut config = Config::default();
    config.ui.sidebar.sections = sections
        .iter()
        .map(|section| (*section).to_owned())
        .collect();
    ClientShellConfig::from_config(&config)
}

fn panel_state(sections: &[&str]) -> ClientShellState {
    let mut state = ClientShellState::new(panel_config(sections));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_endpoint_panel_surfaces_supported(&ClientEndpointId::Local, true);
    state
}

fn panel_frame(id: &str, text: &str, revision: u64) -> EndpointPanelSurface {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
    buffer.set_string(0, 0, text, Style::default());
    let frame = crate::protocol::FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]);
    EndpointPanelSurface {
        boot_id: snapshot().boot_id,
        id: id.into(),
        revision,
        title: id.into(),
        status: EndpointPanelStatus::Running,
        exit_code: None,
        frame,
        mouse_reporting: false,
        sgr_pixel_mouse: false,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn surface_set_params(
    actions: Vec<ClientShellAction>,
) -> Vec<crate::api::schema::PanelSurfaceSetParams> {
    actions
        .into_iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => match request.method {
                crate::api::schema::Method::PanelSurfaceSet(params) => Some(params),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn frame_text(frame: &crate::protocol::FrameData) -> String {
    frame
        .cells
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect()
}

#[test]
fn layout_change_sends_one_surface_declaration_and_repeat_composes_send_none() {
    let mut state = panel_state(&["spaces", "panel:files", "agents"]);
    state.compose(106, 40).expect("frame");
    let hit = state.hits.panel_sections[0].clone();
    assert_eq!(hit.id, "files");
    assert!(hit.body.height > 0);
    let sent = surface_set_params(state.take_pending_panel_actions());
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].id, "files");
    assert_eq!(
        (sent[0].cols, sent[0].rows),
        (hit.body.width, hit.body.height)
    );
    assert!(sent[0].visible);

    state.compose(106, 40).expect("frame");
    assert!(state.take_pending_panel_actions().is_empty());

    state.compose(106, 50).expect("taller frame");
    let resized = surface_set_params(state.take_pending_panel_actions());
    assert_eq!(resized.len(), 1);
    assert_ne!(resized[0].rows, sent[0].rows);
}

#[test]
fn collapsing_the_sidebar_hides_the_panel() {
    let mut state = panel_state(&["panel:files", "spaces"]);
    state.compose(106, 40).expect("frame");
    state.take_pending_panel_actions();
    state.sidebar_collapsed = true;
    state.compose(106, 40).expect("collapsed frame");
    assert!(state.hits.panel_sections.is_empty());
    let sent = surface_set_params(state.take_pending_panel_actions());
    assert_eq!(sent.len(), 1);
    assert!(!sent[0].visible);
    state.compose(106, 40).expect("collapsed frame");
    assert!(state.take_pending_panel_actions().is_empty());
}

#[test]
fn panel_frames_are_drawn_in_the_body_and_stale_revisions_are_ignored() {
    let mut state = panel_state(&["panel:files", "spaces"]);
    state.compose(106, 40).expect("frame");
    assert!(state.receive_panel_surface(&ClientEndpointId::Local, panel_frame("files", "ABCD", 2)));
    assert!(!state.receive_panel_surface(&ClientEndpointId::Local, panel_frame("files", "WXYZ", 1)));
    let frame = state.compose(106, 40).expect("frame");
    let body = state.hits.panel_sections[0].body;
    let row = usize::from(body.y) * usize::from(frame.width) + usize::from(body.x);
    let text: String = frame.cells[row..row + 4]
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect();
    assert_eq!(text, "ABCD");
}

#[test]
fn unsupported_servers_show_a_fallback_and_receive_no_requests() {
    let mut state = panel_state(&["panel:files", "spaces"]);
    state.set_endpoint_panel_surfaces_supported(&ClientEndpointId::Local, false);
    let frame = state.compose(106, 40).expect("frame");
    let text = frame_text(&frame);
    assert!(text.contains("unavailable on"));
    assert!(text.contains("server"));
    assert!(state.take_pending_panel_actions().is_empty());
    assert!(state.visible_endpoint_notice.is_none());
}

#[test]
fn panel_not_found_marks_the_section_without_a_notice() {
    let mut state = panel_state(&["panel:ghost", "spaces"]);
    state.compose(106, 40).expect("frame");
    let actions = state.take_pending_panel_actions();
    let ClientShellAction::Endpoint {
        boot_id, request, ..
    } = &actions[0]
    else {
        panic!("endpoint action");
    };
    let (repaint, follow_up) = state.handle_endpoint_result(
        boot_id,
        &request.id,
        Err(ClientShellEndpointError {
            code: Some("panel_not_found".into()),
            message: "no such panel".into(),
        }),
    );
    assert!(repaint);
    assert!(follow_up.is_empty());
    let frame = state.compose(106, 40).expect("frame");
    assert!(frame_text(&frame).contains("not configured"));
    assert!(state.take_pending_panel_actions().is_empty());
    assert!(state.visible_endpoint_notice.is_none());
}

#[test]
fn inactive_surface_rejections_resend_on_the_next_compose() {
    let mut state = panel_state(&["panel:files", "spaces"]);
    state.compose(106, 40).expect("frame");
    let actions = state.take_pending_panel_actions();
    let ClientShellAction::Endpoint {
        boot_id, request, ..
    } = &actions[0]
    else {
        panic!("endpoint action");
    };
    state.handle_endpoint_result(
        boot_id,
        &request.id,
        Err(ClientShellEndpointError {
            code: Some("surface_inactive".into()),
            message: "inactive".into(),
        }),
    );
    state.compose(106, 40).expect("frame");
    assert_eq!(
        surface_set_params(state.take_pending_panel_actions()).len(),
        1
    );
}

#[test]
fn exited_panels_say_so() {
    let mut state = panel_state(&["panel:files", "spaces"]);
    state.compose(106, 40).expect("frame");
    let mut exited = panel_frame("files", "ABCD", 2);
    exited.status = EndpointPanelStatus::Exited;
    exited.exit_code = Some(1);
    state.receive_panel_surface(&ClientEndpointId::Local, exited);
    let frame = state.compose(106, 40).expect("frame");
    assert!(frame_text(&frame).contains("exited"));
    assert!(frame_text(&frame).contains("restart"));
    assert_eq!(state.hits.panel_sections[0].state, PanelBodyState::Exited);
}

#[test]
fn reconnecting_clears_cached_frames_and_declarations() {
    let mut state = panel_state(&["panel:files", "spaces"]);
    state.compose(106, 40).expect("frame");
    state.take_pending_panel_actions();
    state.receive_panel_surface(&ClientEndpointId::Local, panel_frame("files", "ABCD", 2));
    state.set_endpoint_panel_surfaces_supported(&ClientEndpointId::Local, true);
    state.compose(106, 40).expect("frame");
    assert_eq!(
        surface_set_params(state.take_pending_panel_actions()).len(),
        1
    );
}

#[test]
fn default_sections_draw_no_panels_and_send_nothing() {
    let mut state = panel_state(&["spaces", "agents"]);
    let frame = state.compose(106, 40).expect("frame");
    assert!(frame_text(&frame).contains(" spaces"));
    assert!(state.hits.panel_sections.is_empty());
    assert!(state.take_pending_panel_actions().is_empty());
}

// Panel focus, input and focus width.

fn focus_state(extra: &str) -> ClientShellState {
    let config = toml::from_str::<Config>(&format!(
        r#"
[ui.sidebar]
sections = ["spaces", "panel:files", "panel:logs", "agents"]

[ui.sidebar.panel.files]
focus_width = 60

{extra}
"#
    ))
    .expect("panel config");
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_endpoint_panel_surfaces_supported(&ClientEndpointId::Local, true);
    state.compose(120, 50).expect("frame");
    state.take_pending_panel_actions();
    state
}

fn feed(state: &mut ClientShellState, id: &str, reporting: bool, revision: u64) {
    let mut frame = panel_frame(id, "ABCD", revision);
    frame.mouse_reporting = reporting;
    state.receive_panel_surface(&ClientEndpointId::Local, frame);
    state.compose(120, 50).expect("frame");
    state.take_pending_panel_actions();
}

fn mouse(
    state: &mut ClientShellState,
    kind: MouseEventKind,
    column: u16,
    row: u16,
) -> ClientShellInput {
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })])
}

fn body_point(state: &ClientShellState, id: &str) -> (u16, u16) {
    let hit = state
        .hits
        .panel_sections
        .iter()
        .find(|hit| hit.id == id)
        .expect("panel hit");
    (hit.body.x + 1, hit.body.y + 1)
}

/// `(panel id, events)` of every `panel.input` request in the outcome.
fn panel_inputs(outcome: &ClientShellInput) -> Vec<(String, Vec<serde_json::Value>)> {
    outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => match &request.method {
                crate::api::schema::Method::PanelInput(params) => {
                    Some((params.id.clone(), params.events.clone()))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn restarts(outcome: &ClientShellInput) -> Vec<String> {
    outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => match &request.method {
                crate::api::schema::Method::PanelRestart(params) => Some(params.id.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn pane_inputs(outcome: &ClientShellInput) -> usize {
    outcome
        .requests
        .iter()
        .filter(|request| matches!(request, ClientMessage::ClientShellPaneInput { .. }))
        .count()
}

/// Mirrors the runtime loop: the server answers a resize with a fresh surface, then we compose.
fn settle(state: &mut ClientShellState) {
    state.set_pane_surface(surface());
    state.compose(120, 50).expect("frame");
    state.take_pending_panel_actions();
}

fn click_panel(state: &mut ClientShellState, id: &str) -> ClientShellInput {
    let (column, row) = body_point(state, id);
    mouse(state, MouseEventKind::Down(MouseButton::Left), column, row)
}

#[test]
fn clicking_a_running_panel_focuses_it_and_keys_reach_it_instead_of_the_pane() {
    let mut state = focus_state("");
    feed(&mut state, "files", false, 2);
    click_panel(&mut state, "files");
    assert_eq!(state.focused_panel.as_deref(), Some("files"));

    let outcome = state.handle_input_bytes(b"j");
    let inputs = panel_inputs(&outcome);
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].0, "files");
    assert!(!inputs[0].1.is_empty());
    assert_eq!(pane_inputs(&outcome), 0);
}

#[test]
fn clicking_outside_the_panels_returns_focus_to_the_pane() {
    let mut state = focus_state("");
    feed(&mut state, "files", false, 2);
    click_panel(&mut state, "files");
    settle(&mut state);
    let pane = state.hits.panes[0].inner_rect;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        pane.x + 1,
        pane.y + 1,
    );
    assert_eq!(state.focused_panel, None);

    let outcome = state.handle_input_bytes(b"j");
    assert!(panel_inputs(&outcome).is_empty());
    assert_eq!(pane_inputs(&outcome), 1);
}

#[test]
fn clicking_an_exited_panel_requests_a_restart_and_does_not_focus_it() {
    let mut state = focus_state("");
    let mut exited = panel_frame("files", "ABCD", 2);
    exited.status = EndpointPanelStatus::Exited;
    state.receive_panel_surface(&ClientEndpointId::Local, exited);
    state.compose(120, 50).expect("frame");
    let outcome = click_panel(&mut state, "files");
    assert_eq!(restarts(&outcome), vec!["files".to_owned()]);
    assert_eq!(state.focused_panel, None);
}

#[test]
fn a_prefix_action_leaves_the_panel_and_keybindings_still_apply_while_focused() {
    let mut state = focus_state("");
    feed(&mut state, "files", false, 2);
    click_panel(&mut state, "files");

    // The prefix itself is a binding, not panel input.
    let prefix = state.handle_input_bytes(&[0x02]);
    assert!(panel_inputs(&prefix).is_empty());
    assert_eq!(state.mode, ClientShellMode::Prefix);
    // `prefix+c` creates a tab: a workspace/tab focus action.
    let action = state.handle_input_bytes(b"c");
    assert!(panel_inputs(&action).is_empty());
    assert_eq!(state.focused_panel, None);
}

#[test]
fn paste_goes_to_the_focused_panel() {
    let mut state = focus_state("");
    feed(&mut state, "files", false, 2);
    click_panel(&mut state, "files");
    let outcome =
        state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Paste("hello".into())]);
    let inputs = panel_inputs(&outcome);
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].1[0].to_string().contains("hello"));
    assert_eq!(pane_inputs(&outcome), 0);
}

#[test]
fn mouse_reporting_panels_receive_clicks_drags_scroll_and_middle_button() {
    let mut state = focus_state("");
    feed(&mut state, "files", true, 2);
    let hit = state
        .hits
        .panel_sections
        .iter()
        .find(|hit| hit.id == "files")
        .cloned()
        .expect("files hit");
    let (column, row) = (hit.body.x + 3, hit.body.y + 2);

    let down = mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        column,
        row,
    );
    let inputs = panel_inputs(&down);
    assert_eq!(inputs.len(), 1);
    let text = inputs[0].1[0].to_string();
    assert!(
        text.contains("\"column\":3") && text.contains("\"row\":2"),
        "{text}"
    );
    assert_eq!(state.focused_panel.as_deref(), Some("files"));
    assert_eq!(pane_inputs(&down), 0);

    let drag = mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        column + 1,
        row,
    );
    assert_eq!(panel_inputs(&drag).len(), 1);
    let up = mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        column + 1,
        row,
    );
    assert_eq!(panel_inputs(&up).len(), 1);
    settle(&mut state);
    let scroll = mouse(&mut state, MouseEventKind::ScrollDown, column, row);
    assert_eq!(panel_inputs(&scroll).len(), 1);
    let middle = mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Middle),
        column,
        row,
    );
    assert_eq!(panel_inputs(&middle).len(), 1);
    assert!(
        panel_inputs(&mouse(
            &mut state,
            MouseEventKind::Up(MouseButton::Middle),
            column,
            row
        ))
        .len()
            == 1
    );
}

#[test]
fn panels_without_mouse_reporting_ignore_scroll_and_drags() {
    let mut state = focus_state("");
    feed(&mut state, "files", false, 2);
    let (column, row) = body_point(&state, "files");
    for kind in [
        MouseEventKind::ScrollUp,
        MouseEventKind::ScrollDown,
        MouseEventKind::Drag(MouseButton::Left),
    ] {
        let outcome = mouse(&mut state, kind, column, row);
        assert!(panel_inputs(&outcome).is_empty());
        assert_eq!(pane_inputs(&outcome), 0);
        assert!(outcome.actions.is_empty());
    }
}

#[test]
fn focus_panel_cycles_visible_panels_and_wraps() {
    let mut state = focus_state("[keys]\nfocus_panel = \"ctrl+p\"");
    feed(&mut state, "files", false, 2);
    feed(&mut state, "logs", false, 2);
    let ctrl_p = [0x10];
    state.handle_input_bytes(&ctrl_p);
    assert_eq!(state.focused_panel.as_deref(), Some("files"));
    settle(&mut state);
    state.handle_input_bytes(&ctrl_p);
    assert_eq!(state.focused_panel.as_deref(), Some("logs"));
    settle(&mut state);
    state.handle_input_bytes(&ctrl_p);
    assert_eq!(state.focused_panel.as_deref(), Some("files"));
}

#[test]
fn focus_panel_is_unbound_by_default_and_a_no_op_without_visible_panels() {
    let mut state = focus_state("");
    assert!(state.config.keybinds.keybinds.focus_panel.label().is_none());
    let mut plain = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    plain.set_snapshot(Box::new(snapshot()));
    plain.set_pane_surface(surface());
    plain.compose(120, 50).expect("frame");
    let mut outcome = ClientShellInput::default();
    plain.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::FocusPanel),
        &mut outcome,
    );
    assert_eq!(plain.focused_panel, None);
    assert!(!outcome.repaint && !outcome.resize);
    // Exited panels cannot take input and are skipped.
    let mut exited = panel_frame("files", "ABCD", 2);
    exited.status = EndpointPanelStatus::Exited;
    state.receive_panel_surface(&ClientEndpointId::Local, exited);
    state.compose(120, 50).expect("frame");
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::FocusPanel),
        &mut outcome,
    );
    assert_eq!(state.focused_panel.as_deref(), Some("logs"));
}

#[test]
fn focus_width_widens_the_sidebar_while_focused_and_restores_on_blur() {
    let mut state = focus_state("");
    feed(&mut state, "files", false, 2);
    let saved = state.sidebar_width;
    let base = state.layout(120, 50).sidebar.width;
    assert!(base < 60);

    let focus = click_panel(&mut state, "files");
    assert!(focus.resize);
    state.set_pane_surface(surface());
    assert_eq!(state.layout(120, 50).sidebar.width, 60);
    assert_eq!(state.sidebar_width, saved, "saved preference is untouched");
    assert!(!state.sidebar_width_manual);
    // The panel is redeclared at the wider body.
    state.compose(120, 50).expect("frame");
    let sent = surface_set_params(state.take_pending_panel_actions());
    let files = sent.iter().find(|set| set.id == "files").expect("files");
    assert_eq!(files.cols, 59, "sidebar of 60 minus its border");

    settle(&mut state);
    let pane = state.hits.panes[0].inner_rect;
    let blur = mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        pane.x + 1,
        pane.y + 1,
    );
    assert!(blur.resize);
    assert_eq!(state.layout(120, 50).sidebar.width, base);
}

#[test]
fn focus_width_is_limited_by_the_terminal_width() {
    let mut state = focus_state("");
    feed(&mut state, "files", false, 2);
    click_panel(&mut state, "files");
    assert_eq!(state.layout(70, 50).sidebar.width, 50);
}

#[test]
fn resize_goes_out_only_when_the_effective_width_changes() {
    let mut state = focus_state("");
    feed(&mut state, "files", false, 2);
    feed(&mut state, "logs", false, 2);
    // `logs` sets no focus width: focusing it changes nothing.
    let logs = click_panel(&mut state, "logs");
    assert!(!logs.resize);
    // Moving to `files` widens, clicking it again does not resize.
    let files = click_panel(&mut state, "files");
    assert!(files.resize);
    state.compose(120, 50).expect("frame");
    let again = click_panel(&mut state, "files");
    assert!(!again.resize);
    // Leaving to a panel without focus width narrows again.
    let back = click_panel(&mut state, "logs");
    assert!(back.resize);
}

#[test]
fn focus_width_never_narrows_a_wider_sidebar() {
    let mut state = focus_state("");
    feed(&mut state, "files", false, 2);
    state.sidebar_width = 36;
    state.config.sidebar_max_width = 36;
    let mut config = state.config.sidebar_sections.clone();
    for section in &mut config {
        if let crate::client::shell::panels::SidebarSectionConfig::Panel(panel) = section {
            panel.focus_width = Some(30);
        }
    }
    state.config.sidebar_sections = config;
    state.compose(120, 50).expect("frame");
    let before = state.layout(120, 50).sidebar.width;
    let outcome = click_panel(&mut state, "files");
    assert!(!outcome.resize);
    assert_eq!(state.layout(120, 50).sidebar.width, before);
}

#[test]
fn collapsing_the_sidebar_drops_panel_focus() {
    let mut state = focus_state("");
    feed(&mut state, "files", false, 2);
    click_panel(&mut state, "files");
    let mut outcome = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::ToggleSidebar),
        &mut outcome,
    );
    state.compose(120, 50).expect("frame");
    assert_eq!(state.focused_panel, None);
}
