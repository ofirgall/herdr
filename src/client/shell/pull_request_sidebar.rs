//! The pull requests sidebar section: the endpoint's watched repositories,
//! each a heading with its open pull requests one per line below it.
//!
//! A pull request line is its number and branch, with its status glyphs
//! (glyphs and colors of the herdr-pr-tracker plugin) at the right edge. A
//! heading opens the repository's pull request list and a line opens its pull
//! request, through the sidebar link click (`token_link_click`).
//!
//! A pull request with checks has a `▸` before it; clicking it, or the checks
//! glyph, lists its failing and running checks (GitHub Actions jobs and commit
//! statuses) below the line, each linking to its run, then a line counting the
//! passed and skipped ones, linking to the pull request's checks page. Which
//! lists are open is client state.

use std::collections::HashSet;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};
use unicode_width::UnicodeWidthStr;

use super::render::put_text;
use super::*;
use crate::protocol::{
    ClientShellPullRequest, ClientShellPullRequestCheck, ClientShellPullRequestChecks,
    ClientShellPullRequestRepo, ClientShellPullRequestReview,
};

const CONFLICT: &str = "\u{F182B}";
const REVIEW_APPROVED: &str = "\u{F0008}";
const REVIEW_CHANGES: &str = "\u{F12DF}";
const REVIEW_REQUIRED: &str = "\u{F0208}";
const THREADS: &str = "\u{F017A}";
const BOT_THREADS: &str = "\u{F06A9}";
const CI_PASS: &str = "\u{F058}";
const CI_FAIL: &str = "\u{F530}";
const CI_PENDING: &str = "\u{EAD2}";
const DRAFT: &str = "\u{25CC}";
const QUEUED: &str = "\u{F4DB}";
const WARNING: &str = "\u{26A0}";
const CHECK_SKIPPED: &str = "\u{2013}";
const EXPAND: &str = "\u{25B8}";
const COLLAPSE: &str = "\u{25BE}";
/// Columns before a pull request line's number; the expand toggle sits in them.
const PR_INDENT: u16 = 2;
/// Columns before a check line's glyph.
const CHECK_INDENT: u16 = 4;

/// Draws the section into `area`. `top_rule` draws the divider against a
/// section above it.
pub(super) fn render_pull_request_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    expanded: &HashSet<String>,
    top_rule: bool,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    if area.height == 0 {
        return;
    }
    let title_y = area.y + u16::from(top_rule);
    if top_rule {
        put_text(
            buffer,
            area.x,
            area.y,
            area.width,
            &"─".repeat(area.width as usize),
            Style::default().fg(palette.surface_dim),
        );
    }
    if title_y >= area.bottom() {
        return;
    }
    put_text(
        buffer,
        area.x,
        title_y,
        area.width,
        " prs",
        Style::default()
            .fg(palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let body = Rect::new(
        area.x,
        title_y.saturating_add(2),
        area.width,
        area.bottom().saturating_sub(title_y.saturating_add(2)),
    );
    if body.is_empty() {
        return;
    }
    if snapshot.pull_requests.is_empty() {
        put_text(
            buffer,
            body.x,
            body.y,
            body.width,
            " no watched repos",
            Style::default()
                .fg(palette.overlay0)
                .add_modifier(Modifier::DIM),
        );
        return;
    }
    let links = !config.token_link_click.is_off();
    let mut y = body.y;
    for (index, repo) in snapshot.pull_requests.iter().enumerate() {
        // One blank row between repositories.
        if index > 0 {
            y = y.saturating_add(1);
        }
        if y >= body.bottom() {
            break;
        }
        let row = Rect::new(body.x, y, body.width, 1);
        render_heading(buffer, row, repo, config);
        if let Some(url) = repo.url.as_ref().filter(|_| links) {
            hits.token_links.push(TokenLinkHit {
                rect: row,
                url: url.clone(),
            });
        }
        y += 1;
        if let Some(error) = &repo.error {
            if y >= body.bottom() {
                break;
            }
            put_text(
                buffer,
                body.x + PR_INDENT,
                y,
                body.width.saturating_sub(PR_INDENT),
                &format!("{WARNING} {error}"),
                Style::default()
                    .fg(palette.yellow)
                    .add_modifier(Modifier::DIM),
            );
            y += 1;
        }
        for pr in &repo.pull_requests {
            if y >= body.bottom() {
                break;
            }
            let row = Rect::new(body.x, y, body.width, 1);
            let open = expanded.contains(&pr.url);
            let checks_glyph = render_pull_request(buffer, row, pr, open, config);
            if has_checks(pr) {
                let toggle = Rect::new(row.x, row.y, PR_INDENT.min(row.width), 1);
                hits.pull_request_toggles.push((toggle, pr.url.clone()));
                if let Some(glyph) = checks_glyph {
                    hits.pull_request_toggles.push((glyph, pr.url.clone()));
                }
            }
            if links {
                hits.token_links.push(TokenLinkHit {
                    rect: row,
                    url: pr.url.clone(),
                });
            }
            y += 1;
            if !open {
                continue;
            }
            for check in &pr.check_runs {
                if y >= body.bottom() {
                    break;
                }
                let row = Rect::new(body.x, y, body.width, 1);
                render_check(buffer, row, check, config);
                if let Some(url) = check.url.as_ref().filter(|_| links) {
                    hits.token_links.push(TokenLinkHit {
                        rect: row,
                        url: url.clone(),
                    });
                }
                y += 1;
            }
            if y < body.bottom() {
                let row = Rect::new(body.x, y, body.width, 1);
                if render_check_summary(buffer, row, pr, config) {
                    if links {
                        hits.token_links.push(TokenLinkHit {
                            rect: row,
                            url: format!("{}/checks", pr.url),
                        });
                    }
                    y += 1;
                }
            }
        }
    }
}

fn has_checks(pr: &ClientShellPullRequest) -> bool {
    let counts = &pr.check_counts;
    !pr.check_runs.is_empty()
        || counts.passed > 0
        || counts.failed > 0
        || counts.pending > 0
        || counts.skipped > 0
}

/// ` label ───── N`: the pull request count at the right edge, `…` before the
/// first poll and `⚠` after a failed one.
fn render_heading(
    buffer: &mut Buffer,
    row: Rect,
    repo: &ClientShellPullRequestRepo,
    config: &ClientShellConfig,
) {
    let palette = &config.palette;
    let (count, count_style) = if repo.error.is_some() {
        (WARNING.to_owned(), Style::default().fg(palette.yellow))
    } else if repo.loading {
        ("…".to_owned(), Style::default().fg(palette.overlay0))
    } else {
        (
            repo.pull_requests.len().to_string(),
            Style::default().fg(palette.overlay1),
        )
    };
    let width = usize::from(row.width);
    let count_width = count.width();
    // " " label " " rule " " count
    let label_budget = width.saturating_sub(count_width + 4);
    let label = crate::ui::truncate_end(&repo.label, label_budget);
    let rule_width = width.saturating_sub(label.width() + count_width + 3);
    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            label,
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            "─".repeat(rule_width),
            Style::default().fg(palette.surface_dim),
        ),
        Span::raw(" "),
        Span::styled(count, count_style),
    ]);
    Paragraph::new(line).render(row, buffer);
}

