use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget},
};

use super::*;

pub(super) struct AgentRow {
    pub(super) pane_id: String,
    pub(super) status: crate::api::schema::AgentStatus,
    pub(super) focused: bool,
    pub(super) rows: Vec<Vec<crate::ui::ResolvedToken>>,
}

pub(super) fn ordered_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
) -> Vec<String> {
    if snapshot.agent_view_label.is_some() {
        return snapshot
            .agent_order
            .iter()
            .filter(|pane_id| {
                snapshot
                    .agents
                    .iter()
                    .any(|agent| agent.pane_id == pane_id.as_str())
            })
            .cloned()
            .collect();
    }
    let mut agents = snapshot.agents.iter().collect::<Vec<_>>();
    if sort == crate::config::AgentPanelSortConfig::Priority {
        agents.sort_by_key(|agent| {
            (
                std::cmp::Reverse(status_priority(agent.agent_status)),
                std::cmp::Reverse(agent.state_change_seq),
            )
        });
    }
    agents
        .into_iter()
        .map(|agent| agent.pane_id.clone())
        .collect()
}

pub(super) fn render_agent_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    top_rule: bool,
    hits: &mut ShellHitMap,
) {
    if !render_agent_panel_header(
        buffer,
        area,
        snapshot.agent_view_label.as_deref(),
        config,
        top_rule,
        hits,
    ) {
        return;
    }

    let rows = agent_rows(snapshot, config, None);
    render_agent_list(
        buffer,
        area,
        &rows,
        snapshot
            .agent_view_label
            .as_ref()
            .map(|_| " no matching agents"),
        config,
        agent_scroll,
        top_rule,
        hits,
        |row| row.rows.len(),
        |buffer, rect, row, hits| {
            hits.agents.push((rect, row.pane_id.clone()));
            let links = (!config.token_link_click.is_off()).then_some(&mut hits.token_links);
            render_agent_row(buffer, rect, row, config, links);
        },
    );
}

pub(super) fn render_agent_panel_header(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    config: &ClientShellConfig,
    top_rule: bool,
    hits: &mut ShellHitMap,
) -> bool {
    if area.height == 0 {
        return false;
    }
    // The rule is the divider against the spaces section; without a spaces
    // section above, the header starts on the first row instead.
    let title_y = area.y + u16::from(top_rule);
    if top_rule {
        put_text(
            buffer,
            area.x,
            area.y,
            area.width,
            &"─".repeat(area.width as usize),
            Style::default().fg(config.palette.surface_dim),
        );
    }
    if area.height < 1 + u16::from(top_rule) {
        return false;
    }
    put_text(
        buffer,
        area.x,
        title_y,
        area.width,
        " agents",
        Style::default()
            .fg(config.palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let sort_label = agent_view_label.unwrap_or(match config.agent_panel_sort {
        crate::config::AgentPanelSortConfig::Spaces => "grouped",
        crate::config::AgentPanelSortConfig::Priority => "priority",
    });
    let sort_width = display_width(sort_label).min(area.width as usize) as u16;
    let sort_rect = Rect::new(
        area.right().saturating_sub(sort_width),
        title_y,
        sort_width,
        1,
    );
    hits.agent_sort_toggle = if config.mouse_capture && agent_view_label.is_none() {
        sort_rect
    } else {
        Rect::default()
    };
    put_text(
        buffer,
        sort_rect.x,
        sort_rect.y,
        sort_rect.width,
        sort_label,
        Style::default()
            .fg(if agent_view_label.is_some() {
                config.palette.accent
            } else {
                config.palette.overlay0
            })
            .add_modifier(Modifier::BOLD),
    );
    true
}

pub(super) fn render_agent_list<T>(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[T],
    empty_message: Option<&str>,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    top_rule: bool,
    hits: &mut ShellHitMap,
    row_lines: impl Fn(&T) -> usize,
    mut render_row: impl FnMut(&mut Buffer, Rect, &T, &mut ShellHitMap),
) {
    let header_rows = 2 + u16::from(top_rule);
    let body = Rect::new(
        area.x,
        area.y.saturating_add(header_rows),
        area.width,
        area.height.saturating_sub(header_rows),
    );
    hits.agent_body = body;
    if body.is_empty() || rows.is_empty() {
        *agent_scroll = 0;
        if let Some(message) = empty_message.filter(|_| !body.is_empty()) {
            put_text(
                buffer,
                body.x,
                body.y,
                body.width,
                message,
                Style::default()
                    .fg(config.palette.overlay0)
                    .add_modifier(Modifier::DIM),
            );
        }
        return;
    }

    let row_heights = rows
        .iter()
        .map(|row| row_lines(row).max(1).min(u16::MAX as usize) as u16)
        .collect::<Vec<_>>();
    let gaps = rows
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            }
        })
        .collect::<Vec<_>>();
    let metrics =
        super::scroll::list_scroll_metrics(&row_heights, &gaps, body.height, *agent_scroll);
    hits.agent_max_scroll = metrics.max_offset_from_bottom;
    hits.agent_scroll_metrics = Some(metrics);
    *agent_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    let mut y = body.y;
    for (index, row) in rows.iter().enumerate().skip(*agent_scroll) {
        let height = row_heights[index].min(body.height);
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, height);
        render_row(buffer, rect, row, hits);
        y = y
            .saturating_add(height)
            .saturating_add(if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            });
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.agent_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, &config.palette);
    }
}

