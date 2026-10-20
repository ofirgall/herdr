//! Sidebar panel sections: configuration, render state and surface synchronisation.
//!
//! Panels are server-owned terminals. The client declares the body size of each visible panel
//! with `panel.surface.set` and caches the latest `endpoint.panel-surface.v1` frame to blit into
//! the sidebar. Everything here that decides what to send runs outside of render.

use super::*;
use crate::config::PanelHeight;
use crate::protocol::endpoint::{EndpointPanelStatus, EndpointPanelSurface};
use crate::ui::SectionSpec;

/// One configured sidebar section, in render order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SidebarSectionConfig {
    Spaces,
    Agents,
    Panel(PanelSectionConfig),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelSectionConfig {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) height: PanelHeight,
    /// Width the sidebar grows to while the panel has focus.
    pub(crate) focus_width: Option<u16>,
}

pub(crate) fn sections_from_config(config: &crate::config::Config) -> Vec<SidebarSectionConfig> {
    let (sections, _) = config.ui.sidebar.resolved_sections();
    sections
        .into_iter()
        .map(|section| match section {
            crate::config::SidebarSection::Spaces => SidebarSectionConfig::Spaces,
            crate::config::SidebarSection::Agents => SidebarSectionConfig::Agents,
            crate::config::SidebarSection::Panel(id) => {
                let panel = config.ui.sidebar.panel.get(&id);
                SidebarSectionConfig::Panel(PanelSectionConfig {
                    title: panel
                        .and_then(|panel| panel.title.clone())
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or_else(|| id.clone()),
                    height: panel.map_or(PanelHeight::DEFAULT, |panel| panel.resolved_height()),
                    focus_width: panel.and_then(|panel| panel.focus_width),
                    id,
                })
            }
        })
        .collect()
}

pub(crate) fn section_specs(sections: &[SidebarSectionConfig]) -> Vec<SectionSpec> {
    sections
        .iter()
        .map(|section| match section {
            SidebarSectionConfig::Spaces => SectionSpec::Spaces,
            SidebarSectionConfig::Agents => SectionSpec::Agents,
            SidebarSectionConfig::Panel(panel) => SectionSpec::Panel {
                height: panel.height,
            },
        })
        .collect()
}

/// What a panel body shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PanelBodyState {
    /// The server streams a frame (blank until the first one arrives).
    Frame,
    /// The server has no panel with this id.
    NotConfigured,
    /// The active server cannot host panels.
    Unavailable,
    /// The panel process exited.
    Exited,
}

impl PanelBodyState {
    pub(crate) fn message(self) -> Option<&'static str> {
        match self {
            Self::Frame => None,
            Self::NotConfigured => Some("not configured"),
            Self::Unavailable => Some("unavailable on this server"),
            Self::Exited => Some("exited — click to restart"),
        }
    }
}

/// Where one panel section was drawn. Panel focus and input routing key off this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelSectionHit {
    pub(crate) id: String,
    pub(crate) header: Rect,
    pub(crate) body: Rect,
    pub(crate) state: PanelBodyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SentPanel {
    cols: u16,
    rows: u16,
    visible: bool,
}

/// Client-side panel state of one endpoint.
#[derive(Debug, Default)]
pub(crate) struct EndpointPanels {
    pub(super) supported: bool,
    pub(super) frames: HashMap<String, EndpointPanelSurface>,
    /// Ids the server answered `panel_not_found` for.
    pub(super) unknown: HashSet<String>,
    pub(super) sent: HashMap<String, SentPanel>,
}

/// Read-only panel facts a render needs.
#[derive(Clone, Copy)]
pub(super) struct PanelView<'a> {
    available: bool,
    panels: Option<&'a EndpointPanels>,
}

impl<'a> PanelView<'a> {
    pub(super) fn new(available: bool, panels: Option<&'a EndpointPanels>) -> Self {
        Self { available, panels }
    }

