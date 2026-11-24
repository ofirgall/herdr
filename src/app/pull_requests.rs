//! Watched repositories' open pull requests, polled in the background.
//!
//! The server owns the poll: one worker thread at a time runs `gh api graphql`
//! for every `[pull_requests]` repository and reports back through
//! [`AppEvent::PullRequestsRefreshed`]. The result is a shared runtime fact in
//! [`AppState::pull_requests`](super::state::AppState::pull_requests); clients
//! only draw it.
//!
//! Polls spend the user's GitHub GraphQL budget, which `gh` and agents share.
//! Each poll lists pull requests cheaply and fetches reviews, threads and
//! checks only for those that changed ([`PullRequestDetails`]); the server
//! polls only while a client is attached, and the schedule backs off when
//! GitHub reports its rate limit ([`PullRequestRate`]).

mod github;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tracing::warn;

use super::App;
use crate::config::{PullRequestsConfig, WatchedRepoConfig};
use crate::events::AppEvent;

/// Delay before retrying when the worker thread could not start.
const SPAWN_RETRY_DELAY: Duration = Duration::from_secs(30);
/// Every pull request's details are refetched this often, catching changes
/// that leave the cheap fingerprint unchanged (a resolved thread, say).
const FULL_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// Below this many GraphQL points left, polls wait for the window to refill so
/// `gh` and agents keep working.
const LOW_RATE_BUDGET: u64 = 500;
/// Slack after GitHub's reported refill time before polling again.
const REFILL_GRACE: Duration = Duration::from_secs(5);
/// First wait after a rate limit without a known refill time (a secondary
/// limit); it doubles per consecutive refusal up to [`MAX_RATE_BACKOFF`].
const RATE_BACKOFF: Duration = Duration::from_secs(60);
const MAX_RATE_BACKOFF: Duration = Duration::from_secs(15 * 60);

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
    /// Merged within `merged_hours`; listed after the open ones.
    pub(crate) merged: bool,
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

/// Review and check details of one open pull request, refetched only when its
/// fingerprint changes or on a full refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequestDetail {
    /// The pull request's activity, head commit and check totals when fetched.
    pub(crate) fingerprint: String,
    /// Derived from the latest reviews and requests; `reviewDecision` wins.
    pub(crate) review: PullRequestReview,
    pub(crate) human_threads: u32,
    pub(crate) bot_threads: u32,
    pub(crate) check_runs: Vec<PullRequestCheckState>,
}

/// Details by `(owner/name, number)`.
pub(crate) type PullRequestDetails = HashMap<(String, u64), PullRequestDetail>;

/// GitHub's GraphQL rate-limit window as last reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RateWindow {
    pub(crate) remaining: u64,
    /// Until the window refills, from when the poll finished.
    pub(crate) refill_in: Duration,
    /// The refill time, `HH:MM` UTC.
    pub(crate) refill_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum PullRequestRate {
    /// The poll learned nothing about the budget (it failed otherwise).
    #[default]
    Unknown,
    /// The poll succeeded; the window as of its last request.
    Budget(RateWindow),
    /// GitHub refused a request for its rate limit. The window is known when
    /// `gh api rate_limit` answered: none left means the primary limit, some
    /// left a secondary (burst) limit.
    Limited(Option<RateWindow>),
}

/// One finished poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequestPoll {
    pub(crate) repos: Vec<PullRequestRepoState>,
    /// The details cache to keep for the next poll.
    pub(crate) details: PullRequestDetails,
    pub(crate) rate: PullRequestRate,
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
    details: PullRequestDetails,
    last_full_refresh_at: Option<Instant>,
    /// Consecutive rate-limited polls.
    rate_backoff_steps: u32,
}

