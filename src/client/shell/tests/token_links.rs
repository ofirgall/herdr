//! Sidebar token links: underline, hit rects and click handling.

use super::*;
use crate::client::endpoint::{
    ClientEndpointId, ClientEndpointStatus, ProfileId, SavedSshEndpoint,
};
use crossterm::event::{MouseButton, MouseEventKind};
use ratatui::style::Modifier;

const URL_1: &str = "https://example.com/pull/1";
const URL_2: &str = "https://example.com/pull/2";
const URL_A: &str = "https://example.com/pull/agent";

fn config_with(click: Option<&str>) -> ClientShellConfig {
    let click = click
        .map(|value| format!("token_link_click = \"{value}\"\n"))
        .unwrap_or_default();
    let config: Config = toml::from_str(&format!(
        r#"
[ui.sidebar]
{click}
[ui.sidebar.spaces]
rows = [["workspace", "$pr", "$env"]]
agents = "expanded"
agent_rows = [["state_icon", "$pr"]]

[ui.sidebar.agents]
rows = [["workspace", "$pr"]]
"#
    ))
    .unwrap();
    ClientShellConfig::from_config(&config)
}

fn workspace(id: &str, number: usize, url: &str) -> ClientShellWorkspace {
    let mut workspace = snapshot().workspaces.remove(0);
    workspace.workspace_id = id.into();
    workspace.number = number;
    workspace.label = format!("space-{number}");
    workspace.branch = None;
    workspace.focused = number == 1;
    workspace.tokens = vec![
        ("env".into(), "plain".into()),
        ("pr".into(), "#7009".into()),
    ];
    workspace.token_links = vec![("pr".into(), url.into())];
    workspace
}

fn link_snapshot(workspaces: usize) -> ClientShellSnapshot {
    let mut snapshot = snapshot();
    snapshot.workspaces = (1..=workspaces)
        .map(|number| {
            workspace(
                &format!("ws_{number}"),
                number,
                if number == 2 { URL_2 } else { URL_1 },
            )
        })
        .collect();
    snapshot.agents = vec![ClientShellAgent {
        subagents: Default::default(),
        subagent_history: Default::default(),
        pane_id: "pane_1".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: None,
        display_agent: None,
        agent: Some("claude".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Idle,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: vec![("pr".into(), "#7009".into())],
        token_links: vec![("pr".into(), URL_A.into())],
        focused: false,
    }];
    snapshot
}

fn state_with(config: ClientShellConfig, snapshot: ClientShellSnapshot) -> ClientShellState {
    let mut state = ClientShellState::new(config);
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.compose(106, 40).expect("composed frame");
    state
}

fn mouse(
    state: &mut ClientShellState,
    kind: MouseEventKind,
    x: u16,
    y: u16,
    modifiers: KeyModifiers,
) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers,
    })])
}

fn click(
    state: &mut ClientShellState,
    x: u16,
    y: u16,
    modifiers: KeyModifiers,
) -> Vec<ClientShellAction> {
    let down = mouse(
        state,
        MouseEventKind::Down(MouseButton::Left),
        x,
        y,
        modifiers,
    );
    let up = mouse(
        state,
        MouseEventKind::Up(MouseButton::Left),
        x,
        y,
        modifiers,
    );
    let mut actions = down.actions;
    actions.extend(up.actions);
    actions
}

fn underlined(frame: &FrameData) -> Vec<(u16, u16)> {
    frame
        .cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.modifier & Modifier::UNDERLINED.bits() != 0)
        .map(|(index, _)| {
            (
                (index % frame.width as usize) as u16,
                (index / frame.width as usize) as u16,
            )
        })
        .collect()
}

fn inside(outer: Rect, hit: &TokenLinkHit) -> bool {
    outer.x <= hit.rect.x
        && hit.rect.right() <= outer.right()
        && outer.y <= hit.rect.y
        && hit.rect.bottom() <= outer.bottom()
}

fn space_hit(state: &ClientShellState, workspace: usize) -> TokenLinkHit {
    let rect = state.hits.workspaces[workspace].rect;
    state
        .hits
        .token_links
        .iter()
        .find(|hit| inside(rect, hit))
        .cloned()
        .expect("space token hit")
}

fn child_hit(state: &ClientShellState) -> TokenLinkHit {
    let rect = state.hits.space_agents[0].0;
    state
        .hits
        .token_links
        .iter()
        .find(|hit| inside(rect, hit))
        .cloned()
        .expect("child token hit")
}

fn agent_hit(state: &ClientShellState) -> TokenLinkHit {
    let rect = state.hits.agents[0].0;
    state
        .hits
        .token_links
        .iter()
        .find(|hit| inside(rect, hit))
        .cloned()
        .expect("agent row token hit")
}

fn opened(actions: &[ClientShellAction], url: &str) -> bool {
    matches!(actions, [ClientShellAction::OpenSafeWebUrl(opened)] if opened == url)
}

fn is_workspace_focus(actions: &[ClientShellAction], id: &str) -> bool {
    matches!(actions, [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, crate::api::schema::Method::WorkspaceFocus(target)
            if target.workspace_id == id))
}