pub(super) fn agent_rows(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    machine: Option<&str>,
) -> Vec<AgentRow> {
    ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
        .into_iter()
        .enumerate()
        .filter_map(|(agent_index, pane_id)| {
            agent_row(snapshot, &pane_id, config, machine, agent_index)
        })
        .collect()
}

pub(super) fn agent_row(
    snapshot: &ClientShellSnapshot,
    pane_id: &str,
    config: &ClientShellConfig,
    machine: Option<&str>,
    agent_index: usize,
) -> Option<AgentRow> {
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane_id)?;
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == agent.workspace_id)?;
    let tab = snapshot.tabs.iter().find(|tab| tab.tab_id == agent.tab_id);
    let pane = snapshot
        .panes
        .iter()
        .find(|pane| pane.pane_id == agent.pane_id);
    let mut tab_count = 0;
    let mut tab_position = None;
    for candidate in snapshot
        .tabs
        .iter()
        .filter(|candidate| candidate.workspace_id == agent.workspace_id)
    {
        tab_count += 1;
        if candidate.tab_id == agent.tab_id {
            tab_position = Some(tab_count);
        }
    }
    let tab_label = tab
        .filter(|tab| tab_count > 1 || tab.custom_label)
        .map(|tab| tab.label.as_str());
    let rows = resolve_agent_rows(
        config.agents.rows_for_agent(
            agent
                .agent
                .as_deref()
                .and_then(crate::detect::parse_agent_label),
        ),
        agent,
        &workspace.label,
        tab_label,
        tab_position,
        pane.and_then(|pane| pane.label.as_deref()),
        machine,
        agent_index,
    );
    Some(AgentRow {
        pane_id: agent.pane_id.clone(),
        status: agent.agent_status,
        focused: agent.focused,
        rows,
    })
}

/// Where the hits of a row's linked tokens are recorded; `x` is the screen
/// column of the row's first token span and `y` its screen row.
pub(super) struct LinkSink<'a> {
    pub(super) hits: &'a mut Vec<TokenLinkHit>,
    pub(super) x: u16,
    pub(super) y: u16,
    /// Draw linked tokens underlined (`token_link_underline`).
    pub(super) underline: bool,
}

/// Token spans of one row. Rows without a linked token, and callers that pass
/// no `links` sink (links off, or a sidebar without link support), take the
/// plain path with no extra work; otherwise linked tokens are underlined when
/// the sink asks for it and one hit per visible linked token is recorded.
/// On a `focused` row, tab names are bold unless their token sets `bold`, the
/// way a space's jump number follows its focused weight.
pub(super) fn token_row_spans(
    tokens: &[crate::ui::ResolvedToken],
    focused: bool,
    icon: (&str, Style),
    state_style: Style,
    name_style: Style,
    secondary: Style,
    custom: Style,
    palette: &Palette,
    max_width: usize,
    links: Option<LinkSink<'_>>,
) -> Vec<ratatui::text::Span<'static>> {
    let is_plain_tab = |token: &crate::ui::ResolvedToken| {
        matches!(token.kind, crate::ui::ResolvedTokenKind::Tab(_)) && token.style.bold.is_none()
    };
    // Cloned only for the focused row, and only when it shows a tab name.
    let bolded;
    let tokens = if focused && tokens.iter().any(is_plain_tab) {
        bolded = tokens
            .iter()
            .cloned()
            .map(|mut token| {
                if is_plain_tab(&token) {
                    token.style.bold = Some(true);
                }
                token
            })
            .collect::<Vec<_>>();
        bolded.as_slice()
    } else {
        tokens
    };
    match links {
        Some(sink) if tokens.iter().any(|token| token.link.is_some()) => {
            let (spans, columns) = crate::ui::resolved_token_spans_with_links(
                tokens,
                icon,
                state_style,
                name_style,
                secondary,
                custom,
                palette,
                max_width,
                sink.underline,
            );
            for column in columns {
                let Some(url) = tokens[column.index].link.clone() else {
                    continue;
                };
                sink.hits.push(TokenLinkHit {
                    rect: Rect::new(
                        sink.x.saturating_add(column.start as u16),
                        sink.y,
                        column.width as u16,
                        1,
                    ),
                    url,
                });
            }
            spans
        }
        _ => crate::ui::resolved_token_spans(
            tokens,
            icon,
            state_style,
            name_style,
            secondary,
            custom,
            palette,
            max_width,
        ),
    }
}

