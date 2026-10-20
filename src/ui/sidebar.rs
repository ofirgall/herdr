mod tokens;

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::Span,
};

pub(crate) use self::tokens::{
    agent_rows as sidebar_agent_rows, space_rows as sidebar_space_rows, AgentTokenContext,
    ResolvedToken, ResolvedTokenKind, SpaceTokenContext,
};
use super::text::{display_width, truncate_end};
use crate::app::state::Palette;
use crate::app::AppState;
use crate::config::PanelHeight;
use crate::detect::AgentState;
use crate::terminal::TerminalRuntimeRegistry;

pub(crate) struct AgentPanelEntry {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: crate::layout::PaneId,
    pub agent_kind_label: Option<String>,
    pub state: AgentState,
    pub seen: bool,
    pub last_agent_state_change_seq: Option<u64>,
    pub tokens: std::collections::HashMap<String, String>,
}

fn sidebar_section_heights(total_height: u16, split_ratio: f32) -> (u16, u16) {
    if total_height == 0 {
        return (0, 0);
    }
    if total_height < 6 {
        let workspace_height = total_height.div_ceil(2);
        return (
            workspace_height,
            total_height.saturating_sub(workspace_height),
        );
    }

    let workspace_height = ((total_height as f32) * split_ratio.clamp(0.1, 0.9)).round() as u16;
    let workspace_height = workspace_height.clamp(3, total_height.saturating_sub(3));
    (
        workspace_height,
        total_height.saturating_sub(workspace_height),
    )
}

/// One section of the expanded sidebar, in render order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SectionSpec {
    Spaces,
    Agents,
    Panel { height: PanelHeight },
}

/// Where the section at `spec_index` of the `SectionSpec` slice is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SectionRect {
    pub spec_index: usize,
    pub rect: Rect,
}

/// Lay out the expanded sidebar sections top to bottom.
///
/// Panels take their configured height first, clamped so every section keeps at least its
/// header row. Spaces and agents share the remaining rows by `split_ratio`; a lone one takes all
/// of them. Without either, the last panel absorbs the leftover rows. With the default sections
/// this yields the same rectangles as the original two-section sidebar.
pub(crate) fn sidebar_section_layout(
    area: Rect,
    sections: &[SectionSpec],
    split_ratio: f32,
) -> Vec<SectionRect> {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.is_empty() {
        return sections
            .iter()
            .enumerate()
            .map(|(spec_index, _)| SectionRect {
                spec_index,
                rect: Rect::default(),
            })
            .collect();
    }
    let total = content.height;
    let mut heights = vec![0u16; sections.len()];
    let mut allocated = 0u16;
    for (index, section) in sections.iter().enumerate() {
        let SectionSpec::Panel { height } = section else {
            continue;
        };
        // Every section not yet sized (built-ins anywhere, panels after this one) keeps its
        // header row.
        let reserved = sections
            .iter()
            .enumerate()
            .filter(|(other, section)| {
                !matches!(section, SectionSpec::Panel { .. }) || *other > index
            })
            .count() as u16;
        let max = total.saturating_sub(allocated).saturating_sub(reserved);
        heights[index] = height.rows(total).max(1).min(max);
        allocated += heights[index];
    }
    let flex = total - allocated;
    let has_spaces = sections.contains(&SectionSpec::Spaces);
    let has_agents = sections.contains(&SectionSpec::Agents);
    let (spaces_height, agents_height) = match (has_spaces, has_agents) {
        (true, true) => sidebar_section_heights(flex, split_ratio),
        (true, false) => (flex, 0),
        (false, true) => (0, flex),
        (false, false) => {
            if let Some(last) = sections
                .iter()
                .rposition(|section| matches!(section, SectionSpec::Panel { .. }))
            {
                heights[last] += flex;
            }
            (0, 0)
        }
    };
    let mut y = content.y;
    sections
        .iter()
        .enumerate()
        .map(|(spec_index, section)| {
            let height = match section {
                SectionSpec::Spaces => spaces_height,
                SectionSpec::Agents => agents_height,
                SectionSpec::Panel { .. } => heights[spec_index],
            };
            let rect = Rect::new(content.x, y, content.width, height);
            y += height;
            SectionRect { spec_index, rect }
        })
        .collect()
}

