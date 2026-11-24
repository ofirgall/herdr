//! Fetching watched repositories' pull requests through `gh api graphql`.
//!
//! GitHub charges GraphQL rate-limit points by the connections a query asks
//! for, so a poll runs in two tiers. One cheap document lists every
//! repository's pull requests with their summary signals and a fingerprint of
//! their activity, head commit and check totals. A second document fetches
//! reviews, review threads and check names only for open pull requests whose
//! fingerprint changed since they were last fetched, or for all of them on a
//! full refresh. Every response reports the remaining budget.
//!
//! Everything except [`fetch_pull_requests`], [`run_graphql`],
//! [`rate_limit_window`] and [`run_command`] is pure, so remote parsing, the
//! query documents and the status rules are tested offline. The status rules
//! follow the herdr-pr-tracker plugin: review state falls back to the latest
//! opinionated reviews, cancelled and skipped checks carry no signal, and
//! threads whose last comment is the viewer's do not count.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{
    PullRequestCheckCounts, PullRequestCheckState, PullRequestChecks, PullRequestDetail,
    PullRequestDetails, PullRequestPoll, PullRequestRate, PullRequestRepoState, PullRequestReview,
    PullRequestState, RateWindow,
};
use crate::config::WatchedRepoConfig;

/// Open pull requests listed per repository.
const MAX_PULL_REQUESTS_PER_REPO: usize = 25;
/// Recently merged pull requests listed per repository.
const MAX_MERGED_PULL_REQUESTS_PER_REPO: usize = 10;
/// Review threads read per pull request; counts are floors beyond this.
/// Each thread's last comment is its own connection, so this dominates the
/// details' cost.
const MAX_REVIEW_THREADS: usize = 30;
/// Pull requests whose details one document fetches.
const MAX_DETAILS_PER_DOCUMENT: usize = 40;
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

/// Which fragment a listing search selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Listing {
    /// Open pull requests with their summary signals.
    Open,
    /// Merged pull requests, which show only that they merged.
    Merged,
}

/// One open pull request whose details a poll fetches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DetailRequest {
    pub(super) owner: String,
    pub(super) name: String,
    pub(super) number: u64,
    pub(super) fingerprint: String,
}

impl DetailRequest {
    fn key(&self) -> (String, u64) {
        (format!("{}/{}", self.owner, self.name), self.number)
    }
}

/// One listed pull request before its details are joined in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Summary {
    /// Creation time, or merge time once merged: rows sort oldest first.
    sort_key: String,
    /// Empty for a merged pull request, which has no details.
    fingerprint: String,
    /// GitHub's `reviewDecision`, which wins over the derived review.
    decision: Option<PullRequestReview>,
    pr: PullRequestState,
}

impl Summary {
    fn with_detail(self, detail: Option<&PullRequestDetail>) -> PullRequestState {
        let mut pr = self.pr;
        if pr.merged {
            return pr;
        }
        if let Some(detail) = detail {
            pr.human_threads = detail.human_threads;
            pr.bot_threads = detail.bot_threads;
            pr.check_runs = detail.check_runs.clone();
        }
        pr.review = self
            .decision
            .or_else(|| detail.map(|detail| detail.review))
            .unwrap_or_default();
        pr
    }
}

/// The rate-limit window as reported across a poll's requests.
#[derive(Debug, Default)]
struct RateTracker {
    /// The latest `(remaining, resetAt)`.
    window: Option<(u64, String)>,
    limited: bool,
}

impl RateTracker {
    fn observe(&mut self, value: &Value) {
        let remaining = value
            .pointer("/data/rateLimit/remaining")
            .and_then(Value::as_u64);
        let reset_at = value
            .pointer("/data/rateLimit/resetAt")
            .and_then(Value::as_str);
        if let (Some(remaining), Some(reset_at)) = (remaining, reset_at) {
            self.window = Some((remaining, reset_at.to_owned()));
        }
    }

    fn observe_error(&mut self, error: &str) {
        self.limited |= is_rate_limit_error(error);
    }

    fn finish(self, gh_command: &str) -> PullRequestRate {
        let now = time::OffsetDateTime::now_utc();
        if self.limited {
            return PullRequestRate::Limited(rate_limit_window(gh_command, now));
        }
        self.window
            .and_then(|(remaining, reset_at)| {
                let reset = time::OffsetDateTime::parse(
                    &reset_at,
                    &time::format_description::well_known::Rfc3339,
                )
                .ok()?;
                Some(PullRequestRate::Budget(rate_window(
                    remaining,
                    reset.unix_timestamp(),
                    now,
                )))
            })
            .unwrap_or_default()
    }
}

