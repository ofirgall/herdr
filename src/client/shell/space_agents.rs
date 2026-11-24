//! Agents nested under their space in the expanded sidebar.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use super::*;
use crate::protocol::{ClientShellAgent, ClientShellPane, ClientShellTab};

/// Columns a child is indented relative to its space's first row: the
/// `├─`/`└─` connector under the space's status icon, then a space.
const CHILD_INDENT: u16 = 3;
/// Column of the worktree group rail.
const TREE_RAIL_COLUMN: u16 = 1;
/// Columns occupied by the worktree tree marker on child rows.
pub(super) const TREE_MARKER_WIDTH: u16 = 2;

/// One tree's connector glyphs fitted to their columns: `branch` and `last`
/// fill two columns and `rail` one, so any configured glyph keeps the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TreeGlyphs {
    pub(super) branch: String,
    pub(super) last: String,
    pub(super) rail: String,
}

impl Default for TreeGlyphs {
    fn default() -> Self {
        Self::from_config(&crate::config::TreeGlyphsConfig::default())
    }
}

impl TreeGlyphs {
    pub(super) fn from_config(config: &crate::config::TreeGlyphsConfig) -> Self {
        Self {
            branch: fit_columns(&config.branch, 2),
            last: fit_columns(&config.last, 2),
            rail: fit_columns(&config.rail, 1),
        }
    }

    /// The connector of a child's first row.
    pub(super) fn connector(&self, last: bool) -> &str {
        if last {
            &self.last
        } else {
            &self.branch
        }
    }
}

/// `text` cut or padded with spaces to exactly `width` display columns.
fn fit_columns(text: &str, width: usize) -> String {
    let mut fitted = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        fitted.push(ch);
        used += ch_width;
    }
    fitted.extend(std::iter::repeat_n(' ', width - used));
    fitted
}

/// One agent drawn as a child entry of its space.
pub(super) struct SpaceAgentChild {
    pub(super) pane_id: String,
    pub(super) status: crate::api::schema::AgentStatus,
    pub(super) focused: bool,
    pub(super) rows: Vec<Vec<crate::ui::ResolvedToken>>,
}

impl SpaceAgentChild {
    pub(super) fn height(&self) -> u16 {
        self.rows.len().max(1).min(u16::MAX as usize) as u16
    }
}

/// Per-frame lookup tables, built once so each space does not rescan the snapshot.
pub(super) struct SpaceAgentIndex<'a> {
    by_workspace: HashMap<&'a str, Vec<&'a ClientShellAgent>>,
    /// Tabs by id with their one-based position among their space's tabs.
    tabs: HashMap<&'a str, (&'a ClientShellTab, usize)>,
    panes: HashMap<&'a str, &'a ClientShellPane>,
}

impl<'a> SpaceAgentIndex<'a> {
    /// Groups agents by workspace, ordered by tab number and then by snapshot
    /// order. The agent-view order is deliberately ignored.
    pub(super) fn new(snapshot: &'a ClientShellSnapshot) -> Self {
        let mut positions = HashMap::<&str, usize>::new();
        let tabs = snapshot
            .tabs
            .iter()
            .map(|tab| {
                let position = positions.entry(tab.workspace_id.as_str()).or_insert(0);
                *position += 1;
                (tab.tab_id.as_str(), (tab, *position))
            })
            .collect::<HashMap<_, _>>();
        let panes = snapshot
            .panes
            .iter()
            .map(|pane| (pane.pane_id.as_str(), pane))
            .collect::<HashMap<_, _>>();
        let mut by_workspace = HashMap::<&str, Vec<&ClientShellAgent>>::new();
        for agent in &snapshot.agents {
            by_workspace
                .entry(agent.workspace_id.as_str())
                .or_default()
                .push(agent);
        }
        for agents in by_workspace.values_mut() {
            // Stable sort keeps snapshot order within a tab.
            agents.sort_by_key(|agent| {
                tabs.get(agent.tab_id.as_str())
                    .map_or(usize::MAX, |(tab, _)| tab.number)
            });
        }
        Self {
            by_workspace,
            tabs,
            panes,
        }
    }

    pub(super) fn agents(&self, workspace_id: &str) -> &[&'a ClientShellAgent] {
        self.by_workspace
            .get(workspace_id)
            .map_or(&[], Vec::as_slice)
    }

