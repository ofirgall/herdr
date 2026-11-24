use crate::config::{AgentSidebarToken, SidebarTokenStyle, SpaceSidebarToken, SpacesSidebarConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedToken {
    pub kind: ResolvedTokenKind,
    pub style: SidebarTokenStyle,
    /// Web URL reported with a custom API token.
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedTokenKind {
    StateIcon,
    StateText(String),
    Machine(String),
    Workspace(String),
    Tab(String),
    Pane(String),
    Agent(String),
    AgentIndex(usize),
    /// Jump-number key character of a space and the text drawn for it
    /// (the digit with its configured format applied).
    Index {
        digit: char,
        text: String,
    },
    TerminalTitle(String),
    Branch(String),
    GitStatus {
        ahead: usize,
        behind: usize,
    },
    Custom(String),
    /// Flexible gap that pushes the tokens after it to the right edge.
    Spacer,
}

impl ResolvedTokenKind {
    fn text_value(&self) -> Option<&str> {
        match self {
            Self::StateText(value)
            | Self::Machine(value)
            | Self::Workspace(value)
            | Self::Tab(value)
            | Self::Pane(value)
            | Self::Agent(value)
            | Self::TerminalTitle(value)
            | Self::Branch(value)
            | Self::Custom(value) => Some(value),
            Self::StateIcon
            | Self::AgentIndex(_)
            | Self::Index { .. }
            | Self::GitStatus { .. }
            | Self::Spacer => None,
        }
    }
}

impl ResolvedToken {
    fn new(kind: ResolvedTokenKind, style: SidebarTokenStyle) -> Self {
        Self {
            kind,
            style,
            link: None,
        }
    }

    fn with_link(mut self, link: Option<String>) -> Self {
        self.link = link;
        self
    }

    #[cfg(test)]
    pub(super) fn unstyled(kind: ResolvedTokenKind) -> Self {
        Self::new(kind, SidebarTokenStyle::default())
    }
}

pub(crate) struct AgentTokenContext<'a> {
    pub(crate) machine: Option<&'a str>,
    pub(crate) workspace: &'a str,
    pub(crate) tab: Option<&'a str>,
    /// `tab` without its built-in jump-number prefix; `None` whenever `tab` is.
    pub(crate) tab_stripped: Option<&'a str>,
    /// One-based position of the tab among its space's tabs.
    pub(crate) tab_position: Option<usize>,
    pub(crate) pane: Option<&'a str>,
    pub(crate) agent_label: Option<&'a str>,
    /// Zero-based position of the agent in the agents panel list.
    pub(crate) agent_index: usize,
    /// Whether this agent is the focused pane; `agent_index_focused` resolves
    /// only then.
    pub(crate) focused: bool,
    pub(crate) terminal_title: Option<&'a str>,
    pub(crate) terminal_title_stripped: Option<&'a str>,
    pub(crate) tokens: &'a std::collections::HashMap<String, String>,
    /// API-reported links by token name.
    pub(crate) links: &'a [(String, String)],
}

/// Resolves one agent's token `rows`; callers choose the row set (per-agent
/// overrides for the agents panel, `agent_rows` for space children).
pub(crate) fn agent_rows(
    rows: &[Vec<AgentSidebarToken>],
    context: AgentTokenContext<'_>,
    state_text: &str,
) -> Vec<Vec<ResolvedToken>> {
    rows.iter()
        .filter_map(|row| {
            let resolved = row
                .iter()
                .filter_map(|configured| {
                    let (token, style) = configured.parts();
                    let mut custom_link = None;
                    let kind = match token {
                        AgentSidebarToken::StateIcon => Some(ResolvedTokenKind::StateIcon),
                        AgentSidebarToken::StateText => {
                            Some(ResolvedTokenKind::StateText(state_text.to_string()))
                        }
                        AgentSidebarToken::Machine => context
                            .machine
                            .map(|value| ResolvedTokenKind::Machine(value.to_string())),
                        AgentSidebarToken::Workspace => {
                            Some(ResolvedTokenKind::Workspace(context.workspace.to_string()))
                        }
                        AgentSidebarToken::Tab => context
                            .tab
                            .map(|value| ResolvedTokenKind::Tab(value.to_string())),
                        AgentSidebarToken::TabStripped => context.tab.map(|label| {
                            let stripped = match (configured.tab_prefix(), context.tab_position) {
                                (Some(prefix), Some(position)) => {
                                    strip_tab_prefix(label, position, prefix)
                                }
                                (Some(_), None) => label,
                                (None, _) => context.tab_stripped.unwrap_or(label),
                            };
                            ResolvedTokenKind::Tab(stripped.to_string())
                        }),
                        AgentSidebarToken::Pane => context
                            .pane
                            .map(|value| ResolvedTokenKind::Pane(value.to_string())),
                        AgentSidebarToken::Agent => context
                            .agent_label
                            .map(|value| ResolvedTokenKind::Agent(value.to_string())),
                        AgentSidebarToken::AgentIndex => {
                            Some(ResolvedTokenKind::AgentIndex(context.agent_index))
                        }
                        AgentSidebarToken::AgentIndexFocused => context
                            .focused
                            .then_some(ResolvedTokenKind::AgentIndex(context.agent_index)),
                        AgentSidebarToken::TerminalTitle => context
                            .terminal_title
                            .map(|value| ResolvedTokenKind::TerminalTitle(value.to_string())),
                        AgentSidebarToken::TerminalTitleStripped => context
                            .terminal_title_stripped
                            .map(|value| ResolvedTokenKind::TerminalTitle(value.to_string())),
                        AgentSidebarToken::Custom(name) => {
                            custom_link = custom_link_for(context.links, name);
                            context
                                .tokens
                                .get(name)
                                .cloned()
                                .map(ResolvedTokenKind::Custom)
                        }
                        AgentSidebarToken::Spacer => Some(ResolvedTokenKind::Spacer),
                        AgentSidebarToken::Styled { .. } => None,
                    }?;
                    let style = kind
                        .text_value()
                        .map_or(Some(style), |value| configured.style_for_value(value))?;
                    Some(ResolvedToken::new(kind, style).with_link(custom_link))
                })
                .collect::<Vec<_>>();
            finish_row(resolved)
        })
        .collect()
}

/// Removes the tab's own jump number, as written by a rename plugin, from the
/// start of `label`: an optional `[` or `(`, exactly the digits of `position`,
/// the matching close bracket, at most one symbol, then whitespace. Returns
/// `label` unchanged unless the prefix matches and text remains after it.
pub(crate) fn strip_tab_number(label: &str, position: usize) -> &str {
    let mut digits = [0u8; 20];
    let digits = format_digits(position, &mut digits);
    let mut rest = label;
    let close = match rest.chars().next() {
        Some('[') => Some(']'),
        Some('(') => Some(')'),
        _ => None,
    };
    if close.is_some() {
        rest = &rest[1..];
    }
    let Some(after) = rest.strip_prefix(digits) else {
        return label;
    };
    rest = after;
    if let Some(close) = close {
        let Some(after) = rest.strip_prefix(close) else {
            return label;
        };
        rest = after;
    }
    if let Some(first) = rest.chars().next() {
        if !first.is_alphanumeric() && !first.is_whitespace() {
            rest = &rest[first.len_utf8()..];
        }
    }
    let trimmed = rest.trim_start();
    if trimmed.len() == rest.len() || trimmed.is_empty() {
        return label;
    }
    trimmed
}

/// Removes exactly `before + position + after` from the start of `label`,
/// leaving `label` unchanged unless text remains after it.
fn strip_tab_prefix<'a>(
    label: &'a str,
    position: usize,
    prefix: &crate::config::PlaceholderTemplate,
) -> &'a str {
    let mut digits = [0u8; 20];
    let digits = format_digits(position, &mut digits);
    label
        .strip_prefix(prefix.before.as_str())
        .and_then(|rest| rest.strip_prefix(digits))
        .and_then(|rest| rest.strip_prefix(prefix.after.as_str()))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(label)
}