/// Fetches every watched repository's open pull requests, then those merged
/// since `merged_since` when given. Details come from `cached` where an open
/// pull request's fingerprint is unchanged, unless `full`. Repositories whose
/// checkout cannot be resolved report an error without blocking the others.
pub(super) fn fetch_pull_requests(
    repos: &[WatchedRepoConfig],
    gh_command: &str,
    merged_since: Option<&str>,
    cached: &PullRequestDetails,
    full: bool,
) -> PullRequestPoll {
    let mut poll = PullRequestPoll {
        repos: repos.iter().map(PullRequestRepoState::pending).collect(),
        details: cached.clone(),
        rate: PullRequestRate::Unknown,
    };
    let states = &mut poll.repos;
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
        return poll;
    }
    let mut rate = RateTracker::default();

    // `r0..rN` the open searches, then `rN..r2N` the merged ones.
    let mut searches = resolved
        .iter()
        .map(|(_, repo)| {
            (
                search_query(repo),
                MAX_PULL_REQUESTS_PER_REPO,
                Listing::Open,
            )
        })
        .collect::<Vec<_>>();
    if let Some(since) = merged_since {
        searches.extend(resolved.iter().map(|(_, repo)| {
            (
                merged_query(repo, since),
                MAX_MERGED_PULL_REQUESTS_PER_REPO,
                Listing::Merged,
            )
        }));
    }
    let listing = match run_graphql(gh_command, &list_document(&searches)) {
        Ok(value) => value,
        Err(error) => {
            rate.observe_error(&error);
            for (index, _) in &resolved {
                states[*index].fail(error.clone());
            }
            poll.rate = rate.finish(gh_command);
            return poll;
        }
    };
    rate.observe(&listing);
    let viewer = listing
        .pointer("/data/viewer/login")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut results = parse_summaries(&listing, searches.len());
    let mut merged = results.split_off(resolved.len()).into_iter();
    let mut listed = Vec::new();
    for ((index, repo), result) in resolved.into_iter().zip(results) {
        let result = match (result, merged.next()) {
            (Ok(mut open), Some(Ok(merged))) => {
                open.extend(merged);
                Ok(open)
            }
            (Ok(_), Some(Err(error))) | (Err(error), _) => Err(error),
            (Ok(open), None) => Ok(open),
        };
        match result {
            Ok(summaries) => listed.push((index, repo, summaries)),
            Err(error) => states[index].fail(error),
        }
    }

    let wanted = listed
        .iter()
        .flat_map(|(_, repo, summaries)| {
            summaries
                .iter()
                .filter(|summary| !summary.pr.merged)
                .map(|summary| DetailRequest {
                    owner: repo.owner.clone(),
                    name: repo.name.clone(),
                    number: summary.pr.number,
                    fingerprint: summary.fingerprint.clone(),
                })
        })
        .filter(|request| full || !details_current(cached, request))
        .collect::<Vec<_>>();
    // Repositories whose details could not all be fetched, with why.
    let mut detail_errors = HashMap::<String, String>::new();
    let mut chunks = wanted.chunks(MAX_DETAILS_PER_DOCUMENT);
    for chunk in chunks.by_ref() {
        match run_graphql(gh_command, &detail_document(chunk)) {
            Ok(value) => {
                rate.observe(&value);
                for (request, detail) in chunk.iter().zip(parse_details(&value, chunk, &viewer)) {
                    match detail {
                        Ok(detail) => {
                            poll.details.insert(request.key(), detail);
                        }
                        Err(error) => {
                            detail_errors.entry(request.key().0).or_insert(error);
                        }
                    }
                }
            }
            Err(error) => {
                rate.observe_error(&error);
                for request in chunk {
                    detail_errors
                        .entry(request.key().0)
                        .or_insert_with(|| error.clone());
                }
                if rate.limited {
                    break;
                }
            }
        }
    }
    // Left unfetched after a rate limit.
    for request in chunks.flatten() {
        detail_errors
            .entry(request.key().0)
            .or_insert_with(|| "GitHub rate limit".to_owned());
    }

    for (index, repo, summaries) in listed {
        let slug = repo.slug();
        if let Some(error) = detail_errors.remove(&slug) {
            states[index].fail(error);
            continue;
        }
        let open = summaries
            .iter()
            .filter(|summary| !summary.pr.merged)
            .map(|summary| summary.pr.number)
            .collect::<HashSet<_>>();
        poll.details
            .retain(|(repo, number), _| *repo != slug || open.contains(number));
        let state = &mut states[index];
        state.pull_requests = summaries
            .into_iter()
            .map(|summary| {
                let detail = poll.details.get(&(slug.clone(), summary.pr.number));
                summary.with_detail(detail)
            })
            .collect();
        state.loading = false;
    }
    poll.rate = rate.finish(gh_command);
    poll
}

