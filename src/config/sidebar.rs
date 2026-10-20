mod rules;

pub use rules::SidebarTokenRule;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::detect::Agent;

const MAX_SIDEBAR_ROWS: usize = 16;
const MAX_SIDEBAR_TOKENS_PER_ROW: usize = 16;
const DEFAULT_SIDEBAR_ROW_GAP: u16 = 0;

fn deserialize_sidebar_rows<'de, D, T>(deserializer: D) -> Result<Vec<Vec<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let rows = Vec::<Vec<T>>::deserialize(deserializer)?;
    validate_sidebar_rows(&rows).map_err(serde::de::Error::custom)?;
    Ok(rows)
}

fn validate_sidebar_rows<T>(rows: &[Vec<T>]) -> Result<(), String> {
    if rows.len() > MAX_SIDEBAR_ROWS {
        return Err(format!(
            "sidebar layouts may contain at most {MAX_SIDEBAR_ROWS} rows"
        ));
    }
    if rows
        .iter()
        .any(|row| row.len() > MAX_SIDEBAR_TOKENS_PER_ROW)
    {
        return Err(format!(
            "sidebar rows may contain at most {MAX_SIDEBAR_TOKENS_PER_ROW} tokens"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarTokenColor {
    r: u8,
    g: u8,
    b: u8,
}

impl SidebarTokenColor {
    pub(crate) fn ratatui(self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(self.r, self.g, self.b)
    }
}

impl Serialize for SidebarTokenColor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b))
    }
}

