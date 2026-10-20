use super::*;

use std::sync::{Arc, Mutex};

use crate::config::PanelConfig;
use crate::protocol::endpoint::{EndpointPanelSurface, PANEL_SURFACE_KIND};

type InputReceivers = Arc<Mutex<Vec<tokio::sync::mpsc::Receiver<Bytes>>>>;

/// Headless server with a panel `dash` whose runtime has an input channel.
fn panel_server() -> (HeadlessServer, InputReceivers) {
    let mut server = test_headless_server();
    let receivers: InputReceivers = Arc::default();
    let sink = receivers.clone();
    server.app.test_panel_runtime_factory = Some(Box::new(move |_spec, cols, rows| {
        let (runtime, rx) = crate::terminal::TerminalRuntime::test_with_channel(cols, rows);
        sink.lock().unwrap().push(rx);
        Ok(runtime)
    }));
    let diagnostics = server.app.reconcile_panels(&[PanelConfig {
        id: "dash".into(),
        command: vec!["dash".into()],
        cwd: None,
    }]);
    assert!(diagnostics.is_empty());
    (server, receivers)
}

fn connect_shell(
    server: &mut HeadlessServer,
    client_id: u64,
) -> (
    std::sync::mpsc::Receiver<Vec<u8>>,
    std::sync::mpsc::Receiver<Vec<u8>>,
) {
    let (writer, control_rx, render_rx) = test_client_writer();
    server.handle_server_event(ServerEvent::ClientShellConnected {
        surface_reuse: false,
        surface_delta: false,
        surface_scroll: false,
        surface_underline_color: false,
        client_id,
        surface_cols: 100,
        surface_rows: 40,
        cell_width_px: 8,
        cell_height_px: 16,
        pixel_mouse: false,
        direct_graphics: false,
        endpoint_keybindings: true,
        mouse_capture: false,
        surface_active: true,
        writer,
    });
    drain(&control_rx);
    (control_rx, render_rx)
}

fn drain(rx: &std::sync::mpsc::Receiver<Vec<u8>>) -> Vec<ServerMessage> {
    let mut messages = Vec::new();
    while let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
        messages.push(read_server_message(bytes));
    }
    messages
}

fn panel_frames(messages: &[ServerMessage]) -> Vec<EndpointPanelSurface> {
    messages
        .iter()
        .filter_map(|message| match message {
            ServerMessage::EndpointControl { kind, data } if kind == PANEL_SURFACE_KIND => {
                Some(serde_json::from_str(data).expect("panel surface payload"))
            }
            _ => None,
        })
        .collect()
}

fn send(server: &mut HeadlessServer, client_id: u64, method: api::schema::Method) {
    let boot_id = server.client_shell_boot_id.clone();
    server.handle_server_event(ServerEvent::ClientShellEndpointRequest {
        client_id,
        boot_id,
        request: Box::new(api::schema::Request {
            id: "req".into(),
            method,
        }),
    });
}

fn surface_set(id: &str, cols: u16, rows: u16, visible: bool) -> api::schema::Method {
    api::schema::Method::PanelSurfaceSet(api::schema::PanelSurfaceSetParams {
        id: id.into(),
        cols,
        rows,
        visible,
    })
}

fn response_error_code(messages: &[ServerMessage]) -> Option<String> {
    messages.iter().find_map(|message| match message {
        ServerMessage::ClientShellEndpointResponseChunk { data, .. } => {
            serde_json::from_slice::<api::schema::ErrorResponse>(data)
                .ok()
                .map(|error| error.error.code)
        }
        _ => None,
    })
}

fn panel_terminal(server: &HeadlessServer) -> &crate::terminal::TerminalRuntime {
    let panel = &server.app.state.panels[0];
    server
        .app
        .terminal_runtimes
        .get(&panel.terminal_id)
        .expect("panel runtime")
}

