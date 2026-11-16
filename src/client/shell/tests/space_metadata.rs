use super::*;
use crate::client::endpoint::{
    ClientEndpointId, ClientEndpointStatus, ProfileId, SavedSshEndpoint,
};

const METADATA_TOML: &str = r#"
[ui.sidebar.spaces]
rows = [["workspace", "$env"]]

[[ui.sidebar.spaces.metadata]]
match = { label = "*" }
tokens = { env = "dev" }

[[ui.sidebar.spaces.metadata]]
match = { path = "/tmp/proj" }
tokens = { env = "prod" }
"#;

fn config_from(toml: &str) -> Config {
    toml::from_str(toml).expect("config parses")
}

fn state_for(config: &Config, snapshot: ClientShellSnapshot) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(config));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state
}

fn sidebar_rows(state: &mut ClientShellState) -> Vec<String> {
    let frame = state.compose(106, 30).expect("composed frame");
    frame_rows(&frame)
        .into_iter()
        .map(|row| row.chars().take(30).collect::<String>())
        .collect()
}

fn snapshot_at(cwd: &str) -> ClientShellSnapshot {
    let mut snapshot = snapshot();
    snapshot.workspaces[0].new_workspace_cwd = cwd.into();
    snapshot.workspaces[0].branch = None;
    snapshot
}

fn has_row(rows: &[String], needle: &str) -> bool {
    rows.iter().any(|row| row.contains(needle))
}

#[test]
fn static_tokens_follow_config_order_and_api_values_win_then_fall_back() {
    let config = config_from(METADATA_TOML);
    let mut state = state_for(&config, snapshot_at("/tmp/proj/src"));
    let rows = sidebar_rows(&mut state);
    assert!(has_row(&rows, "prod"), "{rows:#?}");
    assert!(!has_row(&rows, "dev"), "{rows:#?}");

    let mut with_api = snapshot_at("/tmp/proj/src");
    with_api.workspaces[0].tokens = vec![("env".into(), "hotfix".into())];
    state.set_snapshot(Box::new(with_api));
    let rows = sidebar_rows(&mut state);
    assert!(has_row(&rows, "hotfix"), "{rows:#?}");
    assert!(!has_row(&rows, "prod"), "{rows:#?}");

    // API value cleared or expired: the snapshot no longer carries it.
    state.set_snapshot(Box::new(snapshot_at("/tmp/proj/src")));
    assert!(has_row(&sidebar_rows(&mut state), "prod"));

    // A path outside the prefix only gets the label-matched entry.
    state.set_snapshot(Box::new(snapshot_at("/tmp/proj-old")));
    assert!(has_row(&sidebar_rows(&mut state), "dev"));
}

#[test]
fn static_tokens_support_conditional_styles() {
    let config = config_from(
        r##"
[ui.sidebar.spaces]
rows = [[{ token = "$env", rules = [{ equals = "prod", fg = "#ff0000" }] }]]

[[ui.sidebar.spaces.metadata]]
match = { label = "*" }
tokens = { env = "prod" }
"##,
    );
    let mut state = state_for(&config, snapshot_at("/tmp"));
    let frame = state.compose(106, 30).expect("composed frame");
    let rows = frame_rows(&frame);
    let y = rows
        .iter()
        .position(|row| row.contains("prod"))
        .expect("row");
    let byte = rows[y].find("prod").expect("prod");
    let x = rows[y][..byte].chars().count();
    let buffer = frame.to_ratatui_buffer().expect("buffer");
    let cell = &buffer[(x as u16, y as u16)];
    assert_eq!(cell.fg, ratatui::style::Color::Rgb(255, 0, 0));

    // An unmatched space elides the token entirely.
    let mut other = config;
    other.ui.sidebar.spaces.metadata[0].matcher.label = Some("nomatch".into());
    let mut state = state_for(&other, snapshot_at("/tmp"));
    assert!(!has_row(&sidebar_rows(&mut state), "prod"));
}

#[test]
fn static_tokens_apply_to_every_machine() {
    let config = config_from(
        r#"
[ui.sidebar.spaces]
rows = [["workspace", "$env"]]

[[ui.sidebar.spaces.metadata]]
match = { label = "remote-*" }
tokens = { env = "staging" }

[[ui.sidebar.spaces.metadata]]
match = { path = "/tmp" }
tokens = { team = "x" }
"#,
    );
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
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
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.workspaces[0].label = "remote-workspace".into();
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));

    let rows = sidebar_rows(&mut state);
    assert!(has_row(&rows, "remote-work… staging"), "{rows:#?}");
}

#[test]
fn live_reload_refreshes_static_metadata_and_reports_invalid_entries() {
    let mut state = state_for(&config_from(METADATA_TOML), snapshot_at("/tmp/proj"));
    assert!(has_row(&sidebar_rows(&mut state), "prod"));

    let updated = config_from(&METADATA_TOML.replace("\"prod\"", "\"canary\""));
    let diagnostics = state.config.apply_live_config(&updated, &[], &[]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(has_row(&sidebar_rows(&mut state), "canary"));

    let invalid = config_from(
        r#"
[ui.sidebar.spaces]
rows = [["workspace", "$env"]]

[[ui.sidebar.spaces.metadata]]
match = { label = "*" }
tokens = { "my env" = "x" }

[[ui.sidebar.spaces.metadata]]
match = { label = "*" }
tokens = { env = "ok" }
"#,
    );
    let diagnostics = state.config.apply_live_config(&invalid, &[], &[]);
    assert!(
        diagnostics.iter().any(|d| d.contains("my env")),
        "{diagnostics:?}"
    );
    assert!(invalid
        .collect_diagnostics()
        .iter()
        .any(|d| d.contains("my env")));
    assert!(has_row(&sidebar_rows(&mut state), "ok"));
}