impl<'de> Deserialize<'de> for SidebarTokenColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let hex = value.strip_prefix('#').filter(|hex| {
            hex.is_ascii()
                && matches!(hex.len(), 3 | 6)
                && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        let Some(hex) = hex else {
            return Err(serde::de::Error::custom(
                "sidebar token fg must be #RGB or #RRGGBB",
            ));
        };
        let (r, g, b) = if hex.len() == 3 {
            let mut digits = hex
                .bytes()
                .map(|byte| char::from(byte).to_digit(16).expect("validated hex digit") as u8 * 17);
            (
                digits.next().expect("three hex digits"),
                digits.next().expect("three hex digits"),
                digits.next().expect("three hex digits"),
            )
        } else {
            (
                u8::from_str_radix(&hex[0..2], 16).expect("validated hex digits"),
                u8::from_str_radix(&hex[2..4], 16).expect("validated hex digits"),
                u8::from_str_radix(&hex[4..6], 16).expect("validated hex digits"),
            )
        };
        Ok(Self { r, g, b })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SidebarTokenStyle {
    pub fg: Option<SidebarTokenColor>,
    pub bold: Option<bool>,
    pub dim: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSidebarToken {
    StateIcon,
    StateText,
    Machine,
    Workspace,
    Tab,
    Pane,
    Agent,
    AgentIndex,
    TerminalTitle,
    TerminalTitleStripped,
    Custom(String),
    Styled {
        token: Box<AgentSidebarToken>,
        style: SidebarTokenStyle,
        rules: Vec<SidebarTokenRule>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceSidebarToken {
    StateIcon,
    StateText,
    Workspace,
    Branch,
    GitStatus,
    Custom(String),
    Styled {
        token: Box<SpaceSidebarToken>,
        style: SidebarTokenStyle,
        rules: Vec<SidebarTokenRule>,
    },
}

impl AgentSidebarToken {
    pub(crate) fn style_for_value(&self, value: &str) -> Option<SidebarTokenStyle> {
        match self {
            Self::Styled { style, rules, .. } => rules::matching_style(rules, *style, value),
            _ => Some(SidebarTokenStyle::default()),
        }
    }

    pub(crate) fn parts(&self) -> (&Self, SidebarTokenStyle) {
        match self {
            Self::Styled { token, style, .. } => (token, *style),
            token => (token, SidebarTokenStyle::default()),
        }
    }
}

impl SpaceSidebarToken {
    pub(crate) fn style_for_value(&self, value: &str) -> Option<SidebarTokenStyle> {
        match self {
            Self::Styled { style, rules, .. } => rules::matching_style(rules, *style, value),
            _ => Some(SidebarTokenStyle::default()),
        }
    }

    pub(crate) fn parts(&self) -> (&Self, SidebarTokenStyle) {
        match self {
            Self::Styled { token, style, .. } => (token, *style),
            token => (token, SidebarTokenStyle::default()),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStyledSidebarToken {
    token: String,
    #[serde(default)]
    fg: Option<SidebarTokenColor>,
    #[serde(default)]
    bold: Option<bool>,
    #[serde(default)]
    dim: Option<bool>,
    #[serde(default)]
    rules: Vec<SidebarTokenRule>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawSidebarToken {
    Plain(String),
    Styled(RawStyledSidebarToken),
}

impl RawSidebarToken {
    fn parts(self) -> Result<(String, Option<SidebarTokenStyle>, Vec<SidebarTokenRule>), String> {
        match self {
            Self::Plain(token) => Ok((token, None, Vec::new())),
            Self::Styled(token) => {
                if token.rules.len() > 16 {
                    return Err("sidebar tokens may contain at most 16 rules".into());
                }
                if !token.rules.is_empty()
                    && matches!(token.token.as_str(), "state_icon" | "git_status")
                {
                    return Err("sidebar rules require a text-valued token".into());
                }
                Ok((
                    token.token,
                    Some(SidebarTokenStyle {
                        fg: token.fg,
                        bold: token.bold,
                        dim: token.dim,
                    }),
                    token.rules,
                ))
            }
        }
    }
}

fn parse_sidebar_token<T>(value: String, builtins: &[(&str, T)]) -> Result<T, String>
where
    T: Clone + From<String>,
{
    if let Some((_, token)) = builtins.iter().find(|(name, _)| *name == value) {
        return Ok(token.clone());
    }
    let Some(name) = value.strip_prefix('$') else {
        return Err(format!(
            "unknown sidebar token `{value}`; custom tokens must start with `$`"
        ));
    };
    if name.is_empty()
        || name.len() > 32
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return Err(format!("invalid custom sidebar token `{value}`"));
    }
    Ok(T::from(name.to_string()))
}

fn serialize_styled_token<S>(
    name: String,
    style: SidebarTokenStyle,
    rules: &[SidebarTokenRule],
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(None)?;
    map.serialize_entry("token", &name)?;
    if let Some(fg) = style.fg {
        map.serialize_entry("fg", &fg)?;
    }
    if let Some(bold) = style.bold {
        map.serialize_entry("bold", &bold)?;
    }
    if let Some(dim) = style.dim {
        map.serialize_entry("dim", &dim)?;
    }
    if !rules.is_empty() {
        map.serialize_entry("rules", rules)?;
    }
    map.end()
}

fn agent_token_name(token: &AgentSidebarToken) -> String {
    match token {
        AgentSidebarToken::StateIcon => "state_icon".into(),
        AgentSidebarToken::StateText => "state_text".into(),
        AgentSidebarToken::Machine => "machine".into(),
        AgentSidebarToken::Workspace => "workspace".into(),
        AgentSidebarToken::Tab => "tab".into(),
        AgentSidebarToken::Pane => "pane".into(),
        AgentSidebarToken::Agent => "agent".into(),
        AgentSidebarToken::AgentIndex => "agent_index".into(),
        AgentSidebarToken::TerminalTitle => "terminal_title".into(),
        AgentSidebarToken::TerminalTitleStripped => "terminal_title_stripped".into(),
        AgentSidebarToken::Custom(name) => format!("${name}"),
        AgentSidebarToken::Styled { token, .. } => agent_token_name(token),
    }
}

fn space_token_name(token: &SpaceSidebarToken) -> String {
    match token {
        SpaceSidebarToken::StateIcon => "state_icon".into(),
        SpaceSidebarToken::StateText => "state_text".into(),
        SpaceSidebarToken::Workspace => "workspace".into(),
        SpaceSidebarToken::Branch => "branch".into(),
        SpaceSidebarToken::GitStatus => "git_status".into(),
        SpaceSidebarToken::Custom(name) => format!("${name}"),
        SpaceSidebarToken::Styled { token, .. } => space_token_name(token),
    }
}

impl Serialize for AgentSidebarToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Styled {
                token,
                style,
                rules,
            } => serialize_styled_token(agent_token_name(token), *style, rules, serializer),
            token => serializer.serialize_str(&agent_token_name(token)),
        }
    }
}

impl From<String> for AgentSidebarToken {
    fn from(value: String) -> Self {
        Self::Custom(value)
    }
}

impl<'de> Deserialize<'de> for AgentSidebarToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let (value, style, rules) = RawSidebarToken::deserialize(deserializer)?
            .parts()
            .map_err(serde::de::Error::custom)?;
        let token = parse_sidebar_token(
            value,
            &[
                ("state_icon", Self::StateIcon),
                ("state_text", Self::StateText),
                ("machine", Self::Machine),
                ("workspace", Self::Workspace),
                ("tab", Self::Tab),
                ("pane", Self::Pane),
                ("agent", Self::Agent),
                ("agent_index", Self::AgentIndex),
                ("terminal_title", Self::TerminalTitle),
                ("terminal_title_stripped", Self::TerminalTitleStripped),
            ],
        )
        .map_err(serde::de::Error::custom)?;
        Ok(style.map_or(token.clone(), |style| Self::Styled {
            token: Box::new(token),
            style,
            rules,
        }))
    }
}

impl Serialize for SpaceSidebarToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Styled {
                token,
                style,
                rules,
            } => serialize_styled_token(space_token_name(token), *style, rules, serializer),
            token => serializer.serialize_str(&space_token_name(token)),
        }
    }
}

