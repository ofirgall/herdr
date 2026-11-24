use super::*;
use ratatui::{
    text::Line,
    widgets::{Paragraph, Widget},
};

fn workspace_selection_background(palette: &Palette) -> ratatui::style::Color {
    if palette.selection_bg == ratatui::style::Color::Reset {
        palette.active_row_bg
    } else {
        palette.selection_bg
    }
}

pub(in crate::client::shell) fn workspace_active_background(
    palette: &Palette,
    navigating: bool,
) -> ratatui::style::Color {
    // The fallback cursor shares the active-row color; only fill the cursor while navigating.
    if navigating && palette.selection_bg == ratatui::style::Color::Reset {
        palette.sidebar_bg
    } else {
        palette.active_row_bg
    }
}

/// Collapsed sidebar geometry as `(spaces, divider_y, agents)`.
///
/// A disabled section gets `Rect::default()` and the divider is dropped. A lone
/// spaces section keeps the bottom row free for the expand toggle; a lone
/// agents section is returned whole (its renderer already reserves that row).
pub(in crate::client::shell) fn collapsed_sidebar_sections(
    area: Rect,
    spaces_enabled: bool,
    agents_enabled: bool,
) -> (Rect, Option<u16>, Rect) {
    let content = crate::ui::sidebar_content_rect(area);
    if content.is_empty() {
        return (Rect::default(), None, Rect::default());
    }
    match (spaces_enabled, agents_enabled) {
        (false, false) => return (Rect::default(), None, Rect::default()),
        (true, false) => {
            let height = content.height.saturating_sub(1);
            return (
                Rect::new(content.x, content.y, content.width, height),
                None,
                Rect::default(),
            );
        }
        (false, true) => return (Rect::default(), None, content),
        (true, true) => {}
    }
    if content.height < 7 {
        return (content, None, Rect::default());
    }
    let workspace_height = content.height.div_ceil(2);
    let divider_y = content.y + workspace_height;
    let detail_height = content.height.saturating_sub(workspace_height + 1);
    (
        Rect::new(content.x, content.y, content.width, workspace_height),
        Some(divider_y),
        Rect::new(content.x, divider_y + 1, content.width, detail_height),
    )
}