/// The draggable row between spaces and agents: the agents header, when the two sections are
/// adjacent and large enough to have a split.
pub(crate) fn sidebar_section_divider(sections: &[SectionSpec], layout: &[SectionRect]) -> Rect {
    let rect_of = |wanted: &SectionSpec| {
        layout
            .iter()
            .find(|section| &sections[section.spec_index] == wanted)
    };
    let (Some(spaces), Some(agents)) =
        (rect_of(&SectionSpec::Spaces), rect_of(&SectionSpec::Agents))
    else {
        return Rect::default();
    };
    let combined = spaces.rect.height + agents.rect.height;
    if agents.spec_index != spaces.spec_index + 1
        || agents.rect.width == 0
        || combined < 6
        || agents.rect.is_empty()
    {
        return Rect::default();
    }
    Rect::new(agents.rect.x, agents.rect.y, agents.rect.width, 1)
}

/// The rows shared by spaces and agents when they are adjacent; the span a divider drag maps
/// onto the split ratio.
pub(crate) fn sidebar_split_span(sections: &[SectionSpec], layout: &[SectionRect]) -> Rect {
    let rect_of = |wanted: &SectionSpec| {
        layout
            .iter()
            .find(|section| &sections[section.spec_index] == wanted)
    };
    match (rect_of(&SectionSpec::Spaces), rect_of(&SectionSpec::Agents)) {
        (Some(spaces), Some(agents)) if agents.spec_index == spaces.spec_index + 1 => Rect::new(
            spaces.rect.x,
            spaces.rect.y,
            spaces.rect.width,
            spaces.rect.height + agents.rect.height,
        ),
        _ => Rect::default(),
    }
}