/// Writes the decimal digits of `value` into `buffer` without allocating.
fn format_digits(mut value: usize, buffer: &mut [u8; 20]) -> &str {
    let mut start = buffer.len();
    loop {
        start -= 1;
        buffer[start] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    // The slice holds only ASCII digits, so this cannot fail.
    std::str::from_utf8(&buffer[start..]).unwrap_or_default()
}

pub(crate) struct SpaceTokenContext<'a> {
    pub(crate) workspace: &'a str,
    /// Jump-number key character, `None` for a space without a number.
    pub(crate) jump_number: Option<char>,
    pub(crate) branch: Option<&'a str>,
    pub(crate) state_text: &'a str,
    pub(crate) ahead_behind: Option<(usize, usize)>,
    pub(crate) tokens: &'a std::collections::HashMap<String, String>,
    /// API-reported links by token name.
    pub(crate) links: &'a [(String, String)],
    pub(crate) suppress_git_details: bool,
    /// Whether this is the focused space, for tokens' `focused` styles.
    pub(crate) focused: bool,
}

pub(crate) fn space_rows(
    config: &SpacesSidebarConfig,
    context: SpaceTokenContext<'_>,
) -> Vec<Vec<ResolvedToken>> {
    config
        .rows
        .iter()
        .filter_map(|row| {
            let resolved = row
                .iter()
                .filter_map(|configured| {
                    let (token, style) = configured.parts();
                    let mut custom_link = None;
                    let kind = match token {
                        SpaceSidebarToken::Index => {
                            context.jump_number.map(|digit| ResolvedTokenKind::Index {
                                digit,
                                text: match configured.index_format() {
                                    Some(format) => {
                                        format!("{}{digit}{}", format.before, format.after)
                                    }
                                    None => digit.to_string(),
                                },
                            })
                        }
                        SpaceSidebarToken::StateIcon if config.show_status => {
                            Some(ResolvedTokenKind::StateIcon)
                        }
                        SpaceSidebarToken::StateText if config.show_status => {
                            Some(ResolvedTokenKind::StateText(context.state_text.to_string()))
                        }
                        SpaceSidebarToken::StateIcon | SpaceSidebarToken::StateText => None,
                        SpaceSidebarToken::Workspace => {
                            Some(ResolvedTokenKind::Workspace(context.workspace.to_string()))
                        }
                        SpaceSidebarToken::Branch if !context.suppress_git_details => context
                            .branch
                            .map(|branch| ResolvedTokenKind::Branch(branch.to_string())),
                        SpaceSidebarToken::Branch => None,
                        SpaceSidebarToken::GitStatus if !context.suppress_git_details => context
                            .ahead_behind
                            .filter(|(ahead, behind)| *ahead > 0 || *behind > 0)
                            .map(|(ahead, behind)| ResolvedTokenKind::GitStatus { ahead, behind }),
                        SpaceSidebarToken::GitStatus => None,
                        SpaceSidebarToken::Custom(name) => {
                            custom_link = custom_link_for(context.links, name);
                            context
                                .tokens
                                .get(name)
                                .cloned()
                                .map(ResolvedTokenKind::Custom)
                        }
                        SpaceSidebarToken::Spacer => Some(ResolvedTokenKind::Spacer),
                        SpaceSidebarToken::Styled { .. } => None,
                    }?;
                    let style = match &kind {
                        ResolvedTokenKind::Index { digit, .. } => {
                            configured.style_for_value(digit.encode_utf8(&mut [0; 4]))?
                        }
                        _ => kind
                            .text_value()
                            .map_or(Some(style), |value| configured.style_for_value(value))?,
                    };
                    let style = match configured.focused_style() {
                        Some(focused) if context.focused => style.overlaid(focused),
                        _ => style,
                    };
                    Some(ResolvedToken::new(kind, style).with_link(custom_link))
                })
                .collect::<Vec<_>>();
            finish_row(resolved)
        })
        .collect()
}

