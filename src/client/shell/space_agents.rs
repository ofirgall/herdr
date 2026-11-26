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

/// Sidebar presentation state of subagent rows for one endpoint: panes whose
/// subagent rows are expanded and panes showing history instead of the current
/// list. Never persisted or sent to the server.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct SubagentViewState {
    pub(super) expanded: HashSet<String>,
    pub(super) history: HashSet<String>,
}

impl SubagentViewState {
    pub(super) fn is_empty(&self) -> bool {
        self.expanded.is_empty() && self.history.is_empty()
    }

    fn toggle(set: &mut HashSet<String>, pane_id: &str) {
        if !set.remove(pane_id) {
            set.insert(pane_id.to_owned());
        }
    }

    /// Drops panes that no longer have an agent in `snapshot`.
    pub(super) fn prune(&mut self, snapshot: &ClientShellSnapshot) {
        let present = |pane_id: &String| {
            snapshot
                .agents
                .iter()
                .any(|agent| agent.pane_id == *pane_id)
        };
        self.expanded.retain(present);
        self.history.retain(present);
    }
}

/// Deepest nesting level that still indents; deeper rows share its column.
const SUBAGENT_MAX_DEPTH: usize = 6;

/// One subagent row beneath an agent child.
pub(super) struct SubagentRow<'a> {
    pub(super) subagent: &'a crate::api::schema::PaneSubagentInfo,
    pub(super) depth: usize,
    /// Bit `n` is set when the ancestor at nesting level `n` has later siblings,
    /// so its tree rail continues through this row.
    pub(super) rails: u32,
    pub(super) last: bool,
}

/// The subagent part of an agent child with subagents.
pub(super) struct SubagentSection<'a> {
    /// Running subagents in the current list.
    pub(super) running: usize,
    pub(super) expanded: bool,
    pub(super) history_mode: bool,
    /// Rows of the active list in tree order; empty unless `expanded`.
    pub(super) rows: Vec<SubagentRow<'a>>,
}

impl<'a> SubagentSection<'a> {
    fn new(agent: &'a ClientShellAgent, view: Option<&SubagentViewState>) -> Option<Self> {
        if agent.subagent_history.is_empty() && agent.subagents.is_empty() {
            return None;
        }
        let expanded = view.is_some_and(|view| view.expanded.contains(&agent.pane_id));
        let history_mode = view.is_some_and(|view| view.history.contains(&agent.pane_id));
        let running = agent
            .subagents
            .iter()
            .filter(|subagent| subagent.state == crate::api::schema::SubagentStatus::Running)
            .count();
        let rows = if expanded {
            subagent_tree_rows(if history_mode {
                &agent.subagent_history
            } else {
                &agent.subagents
            })
        } else {
            Vec::new()
        };
        Some(Self {
            running,
            expanded,
            history_mode,
            rows,
        })
    }

    /// Rows drawn beneath the agent's own rows: the mode row and the list.
    fn extra_height(&self) -> usize {
        if self.expanded {
            1 + self.rows.len()
        } else {
            0
        }
    }
}

/// `list` ordered so each subagent follows its parent (siblings keep list
/// order). Subagents whose parent is not in the list, or whose parent chain
/// loops, attach at the root.
fn subagent_tree_rows(list: &[crate::api::schema::PaneSubagentInfo]) -> Vec<SubagentRow<'_>> {
    let by_id = list
        .iter()
        .enumerate()
        .map(|(index, subagent)| (subagent.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let parent_of = |index: usize| {
        list[index]
            .parent_id
            .as_deref()
            .and_then(|parent| by_id.get(parent).copied())
    };
    // Walking up must reach a root within `list.len()` steps, or it loops.
    let attached = |index: usize| {
        let mut current = index;
        for _ in 0..=list.len() {
            match parent_of(current) {
                Some(parent) => current = parent,
                None => return true,
            }
        }
        false
    };
    let mut children = vec![Vec::<usize>::new(); list.len()];
    let mut roots = Vec::new();
    for index in 0..list.len() {
        match parent_of(index).filter(|_| attached(index)) {
            Some(parent) => children[parent].push(index),
            None => roots.push(index),
        }
    }
    let mut rows = Vec::with_capacity(list.len());
    // (index, depth, rails, last)
    let mut stack = Vec::<(usize, usize, u32, bool)>::new();
    let push_siblings = |stack: &mut Vec<(usize, usize, u32, bool)>,
                         siblings: &[usize],
                         depth: usize,
                         rails: u32| {
        for (position, index) in siblings.iter().enumerate().rev() {
            stack.push((*index, depth, rails, position + 1 == siblings.len()));
        }
    };
    push_siblings(&mut stack, &roots, 0, 0);
    while let Some((index, depth, rails, last)) = stack.pop() {
        rows.push(SubagentRow {
            subagent: &list[index],
            depth,
            rails,
            last,
        });
        let level = depth.min(SUBAGENT_MAX_DEPTH);
        let rails = if last { rails } else { rails | (1 << level) };
        push_siblings(&mut stack, &children[index], depth + 1, rails);
    }
    rows
}

/// One agent drawn as a child entry of its space.
pub(super) struct SpaceAgentChild<'a> {
    pub(super) pane_id: String,
    pub(super) status: crate::api::schema::AgentStatus,
    pub(super) focused: bool,
    pub(super) rows: Vec<Vec<crate::ui::ResolvedToken>>,
    pub(super) subagents: Option<SubagentSection<'a>>,
}

