//! Static per-space metadata declared in `[[ui.sidebar.spaces.metadata]]`.
//!
//! Entries are parsed leniently into a raw form so one bad entry never
//! rejects the whole config; [`compile`] validates them and reports a
//! diagnostic per ignored entry.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub(crate) const MAX_METADATA_TOKEN_KEY_LEN: usize = 32;
pub(crate) const MAX_METADATA_TOKEN_VALUE_LEN: usize = 80;
pub(crate) const MAX_SPACE_METADATA_ENTRIES: usize = 64;
pub(crate) const MAX_SPACE_METADATA_TOKENS: usize = 16;

/// Shared by the workspace metadata API and static sidebar metadata.
pub(crate) fn is_valid_metadata_token_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_METADATA_TOKEN_KEY_LEN
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

/// Trims, strips control characters and truncates; `None` for an empty value.
pub(crate) fn normalize_metadata_token_value(value: &str) -> Option<String> {
    let normalized = value
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_METADATA_TOKEN_VALUE_LEN)
        .collect::<String>();
    let trimmed = normalized.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct RawSpaceMetadataMatch {
    pub label: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct RawSpaceMetadataEntry {
    #[serde(rename = "match")]
    pub matcher: RawSpaceMetadataMatch,
    pub tokens: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SpaceMetadataMatcher {
    Label(String),
    /// Absolute-or-relative path with `~` already expanded.
    Path(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpaceMetadataEntry {
    pub matcher: SpaceMetadataMatcher,
    pub tokens: Vec<(String, String)>,
}

impl SpaceMetadataEntry {
    fn matches(&self, label: &str, cwd: &str) -> bool {
        match &self.matcher {
            SpaceMetadataMatcher::Label(glob) => glob_matches(glob, label),
            SpaceMetadataMatcher::Path(prefix) => {
                !cwd.is_empty() && std::path::Path::new(cwd).starts_with(prefix)
            }
        }
    }
}

/// Validated static metadata, ready for per-frame matching.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SpaceMetadata {
    entries: Vec<SpaceMetadataEntry>,
}

impl SpaceMetadata {
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Applies every matching entry in config order (later wins) onto `out`.
    pub(crate) fn apply(
        &self,
        label: &str,
        cwd: &str,
        out: &mut std::collections::HashMap<String, String>,
    ) {
        for entry in self.entries.iter().filter(|e| e.matches(label, cwd)) {
            for (key, value) in &entry.tokens {
                out.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Validates raw entries. Invalid entries are dropped with a diagnostic;
/// entries beyond the limits are dropped (first 64 entries / 16 tokens kept).
pub(crate) fn compile(raw: &[RawSpaceMetadataEntry]) -> (SpaceMetadata, Vec<String>) {
    compile_with(raw, crate::worktree::expand_tilde_path)
}

fn compile_with(
    raw: &[RawSpaceMetadataEntry],
    expand: impl Fn(&str) -> PathBuf,
) -> (SpaceMetadata, Vec<String>) {
    let mut diagnostics = Vec::new();
    if raw.len() > MAX_SPACE_METADATA_ENTRIES {
        diagnostics.push(format!(
            "ui.sidebar.spaces.metadata has {} entries; only the first {MAX_SPACE_METADATA_ENTRIES} are used",
            raw.len()
        ));
    }
    let mut entries = Vec::new();
    for (index, entry) in raw.iter().take(MAX_SPACE_METADATA_ENTRIES).enumerate() {
        let name = format!("ui.sidebar.spaces.metadata[{index}]");
        let matcher = match (&entry.matcher.label, &entry.matcher.path) {
            (Some(label), None) if !label.is_empty() => SpaceMetadataMatcher::Label(label.clone()),
            (None, Some(path)) if !path.trim().is_empty() => {
                SpaceMetadataMatcher::Path(expand(path.trim()))
            }
            (Some(_), Some(_)) => {
                diagnostics.push(format!(
                    "{name}: match must set exactly one of label or path, not both; entry ignored"
                ));
                continue;
            }
            (None, None) => {
                diagnostics.push(format!(
                    "{name}: match must set exactly one of label or path; entry ignored"
                ));
                continue;
            }
            _ => {
                diagnostics.push(format!(
                    "{name}: match label or path must not be empty; entry ignored"
                ));
                continue;
            }
        };
        if entry.tokens.len() > MAX_SPACE_METADATA_TOKENS {
            diagnostics.push(format!(
                "{name}: has {} tokens; only the first {MAX_SPACE_METADATA_TOKENS} are used",
                entry.tokens.len()
            ));
        }
        if let Some(key) = entry
            .tokens
            .keys()
            .take(MAX_SPACE_METADATA_TOKENS)
            .find(|key| !is_valid_metadata_token_key(key))
        {
            diagnostics.push(format!(
                "{name}: invalid metadata token key {key:?}; entry ignored"
            ));
            continue;
        }
        let tokens = entry
            .tokens
            .iter()
            .take(MAX_SPACE_METADATA_TOKENS)
            .filter_map(|(key, value)| {
                normalize_metadata_token_value(value).map(|value| (key.clone(), value))
            })
            .collect();
        entries.push(SpaceMetadataEntry { matcher, tokens });
    }
    (SpaceMetadata { entries }, diagnostics)
}

/// Case-sensitive glob with `*` (any run) and `?` (any one char).
pub(crate) fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let text = text.chars().collect::<Vec<_>>();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) && pattern[p] != '*' {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn entry(
        label: Option<&str>,
        path: Option<&str>,
        tokens: &[(&str, &str)],
    ) -> RawSpaceMetadataEntry {
        RawSpaceMetadataEntry {
            matcher: RawSpaceMetadataMatch {
                label: label.map(Into::into),
                path: path.map(Into::into),
            },
            tokens: tokens
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn expand(path: &str) -> PathBuf {
        match path.strip_prefix("~/") {
            Some(rest) => PathBuf::from("/home/me").join(rest),
            None => PathBuf::from(path),
        }
    }

    fn resolve(metadata: &SpaceMetadata, label: &str, cwd: &str) -> HashMap<String, String> {
        let mut out = HashMap::new();
        metadata.apply(label, cwd, &mut out);
        out
    }

    #[test]
    fn shared_rules_validate_keys_and_values() {
        assert!(is_valid_metadata_token_key("my-env_1"));
        assert!(!is_valid_metadata_token_key("my env"));
        assert!(!is_valid_metadata_token_key(""));
        assert!(!is_valid_metadata_token_key(
            &"x".repeat(MAX_METADATA_TOKEN_KEY_LEN + 1)
        ));
        assert_eq!(
            normalize_metadata_token_value("  a\nb "),
            Some("ab".to_string())
        );
        assert_eq!(normalize_metadata_token_value(" \n "), None);
        assert_eq!(
            normalize_metadata_token_value(&"v".repeat(200)).map(|v| v.len()),
            Some(MAX_METADATA_TOKEN_VALUE_LEN)
        );
    }

    #[test]
    fn glob_supports_star_and_question_mark() {
        assert!(glob_matches("api-*", "api-gateway"));
        assert!(glob_matches("api-*", "api-"));
        assert!(!glob_matches("api-*", "web-api"));
        assert!(glob_matches("*-prod", "api-prod"));
        assert!(glob_matches("a*b*c", "aXXbYYc"));
        assert!(!glob_matches("a*b*c", "aXXbYY"));
        assert!(glob_matches("a?c", "abc"));
        assert!(!glob_matches("a?c", "ac"));
        assert!(glob_matches("*", ""));
        assert!(!glob_matches("API-*", "api-x"));
        assert!(glob_matches("exact", "exact"));
        assert!(!glob_matches("exact", "exactly"));
    }

    #[test]
    fn path_match_is_component_wise_with_tilde_expansion() {
        let (metadata, diagnostics) = compile_with(
            &[entry(None, Some("~/work/herdr"), &[("env", "dev")])],
            expand,
        );
        assert!(diagnostics.is_empty());
        assert_eq!(resolve(&metadata, "x", "/home/me/work/herdr")["env"], "dev");
        assert_eq!(
            resolve(&metadata, "x", "/home/me/work/herdr/src")["env"],
            "dev"
        );
        assert!(resolve(&metadata, "x", "/home/me/work/herdr-old").is_empty());
        assert!(resolve(&metadata, "x", "").is_empty());
    }

    #[test]
    fn later_entries_override_earlier_ones() {
        let (metadata, _) = compile_with(
            &[
                entry(Some("api-*"), None, &[("env", "dev"), ("team", "a")]),
                entry(Some("*"), None, &[("env", "prod")]),
            ],
            expand,
        );
        let tokens = resolve(&metadata, "api-x", "/");
        assert_eq!(tokens["env"], "prod");
        assert_eq!(tokens["team"], "a");
    }

    #[test]
    fn invalid_entries_are_ignored_with_diagnostics() {
        let (metadata, diagnostics) = compile_with(
            &[
                entry(Some("a"), None, &[("my env", "x")]),
                entry(Some("a"), Some("/p"), &[("env", "x")]),
                entry(None, None, &[("env", "x")]),
                entry(Some("ok"), None, &[("env", "x"), ("empty", "  ")]),
            ],
            expand,
        );
        assert_eq!(diagnostics.len(), 3, "{diagnostics:?}");
        assert!(diagnostics[0].contains("\"my env\""));
        assert!(diagnostics[1].contains("not both"));
        assert!(diagnostics[2].contains("exactly one"));
        let tokens = resolve(&metadata, "ok", "/");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens["env"], "x");
    }

    #[test]
    fn limits_keep_the_first_entries_and_tokens() {
        let many = (0..MAX_SPACE_METADATA_ENTRIES + 1)
            .map(|i| entry(Some(&format!("s{i}")), None, &[("k", "v")]))
            .collect::<Vec<_>>();
        let (metadata, diagnostics) = compile_with(&many, expand);
        assert_eq!(metadata.entries.len(), MAX_SPACE_METADATA_ENTRIES);
        assert_eq!(diagnostics.len(), 1);

        let tokens = (0..MAX_SPACE_METADATA_TOKENS + 1)
            .map(|i| (format!("k{i:02}"), "v".to_string()))
            .collect();
        let raw = RawSpaceMetadataEntry {
            matcher: RawSpaceMetadataMatch {
                label: Some("a".into()),
                path: None,
            },
            tokens,
        };
        let (metadata, diagnostics) = compile_with(&[raw], expand);
        assert_eq!(metadata.entries[0].tokens.len(), MAX_SPACE_METADATA_TOKENS);
        assert_eq!(diagnostics.len(), 1);
    }
}
