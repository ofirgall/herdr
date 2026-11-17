//! Watched repositories' open pull requests, polled in the background.
//!
//! The server owns the poll: one worker thread at a time runs `gh api graphql`
//! for every `[pull_requests]` repository and reports back through
//! [`AppEvent::PullRequestsRefreshed`]. The result is a shared runtime fact in
//! [`AppState::pull_requests`](super::state::AppState::pull_requests); clients
//! only draw it.

mod github;

use std::time::{Duration, Instant};

use tracing::warn;

use super::App;
use crate::config::{PullRequestsConfig, WatchedRepoConfig};
use crate::events::AppEvent;

/// Delay before retrying when the worker thread could not start.
const SPAWN_RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PullRequestReview {
    #[default]
    None,
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PullRequestChecks {
    #[default]
    None,
    Pass,
    Fail,
    Pending,
}

/// One open pull request and the signals it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequestState {
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) branch: String,
    pub(crate) draft: bool,
    /// Cannot merge without resolving a conflict.
    pub(crate) conflict: bool,
    /// In the merge queue.
    pub(crate) queued: bool,
    pub(crate) review: PullRequestReview,
    pub(crate) checks: PullRequestChecks,
    /// Unresolved review threads whose last comment is a human's, not the viewer's.
    pub(crate) human_threads: u32,
    /// Unresolved review threads whose last comment is a bot's.
    pub(crate) bot_threads: u32,
    /// The head commit's failing and running checks, failing first.
    pub(crate) check_runs: Vec<PullRequestCheckState>,
    /// All of the head commit's checks by outcome, beyond those fetched.
    pub(crate) check_counts: PullRequestCheckCounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PullRequestCheckCounts {
    pub(crate) passed: u32,
    pub(crate) failed: u32,
    pub(crate) pending: u32,
    /// Skipped, neutral, cancelled and stale.
    pub(crate) skipped: u32,
}

impl PullRequestCheckCounts {
    pub(crate) fn add(&mut self, outcome: PullRequestChecks, count: u32) {
        let slot = match outcome {
            PullRequestChecks::Pass => &mut self.passed,
            PullRequestChecks::Fail => &mut self.failed,
            PullRequestChecks::Pending => &mut self.pending,
            PullRequestChecks::None => &mut self.skipped,
        };
        *slot = slot.saturating_add(count);
    }

    /// The worst news: a failure, else a running check, else a pass.
    pub(crate) fn rollup(&self) -> PullRequestChecks {
        if self.failed > 0 {
            PullRequestChecks::Fail
        } else if self.pending > 0 {
            PullRequestChecks::Pending
        } else if self.passed > 0 {
            PullRequestChecks::Pass
        } else {
            PullRequestChecks::None
        }
    }
}

/// One check run (a GitHub Actions job, say) or commit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequestCheckState {
    pub(crate) name: String,
    /// `None` for a skipped, neutral, cancelled or stale result.
    pub(crate) outcome: PullRequestChecks,
    /// The run's page: the job log for an Actions job.
    pub(crate) url: Option<String>,
}

/// One watched repository's latest poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequestRepoState {
    /// Heading: the configured label, else the repository name, else the path.
    pub(crate) label: String,
    /// The repository's pull request list on GitHub, once resolved.
    pub(crate) url: Option<String>,
    /// No poll has finished for this repository yet.
    pub(crate) loading: bool,
    /// Why the latest poll failed; earlier pull requests are kept meanwhile.
    pub(crate) error: Option<String>,
    pub(crate) pull_requests: Vec<PullRequestState>,
}

impl PullRequestRepoState {
    fn pending(repo: &WatchedRepoConfig) -> Self {
        let label = repo.label.clone().unwrap_or_else(|| {
            std::path::Path::new(repo.path.trim_end_matches('/'))
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| repo.path.clone())
        });
        Self {
            label,
            url: None,
            loading: true,
            error: None,
            pull_requests: Vec::new(),
        }
    }

    fn fail(&mut self, error: String) {
        self.error = Some(error);
    }
}

/// Server-side poll schedule for `[pull_requests]`.
#[derive(Debug, Default)]
pub(crate) struct PullRequestPoller {
    config: PullRequestsConfig,
    /// Bumped on every reconfiguration; stale worker results are dropped.
    generation: u64,
    next_run_at: Option<Instant>,
    in_flight: bool,
}

impl App {
    /// Applies `[pull_requests]`. An unchanged config keeps the current
    /// results and schedule.
    pub(crate) fn configure_pull_requests(&mut self, config: &PullRequestsConfig, now: Instant) {
        let poller = &mut self.pull_request_poller;
        if poller.generation != 0 && poller.config == *config {
            return;
        }
        poller.config = config.clone();
        poller.generation = poller.generation.wrapping_add(1).max(1);
        poller.next_run_at = (!config.repos.is_empty()).then_some(now);
        self.state.pull_requests = config
            .repos
            .iter()
            .map(PullRequestRepoState::pending)
            .collect();
    }

    /// Starts a poll when one is due. Returns whether state changed.
    pub(crate) fn handle_pull_request_tasks(&mut self, now: Instant) -> bool {
        let poller = &mut self.pull_request_poller;
        if poller.in_flight || poller.next_run_at.is_none_or(|due| now < due) {
            return false;
        }
        let generation = poller.generation;
        let repos = poller.config.repos.clone();
        let gh_command = poller.config.gh_command.clone();
        let event_tx = self.event_tx.clone();
        let spawned = crate::thread_spawn::spawn_named("herdr-pr-poll", move || {
            let repos = github::fetch_pull_requests(&repos, &gh_command);
            let _ = event_tx.blocking_send(AppEvent::PullRequestsRefreshed { generation, repos });
        });
        match spawned {
            Ok(_) => {
                poller.in_flight = true;
                poller.next_run_at = None;
            }
            Err(err) => {
                warn!(err = %err, "failed to spawn pull request poll thread; retrying later");
                poller.next_run_at = Some(now + SPAWN_RETRY_DELAY);
            }
        }
        false
    }