fn is_pane_focus(actions: &[ClientShellAction], id: &str) -> bool {
    matches!(actions, [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, crate::api::schema::Method::PaneFocus(target)
            if target.pane_id == id))
}

#[test]
fn linked_tokens_are_underlined_exactly_over_their_hit_rects() {
    let mut state = state_with(config_with(None), link_snapshot(2));
    let frame = state.compose(106, 40).expect("frame");
    let rows = frame_rows(&frame);

    // Two space rows, one agent child and one agents-panel row.
    assert_eq!(
        state.hits.token_links.len(),
        4,
        "{:?}",
        state.hits.token_links
    );
    let mut expected = Vec::new();
    for hit in &state.hits.token_links {
        assert_eq!((hit.rect.height, hit.rect.width), (1, 5), "{hit:?}");
        let text = rows[hit.rect.y as usize]
            .chars()
            .skip(hit.rect.x as usize)
            .take(hit.rect.width as usize)
            .collect::<String>();
        assert_eq!(text, "#7009");
        for x in hit.rect.x..hit.rect.right() {
            expected.push((x, hit.rect.y));
        }
    }
    expected.sort_unstable();
    // Nothing else is underlined: not the label, the unlinked `env` token or
    // the state icon.
    let mut cells = underlined(&frame);
    cells.sort_unstable();
    assert_eq!(cells, expected);
    assert_eq!(space_hit(&state, 0).url, URL_1);
    assert_eq!(space_hit(&state, 1).url, URL_2);
    assert_eq!(child_hit(&state).url, URL_A);
    assert_eq!(agent_hit(&state).url, URL_A);
}

#[test]
fn tokens_without_links_are_not_linked() {
    let mut snapshot = link_snapshot(1);
    snapshot.workspaces[0].tokens = vec![("env".into(), "plain".into())];
    snapshot.workspaces[0].token_links.clear();
    snapshot.agents.clear();
    let mut state = state_with(config_with(None), snapshot);
    let frame = state.compose(106, 40).expect("frame");
    assert!(frame_rows(&frame).iter().any(|row| row.contains("plain")));
    assert!(state.hits.token_links.is_empty());
    assert!(underlined(&frame).is_empty());
}

#[test]
fn off_mode_draws_no_underline_and_no_hits() {
    let mut state = state_with(config_with(Some("off")), link_snapshot(2));
    let frame = state.compose(106, 40).expect("frame");
    assert!(state.hits.token_links.is_empty());
    assert!(underlined(&frame).is_empty());
}

#[test]
fn collapsed_sidebar_has_no_underline_and_no_hits() {
    let mut state = state_with(config_with(None), link_snapshot(2));
    state.sidebar_collapsed = true;
    let frame = state.compose(106, 40).expect("frame");
    assert!(state.hits.token_links.is_empty());
    assert!(underlined(&frame).is_empty());
}

#[test]
fn multi_machine_sidebar_has_no_underline_and_no_hits() {
    let mut state = ClientShellState::new(config_with(None));
    let profile = SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        label: "Build".into(),
        target: "dev@build.example".into(),
        session: "agents".into(),
        enabled: true,
    };
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);
    state.set_snapshot(Box::new(link_snapshot(2)));
    state.set_pane_surface(surface());
    let mut remote = link_snapshot(2);
    remote.boot_id = "remote-boot".into();
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));
    let frame = state.compose(106, 40).expect("frame");
    assert!(frame_rows(&frame).iter().any(|row| row.contains("#7009")));
    assert!(state.hits.token_links.is_empty());
    assert!(underlined(&frame).is_empty());
}

#[test]
fn plain_click_on_a_space_token_opens_the_link_without_selecting() {
    let mut state = state_with(config_with(None), link_snapshot(2));
    let hit = space_hit(&state, 1);
    let actions = click(
        &mut state,
        hit.rect.x + 1,
        hit.rect.y,
        KeyModifiers::empty(),
    );
    assert!(opened(&actions, URL_2), "{}", actions.len());
}

#[test]
fn release_outside_the_token_selects_the_space() {
    let mut state = state_with(config_with(None), link_snapshot(2));
    let hit = space_hit(&state, 1);
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        hit.rect.x,
        hit.rect.y,
        KeyModifiers::empty(),
    );
    let up = mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        hit.rect.x - 3,
        hit.rect.y,
        KeyModifiers::empty(),
    );
    assert!(
        is_workspace_focus(&up.actions, "ws_2"),
        "{:?}",
        up.actions.len()
    );
}

#[test]
fn clicking_the_rest_of_a_space_row_selects_it() {
    let mut state = state_with(config_with(None), link_snapshot(2));
    let hit = space_hit(&state, 1);
    let actions = click(
        &mut state,
        hit.rect.x - 4,
        hit.rect.y,
        KeyModifiers::empty(),
    );
    assert!(is_workspace_focus(&actions, "ws_2"));
}

