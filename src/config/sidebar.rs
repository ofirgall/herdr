mod metadata;
mod rules;

pub use metadata::RawSpaceMetadataEntry;
#[cfg(test)]
pub(crate) use metadata::MAX_METADATA_TOKEN_KEY_LEN;
pub(crate) use metadata::{
    is_valid_metadata_token_key, normalize_metadata_token_value, SpaceMetadata,
};
pub use rules::SidebarTokenRule;

use std::collections::BTreeMap;

use crossterm::event::KeyModifiers;
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
    /// Whether the agents section is shown in the sidebar.
    pub enabled: bool,
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
            enabled: true,
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
    /// Whether the spaces section is shown in the sidebar.
    pub enabled: bool,
    #[serde(deserialize_with = "deserialize_sidebar_rows")]
    pub rows: SpaceSidebarRows,
    pub row_gap: u16,
    /// Whether space rows render their aggregate agent status (`state_icon`
    /// and `state_text`). Agent children keep their own status either way.
    pub show_status: bool,
    /// Default visibility of each space's agents as child entries.
    pub agents: SpaceAgentsMode,
    /// Row layout of an agent child entry; reuses the agent token vocabulary.
    #[serde(deserialize_with = "deserialize_sidebar_rows")]
    pub agent_rows: AgentSidebarRows,
    /// Static per-space tokens matched by label glob or path prefix.
    pub metadata: Vec<RawSpaceMetadataEntry>,
    /// Validated form of `metadata`, filled by [`Self::compile_metadata`].
    #[serde(skip)]
    pub(crate) compiled_metadata: SpaceMetadata,
}

/// Whether spaces show their agents as indented children by default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SpaceAgentsMode {
    /// No children and no per-space toggle.
    #[default]
    Off,
    /// Children are hidden until a space is expanded.
    Collapsed,
    /// Children are shown until a space is collapsed.
    Expanded,
}

impl SpaceAgentsMode {
    /// Effective visibility of one space's children given a user override.
    pub(crate) fn expanded(self, override_expanded: Option<bool>) -> bool {
        match self {
            Self::Off => false,
            Self::Collapsed => override_expanded.unwrap_or(false),
            Self::Expanded => override_expanded.unwrap_or(true),
        }
    }
}

impl Default for SpacesSidebarConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rows: vec![
                vec![SpaceSidebarToken::StateIcon, SpaceSidebarToken::Workspace],
                vec![SpaceSidebarToken::Branch, SpaceSidebarToken::GitStatus],
            ],
            row_gap: DEFAULT_SIDEBAR_ROW_GAP,
            show_status: true,
            agents: SpaceAgentsMode::Off,
            agent_rows: vec![vec![AgentSidebarToken::StateIcon, AgentSidebarToken::Tab]],
            metadata: Vec::new(),
            compiled_metadata: SpaceMetadata::default(),
        }
    }
}

impl SpacesSidebarConfig {
    /// Diagnostics for ignored or truncated metadata entries.
    pub(crate) fn metadata_diagnostics(&self) -> Vec<String> {
        metadata::compile(&self.metadata).1
    }

    /// Validates `metadata` into `compiled_metadata`, returning diagnostics.
    pub(crate) fn compile_metadata(&mut self) -> Vec<String> {
        let (compiled, diagnostics) = metadata::compile(&self.metadata);
        self.compiled_metadata = compiled;
        diagnostics
    }
}

/// Which click opens the link of a sidebar token.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TokenLinkClick {
    /// A plain click (no ctrl/alt/super/meta/hyper) opens the link.
    #[default]
    Click,
    /// A click holding exactly these modifiers opens the link.
    Modifier(KeyModifiers),
    /// Links are neither underlined nor clickable.
    Off,
}

impl TokenLinkClick {
    fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.eq_ignore_ascii_case("click") {
            return Some(Self::Click);
        }
        if trimmed.eq_ignore_ascii_case("off") {
            return Some(Self::Off);
        }
        super::model::parse_modifier_combination(trimmed).map(Self::Modifier)
    }

    fn as_config_string(self) -> String {
        match self {
            Self::Click => "click".to_owned(),
            Self::Off => "off".to_owned(),
            Self::Modifier(modifiers) => [
                (KeyModifiers::CONTROL, "ctrl"),
                (KeyModifiers::ALT, "alt"),
                (KeyModifiers::SUPER, "cmd"),
                (KeyModifiers::META, "meta"),
                (KeyModifiers::HYPER, "hyper"),
            ]
            .into_iter()
            .filter(|(modifier, _)| modifiers.contains(*modifier))
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
            .join("+"),
        }
    }

    /// Whether a left click carrying `modifiers` opens a token link.
    /// Shift is ignored: terminals commonly reserve shift-click for their own
    /// selection, so it neither enables nor blocks the click.
    pub fn matches(self, modifiers: KeyModifiers) -> bool {
        let modifiers = modifiers - KeyModifiers::SHIFT;
        match self {
            Self::Click => modifiers.is_empty(),
            Self::Modifier(required) => modifiers == required,
            Self::Off => false,
        }
    }

    pub fn is_off(self) -> bool {
        self == Self::Off
    }
}