#[tokio::test]
async fn surface_set_resizes_panel_tracks_visibility_and_clears_on_disconnect() {
    let (mut server, _inputs) = panel_server();
    let (control_rx, _render_rx) = connect_shell(&mut server, 1);

    send(&mut server, 1, surface_set("dash", 50, 20, true));
    let messages = drain(&control_rx);
    assert_eq!(response_error_code(&messages), None);
    assert_eq!(panel_terminal(&server).current_size(), (20, 50));
    assert_eq!(
        (
            server.app.state.panels[0].cols,
            server.app.state.panels[0].rows
        ),
        (50, 20)
    );
    assert!(server.clients[&1].panel_surfaces["dash"].visible);
    let pane_id = server.app.state.panels[0].pane_id;
    assert_eq!(server.panel_pane_visibility(pane_id), Some(true));
    // The newly visible panel gets its first frame without waiting for output.
    assert_eq!(panel_frames(&messages).len(), 1);

    // Last writer wins.
    let (_control_rx2, _render_rx2) = connect_shell(&mut server, 2);
    send(&mut server, 2, surface_set("dash", 30, 10, true));
    assert_eq!(panel_terminal(&server).current_size(), (10, 30));

    // Hiding drops the claim; the other client keeps the panel visible.
    send(&mut server, 1, surface_set("dash", 50, 20, false));
    assert!(!server.clients[&1].panel_surfaces.contains_key("dash"));
    assert_eq!(server.panel_pane_visibility(pane_id), Some(true));

    server.remove_client_and_resize_if_needed(2);
    assert_eq!(server.panel_pane_visibility(pane_id), Some(false));
    assert!(server.visible_panel_pane_ids().is_empty());
}

#[tokio::test]
async fn surface_set_rejects_unknown_panel_and_inactive_surface() {
    let (mut server, _inputs) = panel_server();
    let (control_rx, _render_rx) = connect_shell(&mut server, 1);
    send(&mut server, 1, surface_set("nope", 10, 10, true));
    assert_eq!(
        response_error_code(&drain(&control_rx)).as_deref(),
        Some("panel_not_found")
    );

    send(
        &mut server,
        1,
        api::schema::Method::ClientShellSurfaceSet(api::schema::ClientShellSurfaceSetParams {
            active: false,
        }),
    );
    drain(&control_rx);
    send(&mut server, 1, surface_set("dash", 10, 10, true));
    assert_eq!(
        response_error_code(&drain(&control_rx)).as_deref(),
        Some("surface_inactive")
    );
}

#[tokio::test]
async fn hidden_panel_output_sends_nothing_and_visible_output_sends_one_frame() {
    let (mut server, _inputs) = panel_server();
    let (control_rx, render_rx) = connect_shell(&mut server, 1);
    let pane_id = server.app.state.panels[0].pane_id;

    // Hidden: output is not visible to any render target and streams no frame.
    panel_terminal(&server).test_process_pty_bytes(b"hidden output");
    let mut sources = HashSet::from([pane_id]);
    assert!(server.take_panel_pty_sources(&mut sources));
    assert!(sources.is_empty());
    assert!(!server.pty_sources_visible_to_any_render_target(&HashSet::from([pane_id])));
    server.stream_panel_surfaces();
    assert!(drain(&control_rx).is_empty());
    assert!(render_rx.try_recv().is_err());

    // Visible: one frame on show, then exactly one frame per changed tick.
    send(&mut server, 1, surface_set("dash", 40, 10, true));
    let frames = panel_frames(&drain(&control_rx));
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].id, "dash");
    assert_eq!(frames[0].revision, 1);
    assert!(frame_text(&frames[0].frame).contains("hidden output"));

    server.stream_panel_surfaces();
    assert!(
        drain(&control_rx).is_empty(),
        "unchanged panel is not resent"
    );

    panel_terminal(&server).test_process_pty_bytes(b"one");
    panel_terminal(&server).test_process_pty_bytes(b"two");
    server.stream_panel_surfaces();
    server.stream_panel_surfaces();
    let frames = panel_frames(&drain(&control_rx));
    assert_eq!(frames.len(), 1, "output coalesces into one frame per tick");
    assert_eq!(frames[0].revision, 2);
    assert!(frame_text(&frames[0].frame).contains("onetwo"));
}

#[tokio::test]
async fn exited_panel_reports_final_state_once() {
    let (mut server, _inputs) = panel_server();
    let (control_rx, _render_rx) = connect_shell(&mut server, 1);
    send(&mut server, 1, surface_set("dash", 40, 10, true));
    drain(&control_rx);

    let pane_id = server.app.state.panels[0].pane_id;
    server.app.handle_panel_process_exit(pane_id, Some(3));
    server.stream_panel_surfaces();
    let frames = panel_frames(&drain(&control_rx));
    assert_eq!(frames.len(), 1);
    assert_eq!(
        frames[0].status,
        crate::protocol::endpoint::EndpointPanelStatus::Exited
    );
    assert_eq!(frames[0].exit_code, Some(3));
    server.stream_panel_surfaces();
    assert!(drain(&control_rx).is_empty());
}

