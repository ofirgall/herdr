//! `[pull_requests]`: GitHub repositories whose open pull requests the server
//! watches through the `gh` CLI.

use serde::{Deserialize, Serialize};

pub const DEFAULT_PULL_REQUEST_POLL_SECONDS: u64 = 120;
pub const MIN_PULL_REQUEST_POLL_SECONDS: u64 = 15;
pub const MAX_PULL_REQUEST_POLL_SECONDS: u64 = 3600;
pub const MAX_WATCHED_REPOS: usize = 32;
pub const DEFAULT_MERGED_PULL_REQUEST_HOURS: u64 = 5;
/// Thirty days.
pub const MAX_MERGED_PULL_REQUEST_HOURS: u64 = 720;
/// GitHub logins are at most 39 characters.
const MAX_GITHUB_LOGIN_CHARS: usize = 39;
/// The search qualifier for the authenticated `gh` user.
pub const CURRENT_GITHUB_USER: &str = "@me";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, try_from = "RawPullRequestsConfig")]
pub struct PullRequestsConfig {
    /// Seconds between GitHub polls.
    pub poll_seconds: u64,
    /// The `gh` executable, a name looked up on `PATH` or an absolute path.
    pub gh_command: String,
    /// Hours a merged pull request stays listed after its merge; 0 lists only
    /// open ones.
    pub merged_hours: u64,
    /// Watched repositories, in display order.
    pub repos: Vec<WatchedRepoConfig>,
}

impl Default for PullRequestsConfig {
    fn default() -> Self {
        Self {
            poll_seconds: DEFAULT_PULL_REQUEST_POLL_SECONDS,
            gh_command: "gh".into(),
            merged_hours: DEFAULT_MERGED_PULL_REQUEST_HOURS,
            repos: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchedRepoConfig {
    /// A local checkout; its `origin` remote names the GitHub repository.
    /// A leading `~/` expands to the home directory.
    pub path: String,
    /// The author whose open pull requests are listed: a GitHub login, or
    /// `@me` (the default) for the user `gh` is logged in as.
    #[serde(default = "current_github_user")]
    pub user: String,
    /// Heading shown for the repository; defaults to the repository name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

fn current_github_user() -> String {
    CURRENT_GITHUB_USER.into()
}

#[derive(Deserialize)]
#[serde(default)]
struct RawPullRequestsConfig {
    poll_seconds: u64,
    gh_command: String,
    merged_hours: u64,
    repos: Vec<WatchedRepoConfig>,
}

impl Default for RawPullRequestsConfig {
    fn default() -> Self {
        let PullRequestsConfig {
            poll_seconds,
            gh_command,
            merged_hours,
            repos,
        } = PullRequestsConfig::default();
        Self {
            poll_seconds,
            gh_command,
            merged_hours,
            repos,
        }
    }
}

impl TryFrom<RawPullRequestsConfig> for PullRequestsConfig {
    type Error = String;

    fn try_from(raw: RawPullRequestsConfig) -> Result<Self, Self::Error> {
        if !(MIN_PULL_REQUEST_POLL_SECONDS..=MAX_PULL_REQUEST_POLL_SECONDS)
            .contains(&raw.poll_seconds)
        {
            return Err(format!(
                "pull_requests.poll_seconds must be between {MIN_PULL_REQUEST_POLL_SECONDS} and {MAX_PULL_REQUEST_POLL_SECONDS}"
            ));
        }
        if raw.gh_command.trim().is_empty() {
            return Err("pull_requests.gh_command must not be empty".into());
        }
        if raw.merged_hours > MAX_MERGED_PULL_REQUEST_HOURS {
            return Err(format!(
                "pull_requests.merged_hours must be at most {MAX_MERGED_PULL_REQUEST_HOURS}"
            ));
        }
        if raw.repos.len() > MAX_WATCHED_REPOS {
            return Err(format!(
                "pull_requests.repos may contain at most {MAX_WATCHED_REPOS} entries"
            ));
        }
        for repo in &raw.repos {
            if repo.path.trim().is_empty() {
                return Err("pull_requests.repos entries need a path".into());
            }
            // The login goes into a GitHub search string, so only characters a
            // login can contain are accepted.
            if !is_valid_github_user(&repo.user) {
                return Err(format!(
                    "pull_requests.repos user `{}` is not a GitHub login or @me",
                    repo.user
                ));
            }
        }
        Ok(Self {
            poll_seconds: raw.poll_seconds,
            gh_command: raw.gh_command,
            merged_hours: raw.merged_hours,
            repos: raw.repos,
        })
    }
}

/// A GitHub login (letters, digits and single inner hyphens, at most 39
/// characters) or `@me`.
pub fn is_valid_github_user(user: &str) -> bool {
    if user == CURRENT_GITHUB_USER {
        return true;
    }
    !user.is_empty()
        && user.len() <= MAX_GITHUB_LOGIN_CHARS
        && user
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
        && !user.starts_with('-')
        && !user.ends_with('-')
        && !user.contains("--")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Result<PullRequestsConfig, toml::de::Error> {
        toml::from_str(input)
    }

    #[test]
    fn repos_parse_with_defaults() {
        let config = parse(
            r#"
            [[repos]]
            path = "~/work/drift"
            user = "octo-cat"

            [[repos]]
            path = "/src/herdr"
            label = "herdr fork"
            "#,
        )
        .unwrap();
        assert_eq!(config.poll_seconds, DEFAULT_PULL_REQUEST_POLL_SECONDS);
        assert_eq!(config.gh_command, "gh");
        assert_eq!(config.merged_hours, 5);
        assert_eq!(parse("merged_hours = 0").unwrap().merged_hours, 0);
        assert_eq!(config.repos.len(), 2);
        assert_eq!(config.repos[0].user, "octo-cat");
        assert_eq!(config.repos[1].user, CURRENT_GITHUB_USER);
        assert_eq!(config.repos[1].label.as_deref(), Some("herdr fork"));
        assert_eq!(parse("").unwrap(), PullRequestsConfig::default());
    }

    #[test]
    fn invalid_values_are_rejected() {
        for input in [
            "poll_seconds = 5",
            "poll_seconds = 7200",
            "merged_hours = 721",
            "gh_command = ' '",
            "[[repos]]\npath = ''",
            "[[repos]]\npath = '/x'\nuser = 'a b'",
            "[[repos]]\npath = '/x'\nuser = 'x repo:evil/repo'",
            "[[repos]]\npath = '/x'\nuser = '-octo'",
            "[[repos]]\npath = '/x'\nuser = ''",
        ] {
            assert!(parse(input).is_err(), "{input}");
        }
        let too_many = "[[repos]]\npath = '/x'\n".repeat(MAX_WATCHED_REPOS + 1);
        assert!(parse(&too_many).is_err());
    }

    #[test]
    fn github_users_are_logins_or_me() {
        for user in ["@me", "octocat", "octo-cat", "a", &"a".repeat(39)] {
            assert!(is_valid_github_user(user), "{user}");
        }
        for user in ["", "octo--cat", "octo-", "me@x", "a b", &"a".repeat(40)] {
            assert!(!is_valid_github_user(user), "{user}");
        }
    }
}