/// Whether `cached` holds details fetched at `request`'s fingerprint.
fn details_current(cached: &PullRequestDetails, request: &DetailRequest) -> bool {
    cached
        .get(&request.key())
        .is_some_and(|detail| detail.fingerprint == request.fingerprint)
}

/// Runs one GraphQL document through `gh`.
fn run_graphql(gh_command: &str, document: &str) -> Result<Value, String> {
    let stdout = run_command(
        Command::new(gh_command)
            .args(["api", "graphql", "-f"])
            .arg(format!("query={document}")),
        GH_TIMEOUT,
    )?;
    serde_json::from_str::<Value>(&stdout).map_err(|err| format!("unreadable gh output: {err}"))
}

/// Whether `gh` failed because GitHub refused the request for its primary or
/// secondary rate limit.
pub(super) fn is_rate_limit_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("rate limit") || error.contains("abuse detection")
}

/// The GraphQL window from `gh api rate_limit`, which costs no points.
fn rate_limit_window(gh_command: &str, now: time::OffsetDateTime) -> Option<RateWindow> {
    let stdout = run_command(
        Command::new(gh_command).args(["api", "rate_limit"]),
        GH_TIMEOUT,
    )
    .ok()?;
    rate_window_from_rest(&serde_json::from_str(&stdout).ok()?, now)
}

/// The GraphQL window in a REST `rate_limit` response.
pub(super) fn rate_window_from_rest(
    value: &Value,
    now: time::OffsetDateTime,
) -> Option<RateWindow> {
    let graphql = value.pointer("/resources/graphql")?;
    let remaining = graphql.get("remaining")?.as_u64()?;
    let reset = graphql.get("reset")?.as_i64()?;
    Some(rate_window(remaining, reset, now))
}

/// A window of `remaining` points that refills at unix time `reset`.
pub(super) fn rate_window(remaining: u64, reset: i64, now: time::OffsetDateTime) -> RateWindow {
    let refill_in = u64::try_from(reset.saturating_sub(now.unix_timestamp())).unwrap_or(0);
    let refill_at = time::OffsetDateTime::from_unix_timestamp(reset)
        .map(|at| format!("{:02}:{:02}", at.hour(), at.minute()))
        .unwrap_or_else(|_| "?".to_owned());
    RateWindow {
        remaining,
        refill_in: Duration::from_secs(refill_in),
        refill_at,
    }
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

/// The GitHub search for one repository's pull requests by its user merged
/// since `since` (`YYYY-MM-DDTHH:MM:SSZ`).
pub(super) fn merged_query(repo: &ResolvedRepo, since: &str) -> String {
    format!(
        "repo:{} is:pr is:merged author:{} merged:>={since} archived:false",
        repo.slug(),
        repo.user
    )
}

/// The `merged:>=` bound `hours` before `now`, or `None` when merged pull
/// requests are not listed.
pub(super) fn merged_since(hours: u64, now: time::OffsetDateTime) -> Option<String> {
    if hours == 0 {
        return None;
    }
    let hours = i64::try_from(hours).ok()?;
    let since = now.checked_sub(time::Duration::hours(hours))?;
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        since.year(),
        u8::from(since.month()),
        since.day(),
        since.hour(),
        since.minute(),
        since.second()
    ))
}