    /// Resolves the child entries of one space from `agent_rows`. The state
    /// icon and text reflect each agent's own status.
    pub(super) fn children(
        &self,
        workspace: &ClientShellWorkspace,
        config: &ClientShellConfig,
    ) -> Vec<SpaceAgentChild> {
        self.agents(&workspace.workspace_id)
            .iter()
            .enumerate()
            .map(|(index, agent)| {
                let tab = self.tabs.get(agent.tab_id.as_str());
                let rows = super::agent_sidebar::resolve_agent_rows(
                    &config.spaces.agent_rows,
                    agent,
                    &workspace.label,
                    tab.map(|(tab, _)| tab.label.as_str()),
                    tab.map(|(_, position)| *position),
                    self.panes
                        .get(agent.pane_id.as_str())
                        .and_then(|pane| pane.label.as_deref()),
                    None,
                    index,
                );
                let rows = if rows.is_empty() {
                    vec![vec![crate::ui::ResolvedToken {
                        kind: crate::ui::ResolvedTokenKind::StateIcon,
                        style: Default::default(),
                        link: None,
                    }]]
                } else {
                    rows
                };
                SpaceAgentChild {
                    pane_id: agent.pane_id.clone(),
                    status: agent.agent_status,
                    focused: agent.focused,
                    rows,
                }
            })
            .collect()
    }
}

/// Draws the `▾`/`▸` toggle on the first row of a space. It sits one column
/// left of the worktree group toggle when both are present.
pub(super) fn render_agent_toggle(
    buffer: &mut Buffer,
    space_rect: Rect,
    group_toggle_present: bool,
    expanded: bool,
    palette: &Palette,
) -> Rect {
    let offset = 1 + u16::from(group_toggle_present);
    let toggle = Rect::new(
        space_rect.right().saturating_sub(offset),
        space_rect.y,
        1,
        1,
    );
    super::render::put_text(
        buffer,
        toggle.x,
        toggle.y,
        toggle.width,
        if expanded { "▾" } else { "▸" },
        Style::default().fg(palette.accent),
    );
    toggle
}

/// Draws the children of one space inside `area` (the rows directly below the
/// space) and records one hit per child. `space_indent` is the column where the
/// space's first-row content starts. A focused child's highlight starts at its
/// own `├─`/`└─` connector, leaving the outer worktree rail on the plain sidebar.
/// `tree_rail` continues the group's `│` connector through the children of a
/// space that has worktree children below it, so the tree stays unbroken.
/// Each child hangs off a `├─`/`└─` connector under the space's status icon;
/// a top-level space shares that column with its worktree rail, so its last
/// agent stays `├─` while worktrees follow.
pub(super) fn render_children(
    buffer: &mut Buffer,
    area: Rect,
    space_indent: u16,
    tree_rail: bool,
    children: &[SpaceAgentChild],
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    let tree_style = Style::default().fg(palette.overlay0);
    // The trunk runs past the last agent when the worktree rail shares its column.
    let trunk_continues = tree_rail && space_indent == TREE_RAIL_COLUMN;
    let mut y = area.y;
    for (child_index, child) in children.iter().enumerate() {
        if y >= area.bottom() {
            break;
        }
        let height = child.height().min(area.bottom() - y);
        let rect = Rect::new(area.x, y, area.width, height);
        hits.space_agents.push((rect, child.pane_id.clone()));
        if child.focused {
            let start = space_indent.min(area.width);
            buffer.set_style(
                Rect::new(area.x + start, y, area.width - start, height),
                Style::default().bg(palette.active_row_bg),
            );
        }
        if tree_rail {
            for row in 0..height {
                super::render::put_text(
                    buffer,
                    area.x,
                    y + row,
                    area.width.min(TREE_MARKER_WIDTH),
                    &format!(" {}", config.tree_worktrees.rail),
                    tree_style,
                );
            }
        }
        let last = child_index + 1 == children.len() && !trunk_continues;
        if space_indent < area.width {
            for row in 0..height {
                // On the shared trunk, continuation rows keep the worktree rail.
                let rail = if trunk_continues {
                    &config.tree_worktrees.rail
                } else {
                    &config.tree_agents.rail
                };
                let connector = match (row, last) {
                    (0, _) => config.tree_agents.connector(last),
                    (_, false) => rail.as_str(),
                    (_, true) => continue,
                };
                super::render::put_text(
                    buffer,
                    area.x + space_indent,
                    y + row,
                    (area.width - space_indent).min(2),
                    connector,
                    tree_style,
                );
            }
        }
        let status_style = Style::default().fg(status_color(child.status, palette));
        let name_style = if child.focused {
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.subtext0)
        };
        let secondary = Style::default().fg(palette.overlay0);
        let icon = (
            status_icon(child.status, &config.status_indicators),
            status_style,
        );
        for (index, tokens) in child.rows.iter().take(height as usize).enumerate() {
            let indent = space_indent + CHILD_INDENT + if index == 0 { 0 } else { 2 };
            let width = area.width.saturating_sub(indent);
            if width == 0 {
                continue;
            }
            let mut spans = Vec::<Span<'static>>::new();
            spans.extend(super::agent_sidebar::token_row_spans(
                tokens,
                icon,
                status_style,
                name_style,
                secondary,
                secondary,
                palette,
                width as usize,
                (!config.token_link_click.is_off()).then(|| super::agent_sidebar::LinkSink {
                    hits: &mut hits.token_links,
                    x: area.x + indent,
                    y: y + index as u16,
                    underline: config.token_link_underline,
                }),
            ));
            Paragraph::new(Line::from(spans)).render(
                Rect::new(area.x + indent, y + index as u16, width, 1),
                buffer,
            );
        }
        y += height;
    }
}

