use super::*;
use crate::protocol::{
    ClientShellPullRequest, ClientShellPullRequestChecks, ClientShellPullRequestRepo,
    ClientShellPullRequestReview,
};

fn pull_request(number: u64, branch: &str) -> ClientShellPullRequest {
    ClientShellPullRequest {
        number,
        title: format!("PR {number}"),
        url: format!("https://github.com/octo/web/pull/{number}"),
        branch: branch.into(),
        draft: false,
        conflict: false,
        queued: false,
        review: ClientShellPullRequestReview::None,
        checks: ClientShellPullRequestChecks::None,
        human_threads: 0,
        bot_threads: 0,
    }
}

fn repos() -> Vec<ClientShellPullRequestRepo> {
    let mut busy = pull_request(7214, "fix-live-session-reconnect");
    busy.human_threads = 2;
    busy.review = ClientShellPullRequestReview::Approved;
    busy.checks = ClientShellPullRequestChecks::Fail;
    let mut draft = pull_request(7221, "chore-voice");
    draft.draft = true;
    vec![
        ClientShellPullRequestRepo {
            label: "web".into(),
            url: Some("https://github.com/octo/web/pulls".into()),
            loading: false,
            error: None,
            pull_requests: vec![busy, draft],
        },
        ClientShellPullRequestRepo {
            label: "api".into(),
            url: None,
            loading: true,
            error: None,
            pull_requests: Vec::new(),
        },
        ClientShellPullRequestRepo {
            label: "herdr".into(),
            url: Some("https://github.com/octo/herdr/pulls".into()),
            loading: false,
            error: Some("gh: not logged in".into()),
            pull_requests: vec![pull_request(12, "sidebar")],
        },
    ]
}

fn composed(
    agents: bool,
    pull_requests: bool,
    repos: Vec<ClientShellPullRequestRepo>,
) -> (ClientShellState, Vec<String>) {
    let mut config = Config::default();
    config.ui.sidebar.agents.enabled = agents;
    config.ui.sidebar.pull_requests.enabled = pull_requests;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let mut snapshot = snapshot();
    snapshot.pull_requests = repos;
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    let frame = state.compose(106, 30).expect("composed frame");
    let rows = frame_rows(&frame)
        .into_iter()
        .map(|row| row.chars().take(26).collect::<String>())
        .collect();
    (state, rows)
}

fn row_with<'a>(rows: &'a [String], needle: &str) -> (usize, &'a String) {
    rows.iter()
        .enumerate()
        .find(|(_, row)| row.contains(needle))
        .unwrap_or_else(|| panic!("no row with {needle:?}: {rows:#?}"))
}

#[test]
fn pull_requests_replace_the_agents_section() {
    let (state, rows) = composed(false, true, repos());
    let text = rows.join("\n");
    assert!(text.contains(" spaces") && text.contains(" prs"), "{text}");
    assert!(!text.contains(" agents"), "{text}");
    assert!(!state.hits.sidebar_section_divider.is_empty());

    let (heading, row) = row_with(&rows, " web ");
    assert!(
        row.contains('─') && row.trim_end_matches(['│', ' ']).ends_with('2'),
        "{row}"
    );
    let (busy, row) = row_with(&rows, "#7214");
    assert_eq!(busy, heading + 1);
    assert!(row.starts_with("  #7214 fix-"), "{row}");
    // Approved, two human threads and failing checks, right-aligned.
    assert!(row.contains("\u{F0008} \u{F017A}2 \u{F530}"), "{row}");
    let (draft, row) = row_with(&rows, "#7221");
    assert_eq!(draft, busy + 1);
    assert!(row.contains("◌#7221 chore-voice"), "{row}");

    // A blank row separates repositories; a loading one shows `…`.
    let (api, row) = row_with(&rows, " api ");
    assert_eq!(api, draft + 2);
    assert!(row.trim_end_matches(['│', ' ']).ends_with('…'), "{row}");

    // A failed poll keeps its pull requests below the error.
    let (herdr, _) = row_with(&rows, " herdr ");
    let (error, row) = row_with(&rows, "⚠ gh: not logged");
    assert_eq!(error, herdr + 1);
    assert!(row.starts_with("  "), "{row}");
    assert_eq!(row_with(&rows, "#12 sidebar").0, error + 1);
}

#[test]
fn rows_and_headings_link_to_github() {
    let (state, rows) = composed(false, true, repos());
    let urls = state
        .hits
        .token_links
        .iter()
        .map(|hit| (hit.rect.y as usize, hit.url.as_str()))
        .collect::<Vec<_>>();
    let line = |needle| row_with(&rows, needle).0;
    for (needle, url) in [
        (" web ", "https://github.com/octo/web/pulls"),
        ("#7214", "https://github.com/octo/web/pull/7214"),
        ("#7221", "https://github.com/octo/web/pull/7221"),
        (" herdr ", "https://github.com/octo/herdr/pulls"),
    ] {
        assert!(urls.contains(&(line(needle), url)), "{needle}: {urls:?}");
    }
    // A repository not yet resolved has no link.
    assert!(!urls.iter().any(|(y, _)| *y == line(" api ")), "{urls:?}");
}

#[test]
fn agents_and_pull_requests_share_the_lower_section() {
    let (_, rows) = composed(true, true, repos());
    let text = rows.join("\n");
    let agents = row_with(&rows, " agents").0;
    let pull_requests = row_with(&rows, " prs").0;
    assert!(agents < pull_requests, "{text}");
}

#[test]
fn without_watched_repos_the_section_says_so() {
    let (_, rows) = composed(false, true, Vec::new());
    row_with(&rows, " no watched repos");
}

#[test]
fn the_section_is_off_by_default() {
    let (_, rows) = composed(true, false, repos());
    assert!(!rows.join("\n").contains(" prs"));
}

#[test]
fn the_lower_section_splits_between_agents_and_pull_requests() {
    use crate::client::shell::sidebar::split_lower_section;
    let area = Rect::new(0, 10, 20, 9);
    assert_eq!(
        split_lower_section(area, true, true),
        (Rect::new(0, 10, 20, 5), Rect::new(0, 15, 20, 4))
    );
    assert_eq!(
        split_lower_section(area, true, false),
        (area, Rect::default())
    );
    assert_eq!(
        split_lower_section(area, false, true),
        (Rect::default(), area)
    );
}