/// The listing document: every `(search, page size, listing)` aliased `r0`,
/// `r1`, ..., with the viewer and the rate-limit window. Only the fragments in
/// use are defined; GitHub rejects unused ones.
pub(super) fn list_document(searches: &[(String, usize, Listing)]) -> String {
    let mut document =
        String::from("query {\n  viewer { login }\n  rateLimit { cost remaining resetAt }\n");
    for (index, (query, first, listing)) in searches.iter().enumerate() {
        // A JSON string literal is a valid GraphQL string literal.
        let literal = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".into());
        let fragment = match listing {
            Listing::Open => "Summary",
            Listing::Merged => "Merged",
        };
        document.push_str(&format!(
            "  r{index}: search(query: {literal}, type: ISSUE, first: {first}) {{ nodes {{ ...{fragment} }} }}\n"
        ));
    }
    document.push_str("}\n");
    // The rollup's per-state totals cover every check, so one context
    // (whose nodes are not read) is enough to reach them.
    document.push_str(
        "fragment Summary on PullRequest {
  number title url isDraft createdAt updatedAt mergeable headRefName headRefOid reviewDecision
  mergeQueueEntry { state }
  commits(last: 1) { nodes { commit { statusCheckRollup { state contexts(first: 1) {
    checkRunCountsByState { state count }
    statusContextCountsByState { state count }
  } } } } }
}
",
    );
    if searches
        .iter()
        .any(|(_, _, listing)| *listing == Listing::Merged)
    {
        document.push_str(
            "fragment Merged on PullRequest { number title url merged mergedAt headRefName }\n",
        );
    }
    document
}

/// Each request's repository alias index: requests of one repository share a
/// `repository` field, in first-seen order.
fn detail_aliases(requests: &[DetailRequest]) -> Vec<usize> {
    let mut repos = Vec::<(&str, &str)>::new();
    requests
        .iter()
        .map(|request| {
            let repo = (request.owner.as_str(), request.name.as_str());
            match repos.iter().position(|seen| *seen == repo) {
                Some(index) => index,
                None => {
                    repos.push(repo);
                    repos.len() - 1
                }
            }
        })
        .collect()
}

/// The details document for `requests`: each repository aliased `d0`, `d1`,
/// ..., each pull request within it `p<number>`.
pub(super) fn detail_document(requests: &[DetailRequest]) -> String {
    let aliases = detail_aliases(requests);
    let mut document = String::from("query {\n  rateLimit { cost remaining resetAt }\n");
    let mut alias = 0;
    while let Some(first) = aliases.iter().position(|index| *index == alias) {
        let repo = &requests[first];
        let owner = serde_json::to_string(&repo.owner).unwrap_or_else(|_| "\"\"".into());
        let name = serde_json::to_string(&repo.name).unwrap_or_else(|_| "\"\"".into());
        document.push_str(&format!(
            "  d{alias}: repository(owner: {owner}, name: {name}) {{"
        ));
        for (request, _) in requests
            .iter()
            .zip(&aliases)
            .filter(|(_, index)| **index == alias)
        {
            let number = request.number;
            document.push_str(&format!(
                " p{number}: pullRequest(number: {number}) {{ ...Detail }}"
            ));
        }
        document.push_str(" }\n");
        alias += 1;
    }
    document.push_str("}\n");
    document.push_str(&format!(
        "fragment Detail on PullRequest {{
  reviewRequests(first: 1) {{ totalCount }}
  latestOpinionatedReviews(first: 20) {{ nodes {{ state }} }}
  reviewThreads(first: {MAX_REVIEW_THREADS}) {{
    nodes {{ isResolved comments(last: 1) {{ nodes {{ author {{ __typename login }} }} }} }}
  }}
  commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ contexts(first: 100) {{ nodes {{
    __typename
    ... on CheckRun {{ name conclusion status detailsUrl }}
    ... on StatusContext {{ context state targetUrl }}
  }} }} }} }} }} }}
}}
"
    ));
    document
}

/// The response's first error, for searches or pull requests missing from
/// `data`.
fn first_error(value: &Value) -> Option<String> {
    value
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(clean_error)
}

/// Splits a listing response into one result per aliased search, oldest
/// first within each. A search missing from `data` reports the response's
/// first error.
pub(super) fn parse_summaries(value: &Value, count: usize) -> Vec<Result<Vec<Summary>, String>> {
    let first_error = first_error(value);
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
            let mut summaries = nodes.iter().filter_map(summary_from).collect::<Vec<_>>();
            // Oldest first, so rows do not move when a status changes.
            summaries.sort_by(|left, right| left.sort_key.cmp(&right.sort_key));
            Ok(summaries)
        })
        .collect()
}