impl From<String> for SpaceSidebarToken {
    fn from(value: String) -> Self {
        Self::Custom(value)
    }
}

impl<'de> Deserialize<'de> for SpaceSidebarToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let (value, style, rules) = RawSidebarToken::deserialize(deserializer)?
            .parts()
            .map_err(serde::de::Error::custom)?;
        let token = parse_sidebar_token(
            value,
            &[
                ("state_icon", Self::StateIcon),
                ("state_text", Self::StateText),
                ("workspace", Self::Workspace),
                ("branch", Self::Branch),
                ("git_status", Self::GitStatus),
            ],
        )
        .map_err(serde::de::Error::custom)?;
        Ok(style.map_or(token.clone(), |style| Self::Styled {
            token: Box::new(token),
            style,
            rules,
        }))
    }
}

type AgentSidebarRows = Vec<Vec<AgentSidebarToken>>;
type SpaceSidebarRows = Vec<Vec<SpaceSidebarToken>>;

fn deserialize_rows_by_agent<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, AgentSidebarRows>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let rows_by_agent = BTreeMap::<String, AgentSidebarRows>::deserialize(deserializer)?;
    for (id, rows) in &rows_by_agent {
        if crate::detect::parse_canonical_agent_label(id).is_none() {
            return Err(serde::de::Error::custom(format!(
                "unknown canonical agent id `{id}` in sidebar rows_by_agent"
            )));
        }
        validate_sidebar_rows(rows).map_err(serde::de::Error::custom)?;
    }
    Ok(rows_by_agent)
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentsSidebarConfig {
    #[serde(deserialize_with = "deserialize_sidebar_rows")]
    pub rows: AgentSidebarRows,
    #[serde(default, deserialize_with = "deserialize_rows_by_agent")]
    pub rows_by_agent: BTreeMap<String, AgentSidebarRows>,
    pub row_gap: u16,
}

impl AgentsSidebarConfig {
    pub(crate) fn rows_for_agent(&self, agent: Option<Agent>) -> &AgentSidebarRows {
        agent
            .and_then(|agent| self.rows_by_agent.get(crate::detect::agent_label(agent)))
            .unwrap_or(&self.rows)
    }
}