/// Resolves one agent's token `rows` from its snapshot entry. `pane_label` is
/// the fallback for the `pane` token when the agent reports no title.
pub(super) fn resolve_agent_rows(
    rows: &[Vec<crate::config::AgentSidebarToken>],
    agent: &crate::protocol::ClientShellAgent,
    workspace_label: &str,
    tab_label: Option<&str>,
    tab_position: Option<usize>,
    pane_label: Option<&str>,
    machine: Option<&str>,
    agent_index: usize,
) -> Vec<Vec<crate::ui::ResolvedToken>> {
    let agent_label = agent
        .display_agent
        .as_deref()
        .or(agent.name.as_deref())
        .or(agent.agent.as_deref())
        .or(agent.title.as_deref());
    let state_text = agent
        .state_labels
        .iter()
        .rev()
        .find(|(status, _)| status == status_text(agent.agent_status))
        .map(|(_, label)| label.as_str())
        .unwrap_or_else(|| sidebar_status_text(agent.agent_status));
    let tokens = agent.tokens.iter().cloned().collect::<HashMap<_, _>>();
    crate::ui::sidebar_agent_rows(
        rows,
        crate::ui::AgentTokenContext {
            machine,
            workspace: workspace_label,
            tab: tab_label,
            tab_stripped: tab_label
                .zip(tab_position)
                .map(|(label, position)| crate::ui::strip_tab_number(label, position)),
            tab_position,
            pane: agent.title.as_deref().or(pane_label),
            agent_label,
            agent_index,
            terminal_title: agent.terminal_title.as_deref(),
            terminal_title_stripped: agent.terminal_title_stripped.as_deref(),
            tokens: &tokens,
            links: &agent.token_links,
        },
        state_text,
    )
}

pub(super) fn render_agent_row(
    buffer: &mut Buffer,
    rect: Rect,
    row: &AgentRow,
    config: &ClientShellConfig,
    mut links: Option<&mut Vec<TokenLinkHit>>,
) {
    let focused_palette = row
        .focused
        .then(|| config.palette.with_focused_tab())
        .flatten();
    let palette = focused_palette.as_ref().unwrap_or(&config.palette);
    let row_style = if row.focused {
        Style::default().bg(palette.active_row_bg)
    } else {
        Style::default()
    };
    let name_style = if row.focused {
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(palette.subtext0)
            .add_modifier(Modifier::BOLD)
    };
    let status_style = Style::default().fg(status_color(row.status, palette));
    let secondary = Style::default().fg(palette.overlay0);
    let icon = (
        status_icon(row.status, &config.status_indicators),
        Style::default().fg(status_color(row.status, palette)),
    );
    let rows = if row.rows.is_empty() {
        vec![vec![crate::ui::ResolvedToken {
            kind: crate::ui::ResolvedTokenKind::StateIcon,
            style: Default::default(),
            link: None,
        }]]
    } else {
        row.rows.clone()
    };
    for (index, tokens) in rows.iter().take(rect.height as usize).enumerate() {
        let indent = if index == 0 { 1 } else { 3 };
        let mut spans = vec![ratatui::text::Span::raw(" ".repeat(indent))];
        spans.extend(token_row_spans(
            tokens,
            row.focused,
            icon,
            status_style,
            name_style,
            secondary,
            secondary,
            palette,
            rect.width.saturating_sub(indent as u16) as usize,
            links.as_deref_mut().map(|hits| LinkSink {
                hits,
                x: rect.x.saturating_add(indent as u16),
                y: rect.y + index as u16,
                underline: config.token_link_underline,
            }),
        ));
        Paragraph::new(Line::from(spans)).style(row_style).render(
            Rect::new(rect.x, rect.y + index as u16, rect.width, 1),
            buffer,
        );
    }
}

fn put_text(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    for (offset, character) in text.chars().take(width as usize).enumerate() {
        if let Some(cell) = buffer.cell_mut((x + offset as u16, y)) {
            cell.set_char(character).set_style(style);
        }
    }
}

fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

pub(super) fn sidebar_status_text(status: crate::api::schema::AgentStatus) -> &'static str {
    use crate::api::schema::AgentStatus;
    match status {
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Working => "working",
        AgentStatus::Idle | AgentStatus::Unknown => "idle",
    }
}