pub(crate) fn render_collapsed_sidebar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    selected_workspace_id: Option<&str>,
    collapsed_groups: &HashSet<String>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    // Jump numbers follow the navigation entries; the collapsed list draws
    // every space in snapshot order, so map them back by snapshot index.
    let mut jump_digits = vec![None; snapshot.workspaces.len()];
    let entries = workspace_entries(snapshot, collapsed_groups);
    for (entry, number) in entries.iter().zip(space_jump_numbers(&entries, snapshot)) {
        if let Some(slot) = jump_digits.get_mut(entry.index) {
            *slot = number.map(JumpNumber::digit);
        }
    }
    let selection_background = workspace_selection_background(palette);
    let active_background = workspace_active_background(palette, selected_workspace_id.is_some());
    render_sidebar_background(buffer, area, palette);
    let (workspace_area, divider_y, detail_area) =
        collapsed_sidebar_sections(area, config.spaces.enabled, config.agents.enabled);
    for (index, workspace) in snapshot
        .workspaces
        .iter()
        .take(workspace_area.height as usize)
        .enumerate()
    {
        let rect = Rect::new(
            workspace_area.x,
            workspace_area.y + index as u16,
            workspace_area.width,
            1,
        );
        let selected = selected_workspace_id == Some(workspace.workspace_id.as_str());
        if selected {
            buffer.set_style(rect, Style::default().bg(selection_background));
        } else if workspace.focused {
            buffer.set_style(rect, Style::default().bg(active_background));
        }
        let number_style = if selected {
            Style::default()
                .fg(palette.overlay1)
                .bg(selection_background)
        } else if workspace.focused {
            Style::default().fg(palette.text).bg(active_background)
        } else {
            Style::default().fg(palette.overlay0)
        };
        let number_style = palette
            .index_color(workspace.focused)
            .map_or(number_style, |color| number_style.fg(color));
        let number_style = if workspace.focused {
            number_style.add_modifier(Modifier::BOLD)
        } else {
            number_style
        };
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width.min(2),
            &format!("{:<2}", jump_digits[index].unwrap_or(' ')),
            number_style,
        );
        let status = workspace.agent_status;
        put_text(
            buffer,
            rect.x.saturating_add(2),
            rect.y,
            rect.width.saturating_sub(2),
            status_icon(status, &config.status_indicators),
            Style::default().fg(status_color(status, palette)),
        );
        hits.workspaces.push(WorkspaceHit {
            rect,
            endpoint_id: ClientEndpointId::Local,
            workspace_id: workspace.workspace_id.clone(),
            indented: false,
            group_toggle: None,
            agent_toggle: None,
            child_rows: 0,
        });
    }

    if let Some(divider_y) = divider_y {
        put_text(
            buffer,
            workspace_area.x,
            divider_y,
            workspace_area.width,
            &"─".repeat(workspace_area.width as usize),
            Style::default().fg(palette.surface_dim),
        );
    }

    let detail_content = Rect::new(
        detail_area.x,
        detail_area.y,
        detail_area.width,
        detail_area.height.saturating_sub(1),
    );
    for (index, pane_id) in super::ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
        .into_iter()
        .take(detail_content.height as usize)
        .enumerate()
    {
        let Some(agent) = snapshot
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane_id)
        else {
            continue;
        };
        let rect = Rect::new(
            detail_content.x,
            detail_content.y + index as u16,
            detail_content.width,
            1,
        );
        if agent.focused {
            buffer.set_style(rect, Style::default().bg(palette.active_row_bg));
        }
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width.min(2),
            &format!("{:<2}", index + 1),
            Style::default().fg(if agent.focused {
                palette.text
            } else {
                palette.overlay0
            }),
        );
        put_text(
            buffer,
            rect.x.saturating_add(2),
            rect.y,
            rect.width.saturating_sub(2),
            status_icon(agent.agent_status, &config.status_indicators),
            Style::default().fg(status_color(agent.agent_status, palette)),
        );
        hits.agents.push((rect, pane_id));
    }
    let toggle_content = crate::ui::sidebar_content_rect(area);
    hits.sidebar_toggle = if area.is_empty() || toggle_content.width == 0 {
        Rect::default()
    } else {
        Rect::new(
            toggle_content.x + toggle_content.width / 2,
            area.bottom().saturating_sub(1),
            1,
            1,
        )
    };
    put_text(
        buffer,
        hits.sidebar_toggle.x,
        hits.sidebar_toggle.y,
        hits.sidebar_toggle.width,
        "»",
        if super::super::global_menu::global_menu_attention(snapshot) {
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.overlay0)
        },
    );
}

pub(crate) fn render_sidebar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    state: &mut ShellRenderState<'_>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    render_sidebar_background(buffer, area, palette);
    hits.sidebar_divider = if area.is_empty() {
        Rect::default()
    } else {
        Rect::new(area.right().saturating_sub(1), area.y, 1, area.height)
    };
    let spaces_enabled = config.spaces.enabled;
    let agents_enabled = config.agents.enabled;
    let pull_requests_enabled = config.pull_requests.enabled;
    // Agents and pull requests share the lower section.
    let lower_enabled = agents_enabled || pull_requests_enabled;
    let (workspace_area, detail_area) = crate::ui::expanded_sidebar_sections(
        area,
        state.sidebar_section_split,
        spaces_enabled,
        lower_enabled,
    );
    hits.sidebar_section_divider = crate::ui::sidebar_section_divider_rect(
        area,
        state.sidebar_section_split,
        spaces_enabled,
        lower_enabled,
    );
    if spaces_enabled {
        render_spaces_section(buffer, workspace_area, snapshot, config, state, hits);
    }
    // The footer sits on the bottom row of the spaces section, or of the whole
    // sidebar content when only one section is shown. In the latter case that
    // row is shared with the `«` toggle, so the footer leaves two columns free.
    let content = crate::ui::sidebar_content_rect(area);
    let (footer_area, lower_area) = if spaces_enabled && lower_enabled {
        (workspace_area, detail_area)
    } else {
        let lower_area = Rect::new(
            detail_area.x,
            detail_area.y,
            detail_area.width,
            detail_area.height.saturating_sub(1),
        );
        (
            Rect::new(
                content.x,
                content.y,
                content.width.saturating_sub(2),
                content.height,
            ),
            lower_area,
        )
    };
    render_sidebar_footer(buffer, footer_area, snapshot, config, hits);
    let (agents_area, pull_requests_area) =
        split_lower_section(lower_area, agents_enabled, pull_requests_enabled);
    if agents_enabled {
        super::render_agent_panel(
            buffer,
            agents_area,
            snapshot,
            config,
            state.agent_scroll,
            spaces_enabled,
            hits,
        );
    }
    if pull_requests_enabled {
        super::pull_request_sidebar::render_pull_request_panel(
            buffer,
            pull_requests_area,
            snapshot,
            config,
            state.expanded_pull_requests,
            spaces_enabled || agents_enabled,
            hits,
        );
    }

    hits.sidebar_toggle = Rect::new(
        area.right().saturating_sub(2),
        area.bottom().saturating_sub(1),
        u16::from(area.width > 1),
        u16::from(area.height > 0),
    );
    put_text(
        buffer,
        hits.sidebar_toggle.x,
        hits.sidebar_toggle.y,
        hits.sidebar_toggle.width,
        "«",
        Style::default().fg(palette.overlay0),
    );
}