/// `▸ #N branch    status`: the status glyphs right-aligned; a draft is
/// dimmed. Returns where the checks glyph was drawn.
fn render_pull_request(
    buffer: &mut Buffer,
    row: Rect,
    pr: &ClientShellPullRequest,
    open: bool,
    config: &ClientShellConfig,
) -> Option<Rect> {
    let palette = &config.palette;
    let mut status = status_spans(pr, palette);
    let status_width = status
        .iter()
        .map(|span| span.content.width())
        .sum::<usize>();
    let marker = if pr.queued {
        Some(QUEUED)
    } else if pr.draft {
        Some(DRAFT)
    } else {
        None
    };
    let number = format!("{}#{}", marker.unwrap_or_default(), pr.number);
    let width = usize::from(row.width.saturating_sub(PR_INDENT));
    let left_budget = width.saturating_sub(status_width + usize::from(status_width > 0));
    let number = crate::ui::truncate_end(&number, left_budget);
    let branch_budget = left_budget.saturating_sub(number.width() + 1);
    let branch = crate::ui::truncate_end(&pr.branch, branch_budget);
    let left_width = number.width()
        + if branch.is_empty() {
            0
        } else {
            1 + branch.width()
        };
    let gap = width.saturating_sub(left_width + status_width);
    let dim = if pr.draft {
        Modifier::DIM
    } else {
        Modifier::empty()
    };
    let toggle = match (!has_checks(pr), open) {
        (true, _) => " ",
        (false, false) => EXPAND,
        (false, true) => COLLAPSE,
    };
    let mut spans = vec![
        Span::styled(toggle, Style::default().fg(palette.overlay0)),
        Span::raw(" ".repeat(usize::from(PR_INDENT - 1))),
        Span::styled(
            number,
            Style::default().fg(palette.overlay1).add_modifier(dim),
        ),
    ];
    if !branch.is_empty() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            branch,
            Style::default().fg(palette.subtext0).add_modifier(dim),
        ));
    }
    let mut checks_glyph = None;
    if status_width > 0 && status_width <= width {
        if pr.checks != ClientShellPullRequestChecks::None
            && pr.checks != ClientShellPullRequestChecks::Unknown
        {
            // The checks glyph is the last status span, at the right edge.
            checks_glyph = Some(Rect::new(row.right().saturating_sub(1), row.y, 1, 1));
        }
        spans.push(Span::raw(" ".repeat(gap)));
        spans.append(&mut status);
    }
    Paragraph::new(Line::from(spans)).render(row, buffer);
    checks_glyph
}