pub(crate) fn agent_panel_entries_from(
    app: &AppState,
    _terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<AgentPanelEntry> {
    let mut entries = app
        .workspaces
        .iter()
        .enumerate()
        .flat_map(|(ws_idx, workspace)| {
            workspace
                .pane_details(&app.terminals)
                .into_iter()
                .map(move |detail| AgentPanelEntry {
                    ws_idx,
                    tab_idx: detail.tab_idx,
                    pane_id: detail.pane_id,
                    agent_kind_label: detail.agent_kind_label,
                    state: detail.state,
                    seen: detail.seen,
                    last_agent_state_change_seq: detail.last_agent_state_change_seq,
                    tokens: detail.tokens,
                })
        })
        .collect();
    crate::app::agent_view::apply_agent_view(app, &mut entries);
    entries
}

pub(crate) fn resolved_token_spans(
    resolved: &[ResolvedToken],
    state_icon: (&str, Style),
    state_text_style: Style,
    workspace_style: Style,
    secondary_style: Style,
    custom_style: Style,
    palette: &Palette,
    max_width: usize,
) -> Vec<Span<'static>> {
    let fixed_widths = resolved
        .iter()
        .map(|token| match &token.kind {
            ResolvedTokenKind::StateIcon => display_width(state_icon.0),
            ResolvedTokenKind::AgentIndex(index) => display_width(&(index + 1).to_string()),
            ResolvedTokenKind::GitStatus { ahead, behind } => {
                usize::from(*ahead > 0) * display_width(&format!("↑{ahead}"))
                    + usize::from(*behind > 0) * display_width(&format!("↓{behind}"))
                    + usize::from(*ahead > 0 && *behind > 0)
            }
            _ => 0,
        })
        .collect::<Vec<_>>();
    let flexible_widths = resolved
        .iter()
        .map(|token| match &token.kind {
            ResolvedTokenKind::StateText(text)
            | ResolvedTokenKind::Machine(text)
            | ResolvedTokenKind::Workspace(text)
            | ResolvedTokenKind::Tab(text)
            | ResolvedTokenKind::Pane(text)
            | ResolvedTokenKind::Agent(text)
            | ResolvedTokenKind::TerminalTitle(text)
            | ResolvedTokenKind::Branch(text)
            | ResolvedTokenKind::Custom(text) => display_width(text),
            _ => 0,
        })
        .collect::<Vec<_>>();
    let minimum_width = |active: &[bool]| {
        let indices = active
            .iter()
            .enumerate()
            .filter_map(|(index, active)| active.then_some(index))
            .collect::<Vec<_>>();
        let content = indices
            .iter()
            .map(|index| fixed_widths[*index] + usize::from(flexible_widths[*index] > 0))
            .sum::<usize>();
        let separators = indices
            .windows(2)
            .map(|pair| display_width(tokens::separator(&resolved[pair[0]], &resolved[pair[1]])))
            .sum::<usize>();
        content + separators
    };
    let mut active = resolved.iter().map(|_| true).collect::<Vec<_>>();
    if minimum_width(&active) > max_width {
        for (index, width) in flexible_widths.iter().enumerate() {
            if *width > 0 {
                active[index] = false;
            }
        }
        for index in (0..resolved.len()).rev() {
            if flexible_widths[index] == 0 {
                continue;
            }
            active[index] = true;
            if minimum_width(&active) > max_width {
                active[index] = false;
            }
        }
    }
    let visible_indices = active
        .iter()
        .enumerate()
        .filter_map(|(index, active)| active.then_some(index))
        .collect::<Vec<_>>();
    let separator_width = visible_indices
        .windows(2)
        .map(|pair| display_width(tokens::separator(&resolved[pair[0]], &resolved[pair[1]])))
        .sum::<usize>();
    let fixed_width = visible_indices
        .iter()
        .map(|index| fixed_widths[*index])
        .sum::<usize>();
    let mut budgets = flexible_widths
        .iter()
        .enumerate()
        .map(|(index, width)| usize::from(active[index] && *width > 0))
        .collect::<Vec<_>>();
    let minimum = budgets.iter().sum::<usize>();
    let mut remaining = max_width
        .saturating_sub(separator_width + fixed_width)
        .saturating_sub(minimum);
    while remaining > 0 {
        let mut grew = false;
        for (budget, width) in budgets.iter_mut().zip(&flexible_widths) {
            if *budget > 0 && *budget < *width {
                *budget += 1;
                remaining -= 1;
                grew = true;
                if remaining == 0 {
                    break;
                }
            }
        }
        if !grew {
            break;
        }
    }

    let mut spans = Vec::new();
    for (position, index) in visible_indices.iter().copied().enumerate() {
        let token = &resolved[index];
        if position > 0 {
            let previous = &resolved[visible_indices[position - 1]];
            spans.push(Span::styled(
                tokens::separator(previous, token),
                Style::default().fg(palette.overlay0),
            ));
        }
        match &token.kind {
            ResolvedTokenKind::StateIcon => spans.push(Span::styled(
                state_icon.0.to_string(),
                apply_token_style(state_icon.1, token.style),
            )),
            ResolvedTokenKind::StateText(text) => spans.push(Span::styled(
                truncate_end(text, budgets[index]),
                apply_token_style(state_text_style, token.style),
            )),
            ResolvedTokenKind::Workspace(text) => spans.push(Span::styled(
                truncate_end(text, budgets[index]),
                apply_token_style(workspace_style, token.style),
            )),
            ResolvedTokenKind::Machine(text)
            | ResolvedTokenKind::Tab(text)
            | ResolvedTokenKind::Pane(text)
            | ResolvedTokenKind::Agent(text)
            | ResolvedTokenKind::Branch(text) => spans.push(Span::styled(
                truncate_end(text, budgets[index]),
                apply_token_style(secondary_style, token.style),
            )),
            ResolvedTokenKind::AgentIndex(index) => spans.push(Span::styled(
                (index + 1).to_string(),
                apply_token_style(secondary_style, token.style),
            )),
            ResolvedTokenKind::GitStatus { ahead, behind } => {
                if *ahead > 0 {
                    spans.push(Span::styled(
                        format!("↑{ahead}"),
                        apply_token_style(Style::default().fg(palette.green), token.style),
                    ));
                }
                if *ahead > 0 && *behind > 0 {
                    spans.push(Span::styled(
                        " ",
                        apply_token_style(Style::default(), token.style),
                    ));
                }
                if *behind > 0 {
                    spans.push(Span::styled(
                        format!("↓{behind}"),
                        apply_token_style(Style::default().fg(palette.red), token.style),
                    ));
                }
            }
            ResolvedTokenKind::TerminalTitle(text) | ResolvedTokenKind::Custom(text) => {
                spans.push(Span::styled(
                    truncate_end(text, budgets[index]),
                    apply_token_style(custom_style, token.style),
                ));
            }
        }
    }
    spans
}