impl SpaceAgentChild<'_> {
    /// Rows of the agent's own tokens.
    fn token_height(&self) -> u16 {
        self.rows.len().max(1).min(u16::MAX as usize) as u16
    }

    pub(super) fn height(&self) -> u16 {
        let extra = self
            .subagents
            .as_ref()
            .map_or(0, SubagentSection::extra_height);
        (usize::from(self.token_height()) + extra).min(u16::MAX as usize) as u16
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
    /// Groups agents by workspace, ordered by tab position (the tab bar order,
    /// which moving a tab changes) and then by snapshot order. The agent-view
    /// order is deliberately ignored.
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
                    .map_or(usize::MAX, |(_, position)| *position)
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
    /// icon and text reflect each agent's own status. `first_index` is the
    /// zero-based `agent_index` of the first child: 0 numbers per space, the
    /// count of children before this space numbers down the whole tree.
    pub(super) fn children(
        &self,
        workspace: &ClientShellWorkspace,
        config: &ClientShellConfig,
        first_index: usize,
        subagent_view: Option<&SubagentViewState>,
    ) -> Vec<SpaceAgentChild<'a>> {
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
                    first_index + index,
                    agent.focused,
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
                    subagents: SubagentSection::new(agent, subagent_view),
                }
            })
            .collect()
    }
}

/// Pane ids of the agents shown as children down the space list, in display
/// order: `entries` in order, and for each space whose children are
/// `expanded`, its agents from `index`. This is the one source of the tree
/// numbering: rendering numbers children by position in it and the agent keys
/// target by position in it. Agent-view filters and the agents-section sort
/// do not apply.
///
/// `entries` must be the collapse-aware desktop entries (collapsed worktree
/// groups already dropped), also on mobile, whose own entries show no children.
pub(super) fn tree_agent_order<'a>(
    snapshot: &ClientShellSnapshot,
    entries: &[WorkspaceEntry],
    expanded: impl Fn(&str) -> bool,
    index: &SpaceAgentIndex<'a>,
) -> Vec<&'a str> {
    entries
        .iter()
        .filter_map(|entry| snapshot.workspaces.get(entry.index))
        .filter(|workspace| expanded(&workspace.workspace_id))
        .flat_map(|workspace| index.agents(&workspace.workspace_id))
        .map(|agent| agent.pane_id.as_str())
        .collect()
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

