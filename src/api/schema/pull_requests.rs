//! Records for the server's `[pull_requests]` poll: the watched repositories'
//! pull requests, as `pull_request.list` returns them and
//! `pull_requests.updated` delivers them.

use serde::{Deserialize, Serialize};

/// One watched repository's latest poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PullRequestRepoInfo {
    /// The GitHub repository, `owner/name`, once its checkout resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The configured label, else the repository name.
    pub label: String,
    /// The repository's pull request list on GitHub, once resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// No poll has finished for this repository yet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub loading: bool,
    /// Why the latest poll failed. The pull requests from the last good poll
    /// are kept meanwhile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Open pull requests oldest first, then recently merged ones.
    pub pull_requests: Vec<PullRequestInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PullRequestInfo {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// The head branch.
    pub branch: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub draft: bool,
    /// Merged recently; merged pull requests carry no other signals.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub merged: bool,
    /// Cannot merge without resolving a conflict.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conflict: bool,
    /// In the merge queue.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub queued: bool,
    pub review: PullRequestReviewState,
    /// The head commit's checks, worst news first.
    pub checks: PullRequestCheckOutcome,
    /// Unresolved review threads whose last comment is a human's other than
    /// the viewer's.
    pub human_threads: u32,
    /// Unresolved review threads whose last comment is a bot's.
    pub bot_threads: u32,
    pub check_counts: PullRequestCheckCountsInfo,
    /// The head commit's failing checks, then its running ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub check_runs: Vec<PullRequestCheckRunInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestReviewState {
    None,
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestCheckOutcome {
    /// No checks, or only skipped, neutral, cancelled or stale ones.
    None,
    Pass,
    Fail,
    Pending,
}

/// Every check of the head commit by outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PullRequestCheckCountsInfo {
    pub passed: u32,
    pub failed: u32,
    pub pending: u32,
    /// Skipped, neutral, cancelled and stale.
    pub skipped: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PullRequestCheckRunInfo {
    pub name: String,
    pub outcome: PullRequestCheckOutcome,
    /// The run's page, the job log for an Actions job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}