fn render_spaces_section(
    buffer: &mut Buffer,
    workspace_area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    state: &mut ShellRenderState<'_>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    put_text(
        buffer,
        workspace_area.x,
        workspace_area.y,
        workspace_area.width,
        " spaces",
        Style::default()
            .fg(palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );

    let entries = workspace_entries(snapshot, state.collapsed_groups);
    // One pass per frame, aligned with `entries`.
    let jump_numbers = space_jump_numbers(&entries, snapshot);
    // Children are a presentation of the spaces list: one entry per space whose
    // agents are expanded, resolved once per frame.
    let agent_mode = config.spaces.agents;
    let agent_index = (agent_mode != crate::config::SpaceAgentsMode::Off)
        .then(|| super::space_agents::SpaceAgentIndex::new(snapshot));
    let space_has_agents = |workspace_id: &str| {
        agent_index
            .as_ref()
            .is_some_and(|index| !index.agents(workspace_id).is_empty())
    };
    let space_expanded = |workspace_id: &str| {
        agent_mode.expanded(
            state
                .space_agent_overrides
                .and_then(|overrides| overrides.get(workspace_id))
                .copied(),
        )
    };
    // Tree numbering mode numbers children down the whole list: each space's
    // first child continues from the shown children before it.
    let tree_positions = agent_index
        .as_ref()
        .filter(|_| config.tree_agent_numbering())
        .map(|index| {
            super::space_agents::tree_agent_order(snapshot, &entries, space_expanded, index)
                .into_iter()
                .enumerate()
                .map(|(position, pane_id)| (pane_id, position))
                .collect::<HashMap<_, _>>()
        });
    let children = entries
        .iter()
        .map(|entry| {
            let workspace = snapshot.workspaces.get(entry.index)?;
            let index = agent_index.as_ref()?;
            let first_index = tree_positions.as_ref().map_or(0, |positions| {
                index
                    .agents(&workspace.workspace_id)
                    .first()
                    .and_then(|agent| positions.get(agent.pane_id.as_str()))
                    .copied()
                    .unwrap_or(0)
            });
            space_expanded(&workspace.workspace_id)
                .then(|| index.children(workspace, config, first_index))
                .filter(|children| !children.is_empty())
        })
        .collect::<Vec<_>>();
    let child_heights = children
        .iter()
        .map(|children| {
            children.as_ref().map_or(0, |children| {
                children
                    .iter()
                    .map(|child| usize::from(child.height()))
                    .sum::<usize>()
            })
        })
        .collect::<Vec<_>>();
    let body = Rect::new(
        workspace_area.x,
        workspace_area.y.saturating_add(WORKSPACE_HEADER_ROWS),
        workspace_area.width,
        workspace_area
            .height
            .saturating_sub(WORKSPACE_HEADER_ROWS + 1),
    );
    hits.workspace_body = body;
    let row_heights = entries
        .iter()
        .enumerate()
        .map(|(entry_position, entry)| {
            snapshot
                .workspaces
                .get(entry.index)
                .map(|workspace| {
                    workspace_rows(
                        workspace,
                        displayed_workspace_status(snapshot, workspace, state.collapsed_groups),
                        entry.indented,
                        jump_numbers[entry_position].map(JumpNumber::digit),
                        &config.spaces,
                    )
                    .len()
                    .max(1)
                    .saturating_add(child_heights[entry_position])
                    .min(u16::MAX as usize) as u16
                })
                .unwrap_or(1)
        })
        .collect::<Vec<_>>();
    let gaps = (0..entries.len())
        .map(|index| {
            space_gap_after(
                &entries[index],
                child_heights[index] > 0,
                entries.get(index + 1),
                &config.spaces,
            )
        })
        .collect::<Vec<_>>();
    let mut metrics = super::scroll::list_scroll_metrics(
        &row_heights,
        &gaps,
        body.height,
        *state.workspace_scroll,
    );
    if !body.is_empty() && std::mem::take(state.reveal_focused_workspace) {
        if let Some(target) = entries
            .iter()
            .position(|entry| snapshot.workspaces[entry.index].focused)
        {
            *state.workspace_scroll = super::scroll::list_scroll_start_to_reveal(
                &row_heights,
                &gaps,
                body.height,
                *state.workspace_scroll,
                target,
            );
            metrics = super::scroll::list_scroll_metrics(
                &row_heights,
                &gaps,
                body.height,
                *state.workspace_scroll,
            );
        }
    }
    hits.workspace_max_scroll = metrics.max_offset_from_bottom;
    hits.workspace_scroll_metrics = Some(metrics);
    *state.workspace_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    let mut y = body.y;
    for (entry_position, entry) in entries.iter().enumerate().skip(*state.workspace_scroll) {
        let Some(workspace) = snapshot.workspaces.get(entry.index) else {
            continue;
        };
        let status = displayed_workspace_status(snapshot, workspace, state.collapsed_groups);
        let rows = workspace_rows(
            workspace,
            status,
            entry.indented,
            jump_numbers[entry_position].map(JumpNumber::digit),
            &config.spaces,
        );
        let space_height = (rows.len().max(1).min(u16::MAX as usize) as u16).min(body.height);
        // A space and its children are one list item, so they are placed or
        // skipped together.
        let row_height = (rows
            .len()
            .max(1)
            .saturating_add(child_heights[entry_position]))
        .min(u16::MAX as usize)
        .min(usize::from(body.height)) as u16;
        if y.saturating_add(row_height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, space_height);
        let selected = state.selected_workspace_id.is_some_and(|target| {
            target.matches(state.active_endpoint_id, &workspace.workspace_id)
        });
        let dragged = state.dragged_workspace_id == Some(workspace.workspace_id.as_str());
        if selected {
            buffer.set_style(rect, Style::default().bg(palette.selection_bg));
        } else if dragged {
            buffer.set_style(rect, Style::default().bg(palette.surface1));
        } else if workspace.focused {
            buffer.set_style(rect, Style::default().bg(palette.active_row_bg));
        }
        render_workspace_rows(
            buffer,
            rect,
            status,
            &config.status_indicators,
            entry,
            rows,
            workspace.focused,
            selected,
            state.selected_workspace_id.is_some(),
            dragged,
            palette,
            &config.tree_worktrees,
            (!config.token_link_click.is_off()).then_some(&mut hits.token_links),
            config.token_link_underline,
        );
        let group_toggle = render_parent_group_toggle(
            buffer,
            rect,
            snapshot,
            entry.index,
            state.collapsed_groups,
            palette,
        );
        let agent_toggle = space_has_agents(&workspace.workspace_id).then(|| {
            super::space_agents::render_agent_toggle(
                buffer,
                rect,
                group_toggle.is_some(),
                space_expanded(&workspace.workspace_id),
                palette,
            )
        });
        let child_rows = row_height - space_height;
        // The group's `│` runs on while a worktree child follows this space.
        let tree_continues = entries
            .get(entry_position + 1)
            .is_some_and(|next| next.indented);
        if let Some(children) = children[entry_position].as_deref() {
            super::space_agents::render_children(
                buffer,
                Rect::new(body.x, y + space_height, content_width, child_rows),
                if entry.indented { SUBSPACE_INDENT } else { 1 },
                tree_continues,
                children,
                config,
                hits,
            );
        }
        hits.workspaces.push(WorkspaceHit {
            rect,
            endpoint_id: ClientEndpointId::Local,
            workspace_id: workspace.workspace_id.clone(),
            indented: entry.indented,
            group_toggle,
            agent_toggle,
            child_rows,
        });
        let gap = gaps[entry_position];
        if tree_continues {
            for row in 0..gap {
                let gap_y = y.saturating_add(row_height + row);
                if gap_y >= body.bottom() {
                    break;
                }
                put_text(
                    buffer,
                    body.x,
                    gap_y,
                    content_width.min(super::space_agents::TREE_MARKER_WIDTH),
                    &format!(" {}", config.tree_worktrees.rail),
                    Style::default().fg(palette.overlay0),
                );
            }
        }
        y = y.saturating_add(row_height + gap);
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.workspace_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, palette);
    }

    if let Some(row) = state.workspace_drop_indicator_row.filter(|row| {
        *row >= workspace_area.y.saturating_add(1)
            && *row < workspace_area.bottom().saturating_sub(1)
    }) {
        put_text(
            buffer,
            body.x,
            row,
            body.width,
            &"─".repeat(body.width as usize),
            Style::default().fg(palette.accent),
        );
    }
}