/// Serde form of [`TokenLinkClick`]: a string, so the config reference lists
/// it as a string rather than an enum.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenLinkClickConfig(TokenLinkClick);

impl TokenLinkClickConfig {
    pub fn mode(self) -> TokenLinkClick {
        self.0
    }
}

impl<'de> Deserialize<'de> for TokenLinkClickConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        TokenLinkClick::parse(&value).map(Self).ok_or_else(|| {
            serde::de::Error::custom(
                "token_link_click must be click, off, ctrl/control, alt/option, cmd/command/super, meta, hyper, or a + separated combination without shift",
            )
        })
    }
}

impl Serialize for TokenLinkClickConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.as_config_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct SidebarConfig {
    pub agents: AgentsSidebarConfig,
    pub spaces: SpacesSidebarConfig,
    /// Which click opens the link of a metadata token that carries one.
    pub token_link_click: TokenLinkClickConfig,
}

impl SidebarConfig {
    /// True when the config disables every sidebar section.
    pub fn all_sections_disabled(&self) -> bool {
        !self.spaces.enabled && !self.agents.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_metadata_example_parses() {
        let config: crate::config::Config = toml::from_str(
            r#"
[ui.sidebar.spaces]
rows = [["workspace", "$env"], ["$owner"]]

[[ui.sidebar.spaces.metadata]]
match = { label = "api-*" }
tokens = { env = "staging", owner = "platform" }

[[ui.sidebar.spaces.metadata]]
match = { path = "~/work/herdr" }
tokens = { env = "dev" }
"#,
        )
        .unwrap();
        let spaces = &config.ui.sidebar.spaces;
        assert_eq!(spaces.metadata.len(), 2);
        assert_eq!(spaces.metadata[0].matcher.label.as_deref(), Some("api-*"));
        assert_eq!(spaces.metadata[1].tokens["env"], "dev");
        assert!(spaces.metadata_diagnostics().is_empty());
        assert!(config.collect_diagnostics().is_empty());
    }

    #[test]
    fn token_link_click_parses_every_accepted_form() {
        let parse = |value: &str| -> Result<TokenLinkClick, _> {
            toml::from_str::<SidebarConfig>(&format!("token_link_click = \"{value}\""))
                .map(|config| config.token_link_click.mode())
        };
        assert_eq!(
            SidebarConfig::default().token_link_click.mode(),
            TokenLinkClick::Click
        );
        assert_eq!(parse("click").unwrap(), TokenLinkClick::Click);
        assert_eq!(parse("off").unwrap(), TokenLinkClick::Off);
        assert_eq!(
            parse("ctrl").unwrap(),
            TokenLinkClick::Modifier(KeyModifiers::CONTROL)
        );
        assert_eq!(
            parse("Option").unwrap(),
            TokenLinkClick::Modifier(KeyModifiers::ALT)
        );
        assert_eq!(
            parse("cmd+alt").unwrap(),
            TokenLinkClick::Modifier(KeyModifiers::SUPER | KeyModifiers::ALT)
        );
        for value in ["super", "command", "meta", "hyper", "control+alt"] {
            assert!(
                matches!(parse(value).unwrap(), TokenLinkClick::Modifier(_)),
                "{value}"
            );
        }
        for value in ["shift", "ctrl+shift", "banana", "", "ctrl+", "none"] {
            assert!(parse(value).is_err(), "{value:?} should be rejected");
        }
    }

    #[test]
    fn token_link_click_round_trips_and_serializes() {
        for value in ["click", "off", "ctrl", "ctrl+alt", "cmd+meta+hyper"] {
            let config: SidebarConfig =
                toml::from_str(&format!("token_link_click = \"{value}\"")).unwrap();
            let round: SidebarConfig = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
            assert_eq!(round, config, "{value}");
        }
    }

    #[test]
    fn token_link_click_matching_ignores_shift_and_requires_exact_modifiers() {
        let none = KeyModifiers::empty();
        assert!(TokenLinkClick::Click.matches(none));
        assert!(TokenLinkClick::Click.matches(KeyModifiers::SHIFT));
        assert!(!TokenLinkClick::Click.matches(KeyModifiers::CONTROL));
        let ctrl = TokenLinkClick::Modifier(KeyModifiers::CONTROL);
        assert!(!ctrl.matches(none));
        assert!(ctrl.matches(KeyModifiers::CONTROL));
        assert!(ctrl.matches(KeyModifiers::CONTROL | KeyModifiers::SHIFT));
        assert!(!ctrl.matches(KeyModifiers::CONTROL | KeyModifiers::ALT));
        assert!(!TokenLinkClick::Off.matches(none));
    }

    #[test]
    fn documented_sections_example_parses() {
        let config: crate::config::Config = toml::from_str(
            "[ui.sidebar.spaces]\nenabled = true\n\n[ui.sidebar.agents]\nenabled = false\n",
        )
        .unwrap();
        assert!(config.ui.sidebar.spaces.enabled);
        assert!(!config.ui.sidebar.agents.enabled);
        assert!(!config.ui.sidebar.all_sections_disabled());
    }

    #[test]
    fn sections_are_enabled_by_default_and_parse_enabled_false() {
        let config = SidebarConfig::default();
        assert!(config.spaces.enabled && config.agents.enabled);
        assert!(!config.all_sections_disabled());

        let agents: AgentsSidebarConfig = toml::from_str("enabled = false").unwrap();
        assert!(!agents.enabled);
        assert_eq!(agents.rows, AgentsSidebarConfig::default().rows);
        let spaces: SpacesSidebarConfig = toml::from_str("enabled = false\nrow_gap = 1").unwrap();
        assert!(!spaces.enabled);
        assert_eq!(spaces.row_gap, 1);

        let both: SidebarConfig =
            toml::from_str("[agents]\nenabled = false\n[spaces]\nenabled = false").unwrap();
        assert!(both.all_sections_disabled());
        let round: SidebarConfig = toml::from_str(&toml::to_string(&both).unwrap()).unwrap();
        assert_eq!(round, both);
    }

    #[test]
    fn space_agents_default_to_off_with_state_icon_and_tab_rows() {
        let spaces = SpacesSidebarConfig::default();
        assert_eq!(spaces.agents, SpaceAgentsMode::Off);
        assert_eq!(
            spaces.agent_rows,
            vec![vec![AgentSidebarToken::StateIcon, AgentSidebarToken::Tab]]
        );
        let parsed: SpacesSidebarConfig = toml::from_str("row_gap = 1").unwrap();
        assert_eq!(parsed.agents, SpaceAgentsMode::Off);
        assert_eq!(parsed.agent_rows, spaces.agent_rows);
    }

    #[test]
    fn space_agents_parse_modes_and_custom_rows() {
        for (value, mode) in [
            ("off", SpaceAgentsMode::Off),
            ("collapsed", SpaceAgentsMode::Collapsed),
            ("expanded", SpaceAgentsMode::Expanded),
        ] {
            let spaces: SpacesSidebarConfig =
                toml::from_str(&format!("agents = \"{value}\"")).unwrap();
            assert_eq!(spaces.agents, mode);
        }
        let spaces: SpacesSidebarConfig = toml::from_str(
            "agents = \"expanded\"\nagent_rows = [[\"state_icon\", \"tab\", \"agent\"], [\"$task\"]]",
        )
        .unwrap();
        assert_eq!(spaces.agent_rows.len(), 2);
        assert_eq!(
            spaces.agent_rows[1],
            vec![AgentSidebarToken::Custom("task".into())]
        );
        let round: SpacesSidebarConfig =
            toml::from_str(&toml::to_string(&spaces).unwrap()).unwrap();
        assert_eq!(round, spaces);
    }

    #[test]
    fn space_agents_reject_unknown_modes_and_oversized_rows() {
        for value in ["\"always\"", "true", "\"Expanded\""] {
            assert!(
                toml::from_str::<SpacesSidebarConfig>(&format!("agents = {value}")).is_err(),
                "accepted {value}"
            );
        }
        let too_many_rows = std::iter::repeat_n("[\"tab\"]", MAX_SIDEBAR_ROWS + 1)
            .collect::<Vec<_>>()
            .join(",");
        assert!(
            toml::from_str::<SpacesSidebarConfig>(&format!("agent_rows = [{too_many_rows}]"))
                .is_err()
        );
        let too_many_tokens = std::iter::repeat_n("\"tab\"", MAX_SIDEBAR_TOKENS_PER_ROW + 1)
            .collect::<Vec<_>>()
            .join(",");
        assert!(toml::from_str::<SpacesSidebarConfig>(&format!(
            "agent_rows = [[{too_many_tokens}]]"
        ))
        .is_err());
        assert!(toml::from_str::<SpacesSidebarConfig>("agent_rows = [[\"bogus\"]]").is_err());
    }

    #[test]
    fn documented_space_agents_example_parses() {
        let config: crate::config::Config = toml::from_str(
            "[ui.sidebar.spaces]\nagents = \"expanded\"\nagent_rows = [\n  [\"state_icon\", \"tab\"],\n]\n",
        )
        .unwrap();
        assert_eq!(config.ui.sidebar.spaces.agents, SpaceAgentsMode::Expanded);
        assert_eq!(
            config.ui.sidebar.spaces.agent_rows,
            SpacesSidebarConfig::default().agent_rows
        );
    }

    #[test]
    fn space_agents_mode_resolves_effective_expansion() {
        use SpaceAgentsMode::*;
        assert!(!Off.expanded(Some(true)));
        assert!(!Collapsed.expanded(None));
        assert!(Collapsed.expanded(Some(true)));
        assert!(Expanded.expanded(None));
        assert!(!Expanded.expanded(Some(false)));
    }

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