    pub(super) fn body_state(&self, id: &str) -> PanelBodyState {
        if !self.available {
            return PanelBodyState::Unavailable;
        }
        let Some(panels) = self.panels else {
            return PanelBodyState::Unavailable;
        };
        if panels.unknown.contains(id) {
            return PanelBodyState::NotConfigured;
        }
        match panels.frames.get(id) {
            Some(frame) if frame.status == EndpointPanelStatus::Exited => PanelBodyState::Exited,
            _ => PanelBodyState::Frame,
        }
    }
}

pub(super) fn section_rect(
    specs: &[SectionSpec],
    layout: &[crate::ui::SectionRect],
    wanted: &SectionSpec,
) -> Rect {
    layout
        .iter()
        .find(|placed| &specs[placed.spec_index] == wanted)
        .map_or_else(Rect::default, |placed| placed.rect)
}

/// Header plus body of every panel in `layout`, with hits recorded for the sync step.
pub(super) fn render_panel_sections(
    buffer: &mut Buffer,
    sections: &[SidebarSectionConfig],
    layout: &[crate::ui::SectionRect],
    panels: PanelView<'_>,
    palette: &Palette,
    hits: &mut ShellHitMap,
) {
    hits.panel_sections.clear();
    for placed in layout {
        let SidebarSectionConfig::Panel(panel) = &sections[placed.spec_index] else {
            continue;
        };
        let area = placed.rect;
        if area.is_empty() {
            continue;
        }
        super::render::put_text(
            buffer,
            area.x,
            area.y,
            area.width,
            &format!(" {}", panel.title),
            Style::default()
                .fg(palette.overlay0)
                .add_modifier(Modifier::BOLD),
        );
        let header = Rect::new(area.x, area.y, area.width, 1);
        let body = Rect::new(
            area.x,
            area.y.saturating_add(1),
            area.width,
            area.height.saturating_sub(1),
        );
        let state = panels.body_state(&panel.id);
        if let Some(message) = state.message() {
            for (row, line) in wrap_words(message, usize::from(body.width.saturating_sub(1)))
                .into_iter()
                .take(usize::from(body.height))
                .enumerate()
            {
                super::render::put_text(
                    buffer,
                    body.x,
                    body.y.saturating_add(row as u16),
                    body.width,
                    &format!(" {line}"),
                    Style::default().fg(palette.overlay0),
                );
            }
        }
        hits.panel_sections.push(PanelSectionHit {
            id: panel.id.clone(),
            header,
            body,
            state,
        });
    }
}

impl ClientShellState {
    pub(super) fn panel_surfaces_available(&self) -> bool {
        self.panel_endpoints
            .get(&self.active_endpoint_id)
            .is_some_and(|panels| panels.supported)
            && self.supports_endpoint_method(&panel_surface_set_method(String::new(), 0, 0, false))
    }

    pub(crate) fn set_endpoint_panel_surfaces_supported(
        &mut self,
        endpoint_id: &ClientEndpointId,
        supported: bool,
    ) {
        // Every (re)connect starts a new server connection that knows nothing we sent before.
        self.panel_endpoints.insert(
            endpoint_id.clone(),
            EndpointPanels {
                supported,
                ..Default::default()
            },
        );
    }

    /// Cache a streamed panel frame. Returns whether the visible sidebar changed.
    pub(crate) fn receive_panel_surface(
        &mut self,
        endpoint_id: &ClientEndpointId,
        surface: EndpointPanelSurface,
    ) -> bool {
        let boot_matches = self
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .and_then(|endpoint| endpoint.snapshot.as_deref())
            .is_none_or(|snapshot| snapshot.boot_id == surface.boot_id);
        if !boot_matches {
            return false;
        }
        let Some(panels) = self.panel_endpoints.get_mut(endpoint_id) else {
            return false;
        };
        if !panels.supported {
            return false;
        }
        if panels
            .frames
            .get(&surface.id)
            .is_some_and(|previous| previous.revision >= surface.revision)
        {
            return false;
        }
        panels.unknown.remove(&surface.id);
        let visible = endpoint_id == &self.active_endpoint_id
            && self
                .hits
                .panel_sections
                .iter()
                .any(|hit| hit.id == surface.id && !hit.body.is_empty());
        panels.frames.insert(surface.id.clone(), surface);
        visible
    }