fn apply_token_style(mut style: Style, patch: crate::config::SidebarTokenStyle) -> Style {
    if let Some(foreground) = patch.fg {
        style = style.fg(foreground.ratatui());
    }
    if let Some(bold) = patch.bold {
        style = if bold {
            style.add_modifier(Modifier::BOLD)
        } else {
            style.remove_modifier(Modifier::BOLD)
        };
    }
    if let Some(dim) = patch.dim {
        style = if dim {
            style.add_modifier(Modifier::DIM)
        } else {
            style.remove_modifier(Modifier::DIM)
        };
    }
    style
}

#[cfg(test)]
mod characterization_tests {
    use super::*;

    /// Frozen copy of the two-section algorithm from before sidebar panels existed. The default
    /// sections must keep producing exactly these rectangles.
    fn reference_sections(area: Rect, split_ratio: f32) -> (Rect, Rect, Rect) {
        let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
        if content.is_empty() {
            return (Rect::default(), Rect::default(), Rect::default());
        }
        let total = content.height;
        let (workspace_height, detail_height) = if total < 6 {
            let workspace_height = total.div_ceil(2);
            (workspace_height, total - workspace_height)
        } else {
            let workspace_height = ((total as f32) * split_ratio.clamp(0.1, 0.9)).round() as u16;
            let workspace_height = workspace_height.clamp(3, total - 3);
            (workspace_height, total - workspace_height)
        };
        let divider = if content.width == 0 || total < 6 {
            Rect::default()
        } else {
            Rect::new(content.x, content.y + workspace_height, content.width, 1)
        };
        (
            Rect::new(content.x, content.y, content.width, workspace_height),
            Rect::new(
                content.x,
                content.y + workspace_height,
                content.width,
                detail_height,
            ),
            divider,
        )
    }

    pub(super) const SPLITS: [f32; 10] = [-1.0, 0.0, 0.1, 0.25, 0.3, 0.5, 0.62, 0.9, 1.0, 7.0];

    pub(super) fn areas() -> impl Iterator<Item = Rect> {
        [(0u16, 0u16), (3, 2), (1, 9), (2, 1), (27, 4), (26, 5)]
            .into_iter()
            .flat_map(|(x, y)| {
                [0u16, 1, 2, 26, 40].into_iter().flat_map(move |width| {
                    (0u16..=70).map(move |height| Rect::new(x, y, width, height))
                })
            })
    }

    #[test]
    fn reference_matches_known_literal_rects() {
        let (workspaces, agents, divider) = reference_sections(Rect::new(0, 0, 27, 20), 0.5);
        assert_eq!(workspaces, Rect::new(0, 0, 26, 10));
        assert_eq!(agents, Rect::new(0, 10, 26, 10));
        assert_eq!(divider, Rect::new(0, 10, 26, 1));
        let (workspaces, agents, divider) = reference_sections(Rect::new(0, 0, 27, 5), 0.5);
        assert_eq!(workspaces, Rect::new(0, 0, 26, 3));
        assert_eq!(agents, Rect::new(0, 3, 26, 2));
        assert_eq!(divider, Rect::default());
        let (workspaces, agents, _) = reference_sections(Rect::new(0, 0, 27, 20), 0.0);
        assert_eq!(workspaces.height, 3);
        assert_eq!(agents.height, 17);
    }

    fn default_sections() -> Vec<SectionSpec> {
        vec![SectionSpec::Spaces, SectionSpec::Agents]
    }

    #[test]
    fn default_layout_matches_the_reference() {
        let sections = default_sections();
        for area in areas() {
            for split in SPLITS {
                let (workspaces, agents, divider) = reference_sections(area, split);
                let layout = sidebar_section_layout(area, &sections, split);
                assert_eq!(
                    layout
                        .iter()
                        .map(|section| section.rect)
                        .collect::<Vec<_>>(),
                    vec![workspaces, agents],
                    "{area:?} split {split}"
                );
                assert_eq!(
                    sidebar_section_divider(&sections, &layout),
                    divider,
                    "{area:?} split {split}"
                );
                let span = sidebar_split_span(&sections, &layout);
                if !workspaces.is_empty() {
                    assert_eq!(span.y, workspaces.y);
                    assert_eq!(span.height, workspaces.height + agents.height);
                }
            }
        }
    }

