//! Fetching watched repositories' open pull requests through `gh api graphql`.
//!
//! Everything except [`fetch_pull_requests`] and [`run_command`] is pure, so
//! remote parsing, the query document and the status rules are tested offline.
//! The status rules follow the herdr-pr-tracker plugin: review state falls back
//! to the latest opinionated reviews, cancelled and skipped checks carry no
//! signal, and threads whose last comment is the viewer's do not count.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{
    PullRequestCheckCounts, PullRequestCheckState, PullRequestChecks, PullRequestRepoState,
    PullRequestReview, PullRequestState,
};
use crate::config::WatchedRepoConfig;

/// Open pull requests fetched per repository.
const MAX_PULL_REQUESTS_PER_REPO: usize = 50;
/// Review threads read per pull request; counts are floors beyond this.
const MAX_REVIEW_THREADS: usize = 100;
const GIT_TIMEOUT: Duration = Duration::from_secs(5);
const GH_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest error text kept for display.
const MAX_ERROR_CHARS: usize = 200;

/// A watched repository resolved to its GitHub `owner/name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedRepo {
    pub(super) owner: String,
    pub(super) name: String,
    pub(super) user: String,
}

impl ResolvedRepo {
    fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Fetches every watched repository's open pull requests. Repositories whose
/// checkout cannot be resolved report an error without blocking the others.
pub(super) fn fetch_pull_requests(
    repos: &[WatchedRepoConfig],
    gh_command: &str,
) -> Vec<PullRequestRepoState> {
    let mut states = repos
        .iter()
        .map(PullRequestRepoState::pending)
        .collect::<Vec<_>>();
    let mut resolved = Vec::new();
    for (index, repo) in repos.iter().enumerate() {
        match resolve_repo(repo) {
            Ok(found) => {
                let state = &mut states[index];
                if repo.label.is_none() {
                    state.label = found.name.clone();
                }
                state.url = Some(format!("https://github.com/{}/pulls", found.slug()));
                resolved.push((index, found));
            }
            Err(error) => states[index].fail(error),
        }
    }
    if resolved.is_empty() {
        return states;
    }
    let queries = resolved
        .iter()
        .map(|(_, repo)| search_query(repo))
        .collect::<Vec<_>>();
    let document = graphql_document(&queries);
    let response = run_command(
        Command::new(gh_command)
            .args(["api", "graphql", "-f"])
            .arg(format!("query={document}")),
        GH_TIMEOUT,
    )
    .and_then(|stdout| {
        serde_json::from_str::<Value>(&stdout).map_err(|err| format!("unreadable gh output: {err}"))
    });
    match response {
        Ok(value) => {
            let results = parse_response(&value, resolved.len());
            for ((index, _), result) in resolved.iter().zip(results) {
                match result {
                    Ok(pull_requests) => {
                        let state = &mut states[*index];
                        state.pull_requests = pull_requests;
                        state.loading = false;
                    }
                    Err(error) => states[*index].fail(error),
                }
            }
        }
        Err(error) => {
            for (index, _) in &resolved {
                states[*index].fail(error.clone());
            }
        }
    }
    states
}

fn resolve_repo(repo: &WatchedRepoConfig) -> Result<ResolvedRepo, String> {
    let path = expand_home(&repo.path);
    if !path.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    let remote = run_command(
        Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["remote", "get-url", "origin"]),
        GIT_TIMEOUT,
    )?;
    let (owner, name) = github_repo_from_remote(remote.trim())
        .ok_or_else(|| "origin is not a GitHub repository".to_owned())?;
    Ok(ResolvedRepo {
        owner,
        name,
        user: repo.user.clone(),
    })
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|home| Path::new(&home).join(rest))
            .unwrap_or_else(|| PathBuf::from(path)),
        None if path == "~" => std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