    pub(crate) fn take_pending_panel_actions(&mut self) -> Vec<ClientShellAction> {
        std::mem::take(&mut self.pending_panel_actions)
    }

    /// Declare the visible panel bodies to the active server. Runs after hits are recorded and
    /// sends exactly one `panel.surface.set` per change.
    pub(super) fn sync_panel_surfaces(&mut self) {
        let endpoint_id = self.active_endpoint_id.clone();
        let available = self.panel_surfaces_available() && self.endpoint_is_online(&endpoint_id);
        // Other endpoints stop streaming when their surface is deactivated; forget what we told them.
        for (id, panels) in self.panel_endpoints.iter_mut() {
            if id != &endpoint_id {
                panels.sent.clear();
            }
        }
        if !available {
            if let Some(panels) = self.panel_endpoints.get_mut(&endpoint_id) {
                panels.sent.clear();
            }
            return;
        }
        let wanted = self
            .hits
            .panel_sections
            .iter()
            .filter(|hit| hit.state != PanelBodyState::NotConfigured)
            .map(|hit| {
                (
                    hit.id.clone(),
                    SentPanel {
                        cols: hit.body.width,
                        rows: hit.body.height,
                        visible: !hit.body.is_empty(),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        let Some(panels) = self.panel_endpoints.get(&endpoint_id) else {
            return;
        };
        let mut changes = Vec::new();
        for (id, want) in &wanted {
            let up_to_date = panels
                .sent
                .get(id)
                .is_some_and(|sent| sent == want || (!sent.visible && !want.visible));
            if !up_to_date && (want.visible || panels.sent.contains_key(id)) {
                changes.push((id.clone(), *want));
            }
        }
        // A section that vanished (collapsed sidebar, mobile layout, removed from config).
        for (id, sent) in &panels.sent {
            if sent.visible && !wanted.contains_key(id) {
                changes.push((
                    id.clone(),
                    SentPanel {
                        visible: false,
                        ..*sent
                    },
                ));
            }
        }
        changes.sort_by(|a, b| a.0.cmp(&b.0));
        for (id, next) in changes {
            let mut outcome = ClientShellInput::default();
            let queued = self.push_endpoint_method_with_kind(
                panel_surface_set_method(id.clone(), next.cols, next.rows, next.visible),
                PendingEndpointKind::PanelSurface {
                    endpoint_id: endpoint_id.clone(),
                    id: id.clone(),
                },
                &mut outcome,
            );
            if queued {
                self.pending_panel_actions.extend(outcome.actions);
                if let Some(panels) = self.panel_endpoints.get_mut(&endpoint_id) {
                    panels.sent.insert(id, next);
                }
            }
        }
    }

    /// Outcome of a `panel.surface.set`. Never surfaces notices: a server that cannot host the
    /// panel simply shows the section's fallback text.
    pub(super) fn complete_panel_surface_request(
        &mut self,
        endpoint_id: &ClientEndpointId,
        id: &str,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
    ) -> (bool, Vec<ClientShellAction>) {
        let Err(error) = result else {
            return (false, Vec::new());
        };
        let Some(panels) = self.panel_endpoints.get_mut(endpoint_id) else {
            return (false, Vec::new());
        };
        match error.code.as_deref() {
            Some("panel_not_found") => {
                panels.sent.remove(id);
                let changed = panels.unknown.insert(id.to_owned());
                (changed, Vec::new())
            }
            _ => {
                // Inactive surface, timeout or cancellation: send the declaration again on the
                // next compose.
                panels.sent.remove(id);
                (false, Vec::new())
            }
        }
    }

    /// Blit the cached frame of every running panel into its sidebar body.
    pub(super) fn blit_panel_frames(&self, frame: &mut FrameData) {
        let Some(panels) = self.panel_endpoints.get(&self.active_endpoint_id) else {
            return;
        };
        for hit in &self.hits.panel_sections {
            if hit.state != PanelBodyState::Frame || hit.body.is_empty() {
                continue;
            }
            let Some(surface) = panels.frames.get(&hit.id) else {
                continue;
            };
            super::blit_pane_surface(frame, &surface.frame, hit.body);
        }
    }
}

impl ClientShellState {
    /// The `focus_width` of the focused panel's section, when one is focused and sets it.
    pub(super) fn focused_panel_focus_width(&self) -> Option<u16> {
        let focused = self.focused_panel.as_deref()?;
        self.config
            .sidebar_sections
            .iter()
            .find_map(|section| match section {
                SidebarSectionConfig::Panel(panel) if panel.id == focused => panel.focus_width,
                _ => None,
            })
    }

    fn sidebar_layout_width(&self) -> Option<u16> {
        self.last_composed_size
            .map(|(cols, rows)| self.layout(cols, rows).sidebar.width)
    }

    /// Move panel focus. A resize goes out only when the effective sidebar width really changes.
    pub(super) fn set_focused_panel(
        &mut self,
        next: Option<String>,
        outcome: &mut ClientShellInput,
    ) {
        if self.focused_panel == next {
            return;
        }
        let width_before = self.sidebar_layout_width();
        self.focused_panel = next;
        outcome.repaint = true;
        if let Some(id) = self.focused_panel.clone() {
            // Last writer wins (D9): a focusing client declares its size again.
            if let Some(panels) = self.panel_endpoints.get_mut(&self.active_endpoint_id) {
                panels.sent.remove(&id);
            }
        }
        if self.sidebar_layout_width() != width_before {
            self.invalidate_pane_surface();
            outcome.resize = true;
        }
    }

    /// Focus left to a section that is no longer drawn (collapsed, mobile, removed, other server).
    pub(super) fn drop_focus_of_hidden_panel(&mut self) {
        let Some(focused) = self.focused_panel.as_deref() else {
            return;
        };
        let visible = self
            .hits
            .panel_sections
            .iter()
            .any(|hit| hit.id == focused && !hit.body.is_empty());
        if !visible {
            self.focused_panel = None;
        }
    }

    /// `focus_panel`: first focusable panel, then the next one, wrapping.
    pub(super) fn cycle_panel_focus(&mut self, outcome: &mut ClientShellInput) {
        let focusable = self
            .hits
            .panel_sections
            .iter()
            .filter(|hit| hit.state == PanelBodyState::Frame && !hit.body.is_empty())
            .map(|hit| hit.id.clone())
            .collect::<Vec<_>>();
        if focusable.is_empty() {
            return;
        }
        let next = self
            .focused_panel
            .as_ref()
            .and_then(|current| focusable.iter().position(|id| id == current))
            .map_or(0, |index| (index + 1) % focusable.len());
        self.set_focused_panel(Some(focusable[next].clone()), outcome);
    }

    /// The panel body as a pane-shaped hit, so pane mouse translation (cells, SGR pixels) is shared.
    pub(super) fn panel_pane_hit(&self, id: &str) -> Option<PaneHit> {
        let hit = self.hits.panel_sections.iter().find(|hit| hit.id == id)?;
        let frame = self
            .panel_endpoints
            .get(&self.active_endpoint_id)
            .and_then(|panels| panels.frames.get(id));
        Some(PaneHit {
            rect: hit.body,
            inner_rect: hit.body,
            scrollbar_rect: None,
            scroll: None,
            pane_id: hit.id.clone(),
            popup: false,
            panel: true,
            mouse_reporting: frame.is_some_and(|frame| frame.mouse_reporting),
            sgr_pixel_mouse: frame.is_some_and(|frame| frame.sgr_pixel_mouse),
            pixel_width: frame.map_or(0, |frame| frame.pixel_width),
            pixel_height: frame.map_or(0, |frame| frame.pixel_height),
        })
    }

    pub(super) fn restart_panel(&mut self, id: &str, outcome: &mut ClientShellInput) {
        self.push_endpoint_method_with_kind(
            crate::api::schema::Method::PanelRestart(crate::api::schema::PanelRestartParams {
                id: id.to_owned(),
            }),
            PendingEndpointKind::Generic,
            outcome,
        );
    }

    /// Turn the semantic input collected for panels into `panel.input` requests.
    pub(super) fn flush_panel_input(&mut self, outcome: &mut ClientShellInput) {
        for (id, events) in std::mem::take(&mut outcome.panel_input) {
            let events = events
                .iter()
                .filter_map(|event| match serde_json::to_value(event) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        tracing::warn!(%error, "failed to encode panel input event");
                        None
                    }
                })
                .collect::<Vec<_>>();
            if events.is_empty() {
                continue;
            }
            self.push_endpoint_method_with_kind(
                crate::api::schema::Method::PanelInput(crate::api::schema::PanelInputParams {
                    id,
                    events,
                }),
                PendingEndpointKind::PanelInput,
                outcome,
            );
        }
    }

    /// Mouse handling for the sidebar panel bodies. Returns true when the event was consumed.
    pub(super) fn handle_panel_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        outcome: &mut ClientShellInput,
    ) -> bool {
        use crossterm::event::{MouseButton, MouseEventKind};
        let point = (mouse.column, mouse.row);
        let pressed = matches!(mouse.kind, MouseEventKind::Down(_));
        let Some(hit) = self
            .hits
            .panel_sections
            .iter()
            .find(|hit| super::contains(hit.body, point))
            .cloned()
        else {
            // A press anywhere else hands focus back and behaves as before.
            if pressed {
                self.set_focused_panel(None, outcome);
            }
            return false;
        };
        if hit.state != PanelBodyState::Frame {
            if pressed {
                self.set_focused_panel(None, outcome);
            }
            if hit.state == PanelBodyState::Exited
                && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            {
                self.restart_panel(&hit.id, outcome);
            }
            return true;
        }
        let Some(pane_hit) = self.panel_pane_hit(&hit.id) else {
            return true;
        };
        match mouse.kind {
            MouseEventKind::Down(button) => {
                self.set_focused_panel(Some(hit.id.clone()), outcome);
                if pane_hit.mouse_reporting {
                    self.push_pane_mouse_event(&pane_hit, mouse, mouse.modifiers, outcome);
                    self.pane_mouse_gesture = Some(ClientPaneMouseGesture {
                        last_position: self.pane_mouse_position(&pane_hit, mouse),
                        hit: pane_hit,
                        button,
                        stripped_modifiers: crossterm::event::KeyModifiers::empty(),
                        last_event: mouse,
                    });
                }
            }
            MouseEventKind::Moved
            | MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight
                if pane_hit.mouse_reporting =>
            {
                self.push_pane_mouse_event(&pane_hit, mouse, mouse.modifiers, outcome);
            }
            // Without mouse reporting there is nothing to scroll or drag; swallow the event.
            _ => {}
        }
        true
    }
}

fn panel_surface_set_method(
    id: String,
    cols: u16,
    rows: u16,
    visible: bool,
) -> crate::api::schema::Method {
    crate::api::schema::Method::PanelSurfaceSet(crate::api::schema::PanelSurfaceSetParams {
        id,
        cols,
        rows,
        visible,
    })
}

/// Greedy word wrap by display width; an over-long word gets a line of its own.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::<String>::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line)
                if UnicodeWidthStr::width(line.as_str()) + 1 + UnicodeWidthStr::width(word)
                    <= width =>
            {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
}
