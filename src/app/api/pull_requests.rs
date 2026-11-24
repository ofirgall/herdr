//! `pull_request.list` and the records `pull_requests.updated` carries: the
//! server's `[pull_requests]` poll as API data, so plugins and scripts read it
//! instead of polling GitHub themselves.

use crate::api::schema::{
    PullRequestCheckCountsInfo, PullRequestCheckOutcome, PullRequestCheckRunInfo, PullRequestInfo,
    PullRequestRepoInfo, PullRequestReviewState, ResponseResult,
};
use crate::app::pull_requests::{
    PullRequestChecks, PullRequestRepoState, PullRequestReview, PullRequestState,
};
use crate::app::App;

use super::responses::encode_success;

impl App {
    pub(super) fn handle_pull_request_list(&mut self, id: String) -> String {
        encode_success(
            id,
            ResponseResult::PullRequestList {
                repos: self.pull_request_repo_infos(),
            },
        )
    }

    pub(crate) fn pull_request_repo_infos(&self) -> Vec<PullRequestRepoInfo> {
        self.state.pull_requests.iter().map(repo_info).collect()
    }
}

fn repo_info(repo: &PullRequestRepoState) -> PullRequestRepoInfo {
    PullRequestRepoInfo {
        repo: repo.repo.clone(),
        label: repo.label.clone(),
        url: repo.url.clone(),
        loading: repo.loading,
        error: repo.error.clone(),
        pull_requests: repo.pull_requests.iter().map(pull_request_info).collect(),
    }
}

fn pull_request_info(pr: &PullRequestState) -> PullRequestInfo {
    PullRequestInfo {
        number: pr.number,
        title: pr.title.clone(),
        url: pr.url.clone(),
        branch: pr.branch.clone(),
        draft: pr.draft,
        merged: pr.merged,
        conflict: pr.conflict,
        queued: pr.queued,
        review: match pr.review {
            PullRequestReview::None => PullRequestReviewState::None,
            PullRequestReview::Approved => PullRequestReviewState::Approved,
            PullRequestReview::ChangesRequested => PullRequestReviewState::ChangesRequested,
            PullRequestReview::ReviewRequired => PullRequestReviewState::ReviewRequired,
        },
        checks: check_outcome(pr.checks),
        human_threads: pr.human_threads,
        bot_threads: pr.bot_threads,
        check_counts: PullRequestCheckCountsInfo {
            passed: pr.check_counts.passed,
            failed: pr.check_counts.failed,
            pending: pr.check_counts.pending,
            skipped: pr.check_counts.skipped,
        },
        check_runs: pr
            .check_runs
            .iter()
            .map(|run| PullRequestCheckRunInfo {
                name: run.name.clone(),
                outcome: check_outcome(run.outcome),
                url: run.url.clone(),
            })
            .collect(),
    }
}

fn check_outcome(checks: PullRequestChecks) -> PullRequestCheckOutcome {
    match checks {
        PullRequestChecks::None => PullRequestCheckOutcome::None,
        PullRequestChecks::Pass => PullRequestCheckOutcome::Pass,
        PullRequestChecks::Fail => PullRequestCheckOutcome::Fail,
        PullRequestChecks::Pending => PullRequestCheckOutcome::Pending,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::api::schema::{EventData, EventKind, SuccessResponse};
    use crate::app::pull_requests::{
        PullRequestCheckCounts, PullRequestCheckState, PullRequestDetails, PullRequestPoll,
        PullRequestRate,
    };
    use crate::config::{Config, PullRequestsConfig, WatchedRepoConfig};

    use super::*;

    fn app_with_hub() -> (App, crate::api::EventHub) {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            event_hub.clone(),
        );
        (app, event_hub)
    }

    fn watched() -> PullRequestsConfig {
        PullRequestsConfig {
            repos: vec![WatchedRepoConfig {
                path: "/src/web".into(),
                user: "@me".into(),
                label: None,
            }],
            ..PullRequestsConfig::default()
        }
    }

    fn fetched() -> PullRequestRepoState {
        let mut state = App::test_pending_pull_request_repo(&watched().repos[0]);
        state.repo = Some("octo/web".into());
        state.label = "web".into();
        state.url = Some("https://github.com/octo/web/pulls".into());
        state.loading = false;
        state.pull_requests = vec![PullRequestState {
            number: 7,
            title: "Fix it".into(),
            url: "https://github.com/octo/web/pull/7".into(),
            branch: "fix-it".into(),
            draft: false,
            merged: false,
            conflict: true,
            queued: false,
            review: PullRequestReview::ChangesRequested,
            checks: PullRequestChecks::Fail,
            human_threads: 2,
            bot_threads: 1,
            check_runs: vec![PullRequestCheckState {
                name: "lint".into(),
                outcome: PullRequestChecks::Fail,
                url: None,
            }],
            check_counts: PullRequestCheckCounts {
                passed: 3,
                failed: 1,
                pending: 0,
                skipped: 0,
            },
        }];
        state
    }

    #[test]
    fn the_list_returns_the_poll_and_a_changing_poll_emits_an_event() {
        let (mut app, event_hub) = app_with_hub();
        let now = Instant::now();
        app.configure_pull_requests(&watched(), now);
        let generation = app.test_pull_request_generation();
        let poll = || PullRequestPoll {
            repos: vec![fetched()],
            details: PullRequestDetails::new(),
            rate: PullRequestRate::Unknown,
        };
        assert!(app.handle_pull_requests_refreshed(generation, poll(), now));

        let response = app.handle_pull_request_list("req".into());
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::PullRequestList { repos } = success.result else {
            panic!("expected pull request list");
        };
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].repo.as_deref(), Some("octo/web"));
        let pr = &repos[0].pull_requests[0];
        assert_eq!((pr.number, pr.branch.as_str()), (7, "fix-it"));
        assert!(pr.conflict);
        assert_eq!(pr.review, PullRequestReviewState::ChangesRequested);
        assert_eq!(pr.checks, PullRequestCheckOutcome::Fail);
        assert_eq!((pr.human_threads, pr.bot_threads), (2, 1));
        assert_eq!(pr.check_runs[0].name, "lint");
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(
            json.pointer("/result/repos/0/pull_requests/0/review"),
            Some(&serde_json::json!("changes_requested"))
        );
        // Unset flags stay off the wire.
        assert!(json
            .pointer("/result/repos/0/pull_requests/0/draft")
            .is_none());

        let events = event_hub.events_after(0);
        let updates = events
            .iter()
            .filter(|(_, event)| event.event == EventKind::PullRequestsUpdated)
            .collect::<Vec<_>>();
        assert_eq!(updates.len(), 1);
        assert!(matches!(
            &updates[0].1.data,
            EventData::PullRequestsUpdated { repos: event_repos } if *event_repos == repos
        ));

        // An identical poll changes nothing and stays quiet.
        let sequence = events.last().map(|(sequence, _)| *sequence).unwrap_or(0);
        app.pull_request_poller_mark_in_flight_for_test();
        assert!(!app.handle_pull_requests_refreshed(generation, poll(), now));
        assert!(event_hub
            .events_after(sequence)
            .iter()
            .all(|(_, event)| event.event != EventKind::PullRequestsUpdated));
    }
}