#[tokio::test]
async fn panel_input_reaches_the_panel_runtime() {
    let (mut server, inputs) = panel_server();
    let (control_rx, _render_rx) = connect_shell(&mut server, 1);
    let events = vec![
        serde_json::to_value(crate::protocol::ClientPaneInputEvent::Paste("hello".into())).unwrap(),
    ];
    send(
        &mut server,
        1,
        api::schema::Method::PanelInput(api::schema::PanelInputParams {
            id: "dash".into(),
            events,
        }),
    );
    assert_eq!(response_error_code(&drain(&control_rx)), None);
    let mut inputs = inputs.lock().unwrap();
    let received = inputs[0].try_recv().expect("paste reaches the panel");
    assert!(String::from_utf8_lossy(&received).contains("hello"));
}

#[tokio::test]
async fn panel_input_validates_target_and_events() {
    let (mut server, _inputs) = panel_server();
    let (control_rx, _render_rx) = connect_shell(&mut server, 1);
    send(
        &mut server,
        1,
        api::schema::Method::PanelInput(api::schema::PanelInputParams {
            id: "nope".into(),
            events: Vec::new(),
        }),
    );
    assert_eq!(
        response_error_code(&drain(&control_rx)).as_deref(),
        Some("panel_not_found")
    );
    send(
        &mut server,
        1,
        api::schema::Method::PanelInput(api::schema::PanelInputParams {
            id: "dash".into(),
            events: vec![serde_json::json!({"not": "an event"})],
        }),
    );
    assert_eq!(
        response_error_code(&drain(&control_rx)).as_deref(),
        Some("invalid_params")
    );
}

/// Per-tick panel cost with a busy panel, hidden vs visible, at 1 and 15
/// populated panes. Run with
/// `cargo test --release --locked --bin herdr panel_tick_scale_profile -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "manual panel scaling profile"]
async fn panel_tick_scale_profile() {
    const SAMPLES: usize = 400;
    println!("panel tick scaling (busy panel: 1 line of output per tick)");
    println!("  panes  panel_state  median_us  p95_us  frames_sent");
    for panes in [1usize, 15] {
        for visible in [false, true] {
            let (mut server, _inputs) = panel_server();
            let (control_rx, _render_rx) = connect_shell(&mut server, 1);
            for index in 0..panes {
                let mut workspace =
                    crate::workspace::Workspace::test_new(&format!("bench-{}", index + 1));
                let root = workspace.tabs[0].root_pane;
                workspace.insert_test_runtime(
                    root,
                    crate::terminal::TerminalRuntime::test_with_scrollback_bytes(
                        120,
                        40,
                        1024 * 1024,
                        b"",
                    ),
                );
                server.app.state.workspaces.push(workspace);
            }
            server.app.state.active = Some(0);
            if visible {
                send(&mut server, 1, surface_set("dash", 60, 20, true));
            }
            drain(&control_rx);
            let pane_id = server.app.state.panels[0].pane_id;
            let mut samples = Vec::with_capacity(SAMPLES);
            let mut frames = 0usize;
            for line in 0..SAMPLES + 20 {
                panel_terminal(&server)
                    .test_process_pty_bytes(format!("busy line {line}\r\n").as_bytes());
                let started = Instant::now();
                let mut sources = HashSet::from([pane_id]);
                let panel_dirty = server.take_panel_pty_sources(&mut sources);
                if panel_dirty {
                    server.stream_panel_surfaces();
                }
                std::hint::black_box(server.visible_panel_pane_ids());
                let hidden_only = !sources.is_empty()
                    && !server.pty_sources_visible_to_any_render_target(&sources);
                std::hint::black_box(hidden_only);
                let elapsed = started.elapsed();
                if line >= 20 {
                    samples.push(elapsed.as_micros());
                }
                while let Ok(bytes) = control_rx.try_recv() {
                    if !panel_frames(&[read_server_message(bytes)]).is_empty() {
                        frames += 1;
                    }
                }
            }
            samples.sort_unstable();
            println!(
                "  {panes:>5}  {:>11}  {:>9}  {:>6}  {frames}",
                if visible { "visible" } else { "hidden" },
                samples[samples.len() / 2],
                samples[samples.len() * 95 / 100],
            );
        }
    }
}