impl ClientShellState {
    /// Whether `workspace_id`'s agent children are shown on `endpoint_id`.
    pub(super) fn space_agents_expanded(
        &self,
        endpoint_id: &ClientEndpointId,
        workspace_id: &str,
    ) -> bool {
        self.config.spaces.agents.expanded(
            self.space_agent_overrides
                .get(endpoint_id)
                .and_then(|overrides| overrides.get(workspace_id))
                .copied(),
        )
    }

    /// Flips only this space between expanded and collapsed.
    pub(super) fn toggle_space_agents(
        &mut self,
        endpoint_id: &ClientEndpointId,
        workspace_id: &str,
    ) {
        let expanded = self.space_agents_expanded(endpoint_id, workspace_id);
        self.space_agent_overrides
            .entry(endpoint_id.clone())
            .or_default()
            .insert(workspace_id.to_owned(), !expanded);
    }

    /// Whether `workspace_id` has agents that the toggle could show or hide.
    pub(super) fn workspace_has_agents(&self, workspace_id: &str) -> bool {
        self.snapshot.as_deref().is_some_and(|snapshot| {
            snapshot
                .agents
                .iter()
                .any(|agent| agent.workspace_id == workspace_id)
        })
    }

    /// Overrides to persist, dropping workspaces that no longer exist on
    /// endpoints whose snapshot is known.
    pub(super) fn space_agent_overrides_for_preferences(
        &self,
    ) -> Vec<preferences::ClientSpaceAgentOverride> {
        let mut saved = Vec::new();
        for (endpoint_id, overrides) in &self.space_agent_overrides {
            let profile_id = match endpoint_id {
                ClientEndpointId::Local => None,
                ClientEndpointId::Ssh(profile_id) => Some(profile_id.to_string()),
            };
            let snapshot = if *endpoint_id == self.active_endpoint_id {
                self.snapshot.as_deref()
            } else {
                self.endpoints
                    .iter()
                    .find(|endpoint| endpoint.endpoint_id == *endpoint_id)
                    .and_then(|endpoint| endpoint.snapshot.as_deref())
            };
            for (workspace_id, expanded) in overrides {
                let exists = snapshot.is_none_or(|snapshot| {
                    snapshot
                        .workspaces
                        .iter()
                        .any(|workspace| workspace.workspace_id == *workspace_id)
                });
                if exists {
                    saved.push(preferences::ClientSpaceAgentOverride {
                        profile_id: profile_id.clone(),
                        workspace_id: workspace_id.clone(),
                        expanded: *expanded,
                    });
                }
            }
        }
        saved.sort_by(|left, right| {
            (&left.profile_id, &left.workspace_id).cmp(&(&right.profile_id, &right.workspace_id))
        });
        saved
    }
}