/// Moves the tab of agent `pane_id` past the nearest tab in the `down`
/// direction whose agents are listed in `entries`, within the agent's space.
fn move_agent_tab_method(
    snapshot: &ClientShellSnapshot,
    entries: &[SidebarEntry],
    workspace_id: &str,
    pane_id: &str,
    down: bool,
) -> Option<crate::api::schema::Method> {
    use crate::api::schema::{Method, TabMoveParams};
    let tab_of = |pane_id: &str| {
        snapshot
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane_id)
            .map(|agent| agent.tab_id.as_str())
    };
    let source_tab = tab_of(pane_id)?;
    let tabs = snapshot
        .tabs
        .iter()
        .filter(|tab| tab.workspace_id == workspace_id)
        .map(|tab| tab.tab_id.as_str())
        .collect::<Vec<_>>();
    let source = tabs.iter().position(|tab| *tab == source_tab)?;
    let shows_agent = |tab: &str| {
        tab != source_tab
            && entries.iter().any(|entry| {
                matches!(entry, SidebarEntry::Agent { workspace_id: space, pane_id }
                    if space == workspace_id && tab_of(pane_id) == Some(tab))
            })
    };
    // `insert_index` counts positions before the source tab is removed.
    let insert_index = if down {
        source + 2 + tabs[source + 1..].iter().position(|tab| shows_agent(tab))?
    } else {
        tabs[..source].iter().rposition(|tab| shows_agent(tab))?
    };
    Some(Method::TabMove(TabMoveParams {
        tab_id: source_tab.to_owned(),
        insert_index,
    }))
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
        // Subagent rows share the agent's hit, so clicking them focuses its pane.
        hits.space_agents.push((rect, child.pane_id.clone()));
        let token_height = child.token_height().min(height);
        if child.focused {
            let start = space_indent.min(area.width);
            buffer.set_style(
                Rect::new(area.x + start, y, area.width - start, token_height),
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
        // Tab tokens on the focused child use `tab_focused`.
        let focused_palette = child.focused.then(|| palette.with_focused_tab()).flatten();
        let token_palette = focused_palette.as_ref().unwrap_or(palette);
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
        // The chevron (and running count) take the right edge of the first row;
        // tokens truncate before it.
        let chevron = child.subagents.as_ref().and_then(|section| {
            let glyph = if section.expanded { "▾" } else { "▸" };
            let count = (section.running > 0).then(|| section.running.to_string());
            let width = count
                .as_ref()
                .map_or(0, |count| super::render::display_width(count) + 1)
                + 1;
            let first_indent = space_indent + CHILD_INDENT;
            (width < area.width.saturating_sub(first_indent)).then_some((count, glyph, width))
        });
        let reserved = chevron.as_ref().map_or(0, |(_, _, width)| width + 1);
        if let Some((count, glyph, width)) = &chevron {
            let mut x = area.x + area.width - width;
            hits.subagent_toggles
                .push((Rect::new(x, y, *width, 1), child.pane_id.clone()));
            if let Some(count) = count {
                let count_width = super::render::display_width(count);
                super::render::put_text(
                    buffer,
                    x,
                    y,
                    count_width,
                    count,
                    Style::default().fg(status_color(
                        crate::api::schema::AgentStatus::Working,
                        palette,
                    )),
                );
                x += count_width + 1;
            }
            super::render::put_text(buffer, x, y, 1, glyph, Style::default().fg(palette.accent));
        }
        for (index, tokens) in child.rows.iter().take(token_height as usize).enumerate() {
            let indent = space_indent + CHILD_INDENT + if index == 0 { 0 } else { 2 };
            let width = area.width.saturating_sub(indent);
            let width = if index == 0 {
                width.saturating_sub(reserved)
            } else {
                width
            };
            if width == 0 {
                continue;
            }
            let mut spans = Vec::<Span<'static>>::new();
            spans.extend(super::agent_sidebar::token_row_spans(
                tokens,
                child.focused,
                icon,
                status_style,
                name_style,
                secondary,
                secondary,
                token_palette,
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
        if let Some(section) = child.subagents.as_ref().filter(|section| section.expanded) {
            render_subagent_rows(
                buffer,
                area,
                y + token_height,
                y + height,
                space_indent + CHILD_INDENT,
                child,
                section,
                config,
                hits,
            );
        }
        y += height;
    }
}

/// Columns from one subagent nesting level's connector to the next.
const SUBAGENT_LEVEL_WIDTH: u16 = 3;

/// Draws the mode row and the subagent rows of an expanded agent child on
/// rows `top..bottom`, with connectors starting at `base` (relative to `area`),
/// and records their hits.
#[allow(clippy::too_many_arguments)]
fn render_subagent_rows(
    buffer: &mut Buffer,
    area: Rect,
    top: u16,
    bottom: u16,
    base: u16,
    child: &SpaceAgentChild<'_>,
    section: &SubagentSection<'_>,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    let tree_style = Style::default().fg(palette.overlay0);
    if base >= area.width || top >= bottom {
        return;
    }
    let right = area.x + area.width;
    let connector_width = |x: u16| (right - x).min(2);
    let text_width = |x: u16| right.saturating_sub(x);

    // Mode row.
    let mode_connector = config.tree_agents.connector(section.rows.is_empty());
    let connector_x = area.x + base;
    super::render::put_text(
        buffer,
        connector_x,
        top,
        connector_width(connector_x),
        mode_connector,
        tree_style,
    );
    hits.subagent_modes.push((
        Rect::new(connector_x, top, right - connector_x, 1),
        child.pane_id.clone(),
    ));
    let active = Style::default()
        .fg(palette.accent)
        .add_modifier(Modifier::BOLD);
    let inactive = Style::default().fg(palette.overlay0);
    let (current_style, history_style) = if section.history_mode {
        (inactive, active)
    } else {
        (active, inactive)
    };
    let mut x = connector_x + SUBAGENT_LEVEL_WIDTH;
    for (text, style) in [
        ("current", current_style),
        (" · ", inactive),
        ("history", history_style),
    ] {
        let width = text_width(x);
        super::render::put_text(buffer, x, top, width, text, style);
        x = x.saturating_add(super::render::display_width(text));
    }

    for (index, row) in section.rows.iter().enumerate() {
        let y = top + 1 + index as u16;
        if y >= bottom {
            break;
        }
        let level = row.depth.min(SUBAGENT_MAX_DEPTH);
        for rail_level in 0..level {
            if row.rails & (1 << rail_level) != 0 {
                let x = connector_x + SUBAGENT_LEVEL_WIDTH * rail_level as u16;
                if x < right {
                    super::render::put_text(buffer, x, y, 1, &config.tree_agents.rail, tree_style);
                }
            }
        }
        let row_x = connector_x + SUBAGENT_LEVEL_WIDTH * level as u16;
        if row_x >= right {
            continue;
        }
        super::render::put_text(
            buffer,
            row_x,
            y,
            connector_width(row_x),
            config.tree_agents.connector(row.last),
            tree_style,
        );
        let content_x = row_x + SUBAGENT_LEVEL_WIDTH;
        let width = usize::from(text_width(content_x));
        if width == 0 {
            continue;
        }
        let (icon, icon_style) = subagent_icon(row.subagent.state, config);
        // The description leads because the sidebar is narrow and it tells
        // subagents apart; the kind follows dimmed when space remains.
        let (primary, secondary) = if row.subagent.description.is_empty() {
            (row.subagent.kind.as_str(), "")
        } else {
            (
                row.subagent.description.as_str(),
                row.subagent.kind.as_str(),
            )
        };
        let primary = crate::ui::truncate_end(primary, width.saturating_sub(2));
        let remaining = width
            .saturating_sub(2)
            .saturating_sub(usize::from(super::render::display_width(&primary)) + 1);
        let mut spans = vec![
            Span::styled(icon, icon_style),
            Span::raw(" "),
            Span::styled(primary, Style::default().fg(palette.subtext0)),
        ];
        if remaining > 0 && !secondary.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                crate::ui::truncate_end(secondary, remaining),
                Style::default().fg(palette.overlay0),
            ));
        }
        Paragraph::new(Line::from(spans))
            .render(Rect::new(content_x, y, text_width(content_x), 1), buffer);
    }
}