/// Keeps only the first spacer of a row and drops a row with nothing but
/// spacers, so an alignment marker never keeps an otherwise empty row alive.
fn finish_row(mut resolved: Vec<ResolvedToken>) -> Option<Vec<ResolvedToken>> {
    let mut seen_spacer = false;
    resolved.retain(|token| {
        if !matches!(token.kind, ResolvedTokenKind::Spacer) {
            return true;
        }
        !std::mem::replace(&mut seen_spacer, true)
    });
    resolved
        .iter()
        .any(|token| !matches!(token.kind, ResolvedTokenKind::Spacer))
        .then_some(resolved)
}

fn custom_link_for(links: &[(String, String)], name: &str) -> Option<String> {
    links
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, url)| url.clone())
}

/// Text between two adjacent visible tokens. A spacer supplies its own gap.
pub(crate) fn separator(previous: &ResolvedToken, current: &ResolvedToken) -> &'static str {
    if matches!(previous.kind, ResolvedTokenKind::Spacer)
        || matches!(current.kind, ResolvedTokenKind::Spacer)
    {
        ""
    } else {
        " "
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentSidebarToken, AgentsSidebarConfig, SpaceSidebarToken};

    struct Entry {
        workspace: String,
        tab: Option<String>,
        tab_stripped: Option<String>,
        tab_position: Option<usize>,
        pane: Option<String>,
        agent_label: Option<String>,
        terminal_title: Option<String>,
        terminal_title_stripped: Option<String>,
        canonical_agent: Option<crate::detect::Agent>,
        tokens: std::collections::HashMap<String, String>,
        links: Vec<(String, String)>,
    }

    fn entry() -> Entry {
        Entry {
            workspace: "repo".into(),
            tab: None,
            tab_stripped: None,
            tab_position: None,
            pane: None,
            agent_label: Some("pi".into()),
            terminal_title: None,
            terminal_title_stripped: None,
            canonical_agent: Some(crate::detect::Agent::Pi),
            tokens: std::collections::HashMap::new(),
            links: Vec::new(),
        }
    }

    impl Entry {
        fn rows<'a>(&self, config: &'a AgentsSidebarConfig) -> &'a [Vec<AgentSidebarToken>] {
            config.rows_for_agent(self.canonical_agent)
        }
    }

    fn context(entry: &Entry) -> AgentTokenContext<'_> {
        AgentTokenContext {
            machine: None,
            workspace: &entry.workspace,
            tab: entry.tab.as_deref(),
            tab_stripped: entry.tab_stripped.as_deref(),
            tab_position: entry.tab_position,
            pane: entry.pane.as_deref(),
            agent_label: entry.agent_label.as_deref(),
            agent_index: 0,
            focused: false,
            terminal_title: entry.terminal_title.as_deref(),
            terminal_title_stripped: entry.terminal_title_stripped.as_deref(),
            tokens: &entry.tokens,
            links: &entry.links,
        }
    }

    #[test]
    fn conditional_styles_merge_first_match_and_keep_missing_values_absent() {
        let config: AgentsSidebarConfig = toml::from_str(r##"
rows = [["state_icon", { token = "machine", fg = "#fff", bold = true, dim = true, rules = [{ equals = "Local", fg = "#f00", bold = false }, { contains = "Loc", fg = "#0f0", dim = false }] }], [{ token = "$missing", rules = [{ equals = "", bold = true }] }]]
"##).unwrap();
        let entry = entry();
        for (machine, color, bold, dim) in [
            ("Local", (255, 0, 0), false, true),
            ("Localhost", (0, 255, 0), true, false),
            ("local", (255, 255, 255), true, true),
        ] {
            let mut context = context(&entry);
            context.machine = Some(machine);
            let rows = agent_rows(entry.rows(&config), context, "working");
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows[0][0],
                ResolvedToken::unstyled(ResolvedTokenKind::StateIcon)
            );
            let token = &rows[0][1];
            assert_eq!(token.kind, ResolvedTokenKind::Machine(machine.into()));
            assert_eq!(
                token.style.fg.unwrap().ratatui(),
                ratatui::style::Color::Rgb(color.0, color.1, color.2)
            );
            assert_eq!(token.style.bold, Some(bold));
            assert_eq!(token.style.dim, Some(dim));
        }
        assert_eq!(
            agent_rows(entry.rows(&config), context(&entry), "working")[0].len(),
            1
        );
    }

    #[test]
    fn conditional_style_survives_truncation_and_removes_theme_modifiers() {
        use ratatui::style::{Color, Modifier, Style};
        let config: AgentsSidebarConfig = toml::from_str(r##"
rows = [[{ token = "workspace", rules = [{ equals = "long-workspace-name", fg = "#f00", bold = false, dim = false }] }]]
"##).unwrap();
        let mut entry = entry();
        entry.workspace = "long-workspace-name".into();
        let rows = agent_rows(entry.rows(&config), context(&entry), "working");
        let theme = Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD | Modifier::DIM);
        for width in [4, 40] {
            let spans = super::super::resolved_token_spans(
                &rows[0],
                ("*", theme),
                theme,
                theme,
                theme,
                theme,
                &super::super::Palette::catppuccin(),
                width,
            );
            assert_eq!(spans.len(), 1);
            assert!(super::super::display_width(&spans[0].content) <= width);
            assert_eq!(spans[0].style.fg, Some(Color::Rgb(255, 0, 0)));
            assert!(!spans[0]
                .style
                .add_modifier
                .intersects(Modifier::BOLD | Modifier::DIM));
            assert!(spans[0]
                .style
                .sub_modifier
                .contains(Modifier::BOLD | Modifier::DIM));
        }
    }

    #[test]
    fn custom_numeric_rules_resolve_in_agent_overrides_and_space_rows() {
        let config: crate::config::SidebarConfig = toml::from_str(
            r#"
[agents]
rows = [["workspace"]]
[agents.rows_by_agent]
pi = [[{ token = "$load", rules = [{ gt = 80, bold = true }, { gt = 50, dim = true }] }]]
[spaces]
rows = [[{ token = "$load", rules = [{ lt = 50, dim = true }] }]]
"#,
        )
        .unwrap();
        let mut entry = entry();
        for (value, bold, dim) in [
            ("90", Some(true), None),
            ("60", None, Some(true)),
            ("20", None, None),
            ("90%", None, None),
        ] {
            entry.tokens.insert("load".into(), value.into());
            let rows = agent_rows(entry.rows(&config.agents), context(&entry), "working");
            assert_eq!(rows[0][0].kind, ResolvedTokenKind::Custom(value.into()));
            assert_eq!(rows[0][0].style.bold, bold);
            assert_eq!(rows[0][0].style.dim, dim);
            let spaces = space_rows(
                &config.spaces,
                SpaceTokenContext {
                    workspace: "repo",
                    jump_number: None,
                    branch: None,
                    state_text: "working",
                    ahead_behind: None,
                    suppress_git_details: false,
                    focused: false,
                    tokens: &entry.tokens,
                    links: &entry.links,
                },
            );
            assert_eq!(spaces[0][0].style.dim, (value == "20").then_some(true));
        }
    }

    #[test]
    fn conditional_hide_removes_tokens_and_empty_rows() {
        let config: crate::config::SidebarConfig = toml::from_str(
            r##"
[agents]
rows = [[{ token = "machine", fg = "#61afef", rules = [{ equals = "Local", hide = true }] }, "agent"]]
[agents.rows_by_agent]
pi = [[{ token = "$load", rules = [{ lt = 50, hide = true }] }], ["agent"]]
[spaces]
rows = [[{ token = "$load", rules = [{ lt = 50, hide = true }] }], ["workspace"]]
"##,
        ).unwrap();
        let encoded = toml::to_string(&config).unwrap();
        assert!(encoded.contains("hide = true"));
        let config: crate::config::SidebarConfig = toml::from_str(&encoded).unwrap();
        let mut entry = entry();
        entry.canonical_agent = None;
        for (machine, count) in [("Local", 1), ("Remote", 2)] {
            let mut ctx = context(&entry);
            ctx.machine = Some(machine);
            let rows = agent_rows(entry.rows(&config.agents), ctx, "working");
            assert_eq!(rows[0].len(), count);
            assert_eq!(
                rows[0].last().unwrap().kind,
                ResolvedTokenKind::Agent("pi".into())
            );
        }
        entry.canonical_agent = Some(crate::detect::Agent::Pi);
        for (value, count) in [("20", 1), ("90", 2)] {
            entry.tokens.insert("load".into(), value.into());
            assert_eq!(
                agent_rows(entry.rows(&config.agents), context(&entry), "working").len(),
                count
            );
            let rows = space_rows(
                &config.spaces,
                SpaceTokenContext {
                    workspace: "repo",
                    jump_number: None,
                    branch: None,
                    state_text: "working",
                    ahead_behind: None,
                    suppress_git_details: false,
                    focused: false,
                    tokens: &entry.tokens,
                    links: &entry.links,
                },
            );
            assert_eq!(rows.len(), count);
        }
    }

    #[test]
    fn conditional_hide_preserves_first_match_wins() {
        for first in ["hide = false", "bold = true"] {
            let config: AgentsSidebarConfig = toml::from_str(&format!(
                "rows = [[{{ token = 'agent', rules = [{{ equals = 'pi', {first} }}, {{ contains = '', hide = true }}] }}]]"
            )).unwrap();
            let entry = entry();
            let rows = agent_rows(entry.rows(&config), context(&entry), "working");
            assert_eq!(rows[0][0].kind, ResolvedTokenKind::Agent("pi".into()));
        }
    }

    #[test]
    fn missing_custom_tokens_elide_rows_and_separators() {
        let entry = entry();
        let config = AgentsSidebarConfig {
            rows: vec![
                vec![
                    AgentSidebarToken::StateIcon,
                    AgentSidebarToken::Custom("missing".into()),
                ],
                vec![AgentSidebarToken::Custom("missing".into())],
                vec![AgentSidebarToken::Agent],
            ],
            ..Default::default()
        };

        let rows = agent_rows(entry.rows(&config), context(&entry), "working");

        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            vec![ResolvedToken::unstyled(ResolvedTokenKind::StateIcon)]
        );
        assert_eq!(
            rows[1],
            vec![ResolvedToken::unstyled(ResolvedTokenKind::Agent(
                "pi".into()
            ))]
        );
    }

    #[test]
    fn machine_token_only_resolves_for_multi_machine_rows() {
        let entry = entry();
        let config = AgentsSidebarConfig {
            rows: vec![vec![
                AgentSidebarToken::Machine,
                AgentSidebarToken::Workspace,
            ]],
            ..Default::default()
        };

        assert_eq!(
            agent_rows(entry.rows(&config), context(&entry), "working"),
            vec![vec![ResolvedToken::unstyled(ResolvedTokenKind::Workspace(
                "repo".into()
            ))]]
        );

        let mut remote_context = context(&entry);
        remote_context.machine = Some("Build");
        assert_eq!(
            agent_rows(entry.rows(&config), remote_context, "working"),
            vec![vec![
                ResolvedToken::unstyled(ResolvedTokenKind::Machine("Build".into())),
                ResolvedToken::unstyled(ResolvedTokenKind::Workspace("repo".into())),
            ]]
        );
    }

    #[test]
    fn state_text_and_arbitrary_values_are_independent_tokens() {
        let mut entry = entry();
        entry
            .tokens
            .insert("summary".into(), "reviewing auth".into());
        let config = AgentsSidebarConfig {
            rows: vec![vec![
                AgentSidebarToken::StateText,
                AgentSidebarToken::Custom("summary".into()),
            ]],
            ..Default::default()
        };

        assert_eq!(
            agent_rows(entry.rows(&config), context(&entry), "deep in the mines"),
            vec![vec![
                ResolvedToken::unstyled(ResolvedTokenKind::StateText("deep in the mines".into())),
                ResolvedToken::unstyled(ResolvedTokenKind::Custom("reviewing auth".into())),
            ]]
        );
    }

    #[test]
    fn terminal_title_builtins_are_distinct_from_custom_tokens() {
        let mut entry = entry();
        entry.terminal_title = Some("⠋ raw title".into());
        entry.terminal_title_stripped = Some("raw title".into());
        entry
            .tokens
            .insert("terminal_title".into(), "custom title".into());
        let config = AgentsSidebarConfig {
            rows: vec![vec![
                AgentSidebarToken::TerminalTitle,
                AgentSidebarToken::TerminalTitleStripped,
                AgentSidebarToken::Custom("terminal_title".into()),
            ]],
            ..Default::default()
        };

        assert_eq!(
            agent_rows(entry.rows(&config), context(&entry), "working"),
            vec![vec![
                ResolvedToken::unstyled(ResolvedTokenKind::TerminalTitle("⠋ raw title".into())),
                ResolvedToken::unstyled(ResolvedTokenKind::TerminalTitle("raw title".into())),
                ResolvedToken::unstyled(ResolvedTokenKind::Custom("custom title".into())),
            ]]
        );
    }

    #[test]
    fn agent_index_focused_token_renders_the_index_only_when_focused() {
        let entry = entry();
        let config = AgentsSidebarConfig {
            rows: vec![vec![
                AgentSidebarToken::AgentIndexFocused,
                AgentSidebarToken::Agent,
            ]],
            ..Default::default()
        };
        let mut focused = context(&entry);
        focused.agent_index = 2;
        focused.focused = true;
        assert_eq!(
            agent_rows(entry.rows(&config), focused, "working"),
            vec![vec![
                ResolvedToken::unstyled(ResolvedTokenKind::AgentIndex(2)),
                ResolvedToken::unstyled(ResolvedTokenKind::Agent("pi".into())),
            ]]
        );

        // Unfocused, the token resolves to nothing; the rest of the row stays.
        let mut unfocused = context(&entry);
        unfocused.agent_index = 2;
        assert_eq!(
            agent_rows(entry.rows(&config), unfocused, "working"),
            vec![vec![ResolvedToken::unstyled(ResolvedTokenKind::Agent(
                "pi".into()
            ))]]
        );

        // A row holding only the token disappears when it is unfocused.
        let only = AgentsSidebarConfig {
            rows: vec![vec![AgentSidebarToken::AgentIndexFocused]],
            ..Default::default()
        };
        assert!(agent_rows(entry.rows(&only), context(&entry), "working").is_empty());
    }

    #[test]
    fn agent_index_token_renders_one_based_list_position() {
        let entry = entry();
        let config = AgentsSidebarConfig {
            rows: vec![vec![
                AgentSidebarToken::AgentIndex,
                AgentSidebarToken::Agent,
            ]],
            ..Default::default()
        };
        let mut context = context(&entry);
        context.agent_index = 11;
        let rows = agent_rows(entry.rows(&config), context, "working");
        assert_eq!(
            rows,
            vec![vec![
                ResolvedToken::unstyled(ResolvedTokenKind::AgentIndex(11)),
                ResolvedToken::unstyled(ResolvedTokenKind::Agent("pi".into())),
            ]]
        );

        let style = ratatui::style::Style::default();
        let text = |width| {
            super::super::resolved_token_spans(
                &rows[0],
                ("*", style),
                style,
                style,
                style,
                style,
                &super::super::Palette::catppuccin(),
                width,
            )
            .iter()
            .map(|span| span.content.to_string())
            .collect::<String>()
        };
        assert_eq!(text(40), "12 pi");
        assert_eq!(text(4), "12 …");
    }

    #[test]
    fn known_agent_override_replaces_default_rows() {
        let mut config = AgentsSidebarConfig {
            rows: vec![vec![AgentSidebarToken::Workspace]],
            ..Default::default()
        };
        config
            .rows_by_agent
            .insert("pi".into(), vec![vec![AgentSidebarToken::Agent]]);
        let mut pi = entry();
        pi.agent_label = Some("renamed pi".into());

        assert_eq!(
            agent_rows(pi.rows(&config), context(&pi), "working"),
            vec![vec![ResolvedToken::unstyled(ResolvedTokenKind::Agent(
                "renamed pi".into()
            ))]]
        );

        pi.canonical_agent = None;
        assert_eq!(
            agent_rows(pi.rows(&config), context(&pi), "working"),
            vec![vec![ResolvedToken::unstyled(ResolvedTokenKind::Workspace(
                "repo".into()
            ))]]
        );
    }

    #[test]
    fn grouped_children_suppress_all_builtin_git_details() {
        let config = SpacesSidebarConfig::default();

        assert_eq!(
            space_rows(
                &config,
                SpaceTokenContext {
                    workspace: "feature",
                    jump_number: None,
                    branch: Some("worktree/feature"),
                    state_text: "idle",
                    ahead_behind: Some((2, 1)),
                    tokens: &std::collections::HashMap::new(),
                    links: &[],
                    suppress_git_details: true,
                    focused: false,
                },
            ),
            vec![vec![
                ResolvedToken::unstyled(ResolvedTokenKind::StateIcon),
                ResolvedToken::unstyled(ResolvedTokenKind::Workspace("feature".into())),
            ]]
        );
    }

    #[test]
    fn index_token_resolves_to_the_jump_number_and_is_elided_without_one() {
        let config = SpacesSidebarConfig {
            rows: vec![vec![SpaceSidebarToken::Index, SpaceSidebarToken::Workspace]],
            ..Default::default()
        };
        let tokens = std::collections::HashMap::new();
        let resolve = |jump_number| {
            space_rows(
                &config,
                SpaceTokenContext {
                    workspace: "repo",
                    jump_number,
                    branch: None,
                    state_text: "idle",
                    ahead_behind: None,
                    tokens: &tokens,
                    links: &[],
                    suppress_git_details: false,
                    focused: false,
                },
            )
        };
        assert_eq!(
            resolve(Some('0')),
            vec![vec![
                ResolvedToken::unstyled(ResolvedTokenKind::Index {
                    digit: '0',
                    text: "0".into()
                }),
                ResolvedToken::unstyled(ResolvedTokenKind::Workspace("repo".into())),
            ]]
        );
        assert_eq!(
            resolve(None),
            vec![vec![ResolvedToken::unstyled(ResolvedTokenKind::Workspace(
                "repo".into()
            ))]]
        );
    }

    #[test]
    fn workspace_custom_token_can_replace_git_specific_details() {
        let tokens = std::collections::HashMap::from([("jj_status".into(), "2 changes".into())]);
        let config = SpacesSidebarConfig {
            rows: vec![vec![SpaceSidebarToken::Custom("jj_status".into())]],
            ..Default::default()
        };

        assert_eq!(
            space_rows(
                &config,
                SpaceTokenContext {
                    workspace: "repo",
                    jump_number: None,
                    branch: None,
                    state_text: "idle",
                    ahead_behind: None,
                    tokens: &tokens,
                    links: &[],
                    suppress_git_details: false,
                    focused: false,
                },
            ),
            vec![vec![ResolvedToken::unstyled(ResolvedTokenKind::Custom(
                "2 changes".into()
            ))]]
        );
    }

    #[test]
    fn show_status_false_hides_space_status_tokens_only() {
        let config = SpacesSidebarConfig {
            rows: vec![vec![
                SpaceSidebarToken::StateIcon,
                SpaceSidebarToken::Workspace,
                SpaceSidebarToken::StateText,
            ]],
            show_status: false,
            ..Default::default()
        };

        assert_eq!(
            space_rows(
                &config,
                SpaceTokenContext {
                    workspace: "repo",
                    jump_number: None,
                    branch: None,
                    state_text: "working",
                    ahead_behind: None,
                    tokens: &std::collections::HashMap::new(),
                    links: &[],
                    suppress_git_details: false,
                    focused: false,
                },
            ),
            vec![vec![ResolvedToken::unstyled(ResolvedTokenKind::Workspace(
                "repo".into()
            ))]]
        );
    }
    #[test]
    fn links_attach_only_to_custom_tokens_with_an_api_link() {
        let config: crate::config::SidebarConfig = toml::from_str(
            r#"
[agents]
rows = [["workspace", "$pr", "$plain"]]
[spaces]
rows = [["workspace", "$pr", "$plain", "$env"]]
"#,
        )
        .unwrap();
        let mut entry = entry();
        entry.tokens.insert("pr".into(), "#7".into());
        entry.tokens.insert("plain".into(), "x".into());
        entry.links = vec![
            ("pr".into(), "https://example.com/7".into()),
            // A link for a token without a value, or a built-in, is ignored.
            ("workspace".into(), "https://example.com/w".into()),
            ("missing".into(), "https://example.com/m".into()),
        ];
        let rows = agent_rows(entry.rows(&config.agents), context(&entry), "working");
        assert_eq!(rows[0][0].link, None);
        assert_eq!(rows[0][1].link.as_deref(), Some("https://example.com/7"));
        assert_eq!(rows[0][2].link, None);

        // A token without an entry in the links has no link; adding one links
        // exactly that token.
        let mut tokens = entry.tokens.clone();
        tokens.insert("env".into(), "staging".into());
        let space = |tokens: &std::collections::HashMap<String, String>,
                     links: &[(String, String)]| {
            space_rows(
                &config.spaces,
                SpaceTokenContext {
                    workspace: "repo",
                    jump_number: None,
                    branch: None,
                    state_text: "idle",
                    ahead_behind: None,
                    tokens,
                    links,
                    suppress_git_details: false,
                    focused: false,
                },
            )
        };
        let rows = space(&tokens, &entry.links);
        assert_eq!(rows[0][3].kind, ResolvedTokenKind::Custom("staging".into()));
        assert_eq!(rows[0][3].link, None);
        assert_eq!(rows[0][1].link.as_deref(), Some("https://example.com/7"));
        let overriding = [("env".to_string(), "https://example.com/env".to_string())];
        let rows = space(&tokens, &overriding);
        assert_eq!(rows[0][3].link.as_deref(), Some("https://example.com/env"));
        assert_eq!(rows[0][1].link, None);
    }

    #[test]
    fn formatted_index_token_wraps_the_digit_and_is_elided_without_a_number() {
        let config: SpacesSidebarConfig = toml::from_str(
            r##"rows = [[{token = "index", format = "[{}]", rules = [{equals = "0", fg = "#ff0000"}]}, "workspace"]]"##,
        )
        .unwrap();
        let tokens = std::collections::HashMap::new();
        let resolve = |jump_number| {
            space_rows(
                &config,
                SpaceTokenContext {
                    workspace: "repo",
                    jump_number,
                    branch: None,
                    state_text: "idle",
                    ahead_behind: None,
                    tokens: &tokens,
                    links: &[],
                    suppress_git_details: false,
                    focused: false,
                },
            )
        };
        let first_token = |jump_number| resolve(jump_number).remove(0).remove(0);
        for (digit, text) in [('1', "[1]"), ('2', "[2]"), ('0', "[0]")] {
            let token = first_token(Some(digit));
            assert_eq!(
                token.kind,
                ResolvedTokenKind::Index {
                    digit,
                    text: text.into()
                }
            );
            // The rule matches the bare digit and styles the whole text.
            assert_eq!(token.style.fg.is_some(), digit == '0', "{digit}");
        }
        assert_eq!(
            resolve(None),
            vec![vec![ResolvedToken::unstyled(ResolvedTokenKind::Workspace(
                "repo".into()
            ))]]
        );
    }

    #[test]
    fn strip_tab_number_removes_only_the_tabs_own_prefix() {
        for (label, position, expected) in [
            ("1| zsh", 1, "zsh"),
            ("[1] build", 1, "build"),
            ("(1) x", 1, "x"),
            ("1. x", 1, "x"),
            ("1 \u{258f} x", 1, "\u{258f} x"),
            ("12| x", 12, "x"),
            ("2024 report", 1, "2024 report"),
            ("2d plan", 2, "2d plan"),
            ("3| zsh", 1, "3| zsh"),
            ("1|| x", 1, "1|| x"),
            ("1| ", 1, "1| "),
            (
                "1| \u{4fee}\u{590d}\u{1f642} title",
                1,
                "\u{4fee}\u{590d}\u{1f642} title",
            ),
        ] {
            assert_eq!(strip_tab_number(label, position), expected, "{label:?}");
        }
        // The stripped text is a subslice of the label.
        let label = "2| zsh";
        let stripped = strip_tab_number(label, 2);
        assert!(std::ptr::eq(stripped.as_ptr(), label[3..].as_ptr()));
    }

    fn tab_stripped_text(
        config: &str,
        tab: &str,
        position: usize,
    ) -> Option<(ResolvedTokenKind, SidebarTokenStyle)> {
        let config: AgentsSidebarConfig = toml::from_str(config).unwrap();
        let mut entry = entry();
        entry.tab = Some(tab.into());
        entry.tab_stripped = Some(strip_tab_number(tab, position).to_string());
        entry.tab_position = Some(position);
        let rows = agent_rows(entry.rows(&config), context(&entry), "idle");
        rows.into_iter()
            .next()
            .and_then(|row| row.into_iter().next())
            .map(|token| (token.kind, token.style))
    }

    #[test]
    fn tab_stripped_resolves_the_stripped_name_and_matches_rules_against_it() {
        let plain = tab_stripped_text(r#"rows = [["tab_stripped"]]"#, "1| zsh", 1);
        assert_eq!(plain.unwrap().0, ResolvedTokenKind::Tab("zsh".into()));
        let tab = tab_stripped_text(r#"rows = [["tab"]]"#, "1| zsh", 1);
        assert_eq!(tab.unwrap().0, ResolvedTokenKind::Tab("1| zsh".into()));
        let ruled = tab_stripped_text(
            r##"rows = [[{ token = "tab_stripped", rules = [{ equals = "zsh", fg = "#ff0000" }] }]]"##,
            "1| zsh",
            1,
        )
        .unwrap();
        assert!(ruled.1.fg.is_some());
        let unmatched = tab_stripped_text(
            r##"rows = [[{ token = "tab_stripped", rules = [{ equals = "zsh", fg = "#ff0000" }] }]]"##,
            "1| fish",
            1,
        )
        .unwrap();
        assert!(unmatched.1.fg.is_none());
    }

    #[test]
    fn tab_stripped_is_elided_when_tab_is() {
        let config: AgentsSidebarConfig =
            toml::from_str(r#"rows = [["tab", "tab_stripped"], ["workspace"]]"#).unwrap();
        let entry = entry();
        let rows = agent_rows(entry.rows(&config), context(&entry), "idle");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 1);
    }

    #[test]
    fn tab_stripped_prefix_replaces_the_built_in_rule() {
        let resolve = |prefix: &str, tab: &str, position| {
            let config =
                format!("rows = [[{{ token = \"tab_stripped\", prefix = \"{prefix}\" }}]]");
            match tab_stripped_text(&config, tab, position).unwrap().0 {
                ResolvedTokenKind::Tab(text) => text,
                other => panic!("unexpected {other:?}"),
            }
        };
        assert_eq!(resolve("{}| ", "2| zsh", 2), "zsh");
        assert_eq!(
            resolve(
                "{}    \u{f0142} \u{2502} ",
                "2    \u{f0142} \u{2502} zsh",
                2
            ),
            "zsh"
        );
        // The built-in rule would strip this; an explicit prefix disables it.
        assert_eq!(resolve("[{}] ", "1| zsh", 1), "1| zsh");
        assert_eq!(resolve("{}| ", "2| zsh", 1), "2| zsh");
        assert_eq!(resolve("{}| ", "2| ", 2), "2| ");
    }

    #[test]
    fn rows_keep_one_spacer_and_drop_spacer_only_rows() {
        let spacer = || ResolvedToken::unstyled(ResolvedTokenKind::Spacer);
        let custom = || ResolvedToken::unstyled(ResolvedTokenKind::Custom("x".into()));
        let row = finish_row(vec![custom(), spacer(), custom(), spacer(), custom()])
            .expect("row with content");
        assert_eq!(
            row.iter()
                .filter(|token| matches!(token.kind, ResolvedTokenKind::Spacer))
                .count(),
            1
        );
        assert_eq!(row.len(), 4);
        assert!(finish_row(vec![spacer(), spacer()]).is_none());
        assert!(finish_row(Vec::new()).is_none());
    }
}