#[test]
fn pressing_a_token_and_dragging_reorders_without_opening() {
    let mut state = state_with(config_with(None), link_snapshot(3));
    let hit = space_hit(&state, 0);
    let third = state.hits.workspaces[2].rect;
    let down = mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        hit.rect.x,
        hit.rect.y,
        KeyModifiers::empty(),
    );
    assert!(down.actions.is_empty());
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        third.x + 2,
        third.bottom(),
        KeyModifiers::empty(),
    );
    assert!(state.chrome_drag.is_some());
    let up = mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        third.x + 2,
        third.bottom(),
        KeyModifiers::empty(),
    );
    assert!(
        matches!(&up.actions[..], [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method, crate::api::schema::Method::WorkspaceMove(_))),
        "only the reorder is requested"
    );
}

#[test]
fn ctrl_mode_needs_ctrl_to_open_and_plain_click_selects() {
    let mut state = state_with(config_with(Some("ctrl")), link_snapshot(2));
    let hit = space_hit(&state, 1);
    let plain = click(&mut state, hit.rect.x, hit.rect.y, KeyModifiers::empty());
    assert!(is_workspace_focus(&plain, "ws_2"));
    state.compose(106, 40).expect("frame");
    let hit = space_hit(&state, 1);
    let with_ctrl = click(&mut state, hit.rect.x, hit.rect.y, KeyModifiers::CONTROL);
    assert!(opened(&with_ctrl, URL_2), "{}", with_ctrl.len());
    // Another modifier on top does not match exactly.
    let extra = click(
        &mut state,
        hit.rect.x,
        hit.rect.y,
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    );
    assert!(is_workspace_focus(&extra, "ws_2"));
}

#[test]
fn off_mode_click_selects_the_space() {
    let mut state = state_with(config_with(Some("off")), link_snapshot(2));
    let rect = state.hits.workspaces[1].rect;
    let actions = click(&mut state, rect.x + 3, rect.y, KeyModifiers::empty());
    assert!(is_workspace_focus(&actions, "ws_2"));
}

#[test]
fn agent_row_and_child_token_open_on_release_without_focusing() {
    let mut state = state_with(config_with(None), link_snapshot(2));
    for hit in [agent_hit(&state), child_hit(&state)] {
        let down = mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            hit.rect.x + 2,
            hit.rect.y,
            KeyModifiers::empty(),
        );
        assert!(down.actions.is_empty(), "focus must wait for nothing");
        let up = mouse(
            &mut state,
            MouseEventKind::Up(MouseButton::Left),
            hit.rect.x + 2,
            hit.rect.y,
            KeyModifiers::empty(),
        );
        assert!(opened(&up.actions, URL_A), "{}", up.actions.len());
    }
}

#[test]
fn agent_token_released_elsewhere_does_nothing() {
    let mut state = state_with(config_with(None), link_snapshot(2));
    let hit = agent_hit(&state);
    let down = mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        hit.rect.x,
        hit.rect.y,
        KeyModifiers::empty(),
    );
    assert!(down.actions.is_empty());
    let up = mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        hit.rect.x - 2,
        hit.rect.y,
        KeyModifiers::empty(),
    );
    assert!(up.actions.is_empty());
    // The pending press is gone: a later release elsewhere stays silent too.
    let again = mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        hit.rect.x,
        hit.rect.y,
        KeyModifiers::empty(),
    );
    assert!(again.actions.is_empty());
}

#[test]
fn agent_token_with_a_non_matching_modifier_focuses_as_today() {
    let mut state = state_with(config_with(None), link_snapshot(2));
    for hit in [agent_hit(&state), child_hit(&state)] {
        let actions = click(&mut state, hit.rect.x, hit.rect.y, KeyModifiers::CONTROL);
        assert!(is_pane_focus(&actions, "pane_1"), "{}", actions.len());
    }

    let mut state = state_with(config_with(Some("ctrl")), link_snapshot(2));
    let hit = agent_hit(&state);
    let actions = click(&mut state, hit.rect.x, hit.rect.y, KeyModifiers::empty());
    assert!(is_pane_focus(&actions, "pane_1"));
    let actions = click(&mut state, hit.rect.x, hit.rect.y, KeyModifiers::CONTROL);
    assert!(opened(&actions, URL_A), "{}", actions.len());
}

#[test]
fn agent_token_click_in_off_mode_focuses() {
    let mut state = state_with(config_with(Some("off")), link_snapshot(2));
    let rect = state.hits.agents[0].0;
    let actions = click(&mut state, rect.x + 2, rect.y, KeyModifiers::empty());
    assert!(is_pane_focus(&actions, "pane_1"));
}

#[test]
fn live_config_reload_applies_token_link_click() {
    let mut config = config_with(None);
    assert_eq!(
        config.token_link_click,
        crate::config::TokenLinkClick::Click
    );
    let reloaded: Config = toml::from_str("[ui.sidebar]\ntoken_link_click = \"alt\"\n").unwrap();
    config.apply_live_config(&reloaded, &[], &[]);
    assert_eq!(
        config.token_link_click,
        crate::config::TokenLinkClick::Modifier(KeyModifiers::ALT)
    );
    config.apply_live_config(&Config::default(), &[], &[]);
    assert_eq!(
        config.token_link_click,
        crate::config::TokenLinkClick::Click
    );
}