/// `owner/name` of a `github.com` remote in scp, ssh, https or git form.
pub(super) fn github_repo_from_remote(remote: &str) -> Option<(String, String)> {
    let rest = remote
        .strip_prefix("git@github.com:")
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))
        .or_else(|| remote.strip_prefix("ssh://git@github.com:22/"))
        .or_else(|| remote.strip_prefix("https://github.com/"))
        .or_else(|| remote.strip_prefix("http://github.com/"))
        .or_else(|| remote.strip_prefix("git://github.com/"))
        .or_else(|| {
            // https://user@github.com/owner/name
            let after_scheme = remote.strip_prefix("https://")?;
            let (_, host_path) = after_scheme.split_once('@')?;
            host_path.strip_prefix("github.com/")
        })?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    };
    (valid(owner) && valid(name)).then(|| (owner.to_owned(), name.to_owned()))
}

/// The GitHub search for one repository's open pull requests by its user.
pub(super) fn search_query(repo: &ResolvedRepo) -> String {
    format!(
        "repo:{} is:pr is:open author:{} archived:false",
        repo.slug(),
        repo.user
    )
}

/// One GraphQL document searching every repository, aliased `r0`, `r1`, ...
pub(super) fn graphql_document(queries: &[String]) -> String {
    let mut document = String::from("query {\n  viewer { login }\n");
    for (index, query) in queries.iter().enumerate() {
        // A JSON string literal is a valid GraphQL string literal.
        let literal = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".into());
        document.push_str(&format!(
            "  r{index}: search(query: {literal}, type: ISSUE, first: {MAX_PULL_REQUESTS_PER_REPO}) {{ nodes {{ ...PR }} }}\n"
        ));
    }
    document.push_str("}\n");
    document.push_str(&format!(
        "fragment PR on PullRequest {{
  number title url isDraft createdAt mergeable headRefName reviewDecision
  reviewRequests(first: 1) {{ totalCount }}
  latestOpinionatedReviews(first: 20) {{ nodes {{ state }} }}
  mergeQueueEntry {{ state }}
  reviewThreads(first: {MAX_REVIEW_THREADS}) {{
    nodes {{ isResolved comments(last: 1) {{ nodes {{ author {{ __typename login }} }} }} }}
  }}
  commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ contexts(first: 100) {{
    checkRunCountsByState {{ state count }}
    statusContextCountsByState {{ state count }}
    nodes {{
    __typename
    ... on CheckRun {{ name conclusion status detailsUrl }}
    ... on StatusContext {{ context state targetUrl }}
  }} }} }} }} }} }}
}}
"
    ));
    document
}

/// Splits a response into one result per aliased search. A search missing
/// from `data` reports the response's first error.
pub(super) fn parse_response(
    value: &Value,
    count: usize,
) -> Vec<Result<Vec<PullRequestState>, String>> {
    let first_error = value
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(clean_error);
    let viewer = value
        .pointer("/data/viewer/login")
        .and_then(Value::as_str)
        .unwrap_or_default();
    (0..count)
        .map(|index| {
            let Some(nodes) = value
                .pointer(&format!("/data/r{index}/nodes"))
                .and_then(Value::as_array)
            else {
                return Err(first_error
                    .clone()
                    .unwrap_or_else(|| "no search result".to_owned()));
            };
            let mut pull_requests = nodes
                .iter()
                .filter_map(|node| pull_request_from(node, viewer))
                .collect::<Vec<_>>();
            // Oldest first, so rows do not move when a status changes.
            pull_requests.sort_by(|left, right| left.0.cmp(&right.0));
            Ok(pull_requests.into_iter().map(|(_, pr)| pr).collect())
        })
        .collect()
}

