//! Per-client panel surfaces: visibility claims, input routing and frame streaming.
//!
//! Panels are server-owned terminals (see `app::panels`). A client opts into a
//! panel's frames with `panel.surface.set`; only visible panels are rendered
//! and streamed, and the claims die with the connection.

use super::*;
use crate::api::schema::PanelRunState;
use crate::app::state::PanelStatus;
use crate::protocol::endpoint::{panel_surface_message, EndpointPanelStatus, EndpointPanelSurface};

impl HeadlessServer {
    /// Handle the connection-local panel methods. Returns `None` when the
    /// method is not one of them, otherwise whether the render state changed.
    pub(super) fn handle_panel_endpoint_request(
        &mut self,
        client_id: u64,
        boot_id: &str,
        request: &api::schema::Request,
    ) -> Option<bool> {
        match &request.method {
            api::schema::Method::PanelSurfaceSet(params) => {
                let reply = self.panel_surface_set(client_id, params);
                Some(self.reply_panel_request(client_id, boot_id, &request.id, reply))
            }
            api::schema::Method::PanelInput(params) => {
                let reply = self.panel_input(client_id, params);
                Some(self.reply_panel_request(client_id, boot_id, &request.id, reply))
            }
            _ => None,
        }
    }

    fn reply_panel_request(
        &mut self,
        client_id: u64,
        boot_id: &str,
        request_id: &str,
        reply: Result<bool, (&'static str, String)>,
    ) -> bool {
        let (message, changed) = match reply {
            Ok(changed) => (
                crate::server::client_commands::success_message_with_result(
                    boot_id.to_owned(),
                    request_id.to_owned(),
                    api::schema::ResponseResult::Ok {},
                ),
                changed,
            ),
            Err((code, text)) => (
                crate::server::client_commands::error_message(
                    boot_id.to_owned(),
                    request_id.to_owned(),
                    code,
                    text,
                ),
                false,
            ),
        };
        self.send_to_client(client_id, message);
        // Send the first frame right after the acknowledgement so a newly
        // visible panel does not wait for its next output.
        self.stream_panel_surfaces();
        changed
    }

    fn panel_surface_set(
        &mut self,
        client_id: u64,
        params: &api::schema::PanelSurfaceSetParams,
    ) -> Result<bool, (&'static str, String)> {
        let not_found = || {
            (
                "panel_not_found",
                format!("panel '{}' does not exist", params.id),
            )
        };
        if !self
            .app
            .state
            .panels
            .iter()
            .any(|panel| panel.id == params.id)
        {
            return Err(not_found());
        }
        let Some(client) = self.clients.get_mut(&client_id) else {
            return Err(("unknown_client", "client is no longer connected".to_owned()));
        };
        if !client.is_active_shell_client() {
            return Err((
                "surface_inactive",
                "this method requires an active client shell surface".to_owned(),
            ));
        }
        if params.visible && (params.cols == 0 || params.rows == 0) {
            return Err((
                "invalid_params",
                "a visible panel needs non-zero cols and rows".to_owned(),
            ));
        }
        let cell_size = client.cell_size;
        let surface = client.panel_surfaces.entry(params.id.clone()).or_default();
        let was_visible = surface.visible;
        surface.visible = params.visible;
        if params.visible {
            surface.cols = params.cols;
            surface.rows = params.rows;
        }
        if params.visible && !was_visible {
            // Force a fresh frame for the newly visible panel.
            surface.sent = None;
        }
        if !params.visible {
            client.panel_surfaces.remove(&params.id);
            self.sync_immediate_pty_sources();
            return Ok(false);
        }
        // Last writer wins: the PTY follows the most recent visible claim.
        self.app.resize_panel(
            &params.id,
            params.cols,
            params.rows,
            cell_size.width_px,
            cell_size.height_px,
        );
        self.sync_immediate_pty_sources();
        Ok(false)
    }

    fn panel_input(
        &mut self,
        client_id: u64,
        params: &api::schema::PanelInputParams,
    ) -> Result<bool, (&'static str, String)> {
        let Some(client) = self.clients.get(&client_id) else {
            return Err(("unknown_client", "client is no longer connected".to_owned()));
        };
        if !client.is_active_shell_client() {
            return Err((
                "surface_inactive",
                "this method requires an active client shell surface".to_owned(),
            ));
        }
        let pixel_mouse = client.pixel_mouse && client.host_sgr_pixels_active == Some(true);
        let Some(panel) = self
            .app
            .state
            .panels
            .iter()
            .find(|panel| panel.id == params.id)
        else {
            return Err((
                "panel_not_found",
                format!("panel '{}' does not exist", params.id),
            ));
        };
        let (terminal_id, pane_id) = (panel.terminal_id.clone(), panel.pane_id);
        let mut events = Vec::with_capacity(params.events.len());
        for value in &params.events {
            match serde_json::from_value::<protocol::ClientPaneInputEvent>(value.clone()) {
                Ok(event) => events.push(event),
                Err(err) => {
                    return Err(("invalid_params", format!("invalid input event: {err}")));
                }
            }
        }
        if !matches!(
            crate::server::client_transport::pane_input_event_limit(&events),
            crate::server::client_transport::InputEventLimit::WithinLimits
        ) {
            return Err((
                "input_too_large",
                "panel input exceeds the per-request limits".to_owned(),
            ));
        }
        let Some(runtime) = self.app.terminal_runtimes.get(&terminal_id) else {
            // An exited panel has no process to receive input.
            return Ok(false);
        };
        crate::server::pane_input::downgrade_ineligible_pixel_mouse(
            &mut events,
            pixel_mouse,
            runtime.current_size(),
            runtime.pixel_size(),
        );
        let scroll_before = runtime.scroll_metrics();
        let mut keyboard_forwarded = false;
        let result =
            apply_client_terminal_input_events(runtime, &events, false, &mut keyboard_forwarded);
        let scroll_changed = runtime.scroll_metrics() != scroll_before;
        if keyboard_forwarded {
            self.note_interactive_input(pane_id, Instant::now());
        }
        if let Err(err) = result {
            warn!(client_id, panel = %params.id, err = %err, "panel input failed");
        }
        Ok(scroll_changed)
    }

    /// Whether any active shell client currently shows the panel owning
    /// `pane_id`. `None` when `pane_id` is not a panel.
    pub(super) fn panel_pane_visibility(&self, pane_id: crate::layout::PaneId) -> Option<bool> {
        let panel = self
            .app
            .state
            .panels
            .iter()
            .find(|panel| panel.pane_id == pane_id)?;
        Some(self.clients.values().any(|client| {
            client.is_active_shell_client()
                && client.writer.is_some()
                && client
                    .panel_surfaces
                    .get(&panel.id)
                    .is_some_and(|surface| surface.visible)
        }))
    }

    /// Remove panel pane ids from `sources`, returning whether any were present.
    pub(super) fn take_panel_pty_sources(
        &self,
        sources: &mut HashSet<crate::layout::PaneId>,
    ) -> bool {
        if self.app.state.panels.is_empty() {
            return false;
        }
        let before = sources.len();
        sources.retain(|pane_id| {
            !self
                .app
                .state
                .panels
                .iter()
                .any(|panel| panel.pane_id == *pane_id)
        });
        sources.len() != before
    }

    /// Pane ids of every panel that some active shell client shows.
    pub(super) fn visible_panel_pane_ids(&self) -> HashSet<crate::layout::PaneId> {
        self.app
            .state
            .panels
            .iter()
            .filter(|panel| self.panel_pane_visibility(panel.pane_id) == Some(true))
            .map(|panel| panel.pane_id)
            .collect()
    }

    /// Send one `endpoint.panel-surface.v1` frame per visible, changed panel
    /// and client. Hidden panels cost nothing here.
    pub(super) fn stream_panel_surfaces(&mut self) {
        if self.handoff_in_progress {
            return;
        }
        let client_ids: Vec<u64> = self
            .clients
            .iter()
            .filter(|(_, client)| {
                client.is_active_shell_client()
                    && client.writer.is_some()
                    && !client.panel_surfaces.is_empty()
            })
            .map(|(&id, _)| id)
            .collect();
        let mut broken = Vec::new();
        for client_id in client_ids {
            let ids: Vec<String> = self
                .clients
                .get(&client_id)
                .map(|client| {
                    client
                        .panel_surfaces
                        .iter()
                        .filter(|(_, surface)| surface.visible)
                        .map(|(id, _)| id.clone())
                        .collect()
                })
                .unwrap_or_default();
            for id in ids {
                let Some(panel) = self.app.state.panels.iter().find(|panel| panel.id == id) else {
                    continue;
                };
                let Some(runtime) = self.app.terminal_runtimes.get(&panel.terminal_id) else {
                    // Exited panel: report the final state once.
                    let status = panel_run_state(&panel.status);
                    let key = (panel.terminal_id.to_string(), 0, status);
                    let sent = self
                        .clients
                        .get(&client_id)
                        .and_then(|c| c.panel_surfaces.get(&id))
                        .and_then(|s| s.sent.clone());
                    if sent.as_ref() == Some(&key) {
                        continue;
                    }
                    let blank = EmptyPanelFrame::build(panel.cols, panel.rows);
                    if !self.send_panel_frame(client_id, &id, key, blank) {
                        broken.push(client_id);
                    }
                    continue;
                };
                if runtime.synchronized_output_active() {
                    continue;
                }
                let status = panel_run_state(&panel.status);
                let key = (panel.terminal_id.to_string(), runtime.content_seq(), status);
                let unchanged = self
                    .clients
                    .get(&client_id)
                    .and_then(|c| c.panel_surfaces.get(&id))
                    .is_some_and(|s| s.sent.as_ref() == Some(&key));
                if unchanged {
                    continue;
                }
                let (rows, cols) = runtime.current_size();
                let area = Rect::new(0, 0, cols, rows);
                let (buffer, cursor) =
                    crate::server::render_stream::render_terminal_virtual(runtime, area);
                let hyperlinks = runtime.visible_hyperlinks(area);
                let frame = protocol::FrameData::from_ratatui_buffer_with_hyperlinks(
                    &buffer,
                    cursor,
                    &hyperlinks,
                );
                let parts = PanelFrameParts {
                    frame,
                    mouse_reporting: runtime.mouse_reporting_enabled(),
                    sgr_pixel_mouse: runtime.sgr_pixel_mouse_enabled(),
                    cols,
                    rows,
                };
                if !self.send_panel_frame(client_id, &id, key, parts) {
                    broken.push(client_id);
                }
            }
        }
        for client_id in broken {
            self.remove_client_and_resize_if_needed(client_id);
        }
    }

    /// Encode and queue one frame. Returns false when the client writer is gone.
    fn send_panel_frame(
        &mut self,
        client_id: u64,
        id: &str,
        key: (String, u64, PanelRunState),
        parts: PanelFrameParts,
    ) -> bool {
        let Some(panel) = self.app.state.panels.iter().find(|panel| panel.id == id) else {
            return true;
        };
        let (status, exit_code) = match panel.status {
            PanelStatus::Running => (EndpointPanelStatus::Running, None),
            PanelStatus::Exited(code) => (EndpointPanelStatus::Exited, code),
        };
        let title = self
            .app
            .state
            .terminals
            .get(&panel.terminal_id)
            .and_then(|terminal| terminal.manual_label.clone())
            .unwrap_or_else(|| panel.id.clone());
        let Some(client) = self.clients.get_mut(&client_id) else {
            return true;
        };
        let cell = client.cell_size;
        let (pixel_width, pixel_height) = if cell.is_known() {
            (
                u32::from(parts.cols) * cell.width_px,
                u32::from(parts.rows) * cell.height_px,
            )
        } else {
            (0, 0)
        };
        let Some(surface) = client.panel_surfaces.get_mut(id) else {
            return true;
        };
        surface.revision = surface.revision.saturating_add(1);
        let message = panel_surface_message(&EndpointPanelSurface {
            boot_id: self.client_shell_boot_id.clone(),
            id: id.to_owned(),
            revision: surface.revision,
            title,
            status,
            exit_code,
            frame: parts.frame,
            mouse_reporting: parts.mouse_reporting,
            sgr_pixel_mouse: parts.sgr_pixel_mouse,
            pixel_width,
            pixel_height,
        });
        let framed = match message
            .map_err(|err| err.to_string())
            .and_then(|message| Self::frame_server_message(&message).map_err(|e| e.to_string()))
        {
            Ok(framed) => framed,
            Err(err) => {
                warn!(client_id, panel = id, err = %err, "failed to encode panel surface");
                return true;
            }
        };
        surface.sent = Some(key);
        match client.writer.as_ref() {
            Some(writer) => writer.control.send(framed).is_ok(),
            None => true,
        }
    }
}

fn panel_run_state(status: &PanelStatus) -> PanelRunState {
    match status {
        PanelStatus::Running => PanelRunState::Running,
        PanelStatus::Exited(_) => PanelRunState::Exited,
    }
}

struct PanelFrameParts {
    frame: protocol::FrameData,
    mouse_reporting: bool,
    sgr_pixel_mouse: bool,
    cols: u16,
    rows: u16,
}

struct EmptyPanelFrame;

impl EmptyPanelFrame {
    fn build(cols: u16, rows: u16) -> PanelFrameParts {
        let area = Rect::new(0, 0, cols.max(1), rows.max(1));
        let buffer = ratatui::buffer::Buffer::empty(area);
        PanelFrameParts {
            frame: protocol::FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]),
            mouse_reporting: false,
            sgr_pixel_mouse: false,
            cols: area.width,
            rows: area.height,
        }
    }
}