/// State icon and style of a subagent row: running and done reuse the agent
/// status styling, interrupted is a `!` in the warning color.
fn subagent_icon(
    state: crate::api::schema::SubagentStatus,
    config: &ClientShellConfig,
) -> (String, Style) {
    use crate::api::schema::{AgentStatus, SubagentStatus};
    let palette = &config.palette;
    let status = match state {
        SubagentStatus::Running => AgentStatus::Working,
        SubagentStatus::Done => AgentStatus::Done,
        SubagentStatus::Interrupted => {
            return ("!".to_owned(), Style::default().fg(palette.peach));
        }
    };
    (
        status_icon(status, &config.status_indicators).to_string(),
        Style::default().fg(status_color(status, palette)),
    )
}

/// One focusable row of the spaces sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SidebarEntry {
    Space(String),
    Agent {
        workspace_id: String,
        pane_id: String,
    },
}

/// Where the previous/next sidebar entry keybind last moved, and the focus it
/// moved from, so quick presses advance before the requested focus arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SidebarEntryCursor {
    entry: SidebarEntry,
    from_workspace: Option<String>,
    from_pane: Option<String>,
}

impl ClientShellState {
    /// The spaces sidebar's focusable rows in display order: each listed space,
    /// followed by the agents shown under it when they are expanded.
    pub(super) fn sidebar_entries(&self, snapshot: &ClientShellSnapshot) -> Vec<SidebarEntry> {
        // The mobile layout lists spaces without their agents.
        let agent_index = (self.config.spaces.agents != crate::config::SpaceAgentsMode::Off
            && !self.mobile_layout_active())
        .then(|| SpaceAgentIndex::new(snapshot));
        let mut entries = Vec::new();
        for entry in self.navigation_workspace_entries(snapshot) {
            let Some(workspace) = snapshot.workspaces.get(entry.index) else {
                continue;
            };
            let workspace_id = &workspace.workspace_id;
            entries.push(SidebarEntry::Space(workspace_id.clone()));
            let Some(index) = &agent_index else {
                continue;
            };
            if self.space_agents_expanded(&self.active_endpoint_id, workspace_id) {
                entries.extend(index.agents(workspace_id).iter().map(|agent| {
                    SidebarEntry::Agent {
                        workspace_id: workspace_id.clone(),
                        pane_id: agent.pane_id.clone(),
                    }
                }));
            }
        }
        entries
    }