/// The details of each request, in order. A pull request missing from `data`
/// reports the response's first error.
pub(super) fn parse_details(
    value: &Value,
    requests: &[DetailRequest],
    viewer: &str,
) -> Vec<Result<PullRequestDetail, String>> {
    let first_error = first_error(value);
    requests
        .iter()
        .zip(detail_aliases(requests))
        .map(|(request, alias)| {
            value
                .pointer(&format!("/data/d{alias}/p{}", request.number))
                .filter(|node| node.is_object())
                .map(|node| detail_from(node, viewer, &request.fingerprint))
                .ok_or_else(|| {
                    first_error
                        .clone()
                        .unwrap_or_else(|| "no pull request result".to_owned())
                })
        })
        .collect()
}

/// One listing node. A merged one carries only that it merged. Nodes that
/// are not pull requests (issues match the inline fragment as empty objects)
/// are dropped.
fn summary_from(node: &Value) -> Option<Summary> {
    let number = node.get("number")?.as_u64()?;
    let text = |key: &str| {
        node.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    if node.get("merged").and_then(Value::as_bool) == Some(true) {
        return Some(Summary {
            sort_key: text("mergedAt"),
            fingerprint: String::new(),
            decision: None,
            pr: PullRequestState {
                number,
                title: text("title").trim().to_owned(),
                url: text("url"),
                branch: text("headRefName"),
                draft: false,
                merged: true,
                conflict: false,
                queued: false,
                review: PullRequestReview::None,
                checks: PullRequestChecks::None,
                human_threads: 0,
                bot_threads: 0,
                check_runs: Vec::new(),
                check_counts: PullRequestCheckCounts::default(),
            },
        });
    }
    let rollup = node.pointer("/commits/nodes/0/commit/statusCheckRollup");
    let connection = rollup.and_then(|rollup| rollup.get("contexts"));
    let check_counts = check_counts(connection, &[]);
    let decision = review_decision(node);
    // What can change the details: new activity (reviews, comments, review
    // requests), a new head commit (its checks), or check progress.
    let fingerprint = format!(
        "{}|{}|{}|{}|{:?}",
        text("updatedAt"),
        text("headRefOid"),
        rollup
            .and_then(|rollup| rollup.get("state"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
        text("reviewDecision"),
        check_counts,
    );
    Some(Summary {
        sort_key: text("createdAt"),
        fingerprint,
        decision,
        pr: PullRequestState {
            number,
            title: text("title").trim().to_owned(),
            url: text("url"),
            branch: text("headRefName"),
            draft: node.get("isDraft").and_then(Value::as_bool) == Some(true),
            merged: false,
            // UNKNOWN is computed lazily by GitHub; only CONFLICTING is news.
            conflict: node.get("mergeable").and_then(Value::as_str) == Some("CONFLICTING"),
            queued: node
                .get("mergeQueueEntry")
                .is_some_and(|entry| !entry.is_null()),
            review: decision.unwrap_or_default(),
            checks: check_counts.rollup(),
            human_threads: 0,
            bot_threads: 0,
            check_runs: Vec::new(),
            check_counts,
        },
    })
}

/// One pull request's details node, fetched at `fingerprint`.
fn detail_from(node: &Value, viewer: &str, fingerprint: &str) -> PullRequestDetail {
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
    let contexts = node
        .pointer("/commits/nodes/0/commit/statusCheckRollup/contexts/nodes")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    PullRequestDetail {
        fingerprint: fingerprint.to_owned(),
        review: derived_review(node),
        human_threads,
        bot_threads,
        check_runs: check_runs_of(contexts),
    }
}

/// GitHub's `reviewDecision`, which it fills only when branch protection
/// requires a review.
fn review_decision(node: &Value) -> Option<PullRequestReview> {
    match node.get("reviewDecision").and_then(Value::as_str) {
        Some("APPROVED") => Some(PullRequestReview::Approved),
        Some("CHANGES_REQUESTED") => Some(PullRequestReview::ChangesRequested),
        Some("REVIEW_REQUIRED") => Some(PullRequestReview::ReviewRequired),
        _ => None,
    }
}

/// The review state derived from the latest opinionated reviews and pending
/// review requests, for when GitHub gives no decision.
fn derived_review(node: &Value) -> PullRequestReview {
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

    fn resolved(name: &str, user: &str) -> ResolvedRepo {
        ResolvedRepo {
            owner: "octo".into(),
            name: name.into(),
            user: user.into(),
        }
    }

    #[test]
    fn the_listing_aliases_one_search_per_repository() {
        let searches = vec![
            (search_query(&resolved("web", "@me")), 25, Listing::Open),
            (search_query(&resolved("api", "hubot")), 25, Listing::Open),
            (
                merged_query(&resolved("web", "@me"), "2026-11-17T05:00:00Z"),
                10,
                Listing::Merged,
            ),
        ];
        assert_eq!(
            searches[1].0,
            "repo:octo/api is:pr is:open author:hubot archived:false"
        );
        assert_eq!(
            searches[2].0,
            "repo:octo/web is:pr is:merged author:@me merged:>=2026-11-17T05:00:00Z archived:false"
        );
        let document = list_document(&searches);
        assert!(document.contains(
            "r2: search(query: \"repo:octo/web is:pr is:merged author:@me merged:>=2026-11-17T05:00:00Z archived:false\", type: ISSUE, first: 10) { nodes { ...Merged } }"
        ));
        assert!(document.contains(
            "r0: search(query: \"repo:octo/web is:pr is:open author:@me archived:false\", type: ISSUE, first: 25) { nodes { ...Summary } }"
        ));
        assert!(document.contains("r1: search(query: \"repo:octo/api"));
        assert!(document.contains("viewer { login }"));
        assert!(document.contains("rateLimit { cost remaining resetAt }"));
        assert!(document.contains("fragment Summary on PullRequest"));
        assert!(document.contains("fragment Merged on PullRequest"));
        // The listing reads no per-thread or per-check connections.
        assert!(!document.contains("reviewThreads"));
        assert!(!document.contains("nodes { __typename"));
        // GitHub rejects a fragment no search uses.
        assert!(!list_document(&searches[..2]).contains("fragment Merged"));
    }

    fn request(name: &str, number: u64) -> DetailRequest {
        DetailRequest {
            owner: "octo".into(),
            name: name.into(),
            number,
            fingerprint: format!("f{number}"),
        }
    }

    #[test]
    fn the_details_document_groups_pull_requests_by_repository() {
        let requests = [request("web", 7), request("api", 3), request("web", 9)];
        let document = detail_document(&requests);
        assert!(document.contains(
            "d0: repository(owner: \"octo\", name: \"web\") { p7: pullRequest(number: 7) { ...Detail } p9: pullRequest(number: 9) { ...Detail } }"
        ));
        assert!(document.contains(
            "d1: repository(owner: \"octo\", name: \"api\") { p3: pullRequest(number: 3) { ...Detail } }"
        ));
        assert!(document.contains(&format!("reviewThreads(first: {MAX_REVIEW_THREADS})")));
        assert!(document.contains("rateLimit { cost remaining resetAt }"));

        let value = json!({
            "data": {
                "d0": { "p7": detail_node(), "p9": null },
                "d1": { "p3": detail_node() },
            },
            "errors": [{ "message": "Could not resolve to a PullRequest with the number of 9." }],
        });
        let details = parse_details(&value, &requests, "me");
        let first = details[0].as_ref().unwrap();
        assert_eq!(first.fingerprint, "f7");
        assert_eq!((first.human_threads, first.bot_threads), (1, 1));
        assert_eq!(first.review, PullRequestReview::ChangesRequested);
        assert_eq!(details[1].as_ref().unwrap().fingerprint, "f3");
        assert_eq!(
            details[2],
            Err("Could not resolve to a PullRequest with the number of 9.".to_owned())
        );
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

    #[test]
    fn merged_since_counts_back_from_now() {
        // 2026-11-17T03:04:05.5Z
        let now = time::OffsetDateTime::from_unix_timestamp_nanos(1_794_884_645_500_000_000)
            .expect("timestamp");
        assert_eq!(
            merged_since(5, now).as_deref(),
            Some("2026-11-16T22:04:05Z")
        );
        assert_eq!(merged_since(0, now), None);
    }

    fn node(number: u64, created: &str) -> Value {
        json!({
            "number": number,
            "title": format!("  PR {number} "),
            "url": format!("https://github.com/octo/web/pull/{number}"),
            "isDraft": false,
            "createdAt": created,
            "updatedAt": created,
            "mergeable": "MERGEABLE",
            "headRefName": format!("branch-{number}"),
            "headRefOid": "abc",
            "reviewDecision": null,
            "mergeQueueEntry": null,
            "commits": { "nodes": [] },
        })
    }

    fn detail_node() -> Value {
        json!({
            "reviewRequests": { "totalCount": 1 },
            "latestOpinionatedReviews": { "nodes": [{ "state": "APPROVED" }, { "state": "CHANGES_REQUESTED" }] },
            "reviewThreads": { "nodes": [
                { "isResolved": false, "comments": { "nodes": [{ "author": { "__typename": "User", "login": "reviewer" } }] } },
                { "isResolved": false, "comments": { "nodes": [{ "author": { "__typename": "Bot", "login": "coderabbit" } }] } },
                { "isResolved": false, "comments": { "nodes": [{ "author": { "__typename": "User", "login": "me" } }] } },
                { "isResolved": true, "comments": { "nodes": [{ "author": { "__typename": "User", "login": "reviewer" } }] } },
            ] },
            "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "contexts": { "nodes": [
                { "__typename": "CheckRun", "name": "lint", "conclusion": "FAILURE", "status": "COMPLETED" },
            ] } } } }] },
        })
    }

    #[test]
    fn listings_parse_into_ordered_summaries_per_search() {
        let mut conflicted = node(7, "2026-01-02T00:00:00Z");
        conflicted["mergeable"] = json!("CONFLICTING");
        conflicted["isDraft"] = json!(true);
        conflicted["mergeQueueEntry"] = json!({ "state": "QUEUED" });
        conflicted["commits"] = json!({ "nodes": [{ "commit": { "statusCheckRollup": {
            "state": "FAILURE",
            "contexts": { "checkRunCountsByState": [
                { "state": "SUCCESS", "count": 3 },
                { "state": "FAILURE", "count": 1 },
            ] },
        } } }] });
        let mut approved = node(3, "2026-01-01T00:00:00Z");
        approved["reviewDecision"] = json!("APPROVED");
        let value = json!({
            "data": {
                "viewer": { "login": "me" },
                "r0": { "nodes": [conflicted, approved, {}] },
                "r1": { "nodes": [] },
            }
        });
        let results = parse_summaries(&value, 2);
        let first = results[0].as_ref().unwrap();
        assert_eq!(
            first
                .iter()
                .map(|summary| summary.pr.number)
                .collect::<Vec<_>>(),
            vec![3, 7]
        );
        assert_eq!(first[0].decision, Some(PullRequestReview::Approved));
        assert_eq!(first[0].pr.title, "PR 3");
        assert_eq!(first[0].pr.branch, "branch-3");
        let pr = &first[1].pr;
        assert!(pr.conflict && pr.draft && pr.queued);
        assert_eq!(pr.checks, PullRequestChecks::Fail);
        assert_eq!((pr.check_counts.passed, pr.check_counts.failed), (3, 1));
        assert_eq!(results[1], Ok(Vec::new()));
    }

    #[test]
    fn the_fingerprint_follows_activity_head_and_check_progress() {
        let fingerprint = |node: &Value| summary_from(node).unwrap().fingerprint;
        let base = node(1, "2026-01-01T00:00:00Z");
        let same = fingerprint(&base);
        assert_eq!(fingerprint(&base.clone()), same);
        let mut title = base.clone();
        title["title"] = json!("renamed");
        assert_eq!(fingerprint(&title), same);
        let mut updated = base.clone();
        updated["updatedAt"] = json!("2026-01-02T00:00:00Z");
        assert_ne!(fingerprint(&updated), same);
        let mut pushed = base.clone();
        pushed["headRefOid"] = json!("def");
        assert_ne!(fingerprint(&pushed), same);
        let mut progressed = base;
        progressed["commits"] = json!({ "nodes": [{ "commit": { "statusCheckRollup": {
            "state": "PENDING",
            "contexts": { "checkRunCountsByState": [{ "state": "IN_PROGRESS", "count": 1 }] },
        } } }] });
        assert_ne!(fingerprint(&progressed), same);
    }

    #[test]
    fn details_join_their_summary_and_the_decision_wins() {
        let detail = detail_from(&detail_node(), "me", "f");
        let mut decided = node(1, "");
        decided["reviewDecision"] = json!("APPROVED");
        let pr = summary_from(&decided).unwrap().with_detail(Some(&detail));
        assert_eq!(pr.review, PullRequestReview::Approved);
        assert_eq!((pr.human_threads, pr.bot_threads), (1, 1));
        assert_eq!(
            pr.check_runs
                .iter()
                .map(|run| run.name.as_str())
                .collect::<Vec<_>>(),
            vec!["lint"]
        );
        let undecided = summary_from(&node(1, "")).unwrap();
        assert_eq!(
            undecided.clone().with_detail(Some(&detail)).review,
            PullRequestReview::ChangesRequested
        );
        assert_eq!(undecided.with_detail(None).review, PullRequestReview::None);
        let mut requested = detail_node();
        requested["latestOpinionatedReviews"] = json!({ "nodes": [] });
        assert_eq!(
            derived_review(&requested),
            PullRequestReview::ReviewRequired
        );
    }

    #[test]
    fn only_changed_pull_requests_need_details() {
        let mut cached = PullRequestDetails::new();
        cached.insert(
            ("octo/web".to_owned(), 7),
            detail_from(&detail_node(), "me", "f7"),
        );
        assert!(details_current(&cached, &request("web", 7)));
        let mut changed = request("web", 7);
        changed.fingerprint = "f8".into();
        assert!(!details_current(&cached, &changed));
        assert!(!details_current(&cached, &request("api", 7)));
    }

    #[test]
    fn merged_pull_requests_order_by_merge_time_and_drop_their_signals() {
        let merged = |number: u64, at: &str| {
            json!({
                "number": number,
                "title": "t",
                "url": "u",
                "merged": true,
                "mergedAt": at,
                "headRefName": "b",
            })
        };
        let value = json!({ "data": { "r0": { "nodes": [
            merged(4, "2026-11-17T02:00:00Z"),
            merged(9, "2026-11-17T01:00:00Z"),
        ] } } });
        let summaries = parse_summaries(&value, 1).remove(0).unwrap();
        let prs = summaries
            .into_iter()
            .map(|summary| summary.with_detail(None))
            .collect::<Vec<_>>();
        assert_eq!(
            prs.iter().map(|pr| pr.number).collect::<Vec<_>>(),
            vec![9, 4]
        );
        assert!(prs[0].merged && !prs[0].conflict);
        assert_eq!(prs[0].review, PullRequestReview::None);
    }

    #[test]
    fn missing_searches_report_the_first_error() {
        let value = json!({
            "data": { "r0": { "nodes": [] } },
            "errors": [{ "message": "Could not resolve to a Repository\nmore" }],
        });
        let results = parse_summaries(&value, 2);
        assert_eq!(results[0], Ok(Vec::new()));
        assert_eq!(
            results[1],
            Err("Could not resolve to a Repository".to_owned())
        );
        assert_eq!(
            parse_summaries(&json!({}), 1)[0],
            Err("no search result".to_owned())
        );
    }

    #[test]
    fn rate_limit_errors_and_windows_are_recognized() {
        assert!(is_rate_limit_error(
            "gh: API rate limit already exceeded for user ID 256115739."
        ));
        assert!(is_rate_limit_error(
            "gh: You have exceeded a secondary rate limit. Please wait a few minutes"
        ));
        assert!(!is_rate_limit_error(
            "gh: Could not resolve to a Repository"
        ));

        // 2026-11-24T08:55:54Z
        let now = time::OffsetDateTime::from_unix_timestamp(1_795_510_554).expect("timestamp");
        let rest = json!({ "resources": { "graphql": { "limit": 5000, "remaining": 0, "reset": 1_795_512_394 } } });
        assert_eq!(
            rate_window_from_rest(&rest, now),
            Some(RateWindow {
                remaining: 0,
                refill_in: Duration::from_secs(1840),
                refill_at: "09:26".into(),
            })
        );
        // A refill already past waits for nothing.
        assert_eq!(rate_window(10, 0, now).refill_in, Duration::ZERO);

        let mut tracker = RateTracker::default();
        tracker.observe(&json!({ "data": { "rateLimit": { "remaining": 4000, "resetAt": "2026-11-24T09:26:34Z" } } }));
        tracker.observe(&json!({ "data": {} }));
        assert_eq!(tracker.window, Some((4000, "2026-11-24T09:26:34Z".into())));
        tracker.observe_error("gh: not logged in");
        assert!(!tracker.limited);
        tracker.observe_error("gh: API rate limit already exceeded");
        assert!(tracker.limited);
    }

    #[test]
    fn home_relative_paths_expand() {
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(expand_home("~/src/x"), Path::new(&home).join("src/x"));
        }
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
    }
}