/// `    ✓ 96  – 40  +2`: the passed and skipped checks, and failing or running
/// ones beyond those listed. Returns whether anything was drawn.
fn render_check_summary(
    buffer: &mut Buffer,
    row: Rect,
    pr: &ClientShellPullRequest,
    config: &ClientShellConfig,
) -> bool {
    let palette = &config.palette;
    let counts = &pr.check_counts;
    let listed = u32::try_from(pr.check_runs.len()).unwrap_or(u32::MAX);
    let unlisted = counts
        .failed
        .saturating_add(counts.pending)
        .saturating_sub(listed);
    let mut spans = vec![Span::raw(" ".repeat(usize::from(CHECK_INDENT)))];
    let mut push = |text: String, style: Style| {
        if spans.len() > 1 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(text, style));
    };
    if counts.passed > 0 {
        push(
            format!("{CI_PASS} {}", counts.passed),
            Style::default().fg(palette.green),
        );
    }
    if counts.skipped > 0 {
        push(
            format!("{CHECK_SKIPPED} {}", counts.skipped),
            Style::default()
                .fg(palette.overlay0)
                .add_modifier(Modifier::DIM),
        );
    }
    if unlisted > 0 {
        push(
            format!("+{unlisted} more"),
            Style::default().fg(palette.overlay1),
        );
    }
    if spans.len() == 1 {
        return false;
    }
    Paragraph::new(Line::from(spans)).render(row, buffer);
    true
}

/// `    ✓ name`: one check under an open pull request.
fn render_check(
    buffer: &mut Buffer,
    row: Rect,
    check: &ClientShellPullRequestCheck,
    config: &ClientShellConfig,
) {
    let palette = &config.palette;
    let (glyph, glyph_style, name_style) = match check.outcome {
        ClientShellPullRequestChecks::Pass => (
            CI_PASS,
            Style::default().fg(palette.green),
            Style::default().fg(palette.subtext0),
        ),
        ClientShellPullRequestChecks::Fail => (
            CI_FAIL,
            Style::default()
                .fg(palette.red)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(palette.text),
        ),
        ClientShellPullRequestChecks::Pending => (
            CI_PENDING,
            Style::default().fg(palette.peach),
            Style::default().fg(palette.subtext0),
        ),
        ClientShellPullRequestChecks::None | ClientShellPullRequestChecks::Unknown => {
            let dim = Style::default()
                .fg(palette.overlay0)
                .add_modifier(Modifier::DIM);
            (CHECK_SKIPPED, dim, dim)
        }
    };
    let width = usize::from(row.width.saturating_sub(CHECK_INDENT));
    let name = crate::ui::truncate_end(&check.name, width.saturating_sub(2));
    let line = Line::from(vec![
        Span::raw(" ".repeat(usize::from(CHECK_INDENT))),
        Span::styled(glyph, glyph_style),
        Span::raw(" "),
        Span::styled(name, name_style),
    ]);
    Paragraph::new(line).render(row, buffer);
}

/// Conflict, review, human threads, bot threads, checks, space separated.
fn status_spans(
    pr: &ClientShellPullRequest,
    palette: &crate::app::state::Palette,
) -> Vec<Span<'static>> {
    let bold = Modifier::BOLD;
    let mut glyphs = Vec::<(String, Style)>::new();
    if pr.conflict {
        glyphs.push((
            CONFLICT.into(),
            Style::default().fg(palette.red).add_modifier(bold),
        ));
    }
    match pr.review {
        ClientShellPullRequestReview::Approved => {
            glyphs.push((REVIEW_APPROVED.into(), Style::default().fg(palette.green)));
        }
        ClientShellPullRequestReview::ChangesRequested => glyphs.push((
            REVIEW_CHANGES.into(),
            Style::default().fg(palette.red).add_modifier(bold),
        )),
        ClientShellPullRequestReview::ReviewRequired => glyphs.push((
            REVIEW_REQUIRED.into(),
            Style::default().fg(palette.blue).add_modifier(bold),
        )),
        ClientShellPullRequestReview::None | ClientShellPullRequestReview::Unknown => {}
    }
    if pr.human_threads > 0 {
        glyphs.push((
            format!("{THREADS}{}", pr.human_threads),
            Style::default().fg(palette.yellow).add_modifier(bold),
        ));
    }
    if pr.bot_threads > 0 {
        glyphs.push((
            format!("{BOT_THREADS}{}", pr.bot_threads),
            Style::default().fg(palette.yellow),
        ));
    }
    match pr.checks {
        ClientShellPullRequestChecks::Pass => {
            glyphs.push((CI_PASS.into(), Style::default().fg(palette.green)));
        }
        ClientShellPullRequestChecks::Fail => glyphs.push((
            CI_FAIL.into(),
            Style::default().fg(palette.red).add_modifier(bold),
        )),
        ClientShellPullRequestChecks::Pending => {
            glyphs.push((CI_PENDING.into(), Style::default().fg(palette.peach)));
        }
        ClientShellPullRequestChecks::None | ClientShellPullRequestChecks::Unknown => {}
    }
    let mut spans = Vec::new();
    for (index, (text, style)) in glyphs.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(text, style));
    }
    spans
}