/// Draws the `new` / `menu` controls on the bottom row of `area`.
pub(in crate::client::shell) fn render_sidebar_footer(
    buffer: &mut Buffer,
    workspace_area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    let footer_y = workspace_area.bottom().saturating_sub(1);
    if config.mouse_capture {
        hits.new_workspace = Rect::new(
            workspace_area.x,
            footer_y,
            5.min(workspace_area.width),
            u16::from(workspace_area.height > 0),
        );
        put_text(
            buffer,
            workspace_area.x,
            footer_y,
            workspace_area.width,
            " new",
            Style::default().fg(palette.overlay0),
        );
        let attention = super::super::global_menu::global_menu_attention(snapshot);
        let launcher_width = if attention { 8 } else { 6 }.min(workspace_area.width);
        hits.global_launcher = Rect::new(
            workspace_area.right().saturating_sub(launcher_width),
            footer_y,
            launcher_width,
            1,
        );
        if attention {
            let start_x = workspace_area.right().saturating_sub(6);
            put_text(
                buffer,
                start_x,
                footer_y,
                2,
                "● ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            );
            put_text(
                buffer,
                start_x.saturating_add(2),
                footer_y,
                4,
                "menu",
                Style::default().fg(palette.overlay0),
            );
        } else {
            put_right_text(
                buffer,
                workspace_area,
                footer_y,
                "menu",
                Style::default().fg(palette.overlay0),
            );
        }
    }
}

/// Jump number of a space: `1`..=`10`, shown as `1`..`9` and `0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JumpNumber(u8);

impl JumpNumber {
    pub(crate) const MAX: usize = 10;

    /// `number` is one-based; `0` and values past `MAX` have no jump number.
    pub(crate) fn new(number: usize) -> Option<Self> {
        (1..=Self::MAX)
            .contains(&number)
            .then_some(Self(number as u8))
    }

    pub(crate) fn number(self) -> usize {
        usize::from(self.0)
    }

    /// The key character: `'1'`..`'9'`, and `'0'` for the tenth number.
    pub(crate) fn digit(self) -> char {
        char::from(b'0' + self.0 % 10)
    }
}

/// Jump numbers for `entries`, one pass: spaces flagged `skip_index` get none,
/// the rest are numbered in order up to `JumpNumber::MAX`.
pub(crate) fn space_jump_numbers(
    entries: &[WorkspaceEntry],
    snapshot: &ClientShellSnapshot,
) -> Vec<Option<JumpNumber>> {
    let mut next = 1;
    entries
        .iter()
        .map(|entry| {
            let skipped = snapshot
                .workspaces
                .get(entry.index)
                .is_none_or(|workspace| workspace.skip_index);
            if skipped {
                return None;
            }
            let number = JumpNumber::new(next);
            next += 1;
            number
        })
        .collect()
}

/// The entry that jump number `number` (one-based) reaches.
pub(crate) fn entry_for_jump_number(
    entries: &[WorkspaceEntry],
    snapshot: &ClientShellSnapshot,
    number: usize,
) -> Option<WorkspaceEntry> {
    let number = JumpNumber::new(number)?.number();
    entries
        .iter()
        .filter(|entry| {
            snapshot
                .workspaces
                .get(entry.index)
                .is_some_and(|workspace| !workspace.skip_index)
        })
        .nth(number - 1)
        .copied()
}

/// Splits the lower sidebar section into `(agents, pull_requests)`: the whole
/// area for the one enabled, the top and bottom halves when both are.
pub(in crate::client::shell) fn split_lower_section(
    area: Rect,
    agents_enabled: bool,
    pull_requests_enabled: bool,
) -> (Rect, Rect) {
    match (agents_enabled, pull_requests_enabled) {
        (true, true) => {
            let agents_height = area.height.div_ceil(2);
            (
                Rect::new(area.x, area.y, area.width, agents_height),
                Rect::new(
                    area.x,
                    area.y + agents_height,
                    area.width,
                    area.height - agents_height,
                ),
            )
        }
        (true, false) => (area, Rect::default()),
        (false, true) => (Rect::default(), area),
        (false, false) => (Rect::default(), Rect::default()),
    }
}

/// Column where an indented worktree child's content starts: one column of
/// margin, the `├─`/`└─` connector, and a space.
pub(in crate::client::shell) const SUBSPACE_INDENT: u16 = 4;

/// Blank rows drawn after an entry, before `next`: `row_gap` before a
/// top-level space, `subspace_row_gap` before a worktree child, none after the
/// last entry. With `row_gap_after_agents_only`, only an entry whose agent
/// children are shown, or the last worktree child of a group, is followed by
/// a gap, so a group always stays apart from the next top-level space.
fn space_gap_after(
    entry: &WorkspaceEntry,
    shows_agents: bool,
    next: Option<&WorkspaceEntry>,
    config: &SpacesSidebarConfig,
) -> u16 {
    let closes_group = entry.indented && next.is_some_and(|next| !next.indented);
    if config.row_gap_after_agents_only && !shows_agents && !closes_group {
        return 0;
    }
    next.map_or(0, |next| {
        if next.indented {
            config.subspace_row_gap
        } else {
            config.row_gap
        }
    })
}

pub(crate) fn workspace_entries(
    snapshot: &ClientShellSnapshot,
    collapsed_groups: &HashSet<String>,
) -> Vec<WorkspaceEntry> {
    let mut members = HashMap::<&str, Vec<usize>>::new();
    for (index, workspace) in snapshot.workspaces.iter().enumerate() {
        if let Some(worktree) = &workspace.worktree {
            members.entry(&worktree.key).or_default().push(index);
        }
    }
    let grouped = members
        .iter()
        .filter(|(_, indices)| {
            indices.iter().any(|index| {
                snapshot.workspaces[*index]
                    .worktree
                    .as_ref()
                    .is_some_and(|worktree| worktree.is_linked_worktree)
            }) && indices.iter().any(|index| {
                snapshot.workspaces[*index]
                    .worktree
                    .as_ref()
                    .is_some_and(|worktree| !worktree.is_linked_worktree)
            })
        })
        .map(|(key, _)| *key)
        .collect::<HashSet<_>>();
    let mut emitted = HashSet::<&str>::new();
    let mut entries = Vec::new();
    for (index, workspace) in snapshot.workspaces.iter().enumerate() {
        let Some(worktree) = workspace
            .worktree
            .as_ref()
            .filter(|worktree| grouped.contains(worktree.key.as_str()))
        else {
            entries.push(WorkspaceEntry {
                index,
                indented: false,
                last_child: false,
            });
            continue;
        };
        if !emitted.insert(&worktree.key) {
            continue;
        }
        let Some(group_members) = members.get(worktree.key.as_str()) else {
            continue;
        };
        for parent in group_members.iter().copied().filter(|member| {
            snapshot.workspaces[*member]
                .worktree
                .as_ref()
                .is_some_and(|worktree| !worktree.is_linked_worktree)
        }) {
            entries.push(WorkspaceEntry {
                index: parent,
                indented: false,
                last_child: false,
            });
        }
        if collapsed_groups.contains(&worktree.key) {
            if let Some(active) = group_members.iter().copied().find(|member| {
                let workspace = &snapshot.workspaces[*member];
                workspace.focused
                    && workspace
                        .worktree
                        .as_ref()
                        .is_some_and(|worktree| worktree.is_linked_worktree)
            }) {
                entries.push(WorkspaceEntry {
                    index: active,
                    indented: true,
                    last_child: true,
                });
            }
            continue;
        }
        let children = group_members
            .iter()
            .copied()
            .filter(|member| {
                snapshot.workspaces[*member]
                    .worktree
                    .as_ref()
                    .is_some_and(|worktree| worktree.is_linked_worktree)
            })
            .collect::<Vec<_>>();
        for (child_index, child) in children.iter().enumerate() {
            entries.push(WorkspaceEntry {
                index: *child,
                indented: true,
                last_child: child_index + 1 == children.len(),
            });
        }
    }
    entries
}

fn parent_group_key(snapshot: &ClientShellSnapshot, index: usize) -> Option<String> {
    let workspace = snapshot.workspaces.get(index)?;
    let worktree = workspace.worktree.as_ref()?;
    if worktree.is_linked_worktree {
        return None;
    }
    snapshot
        .workspaces
        .iter()
        .any(|candidate| {
            candidate.worktree.as_ref().is_some_and(|candidate| {
                candidate.key == worktree.key && candidate.is_linked_worktree
            })
        })
        .then(|| worktree.key.clone())
}

pub(in crate::client::shell) fn workspace_close_is_group(
    snapshot: &ClientShellSnapshot,
    workspace: &ClientShellWorkspace,
) -> bool {
    let Some(worktree) = workspace
        .worktree
        .as_ref()
        .filter(|worktree| !worktree.is_linked_worktree)
    else {
        return false;
    };
    let mut has_child = false;
    for member in &snapshot.workspaces {
        if member.workspace_id == workspace.workspace_id {
            continue;
        }
        if let Some(candidate) = member
            .worktree
            .as_ref()
            .filter(|candidate| candidate.key == worktree.key)
        {
            if !candidate.is_linked_worktree {
                return false;
            }
            has_child = true;
        }
    }
    has_child
}

pub(in crate::client::shell) fn render_parent_group_toggle(
    buffer: &mut Buffer,
    workspace_rect: Rect,
    snapshot: &ClientShellSnapshot,
    workspace_index: usize,
    collapsed_groups: &HashSet<String>,
    palette: &Palette,
) -> Option<(Rect, String)> {
    let key = parent_group_key(snapshot, workspace_index)?;
    let toggle = Rect::new(
        workspace_rect.right().saturating_sub(1),
        workspace_rect.y,
        1,
        1,
    );
    put_text(
        buffer,
        toggle.x,
        toggle.y,
        toggle.width,
        if collapsed_groups.contains(&key) {
            "▸"
        } else {
            "▾"
        },
        Style::default().fg(palette.accent),
    );
    Some((toggle, key))
}

pub(in crate::client::shell) fn displayed_workspace_status(
    snapshot: &ClientShellSnapshot,
    workspace: &ClientShellWorkspace,
    collapsed_groups: &HashSet<String>,
) -> crate::api::schema::AgentStatus {
    let Some(worktree) = workspace
        .worktree
        .as_ref()
        .filter(|worktree| !worktree.is_linked_worktree)
    else {
        return workspace.agent_status;
    };
    if !collapsed_groups.contains(&worktree.key) {
        return workspace.agent_status;
    }
    snapshot
        .workspaces
        .iter()
        .filter(|candidate| {
            candidate.worktree.as_ref().is_some_and(|member| {
                member.key == worktree.key
                    && (member.is_linked_worktree
                        || candidate.workspace_id == workspace.workspace_id)
            })
        })
        .map(|candidate| candidate.agent_status)
        .max_by_key(|status| status_priority(*status))
        .unwrap_or(workspace.agent_status)
}

pub(in crate::client::shell) fn workspace_rows(
    workspace: &ClientShellWorkspace,
    status: crate::api::schema::AgentStatus,
    indented: bool,
    jump_number: Option<char>,
    config: &SpacesSidebarConfig,
) -> Vec<Vec<crate::ui::ResolvedToken>> {
    let label = if indented && !workspace.custom_label {
        workspace
            .branch
            .as_deref()
            .and_then(|branch| branch.strip_prefix("worktree/").or(Some(branch)))
            .unwrap_or(&workspace.label)
    } else {
        &workspace.label
    };
    let token_values = workspace.tokens.iter().cloned().collect::<HashMap<_, _>>();
    crate::ui::sidebar_space_rows(
        config,
        crate::ui::SpaceTokenContext {
            workspace: label,
            jump_number,
            branch: workspace.branch.as_deref(),
            state_text: status_text(status),
            ahead_behind: workspace.git_ahead_behind,
            tokens: &token_values,
            links: &workspace.token_links,
            suppress_git_details: indented,
            focused: workspace.focused,
        },
    )
}

// The row state flags and the optional link sink are independent inputs shared
// by the single- and multi-machine sidebars.
#[allow(clippy::too_many_arguments)]
pub(in crate::client::shell) fn render_workspace_rows(
    buffer: &mut Buffer,
    area: Rect,
    status: crate::api::schema::AgentStatus,
    indicators: &super::StatusIndicators,
    entry: &WorkspaceEntry,
    rows: Vec<Vec<crate::ui::ResolvedToken>>,
    focused: bool,
    selected: bool,
    navigating: bool,
    dragged: bool,
    palette: &Palette,
    tree: &super::space_agents::TreeGlyphs,
    mut links: Option<&mut Vec<TokenLinkHit>>,
    underline_links: bool,
) {
    for (row_index, row) in rows.iter().enumerate() {
        let y = area.y + row_index as u16;
        if y >= area.bottom() {
            break;
        }
        let mut x = area.x;
        if entry.indented {
            // The connector sits under the parent's first column so the tree
            // stays close to the left edge; continuation rows indent 2 more.
            let prefix = if row_index == 0 {
                format!(" {} ", tree.connector(entry.last_child))
            } else if entry.last_child {
                "      ".to_owned()
            } else {
                format!(" {}    ", tree.rail)
            };
            x = put_segment(
                buffer,
                x,
                y,
                area.right(),
                &prefix,
                Style::default().fg(palette.overlay0),
            );
        } else if row_index == 0 {
            x = x.saturating_add(1);
        } else {
            x = x.saturating_add(3);
        }
        let highlighted = focused || dragged;
        let workspace_style = Style::default()
            .fg(if highlighted {
                palette.text
            } else {
                palette.subtext0
            })
            .add_modifier(if highlighted {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        let secondary_style = Style::default().fg(if focused {
            palette.mauve
        } else {
            palette.overlay0
        });
        // Token spans read `palette.index`; the focused row swaps in its own
        // number color. Cloned only for that one row.
        let focused_palette;
        let token_palette = if focused && palette.index_focused.is_some() {
            focused_palette = Palette {
                index: palette.index_color(true),
                ..palette.clone()
            };
            &focused_palette
        } else {
            palette
        };
        let spans = super::agent_sidebar::token_row_spans(
            row,
            // Space rows have no tab token.
            false,
            (
                status_icon(status, indicators),
                Style::default().fg(status_color(status, palette)),
            ),
            Style::default().fg(status_color(status, palette)),
            workspace_style,
            secondary_style,
            Style::default().fg(palette.overlay1),
            token_palette,
            area.right().saturating_sub(2).saturating_sub(x) as usize,
            links
                .as_deref_mut()
                .map(|hits| super::agent_sidebar::LinkSink {
                    hits,
                    x,
                    y,
                    underline: underline_links,
                }),
        );
        Paragraph::new(Line::from(spans)).render(
            Rect::new(x, y, area.right().saturating_sub(2).saturating_sub(x), 1),
            buffer,
        );
    }

    let background = if selected {
        Some(workspace_selection_background(palette))
    } else if dragged {
        Some(palette.surface1)
    } else if focused {
        Some(workspace_active_background(palette, navigating))
    } else {
        None
    };
    if let Some(background) = background {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buffer[(x, y)].set_bg(background);
            }
        }
    }
}