    /// Position of the current row in `entries`: the cursor's entry while its
    /// focus request is in flight, otherwise the focused agent's row, then the
    /// focused space's row. A space whose focus landed on one of its shown
    /// agents is therefore on that agent's row, the row the sidebar highlights.
    fn current_sidebar_entry(
        &self,
        snapshot: &ClientShellSnapshot,
        entries: &[SidebarEntry],
    ) -> Option<usize> {
        let focused_workspace = snapshot.focused_workspace_id.as_deref();
        let focused_pane = snapshot.focused_pane_id.as_deref();
        if let Some(cursor) = &self.sidebar_entry_cursor {
            // Focus has not moved since the cursor's request was sent.
            let pending = cursor.from_workspace.as_deref() == focused_workspace
                && cursor.from_pane.as_deref() == focused_pane;
            if pending {
                if let Some(position) = entries.iter().position(|entry| *entry == cursor.entry) {
                    return Some(position);
                }
            }
        }
        entries
            .iter()
            .position(|entry| {
                matches!(entry, SidebarEntry::Agent { pane_id, .. }
                    if focused_pane == Some(pane_id.as_str()))
            })
            .or_else(|| {
                entries.iter().position(|entry| {
                    matches!(entry, SidebarEntry::Space(workspace_id)
                        if focused_workspace == Some(workspace_id.as_str()))
                })
            })
    }

    /// The request that moves the current sidebar entry one place `down` or up,
    /// without wrapping. A space passes its neighbouring space, as a drag
    /// would: a worktree group moves as one block, led by its root. An agent
    /// moves its tab past the nearest tab with a shown agent, so the agent
    /// trades places with that agent; agents sharing a tab move together.
    pub(super) fn move_sidebar_entry(&mut self, down: bool) -> Option<crate::api::schema::Method> {
        let snapshot = self.snapshot.as_deref()?;
        let entries = self.sidebar_entries(snapshot);
        let current = self.current_sidebar_entry(snapshot, &entries)?;
        match &entries[current] {
            SidebarEntry::Space(workspace_id) => {
                self.move_space_method(snapshot, workspace_id, down)
            }
            SidebarEntry::Agent {
                workspace_id,
                pane_id,
            } => move_agent_tab_method(snapshot, &entries, workspace_id, pane_id, down),
        }
    }