    pub(crate) fn next_pull_request_deadline(&self) -> Option<Instant> {
        let poller = &self.pull_request_poller;
        (!poller.in_flight).then_some(poller.next_run_at).flatten()
    }

    /// Stores a finished poll. Returns whether state changed.
    pub(crate) fn handle_pull_requests_refreshed(
        &mut self,
        generation: u64,
        repos: Vec<PullRequestRepoState>,
        now: Instant,
    ) -> bool {
        let poller = &mut self.pull_request_poller;
        poller.in_flight = false;
        if generation != poller.generation {
            // Reconfigured mid-poll: the new config's poll is already due.
            return false;
        }
        poller.next_run_at = Some(now + Duration::from_secs(poller.config.poll_seconds));
        let mut changed = false;
        for (current, fetched) in self.state.pull_requests.iter_mut().zip(repos) {
            let next = merge_poll(current, fetched);
            if *current != next {
                *current = next;
                changed = true;
            }
        }
        changed
    }
}

/// A failed poll keeps the pull requests already shown, with the error.
fn merge_poll(
    current: &PullRequestRepoState,
    fetched: PullRequestRepoState,
) -> PullRequestRepoState {
    if fetched.error.is_none() {
        return fetched;
    }
    PullRequestRepoState {
        label: fetched.label,
        url: fetched.url.or_else(|| current.url.clone()),
        loading: false,
        error: fetched.error,
        pull_requests: current.pull_requests.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        )
    }

    fn config(paths: &[&str]) -> PullRequestsConfig {
        PullRequestsConfig {
            repos: paths
                .iter()
                .map(|path| WatchedRepoConfig {
                    path: (*path).into(),
                    user: "@me".into(),
                    label: None,
                })
                .collect(),
            ..PullRequestsConfig::default()
        }
    }

    fn pull_request(number: u64) -> PullRequestState {
        PullRequestState {
            number,
            title: format!("PR {number}"),
            url: format!("https://github.com/o/r/pull/{number}"),
            branch: format!("b{number}"),
            draft: false,
            conflict: false,
            queued: false,
            review: PullRequestReview::None,
            checks: PullRequestChecks::None,
            human_threads: 0,
            bot_threads: 0,
            check_runs: Vec::new(),
            check_counts: PullRequestCheckCounts::default(),
        }
    }

    #[test]
    fn configuring_seeds_pending_repos_and_schedules_a_poll() {
        let mut app = test_app();
        let now = Instant::now();
        app.configure_pull_requests(&config(&["/src/web/", "/src/api"]), now);
        assert_eq!(
            app.state
                .pull_requests
                .iter()
                .map(|repo| (repo.label.as_str(), repo.loading))
                .collect::<Vec<_>>(),
            vec![("web", true), ("api", true)]
        );
        assert_eq!(app.next_pull_request_deadline(), Some(now));

        app.configure_pull_requests(&PullRequestsConfig::default(), now);
        assert!(app.state.pull_requests.is_empty());
        assert_eq!(app.next_pull_request_deadline(), None);
    }

    #[test]
    fn results_apply_only_for_the_current_generation() {
        let mut app = test_app();
        let now = Instant::now();
        app.configure_pull_requests(&config(&["/src/web"]), now);
        let generation = app.pull_request_poller.generation;
        app.pull_request_poller.in_flight = true;

        let mut fetched = PullRequestRepoState::pending(&config(&["/src/web"]).repos[0]);
        fetched.loading = false;
        fetched.pull_requests = vec![pull_request(1)];
        assert!(!app.handle_pull_requests_refreshed(generation + 1, vec![fetched.clone()], now));
        assert!(app.state.pull_requests[0].loading);
        assert!(!app.pull_request_poller.in_flight);

        assert!(app.handle_pull_requests_refreshed(generation, vec![fetched.clone()], now));
        assert_eq!(app.state.pull_requests[0], fetched);
        assert_eq!(
            app.next_pull_request_deadline(),
            Some(now + Duration::from_secs(app.pull_request_poller.config.poll_seconds))
        );
        // An identical poll is not a change.
        assert!(!app.handle_pull_requests_refreshed(generation, vec![fetched], now));
    }

    #[test]
    fn a_failed_poll_keeps_the_shown_pull_requests() {
        let repo = &config(&["/src/web"]).repos[0];
        let mut current = PullRequestRepoState::pending(repo);
        current.loading = false;
        current.url = Some("https://github.com/o/web/pulls".into());
        current.pull_requests = vec![pull_request(4)];
        let mut fetched = PullRequestRepoState::pending(repo);
        fetched.fail("gh: not logged in".into());
        let merged = merge_poll(&current, fetched);
        assert_eq!(merged.pull_requests, current.pull_requests);
        assert_eq!(merged.url, current.url);
        assert_eq!(merged.error.as_deref(), Some("gh: not logged in"));
        assert!(!merged.loading);
    }

    #[test]
    fn an_unchanged_config_keeps_results() {
        let mut app = test_app();
        let now = Instant::now();
        let watched = config(&["/src/web"]);
        app.configure_pull_requests(&watched, now);
        app.state.pull_requests[0].pull_requests = vec![pull_request(9)];
        app.configure_pull_requests(&watched, now);
        assert_eq!(app.state.pull_requests[0].pull_requests.len(), 1);
    }
}