impl PullRequestPoller {
    /// The wait before the next poll after one that reported `rate`, and the
    /// error to show in place of GitHub's when it was refused.
    fn schedule_after(&mut self, rate: &PullRequestRate) -> (Duration, Option<String>) {
        let interval = Duration::from_secs(self.config.poll_seconds);
        match rate {
            PullRequestRate::Unknown => {
                self.rate_backoff_steps = 0;
                (interval, None)
            }
            PullRequestRate::Budget(window) => {
                self.rate_backoff_steps = 0;
                if window.remaining < LOW_RATE_BUDGET {
                    (interval.max(window.refill_in + REFILL_GRACE), None)
                } else {
                    (interval, None)
                }
            }
            PullRequestRate::Limited(Some(window)) if window.remaining == 0 => {
                self.rate_backoff_steps = self.rate_backoff_steps.saturating_add(1);
                (
                    interval.max(window.refill_in + REFILL_GRACE),
                    Some(format!(
                        "GitHub rate limit reached; polling resumes at {} UTC",
                        window.refill_at
                    )),
                )
            }
            PullRequestRate::Limited(_) => {
                self.rate_backoff_steps = self.rate_backoff_steps.saturating_add(1);
                let doublings = self.rate_backoff_steps.saturating_sub(1).min(8);
                let backoff = RATE_BACKOFF
                    .saturating_mul(1 << doublings)
                    .min(MAX_RATE_BACKOFF);
                let wait = interval.max(backoff);
                (
                    wait,
                    Some(format!(
                        "GitHub rate limit; retrying in {} min",
                        wait.as_secs().div_ceil(60)
                    )),
                )
            }
        }
    }
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
        poller.details.clear();
        poller.last_full_refresh_at = None;
        poller.rate_backoff_steps = 0;
        self.state.pull_requests = config
            .repos
            .iter()
            .map(PullRequestRepoState::pending)
            .collect();
    }

    /// Starts a poll when one is due. The server calls this only while a
    /// client is attached. Returns whether state changed.
    pub(crate) fn handle_pull_request_tasks(&mut self, now: Instant) -> bool {
        let poller = &mut self.pull_request_poller;
        if poller.in_flight || poller.next_run_at.is_none_or(|due| now < due) {
            return false;
        }
        let generation = poller.generation;
        let repos = poller.config.repos.clone();
        let gh_command = poller.config.gh_command.clone();
        let merged_hours = poller.config.merged_hours;
        let full = poller
            .last_full_refresh_at
            .is_none_or(|at| now.saturating_duration_since(at) >= FULL_REFRESH_INTERVAL);
        let details = poller.details.clone();
        let event_tx = self.event_tx.clone();
        let spawned = crate::thread_spawn::spawn_named("herdr-pr-poll", move || {
            let merged_since = github::merged_since(merged_hours, time::OffsetDateTime::now_utc());
            let poll = github::fetch_pull_requests(
                &repos,
                &gh_command,
                merged_since.as_deref(),
                &details,
                full,
            );
            let _ = event_tx.blocking_send(AppEvent::PullRequestsRefreshed { generation, poll });
        });
        match spawned {
            Ok(_) => {
                poller.in_flight = true;
                poller.next_run_at = None;
                if full {
                    poller.last_full_refresh_at = Some(now);
                }
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

    /// Stores a finished poll and schedules the next one. Returns whether
    /// state changed.
    pub(crate) fn handle_pull_requests_refreshed(
        &mut self,
        generation: u64,
        poll: PullRequestPoll,
        now: Instant,
    ) -> bool {
        let poller = &mut self.pull_request_poller;
        poller.in_flight = false;
        if generation != poller.generation {
            // Reconfigured mid-poll: the new config's poll is already due.
            return false;
        }
        let PullRequestPoll {
            mut repos,
            details,
            rate,
        } = poll;
        let (wait, notice) = poller.schedule_after(&rate);
        poller.next_run_at = Some(now + wait);
        poller.details = details;
        if let Some(notice) = notice {
            for repo in repos.iter_mut().filter(|repo| repo.error.is_some()) {
                repo.error = Some(notice.clone());
            }
        }
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
            merged: false,
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
        assert!(!app.handle_pull_requests_refreshed(
            generation + 1,
            poll(vec![fetched.clone()]),
            now
        ));
        assert!(app.state.pull_requests[0].loading);
        assert!(!app.pull_request_poller.in_flight);

        assert!(app.handle_pull_requests_refreshed(generation, poll(vec![fetched.clone()]), now));
        assert_eq!(app.state.pull_requests[0], fetched);
        assert_eq!(
            app.next_pull_request_deadline(),
            Some(now + Duration::from_secs(app.pull_request_poller.config.poll_seconds))
        );
        // An identical poll is not a change.
        assert!(!app.handle_pull_requests_refreshed(generation, poll(vec![fetched]), now));
    }

    fn poll(repos: Vec<PullRequestRepoState>) -> PullRequestPoll {
        PullRequestPoll {
            repos,
            details: PullRequestDetails::new(),
            rate: PullRequestRate::Unknown,
        }
    }

    fn window(remaining: u64, refill_in_secs: u64) -> RateWindow {
        RateWindow {
            remaining,
            refill_in: Duration::from_secs(refill_in_secs),
            refill_at: "09:26".into(),
        }
    }

    #[test]
    fn the_schedule_waits_out_a_low_or_exhausted_budget() {
        let mut poller = PullRequestPoller::default();
        let interval = Duration::from_secs(poller.config.poll_seconds);
        assert_eq!(
            poller.schedule_after(&PullRequestRate::Budget(window(4000, 1800))),
            (interval, None)
        );
        // Low: wait for the refill quietly, leaving the rest to `gh` and agents.
        assert_eq!(
            poller.schedule_after(&PullRequestRate::Budget(window(100, 1800))),
            (Duration::from_secs(1800) + REFILL_GRACE, None)
        );
        // A refill sooner than the interval does not poll early.
        assert_eq!(
            poller.schedule_after(&PullRequestRate::Budget(window(100, 1))),
            (interval, None)
        );
        let (wait, notice) =
            poller.schedule_after(&PullRequestRate::Limited(Some(window(0, 1800))));
        assert_eq!(wait, Duration::from_secs(1800) + REFILL_GRACE);
        assert_eq!(
            notice.as_deref(),
            Some("GitHub rate limit reached; polling resumes at 09:26 UTC")
        );
    }

    #[test]
    fn secondary_rate_limits_back_off_exponentially_until_a_success() {
        let mut poller = PullRequestPoller::default();
        poller.config.poll_seconds = 15;
        let waits = (0..7)
            .map(|_| {
                poller
                    .schedule_after(&PullRequestRate::Limited(Some(window(3000, 1800))))
                    .0
                    .as_secs()
            })
            .collect::<Vec<_>>();
        assert_eq!(waits, vec![60, 120, 240, 480, 900, 900, 900]);
        assert_eq!(
            poller.schedule_after(&PullRequestRate::Limited(None)),
            (
                MAX_RATE_BACKOFF,
                Some("GitHub rate limit; retrying in 15 min".into())
            )
        );
        poller.schedule_after(&PullRequestRate::Unknown);
        assert_eq!(
            poller.schedule_after(&PullRequestRate::Limited(None)).0,
            RATE_BACKOFF
        );
    }

    #[test]
    fn a_rate_limited_poll_keeps_rows_and_explains_itself() {
        let mut app = test_app();
        let now = Instant::now();
        app.configure_pull_requests(&config(&["/src/web", "/src/api"]), now);
        let generation = app.pull_request_poller.generation;
        app.state.pull_requests[0].pull_requests = vec![pull_request(5)];
        let repos = config(&["/src/web", "/src/api"])
            .repos
            .iter()
            .map(|repo| {
                let mut state = PullRequestRepoState::pending(repo);
                state.fail("gh: API rate limit already exceeded for user ID 1.".into());
                state
            })
            .collect();
        let mut details = PullRequestDetails::new();
        details.insert(
            ("octo/web".into(), 5),
            PullRequestDetail {
                fingerprint: "f".into(),
                review: PullRequestReview::None,
                human_threads: 2,
                bot_threads: 0,
                check_runs: Vec::new(),
            },
        );
        app.pull_request_poller.in_flight = true;
        assert!(app.handle_pull_requests_refreshed(
            generation,
            PullRequestPoll {
                repos,
                details: details.clone(),
                rate: PullRequestRate::Limited(Some(window(0, 600))),
            },
            now,
        ));
        let web = &app.state.pull_requests[0];
        assert_eq!(web.pull_requests, vec![pull_request(5)]);
        assert_eq!(
            web.error.as_deref(),
            Some("GitHub rate limit reached; polling resumes at 09:26 UTC")
        );
        assert_eq!(
            app.next_pull_request_deadline(),
            Some(now + Duration::from_secs(600) + REFILL_GRACE)
        );
        assert_eq!(app.pull_request_poller.details, details);
        // Reconfiguring forgets the details of the old repositories.
        app.configure_pull_requests(&config(&["/src/other"]), now);
        assert!(app.pull_request_poller.details.is_empty());
    }

    #[test]
    fn full_refreshes_recur_on_their_interval() {
        let mut app = test_app();
        let now = Instant::now();
        app.configure_pull_requests(&config(&["/nonexistent/herdr-pr-test"]), now);
        assert!(app.pull_request_poller.last_full_refresh_at.is_none());
        assert!(!app.handle_pull_request_tasks(now));
        assert!(app.pull_request_poller.in_flight);
        assert_eq!(app.pull_request_poller.last_full_refresh_at, Some(now));

        let generation = app.pull_request_poller.generation;
        let later = now + Duration::from_secs(300);
        app.handle_pull_requests_refreshed(generation, poll(Vec::new()), now);
        app.pull_request_poller.next_run_at = Some(later);
        app.handle_pull_request_tasks(later);
        // Not yet due for a full refresh: the stamp stays.
        assert_eq!(app.pull_request_poller.last_full_refresh_at, Some(now));

        let much_later = now + FULL_REFRESH_INTERVAL;
        app.handle_pull_requests_refreshed(generation, poll(Vec::new()), later);
        app.pull_request_poller.next_run_at = Some(much_later);
        app.handle_pull_request_tasks(much_later);
        assert_eq!(
            app.pull_request_poller.last_full_refresh_at,
            Some(much_later)
        );
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