    fn panel(height: PanelHeight) -> SectionSpec {
        SectionSpec::Panel { height }
    }

    fn heights(layout: &[SectionRect]) -> Vec<u16> {
        layout.iter().map(|section| section.rect.height).collect()
    }

    #[test]
    fn panel_between_spaces_and_agents_takes_its_height_first() {
        let sections = vec![
            SectionSpec::Spaces,
            panel(PanelHeight::Rows(8)),
            SectionSpec::Agents,
        ];
        let layout = sidebar_section_layout(Rect::new(0, 0, 27, 30), &sections, 0.5);
        assert_eq!(heights(&layout), vec![11, 8, 11]);
        assert_eq!(layout[0].rect.y, 0);
        assert_eq!(layout[1].rect.y, 11);
        assert_eq!(layout[2].rect.y, 19);
        assert_eq!(layout[1].rect, Rect::new(0, 11, 26, 8));
        // Spaces and agents are not adjacent, so there is no divider to drag.
        assert_eq!(sidebar_section_divider(&sections, &layout), Rect::default());
        assert_eq!(sidebar_split_span(&sections, &layout), Rect::default());
    }

    #[test]
    fn percent_heights_and_panel_ordering_follow_sections() {
        let sections = vec![
            panel(PanelHeight::Percent(30)),
            SectionSpec::Spaces,
            SectionSpec::Agents,
        ];
        let layout = sidebar_section_layout(Rect::new(2, 1, 27, 40), &sections, 0.5);
        assert_eq!(layout[0].rect, Rect::new(2, 1, 26, 12));
        assert_eq!(heights(&layout), vec![12, 14, 14]);
        assert_eq!(layout[1].rect.y, 13);
        let divider = sidebar_section_divider(&sections, &layout);
        assert_eq!(divider, Rect::new(2, 27, 26, 1));
        assert_eq!(
            sidebar_split_span(&sections, &layout),
            Rect::new(2, 13, 26, 28)
        );
    }

    #[test]
    fn panel_heights_are_clamped_to_keep_every_header() {
        let sections = vec![
            SectionSpec::Spaces,
            panel(PanelHeight::Rows(500)),
            panel(PanelHeight::Percent(100)),
            SectionSpec::Agents,
        ];
        for total in 4u16..=40 {
            let layout = sidebar_section_layout(Rect::new(0, 0, 27, total), &sections, 0.5);
            let heights = heights(&layout);
            assert_eq!(heights.iter().sum::<u16>(), total, "{total}");
            assert!(
                heights.iter().all(|height| *height >= 1),
                "{total}: {heights:?}"
            );
            let mut y = 0;
            for section in &layout {
                assert_eq!(section.rect.y, y);
                y += section.rect.height;
            }
        }
    }

    #[test]
    fn too_small_sidebars_do_not_overflow() {
        let sections = vec![
            SectionSpec::Spaces,
            panel(PanelHeight::Rows(5)),
            SectionSpec::Agents,
        ];
        for total in 0u16..=3 {
            let layout = sidebar_section_layout(Rect::new(0, 0, 27, total), &sections, 0.5);
            assert!(heights(&layout).iter().sum::<u16>() <= total);
        }
    }

    #[test]
    fn omitted_builtin_sections_leave_the_rest_to_the_remaining_one() {
        let only_agents = vec![panel(PanelHeight::Rows(6)), SectionSpec::Agents];
        let layout = sidebar_section_layout(Rect::new(0, 0, 27, 20), &only_agents, 0.5);
        assert_eq!(heights(&layout), vec![6, 14]);
        assert_eq!(
            sidebar_section_divider(&only_agents, &layout),
            Rect::default()
        );

        let only_spaces = vec![SectionSpec::Spaces, panel(PanelHeight::Rows(6))];
        let layout = sidebar_section_layout(Rect::new(0, 0, 27, 20), &only_spaces, 0.5);
        assert_eq!(heights(&layout), vec![14, 6]);

        let only_panels = vec![panel(PanelHeight::Rows(4)), panel(PanelHeight::Rows(4))];
        let layout = sidebar_section_layout(Rect::new(0, 0, 27, 20), &only_panels, 0.5);
        assert_eq!(heights(&layout), vec![4, 16]);
    }
}