    fn move_space_method(
        &self,
        snapshot: &ClientShellSnapshot,
        workspace_id: &str,
        down: bool,
    ) -> Option<crate::api::schema::Method> {
        let is_linked = |workspace: &ClientShellWorkspace| {
            workspace
                .worktree
                .as_ref()
                .is_some_and(|worktree| worktree.is_linked_worktree)
        };
        let workspace = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == workspace_id)?;
        // A linked worktree moves its group, which its root leads.
        let root = if is_linked(workspace) {
            let key = &workspace.worktree.as_ref()?.key;
            snapshot.workspaces.iter().find(|candidate| {
                !is_linked(candidate)
                    && candidate
                        .worktree
                        .as_ref()
                        .is_some_and(|worktree| worktree.key == *key)
            })?
        } else {
            workspace
        };
        let roots = snapshot
            .workspaces
            .iter()
            .filter(|workspace| !is_linked(workspace))
            .collect::<Vec<_>>();
        let position = roots
            .iter()
            .position(|candidate| candidate.workspace_id == root.workspace_id)?;
        let before = if down {
            if position + 1 >= roots.len() {
                return None;
            }
            roots
                .get(position + 2)
                .map(|workspace| workspace.workspace_id.as_str())
        } else {
            Some(roots.get(position.checked_sub(1)?)?.workspace_id.as_str())
        };
        self.workspace_move_method(&root.workspace_id, before)
    }

    /// The focus request for the sidebar entry `delta` rows from the current
    /// one, wrapping around, and remembers it as the cursor.
    pub(super) fn step_sidebar_entry(
        &mut self,
        delta: isize,
    ) -> Option<crate::api::schema::Method> {
        use crate::api::schema::{Method, PaneTarget, WorkspaceTarget};
        let snapshot = self.snapshot.as_deref()?;
        let entries = self.sidebar_entries(snapshot);
        if entries.is_empty() {
            return None;
        }
        let len = entries.len() as isize;
        let next = match self.current_sidebar_entry(snapshot, &entries) {
            Some(current) => {
                // From one of a space's agents, that space is already focused,
                // so stepping onto its row would change nothing: pass it.
                let own_space = match &entries[current] {
                    SidebarEntry::Agent { workspace_id, .. } => Some(workspace_id),
                    SidebarEntry::Space(_) => None,
                };
                let mut next = current as isize;
                for _ in 0..len {
                    next = (next + delta).rem_euclid(len);
                    if !matches!((&entries[next as usize], own_space),
                        (SidebarEntry::Space(space), Some(own)) if space == own)
                    {
                        break;
                    }
                }
                next as usize
            }
            None if delta < 0 => entries.len() - 1,
            None => 0,
        };
        let entry = entries[next].clone();
        self.sidebar_entry_cursor = Some(SidebarEntryCursor {
            entry: entry.clone(),
            from_workspace: snapshot.focused_workspace_id.clone(),
            from_pane: snapshot.focused_pane_id.clone(),
        });
        Some(match entry {
            SidebarEntry::Space(workspace_id) => {
                self.reveal_workspace(&workspace_id);
                Method::WorkspaceFocus(WorkspaceTarget { workspace_id })
            }
            SidebarEntry::Agent {
                workspace_id,
                pane_id,
            } => {
                self.reveal_workspace(&workspace_id);
                Method::PaneFocus(PaneTarget { pane_id })
            }
        })
    }

    /// Scrolls the space list to the space holding `pane_id`'s child row, the
    /// way `step_sidebar_entry` does. A space and its children are one list
    /// item, so revealing the space reveals the child.
    pub(super) fn reveal_agent_space(&mut self, pane_id: &str) {
        let workspace_id = self.snapshot.as_deref().and_then(|snapshot| {
            snapshot
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane_id)
                .map(|agent| agent.workspace_id.clone())
        });
        if let Some(workspace_id) = workspace_id {
            self.reveal_workspace(&workspace_id);
        }
    }

    /// Whether the agent keys follow the space tree: tree numbering mode on
    /// the single-machine sidebar. The multi-machine sidebar (more than one
    /// endpoint) keeps its aggregate agent list.
    pub(super) fn tree_agent_keys_active(&self) -> bool {
        self.config.tree_agent_numbering() && self.endpoints.len() <= 1
    }

    /// Pane ids the agent keys target in tree numbering mode, in tree order.
    /// Always built from the desktop collapse-aware entries, also on mobile, so
    /// the keys agree with the numbers the desktop tree shows.
    pub(super) fn tree_agent_targets(&self, snapshot: &ClientShellSnapshot) -> Vec<String> {
        let empty = HashSet::new();
        let entries = super::render::workspace_entries(
            snapshot,
            self.collapsed_groups_for_endpoint(&self.active_endpoint_id)
                .unwrap_or(&empty),
        );
        let index = SpaceAgentIndex::new(snapshot);
        tree_agent_order(
            snapshot,
            &entries,
            |workspace_id| self.space_agents_expanded(&self.active_endpoint_id, workspace_id),
            &index,
        )
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

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

    /// Flips the subagent rows of `pane_id` between expanded and collapsed.
    pub(super) fn toggle_subagents_expanded(
        &mut self,
        endpoint_id: &ClientEndpointId,
        pane_id: &str,
    ) {
        let view = self.subagent_views.entry(endpoint_id.clone()).or_default();
        SubagentViewState::toggle(&mut view.expanded, pane_id);
    }

    /// Flips `pane_id` between its current and history subagent lists.
    pub(super) fn toggle_subagent_history(
        &mut self,
        endpoint_id: &ClientEndpointId,
        pane_id: &str,
    ) {
        let view = self.subagent_views.entry(endpoint_id.clone()).or_default();
        SubagentViewState::toggle(&mut view.history, pane_id);
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