impl Default for AgentsSidebarConfig {
    fn default() -> Self {
        Self {
            rows: vec![
                vec![
                    AgentSidebarToken::StateIcon,
                    AgentSidebarToken::Machine,
                    AgentSidebarToken::Workspace,
                    AgentSidebarToken::Tab,
                ],
                vec![AgentSidebarToken::Agent],
            ],
            rows_by_agent: BTreeMap::new(),
            row_gap: DEFAULT_SIDEBAR_ROW_GAP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SpacesSidebarConfig {
    #[serde(deserialize_with = "deserialize_sidebar_rows")]
    pub rows: SpaceSidebarRows,
    pub row_gap: u16,
}

impl Default for SpacesSidebarConfig {
    fn default() -> Self {
        Self {
            rows: vec![
                vec![SpaceSidebarToken::StateIcon, SpaceSidebarToken::Workspace],
                vec![SpaceSidebarToken::Branch, SpaceSidebarToken::GitStatus],
            ],
            row_gap: DEFAULT_SIDEBAR_ROW_GAP,
        }
    }
}

/// One entry of `[ui.sidebar] sections`, in render order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarSection {
    Spaces,
    Agents,
    /// A server-owned terminal panel, written `"panel:<id>"`.
    Panel(String),
}

/// Height of a panel section: terminal rows, or a percentage of the sidebar height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelHeight {
    Rows(u16),
    Percent(u16),
}

impl PanelHeight {
    /// Panels take 30% of the sidebar unless configured otherwise.
    pub const DEFAULT: PanelHeight = PanelHeight::Percent(30);

    /// Rows this height asks for in a sidebar of `total_rows` rows.
    pub fn rows(self, total_rows: u16) -> u16 {
        match self {
            PanelHeight::Rows(rows) => rows,
            PanelHeight::Percent(percent) => {
                ((u32::from(total_rows) * u32::from(percent) + 50) / 100) as u16
            }
        }
    }
}

/// Raw `height` value: an integer row count or a string such as `"40%"`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PanelHeightSetting {
    Rows(i64),
    Text(String),
}

impl PanelHeightSetting {
    fn resolve(&self) -> Result<PanelHeight, String> {
        match self {
            PanelHeightSetting::Rows(rows) => u16::try_from(*rows)
                .ok()
                .filter(|rows| *rows > 0)
                .map(PanelHeight::Rows)
                .ok_or_else(|| format!("height {rows} must be between 1 and {}", u16::MAX)),
            PanelHeightSetting::Text(text) => {
                let trimmed = text.trim();
                if let Some(percent) = trimmed.strip_suffix('%') {
                    return percent
                        .trim()
                        .parse::<u16>()
                        .ok()
                        .filter(|percent| (1..=100).contains(percent))
                        .map(PanelHeight::Percent)
                        .ok_or_else(|| {
                            format!("height \"{text}\" must be a percentage from 1% to 100%")
                        });
                }
                trimmed
                    .parse::<u16>()
                    .ok()
                    .filter(|rows| *rows > 0)
                    .map(PanelHeight::Rows)
                    .ok_or_else(|| {
                        format!(
                            "height \"{text}\" must be a row count or a percentage like \"30%\""
                        )
                    })
            }
        }
    }
}

/// `[ui.sidebar.panel.<id>]`: how the client presents a server-owned panel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SidebarPanelConfig {
    /// Section header text. Default: the panel id.
    pub title: Option<String>,
    /// Section height in rows or as a percentage. Default: "30%".
    pub height: Option<PanelHeightSetting>,
    /// Sidebar width while the panel has focus. Default: unchanged.
    pub focus_width: Option<u16>,
}

impl SidebarPanelConfig {
    /// Configured height, or the default when unset or invalid.
    pub fn resolved_height(&self) -> PanelHeight {
        self.height
            .as_ref()
            .and_then(|height| height.resolve().ok())
            .unwrap_or(PanelHeight::DEFAULT)
    }
}