/// One search node, keyed by its creation time. Nodes that are not pull
/// requests (issues match the inline fragment as empty objects) are dropped.
fn pull_request_from(node: &Value, viewer: &str) -> Option<(String, PullRequestState)> {
    let number = node.get("number")?.as_u64()?;
    let text = |key: &str| {
        node.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut human_threads = 0;
    let mut bot_threads = 0;
    for thread in node
        .pointer("/reviewThreads/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if thread.get("isResolved").and_then(Value::as_bool) != Some(false) {
            continue;
        }
        let author = thread.pointer("/comments/nodes/0/author");
        let login = author
            .and_then(|author| author.get("login"))
            .and_then(Value::as_str);
        if !viewer.is_empty() && login == Some(viewer) {
            continue;
        }
        let is_bot = author
            .and_then(|author| author.get("__typename"))
            .and_then(Value::as_str)
            == Some("Bot");
        if is_bot {
            bot_threads += 1;
        } else {
            human_threads += 1;
        }
    }
    let connection = node.pointer("/commits/nodes/0/commit/statusCheckRollup/contexts");
    let contexts = connection
        .and_then(|connection| connection.get("nodes"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let check_counts = check_counts(connection, contexts);
    Some((
        text("createdAt"),
        PullRequestState {
            number,
            title: text("title").trim().to_owned(),
            url: text("url"),
            branch: text("headRefName"),
            draft: node.get("isDraft").and_then(Value::as_bool) == Some(true),
            // UNKNOWN is computed lazily by GitHub; only CONFLICTING is news.
            conflict: node.get("mergeable").and_then(Value::as_str) == Some("CONFLICTING"),
            queued: node
                .get("mergeQueueEntry")
                .is_some_and(|entry| !entry.is_null()),
            review: review_of(node),
            checks: check_counts.rollup(),
            human_threads,
            bot_threads,
            check_runs: check_runs_of(contexts),
            check_counts,
        },
    ))
}

/// `reviewDecision` where GitHub gives one, otherwise derived from the latest
/// opinionated reviews: GitHub only fills the decision when branch protection
/// requires a review.
fn review_of(node: &Value) -> PullRequestReview {
    match node.get("reviewDecision").and_then(Value::as_str) {
        Some("APPROVED") => return PullRequestReview::Approved,
        Some("CHANGES_REQUESTED") => return PullRequestReview::ChangesRequested,
        Some("REVIEW_REQUIRED") => return PullRequestReview::ReviewRequired,
        _ => {}
    }
    let states = node
        .pointer("/latestOpinionatedReviews/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|review| review.get("state").and_then(Value::as_str))
        .collect::<Vec<_>>();
    if states.contains(&"CHANGES_REQUESTED") {
        PullRequestReview::ChangesRequested
    } else if states.contains(&"APPROVED") {
        PullRequestReview::Approved
    } else if node
        .pointer("/reviewRequests/totalCount")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0)
    {
        PullRequestReview::ReviewRequired
    } else {
        PullRequestReview::None
    }
}

/// A check run conclusion or status, or a commit status state: `None` for a
/// cancelled, skipped, neutral or stale one, which carries no signal.
fn state_outcome(state: &str) -> PullRequestChecks {
    match state {
        "SKIPPED" | "NEUTRAL" | "CANCELLED" | "STALE" => PullRequestChecks::None,
        "FAILURE" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" | "ERROR" => {
            PullRequestChecks::Fail
        }
        "IN_PROGRESS" | "QUEUED" | "PENDING" | "WAITING" | "REQUESTED" | "EXPECTED" => {
            PullRequestChecks::Pending
        }
        _ => PullRequestChecks::Pass,
    }
}

/// One check run's or status context's outcome.
fn check_outcome(context: &Value) -> PullRequestChecks {
    let text = |key: &str| context.get(key).and_then(Value::as_str);
    let running = text("status").is_some_and(|status| status != "COMPLETED");
    match text("conclusion").or_else(|| text("state")) {
        _ if running => PullRequestChecks::Pending,
        None => PullRequestChecks::Pending,
        Some(outcome) => state_outcome(outcome),
    }
}

/// Every check by outcome: GitHub's per-state totals, which count past the
/// fetched page, else the fetched contexts.
fn check_counts(connection: Option<&Value>, contexts: &[Value]) -> PullRequestCheckCounts {
    let mut counts = PullRequestCheckCounts::default();
    let by_state = ["checkRunCountsByState", "statusContextCountsByState"]
        .into_iter()
        .filter_map(|key| connection?.get(key)?.as_array())
        .flatten()
        .collect::<Vec<_>>();
    if by_state.is_empty() {
        for context in contexts {
            counts.add(check_outcome(context), 1);
        }
        return counts;
    }
    for entry in by_state {
        let state = entry
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let count = entry
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        counts.add(
            state_outcome(state),
            u32::try_from(count).unwrap_or(u32::MAX),
        );
    }
    counts
}

/// Collapses check runs and status contexts, worst news first.
#[cfg(test)]
fn rollup_checks(contexts: &[Value]) -> PullRequestChecks {
    check_counts(None, contexts).rollup()
}

/// The failing check runs and status contexts, then the running ones, by
/// name within each.
pub(super) fn check_runs_of(contexts: &[Value]) -> Vec<PullRequestCheckState> {
    let rank = |outcome: PullRequestChecks| match outcome {
        PullRequestChecks::Fail => 0,
        PullRequestChecks::Pending => 1,
        PullRequestChecks::Pass | PullRequestChecks::None => 2,
    };
    let mut runs = contexts
        .iter()
        .filter_map(|context| {
            let text = |key: &str| {
                context
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
            };
            let outcome = check_outcome(context);
            if !matches!(
                outcome,
                PullRequestChecks::Fail | PullRequestChecks::Pending
            ) {
                return None;
            }
            let name = text("name").or_else(|| text("context"))?;
            Some(PullRequestCheckState {
                name: name.to_owned(),
                outcome,
                url: text("detailsUrl")
                    .or_else(|| text("targetUrl"))
                    .map(str::to_owned),
            })
        })
        .collect::<Vec<_>>();
    runs.sort_by(|left, right| {
        rank(left.outcome)
            .cmp(&rank(right.outcome))
            .then_with(|| left.name.cmp(&right.name))
    });
    runs
}

fn clean_error(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    line.trim().chars().take(MAX_ERROR_CHARS).collect()
}

/// Runs `command` without a shell, returning stdout or a short error. The
/// child is killed when it outlives `timeout`.
fn run_command(command: &mut Command, timeout: Duration) -> Result<String, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::platform::configure_background_command(command);
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .spawn()
        .map_err(|err| clean_error(&format!("{program}: {err}")))?;
    // Read both pipes on their own threads so a large response cannot fill a
    // pipe and stall the child while it is being waited on.
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        crate::thread_spawn::spawn_named("herdr-pr-pipe", move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let readers = reader(child.stdout.take().map(|pipe| Box::new(pipe) as _)).and_then(|stdout| {
        reader(child.stderr.take().map(|pipe| Box::new(pipe) as _)).map(|stderr| (stdout, stderr))
    });
    let (stdout, stderr) = match readers {
        Ok(readers) => readers,
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{program}: {err}"));
        }
    };
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} timed out"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(err) => return Err(clean_error(&format!("{program}: {err}"))),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if status.success() {
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    } else {
        let message = String::from_utf8_lossy(&stderr);
        Err(if message.trim().is_empty() {
            format!("{program} failed ({status})")
        } else {
            clean_error(&message)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn github_remotes_in_every_form_resolve() {
        let expected = Some(("octo".to_owned(), "web-app".to_owned()));
        for remote in [
            "git@github.com:octo/web-app.git",
            "git@github.com:octo/web-app",
            "ssh://git@github.com/octo/web-app.git",
            "https://github.com/octo/web-app",
            "https://github.com/octo/web-app.git/",
            "https://token@github.com/octo/web-app.git",
        ] {
            assert_eq!(github_repo_from_remote(remote), expected, "{remote}");
        }
        for remote in [
            "git@gitlab.com:octo/web-app.git",
            "https://github.com/octo",
            "https://github.com/octo/web app",
            "",
        ] {
            assert_eq!(github_repo_from_remote(remote), None, "{remote}");
        }
    }

    #[test]
    fn the_document_aliases_one_search_per_repository() {
        let repo = |name: &str, user: &str| ResolvedRepo {
            owner: "octo".into(),
            name: name.into(),
            user: user.into(),
        };
        let queries = vec![
            search_query(&repo("web", "@me")),
            search_query(&repo("api", "hubot")),
        ];
        assert_eq!(
            queries[1],
            "repo:octo/api is:pr is:open author:hubot archived:false"
        );
        let document = graphql_document(&queries);
        assert!(document.contains(
            "r0: search(query: \"repo:octo/web is:pr is:open author:@me archived:false\""
        ));
        assert!(document.contains("r1: search(query: \"repo:octo/api"));
        assert!(document.contains("viewer { login }"));
        assert!(document.contains("fragment PR on PullRequest"));
    }

    #[test]
    fn check_rollup_reports_the_worst_news() {
        let run = |conclusion: Option<&str>, status: &str| json!({ "__typename": "CheckRun", "conclusion": conclusion, "status": status });
        let status = |state: &str| json!({ "__typename": "StatusContext", "state": state });
        assert_eq!(rollup_checks(&[]), PullRequestChecks::None);
        assert_eq!(
            rollup_checks(&[run(Some("SUCCESS"), "COMPLETED"), status("SUCCESS")]),
            PullRequestChecks::Pass
        );
        assert_eq!(
            rollup_checks(&[run(Some("SUCCESS"), "COMPLETED"), run(None, "IN_PROGRESS")]),
            PullRequestChecks::Pending
        );
        assert_eq!(
            rollup_checks(&[run(None, "QUEUED"), run(Some("TIMED_OUT"), "COMPLETED")]),
            PullRequestChecks::Fail
        );
        assert_eq!(
            rollup_checks(&[status("PENDING")]),
            PullRequestChecks::Pending
        );
        assert_eq!(
            rollup_checks(&[
                run(Some("CANCELLED"), "COMPLETED"),
                run(Some("SKIPPED"), "COMPLETED")
            ]),
            PullRequestChecks::None
        );
    }

    #[test]
    fn check_runs_list_failing_then_running_checks() {
        let runs = check_runs_of(&[
            json!({ "__typename": "CheckRun", "name": "test", "conclusion": "SUCCESS", "status": "COMPLETED", "detailsUrl": "https://github.com/o/r/actions/runs/1/job/2" }),
            json!({ "__typename": "CheckRun", "name": "lint", "conclusion": "FAILURE", "status": "COMPLETED", "detailsUrl": "" }),
            json!({ "__typename": "CheckRun", "name": "deploy", "conclusion": "SKIPPED", "status": "COMPLETED" }),
            json!({ "__typename": "StatusContext", "context": "ci/build", "state": "PENDING", "targetUrl": "https://ci.example/1" }),
            json!({ "__typename": "CheckRun", "name": "build", "conclusion": "SUCCESS", "status": "COMPLETED" }),
            json!({ "__typename": "CheckRun" }),
        ]);
        assert_eq!(
            runs.iter()
                .map(|run| (run.name.as_str(), run.outcome, run.url.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("lint", PullRequestChecks::Fail, None),
                (
                    "ci/build",
                    PullRequestChecks::Pending,
                    Some("https://ci.example/1")
                ),
            ]
        );
    }

    #[test]
    fn check_counts_prefer_githubs_totals() {
        let connection = json!({
            "checkRunCountsByState": [
                { "state": "SUCCESS", "count": 67 },
                { "state": "SKIPPED", "count": 40 },
                { "state": "CANCELLED", "count": 1 },
                { "state": "IN_PROGRESS", "count": 2 },
                { "state": "STARTUP_FAILURE", "count": 1 }
            ],
            "statusContextCountsByState": [
                { "state": "SUCCESS", "count": 1 },
                { "state": "EXPECTED", "count": 1 }
            ]
        });
        let fetched =
            [json!({ "__typename": "CheckRun", "conclusion": "SUCCESS", "status": "COMPLETED" })];
        let counts = check_counts(Some(&connection), &fetched);
        assert_eq!(
            counts,
            PullRequestCheckCounts {
                passed: 68,
                failed: 1,
                pending: 3,
                skipped: 41
            }
        );
        assert_eq!(counts.rollup(), PullRequestChecks::Fail);
        // Without totals the fetched contexts are counted.
        assert_eq!(
            check_counts(Some(&json!({})), &fetched),
            PullRequestCheckCounts {
                passed: 1,
                ..Default::default()
            }
        );
    }

    fn node(number: u64, created: &str) -> Value {
        json!({
            "number": number,
            "title": format!("  PR {number} "),
            "url": format!("https://github.com/octo/web/pull/{number}"),
            "isDraft": false,
            "createdAt": created,
            "mergeable": "MERGEABLE",
            "headRefName": format!("branch-{number}"),
            "reviewDecision": null,
            "reviewRequests": { "totalCount": 0 },
            "latestOpinionatedReviews": { "nodes": [] },
            "mergeQueueEntry": null,
            "reviewThreads": { "nodes": [] },
            "commits": { "nodes": [] },
        })
    }

    #[test]
    fn responses_parse_into_ordered_pull_requests_per_search() {
        let mut conflicted = node(7, "2026-01-02T00:00:00Z");
        conflicted["mergeable"] = json!("CONFLICTING");
        conflicted["isDraft"] = json!(true);
        conflicted["mergeQueueEntry"] = json!({ "state": "QUEUED" });
        conflicted["latestOpinionatedReviews"] =
            json!({ "nodes": [{ "state": "APPROVED" }, { "state": "CHANGES_REQUESTED" }] });
        conflicted["reviewThreads"] = json!({ "nodes": [
            { "isResolved": false, "comments": { "nodes": [{ "author": { "__typename": "User", "login": "reviewer" } }] } },
            { "isResolved": false, "comments": { "nodes": [{ "author": { "__typename": "Bot", "login": "coderabbit" } }] } },
            { "isResolved": false, "comments": { "nodes": [{ "author": { "__typename": "User", "login": "me" } }] } },
            { "isResolved": true, "comments": { "nodes": [{ "author": { "__typename": "User", "login": "reviewer" } }] } },
        ] });
        let mut requested = node(3, "2026-01-01T00:00:00Z");
        requested["reviewRequests"] = json!({ "totalCount": 1 });
        let value = json!({
            "data": {
                "viewer": { "login": "me" },
                "r0": { "nodes": [conflicted, requested, {}] },
                "r1": { "nodes": [] },
            }
        });
        let results = parse_response(&value, 2);
        let first = results[0].as_ref().unwrap();
        assert_eq!(
            first.iter().map(|pr| pr.number).collect::<Vec<_>>(),
            vec![3, 7]
        );
        assert_eq!(first[0].review, PullRequestReview::ReviewRequired);
        assert_eq!(first[0].title, "PR 3");
        assert_eq!(first[0].branch, "branch-3");
        let pr = &first[1];
        assert!(pr.conflict && pr.draft && pr.queued);
        assert_eq!(pr.review, PullRequestReview::ChangesRequested);
        assert_eq!((pr.human_threads, pr.bot_threads), (1, 1));
        assert_eq!(results[1], Ok(Vec::new()));
    }

    #[test]
    fn the_review_decision_wins_over_derived_reviews() {
        let mut approved = node(1, "");
        approved["reviewDecision"] = json!("APPROVED");
        approved["latestOpinionatedReviews"] =
            json!({ "nodes": [{ "state": "CHANGES_REQUESTED" }] });
        assert_eq!(review_of(&approved), PullRequestReview::Approved);
        let mut derived = node(1, "");
        derived["latestOpinionatedReviews"] = json!({ "nodes": [{ "state": "APPROVED" }] });
        derived["reviewRequests"] = json!({ "totalCount": 2 });
        assert_eq!(review_of(&derived), PullRequestReview::Approved);
        assert_eq!(review_of(&node(1, "")), PullRequestReview::None);
    }

    #[test]
    fn missing_searches_report_the_first_error() {
        let value = json!({
            "data": { "r0": { "nodes": [] } },
            "errors": [{ "message": "Could not resolve to a Repository\nmore" }],
        });
        let results = parse_response(&value, 2);
        assert_eq!(results[0], Ok(Vec::new()));
        assert_eq!(
            results[1],
            Err("Could not resolve to a Repository".to_owned())
        );
        assert_eq!(
            parse_response(&json!({}), 1)[0],
            Err("no search result".to_owned())
        );
    }

    #[test]
    fn home_relative_paths_expand() {
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(expand_home("~/src/x"), Path::new(&home).join("src/x"));
        }
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
    }
}