fn default_sidebar_sections() -> Vec<String> {
    vec!["spaces".to_owned(), "agents".to_owned()]
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SidebarConfig {
    pub agents: AgentsSidebarConfig,
    pub spaces: SpacesSidebarConfig,
    /// Expanded sidebar sections in order: "spaces", "agents" and "panel:<id>".
    pub sections: Vec<String>,
    /// Presentation of panels declared by `[[panels]]`, keyed by panel id.
    pub panel: BTreeMap<String, SidebarPanelConfig>,
}

impl Default for SidebarConfig {
    fn default() -> Self {
        Self {
            agents: AgentsSidebarConfig::default(),
            spaces: SpacesSidebarConfig::default(),
            sections: default_sidebar_sections(),
            panel: BTreeMap::new(),
        }
    }
}

impl SidebarConfig {
    /// Parsed `sections`, dropping unknown and duplicate entries with a warning.
    /// A list with no usable entry falls back to the default sections.
    pub fn resolved_sections(&self) -> (Vec<SidebarSection>, Vec<String>) {
        let mut diagnostics = Vec::new();
        let mut sections = Vec::<SidebarSection>::new();
        for entry in &self.sections {
            let parsed = match entry.trim() {
                "spaces" => Some(SidebarSection::Spaces),
                "agents" => Some(SidebarSection::Agents),
                other => other
                    .strip_prefix("panel:")
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(|id| SidebarSection::Panel(id.to_owned())),
            };
            let Some(section) = parsed else {
                diagnostics.push(format!(
                    "ui.sidebar.sections: unknown entry \"{entry}\"; ignoring (use \"spaces\", \"agents\" or \"panel:<id>\")"
                ));
                continue;
            };
            if sections.contains(&section) {
                diagnostics.push(format!(
                    "ui.sidebar.sections: duplicate entry \"{entry}\"; ignoring"
                ));
                continue;
            }
            sections.push(section);
        }
        if sections.is_empty() {
            diagnostics.push(
                "ui.sidebar.sections: no usable entries; using [\"spaces\", \"agents\"]".to_owned(),
            );
            sections = vec![SidebarSection::Spaces, SidebarSection::Agents];
        }
        (sections, diagnostics)
    }

    /// Warnings for `sections` and `[ui.sidebar.panel.<id>]`.
    pub fn diagnostics(&self) -> Vec<String> {
        let mut diagnostics = self.resolved_sections().1;
        for (id, panel) in &self.panel {
            if let Some(Err(error)) = panel.height.as_ref().map(PanelHeightSetting::resolve) {
                diagnostics.push(format!(
                    "ui.sidebar.panel.{id}: {error}; using the default height"
                ));
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_compact_agent_and_existing_space_layouts() {
        let config = SidebarConfig::default();
        assert_eq!(
            config.agents.rows,
            vec![
                vec![
                    AgentSidebarToken::StateIcon,
                    AgentSidebarToken::Machine,
                    AgentSidebarToken::Workspace,
                    AgentSidebarToken::Tab,
                ],
                vec![AgentSidebarToken::Agent],
            ]
        );
        assert!(config.agents.rows_by_agent.is_empty());
        assert_eq!(config.agents.row_gap, 0);
        assert_eq!(
            config.spaces.rows,
            vec![
                vec![SpaceSidebarToken::StateIcon, SpaceSidebarToken::Workspace],
                vec![SpaceSidebarToken::Branch, SpaceSidebarToken::GitStatus],
            ]
        );
        assert_eq!(config.spaces.row_gap, 0);
    }

    #[test]
    fn parses_builtin_and_arbitrary_custom_tokens() {
        let config: crate::config::Config = toml::from_str(
            r#"
[ui.sidebar.agents]
rows = [["state_icon", "agent_index", "workspace"], ["state_text", "agent", "$summary"], ["terminal_title", "terminal_title_stripped", "$terminal_title"]]
row_gap = 1

[ui.sidebar.agents.rows_by_agent]
claude = [["terminal_title_stripped"], ["agent", "$model"]]

[ui.sidebar.spaces]
rows = [["workspace"], ["$jj_status"]]
row_gap = 3
"#,
        )
        .expect("sidebar token config");

        assert_eq!(
            config.ui.sidebar.agents.rows[0],
            vec![
                AgentSidebarToken::StateIcon,
                AgentSidebarToken::AgentIndex,
                AgentSidebarToken::Workspace,
            ]
        );
        assert_eq!(
            config.ui.sidebar.agents.rows[1],
            vec![
                AgentSidebarToken::StateText,
                AgentSidebarToken::Agent,
                AgentSidebarToken::Custom("summary".into()),
            ]
        );
        assert_eq!(
            config.ui.sidebar.agents.rows[2],
            vec![
                AgentSidebarToken::TerminalTitle,
                AgentSidebarToken::TerminalTitleStripped,
                AgentSidebarToken::Custom("terminal_title".into()),
            ]
        );
        assert_eq!(
            config.ui.sidebar.agents.rows_by_agent["claude"],
            vec![
                vec![AgentSidebarToken::TerminalTitleStripped],
                vec![
                    AgentSidebarToken::Agent,
                    AgentSidebarToken::Custom("model".into()),
                ],
            ]
        );
        assert_eq!(config.ui.sidebar.agents.row_gap, 1);
        assert_eq!(
            config.ui.sidebar.spaces.rows[1],
            vec![SpaceSidebarToken::Custom("jj_status".into())]
        );
        assert_eq!(config.ui.sidebar.spaces.row_gap, 3);
    }

    #[test]
    fn parses_occurrence_styles_without_changing_plain_tokens() {
        let config: crate::config::Config = toml::from_str(
            r##"
[ui.sidebar.agents]
rows = [[{ token = "workspace", fg = "#abc", bold = false }, "workspace"], [{ token = "$summary", dim = false }]]

[ui.sidebar.agents.rows_by_agent]
claude = [[{ token = "agent", fg = "#112233", bold = true, dim = false }]]

[ui.sidebar.spaces]
rows = [[{ token = "git_status", fg = "#ff00aa" }], [{ token = "$jj", bold = true }]]
"##,
        )
        .unwrap();

        let (token, style) = config.ui.sidebar.agents.rows[0][0].parts();
        assert_eq!(token, &AgentSidebarToken::Workspace);
        assert_eq!(style.bold, Some(false));
        assert_eq!(
            style.fg.unwrap().ratatui(),
            ratatui::style::Color::Rgb(0xaa, 0xbb, 0xcc)
        );
        assert_eq!(
            config.ui.sidebar.agents.rows[0][1],
            AgentSidebarToken::Workspace
        );

        let (token, style) = config.ui.sidebar.agents.rows_by_agent["claude"][0][0].parts();
        assert_eq!(token, &AgentSidebarToken::Agent);
        assert_eq!(style.bold, Some(true));
        assert_eq!(style.dim, Some(false));

        let (token, style) = config.ui.sidebar.spaces.rows[0][0].parts();
        assert_eq!(token, &SpaceSidebarToken::GitStatus);
        assert_eq!(
            style.fg.unwrap().ratatui(),
            ratatui::style::Color::Rgb(0xff, 0x00, 0xaa)
        );
        let (token, style) = config.ui.sidebar.spaces.rows[1][0].parts();
        assert_eq!(token, &SpaceSidebarToken::Custom("jj".into()));
        assert_eq!(style.bold, Some(true));
    }

    #[test]
    fn conditional_sidebar_rules_round_trip() {
        let input = r##"
[agents]
rows = [[{ token = "machine", fg = "#fff", rules = [{ equals = "Local", fg = "#f00" }, { starts_with = "fed", ignore_case = true, bold = true }] }]]
[agents.rows_by_agent]
pi = [[{ token = "$load", rules = [{ gt = 80, dim = false }, { lt = 20.5, dim = true }] }]]
[spaces]
rows = [[{ token = "$status", rules = [{ contains = "error", bold = true }] }]]
"##;
        let config: SidebarConfig = toml::from_str(input).expect("conditional sidebar config");
        let encoded = toml::to_string(&config).unwrap();
        assert!(encoded.contains("rules"));
        assert_eq!(toml::from_str::<SidebarConfig>(&encoded).unwrap(), config);
    }

    #[test]
    fn conditional_sidebar_rules_reject_invalid_conditions_and_nontext_tokens() {
        for rule in [
            "{ bold = true }",
            "{ equals = 'x', contains = 'x' }",
            "{ regex = 'x' }",
            "{ gt = '80' }",
            "{ equals = 80 }",
            "{ gt = nan }",
            "{ lt = inf }",
            "{ gt = 80, ignore_case = false }",
            "{ equals = 'x', underline = true }",
            "{ equals = 'x', fg = 'red' }",
        ] {
            let input = format!("[agents]\nrows = [[{{ token = 'machine', rules = [{rule}] }}]]");
            assert!(toml::from_str::<SidebarConfig>(&input).is_err(), "{rule}");
        }
        for (section, token) in [
            ("agents", "state_icon"),
            ("spaces", "state_icon"),
            ("spaces", "git_status"),
        ] {
            let input = format!(
                "[{section}]\nrows = [[{{ token = '{token}', rules = [{{ equals = 'x' }}] }}]]"
            );
            assert!(toml::from_str::<SidebarConfig>(&input).is_err());
        }
        for count in [16, 17] {
            let rules = std::iter::repeat_n("{ equals = 'x' }", count)
                .collect::<Vec<_>>()
                .join(",");
            let input = format!("[agents]\nrows = [[{{ token = 'machine', rules = [{rules}] }}]]");
            assert_eq!(toml::from_str::<SidebarConfig>(&input).is_ok(), count == 16);
        }
    }

    #[test]
    fn rejects_invalid_occurrence_styles() {
        for entry in [
            r##"{ token = "workspace", fg = "red" }"##,
            r##"{ token = "workspace", fg = "#abcd" }"##,
            r##"{ token = "workspace", underline = true }"##,
        ] {
            let input = format!("[ui.sidebar.agents]\nrows = [[{entry}]]\n");
            assert!(
                toml::from_str::<crate::config::Config>(&input).is_err(),
                "accepted {entry}"
            );
        }
    }

    #[test]
    fn rejects_unknown_bare_and_malformed_custom_tokens() {
        for token in ["summary", "$", "$bad.name"] {
            let input = format!("[ui.sidebar.agents]\\nrows = [[\"{token}\"]]\\n");
            assert!(toml::from_str::<crate::config::Config>(&input).is_err());
        }
    }

    #[test]
    fn rejects_oversized_sidebar_layouts() {
        let too_many_rows = std::iter::repeat_n("[\"agent\"]", MAX_SIDEBAR_ROWS + 1)
            .collect::<Vec<_>>()
            .join(",");
        let input = format!("[ui.sidebar.agents]\nrows = [{too_many_rows}]\n");
        assert!(toml::from_str::<crate::config::Config>(&input).is_err());

        let too_many_tokens = std::iter::repeat_n("\"workspace\"", MAX_SIDEBAR_TOKENS_PER_ROW + 1)
            .collect::<Vec<_>>()
            .join(",");
        let input = format!("[ui.sidebar.spaces]\nrows = [[{too_many_tokens}]]\n");
        assert!(toml::from_str::<crate::config::Config>(&input).is_err());

        let input = format!("[ui.sidebar.agents.rows_by_agent]\nclaude = [{too_many_rows}]\n");
        assert!(toml::from_str::<crate::config::Config>(&input).is_err());
    }

    #[test]
    fn accepts_every_canonical_agent_override_key() {
        let agents = Agent::ALL;
        let entries = agents
            .iter()
            .map(|agent| format!("{} = [[\"agent\"]]", crate::detect::agent_label(*agent)))
            .collect::<Vec<_>>()
            .join("\n");
        let input = format!("[ui.sidebar.agents.rows_by_agent]\n{entries}\n");
        let config: crate::config::Config = toml::from_str(&input).expect("canonical keys");

        assert_eq!(config.ui.sidebar.agents.rows_by_agent.len(), agents.len());
    }

    #[test]
    fn rejects_alias_case_whitespace_and_unknown_override_keys() {
        for key in ["claude-code", "Claude", "' claude '", "unknown"] {
            let input = format!("[ui.sidebar.agents.rows_by_agent]\n{key} = [[\"agent\"]]\n");
            assert!(
                toml::from_str::<crate::config::Config>(&input).is_err(),
                "accepted key {key:?}"
            );
        }
    }
}

#[cfg(test)]
mod section_tests {
    use super::*;

    fn parse(input: &str) -> crate::config::Config {
        toml::from_str(input).expect("sidebar section config")
    }

    #[test]
    fn default_sections_are_spaces_then_agents() {
        let config = SidebarConfig::default();
        let (sections, diagnostics) = config.resolved_sections();
        assert_eq!(
            sections,
            vec![SidebarSection::Spaces, SidebarSection::Agents]
        );
        assert!(diagnostics.is_empty());
        assert!(config.panel.is_empty());
        assert!(config.diagnostics().is_empty());
    }

    #[test]
    fn parses_sections_and_panel_presentation() {
        let config = parse(
            r#"
[ui.sidebar]
sections = ["spaces", "panel:prs", "agents"]

[ui.sidebar.panel.prs]
title = "Pull requests"
height = "40%"
focus_width = 80

[ui.sidebar.panel.logs]
height = 8
"#,
        );
        let (sections, diagnostics) = config.ui.sidebar.resolved_sections();
        assert_eq!(
            sections,
            vec![
                SidebarSection::Spaces,
                SidebarSection::Panel("prs".into()),
                SidebarSection::Agents
            ]
        );
        assert!(diagnostics.is_empty());
        let prs = &config.ui.sidebar.panel["prs"];
        assert_eq!(prs.title.as_deref(), Some("Pull requests"));
        assert_eq!(prs.resolved_height(), PanelHeight::Percent(40));
        assert_eq!(prs.focus_width, Some(80));
        let logs = &config.ui.sidebar.panel["logs"];
        assert_eq!(logs.resolved_height(), PanelHeight::Rows(8));
        assert_eq!(logs.title, None);
        assert_eq!(logs.focus_width, None);
        assert!(config.ui.sidebar.diagnostics().is_empty());
    }

    #[test]
    fn unknown_and_duplicate_sections_are_dropped_with_warnings() {
        let config = parse(
            r#"
[ui.sidebar]
sections = ["spaces", "bogus", "panel:", "spaces", "panel:a", "panel:a", "agents"]
"#,
        );
        let (sections, diagnostics) = config.ui.sidebar.resolved_sections();
        assert_eq!(
            sections,
            vec![
                SidebarSection::Spaces,
                SidebarSection::Panel("a".into()),
                SidebarSection::Agents
            ]
        );
        assert_eq!(diagnostics.len(), 4, "{diagnostics:?}");
        assert!(diagnostics[0].contains("bogus"));
        assert!(diagnostics[1].contains("panel:"));
        assert!(diagnostics[2].contains("duplicate"));
        assert!(diagnostics[3].contains("duplicate"));
    }

    #[test]
    fn a_section_list_without_usable_entries_falls_back_to_the_default() {
        let config = parse("[ui.sidebar]\nsections = [\"nope\"]\n");
        let (sections, diagnostics) = config.ui.sidebar.resolved_sections();
        assert_eq!(
            sections,
            vec![SidebarSection::Spaces, SidebarSection::Agents]
        );
        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn panel_only_sections_are_allowed() {
        let config = parse("[ui.sidebar]\nsections = [\"panel:a\"]\n");
        let (sections, diagnostics) = config.ui.sidebar.resolved_sections();
        assert_eq!(sections, vec![SidebarSection::Panel("a".into())]);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn invalid_panel_heights_warn_and_use_the_default() {
        let config = parse(
            r#"
[ui.sidebar.panel.a]
height = "wide"
[ui.sidebar.panel.b]
height = 0
[ui.sidebar.panel.c]
height = "150%"
[ui.sidebar.panel.d]
height = -3
[ui.sidebar.panel.e]
height = "12"
"#,
        );
        let diagnostics = config.ui.sidebar.diagnostics();
        assert_eq!(diagnostics.len(), 4, "{diagnostics:?}");
        for id in ["a", "b", "c", "d"] {
            assert!(diagnostics
                .iter()
                .any(|d| d.contains(&format!("panel.{id}"))));
            assert_eq!(
                config.ui.sidebar.panel[id].resolved_height(),
                PanelHeight::DEFAULT
            );
        }
        assert_eq!(
            config.ui.sidebar.panel["e"].resolved_height(),
            PanelHeight::Rows(12)
        );
    }

    #[test]
    fn panel_height_rows_round_percentages() {
        assert_eq!(PanelHeight::Percent(30).rows(20), 6);
        assert_eq!(PanelHeight::Percent(50).rows(5), 3);
        assert_eq!(PanelHeight::Rows(9).rows(3), 9);
    }

    #[test]
    fn sidebar_diagnostics_reach_the_config_diagnostics() {
        let config = parse("[ui.sidebar]\nsections = [\"bogus\", \"spaces\"]\n");
        assert!(config
            .collect_diagnostics()
            .iter()
            .any(|d| d.contains("bogus")));
    }
}
