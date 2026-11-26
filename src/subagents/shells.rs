//! Claude background shells (design D10).
//!
//! Claude writes no metadata for background shells, so the only source is the
//! session transcript (`<sid>.jsonl`). [`ShellLog`] is an incremental,
//! allocation-light parser fed raw byte chunks as the file grows; the pure
//! [`reconcile_shells`] / [`reconcile_shell_history`] fold its records into
//! the displayed lists with the same lifecycle rules as subagents.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Final status of a background shell as recorded in the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellStatus {
    Running,
    Done,
    Failed,
    Killed,
}

/// One background shell as reconstructed from the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellRecord {
    /// Claude's `backgroundTaskId`.
    pub id: String,
    pub command: String,
    /// `input.description`, or the command's first line when empty.
    pub description: String,
    pub started_at: Option<SystemTime>,
    pub ended_at: Option<SystemTime>,
    pub status: ShellStatus,
}

impl ShellRecord {
    fn ended(&self) -> bool {
        self.status != ShellStatus::Running
    }
}

/// Displayed state of a background shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellState {
    Running,
    Done,
    Failed,
    Killed,
    /// History only: unfinished, but started before this session was observed.
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundShell {
    pub id: String,
    pub command: String,
    pub description: String,
    pub state: ShellState,
    pub started_at: Option<SystemTime>,
    /// `None` while running or interrupted.
    pub ended_at: Option<SystemTime>,
    /// Current list only: when Herdr observed the end (linger start).
    pub done_at: Option<Instant>,
}

struct PendingStart {
    command: String,
    description: String,
}

/// Incremental transcript parser. Feed it consecutive byte chunks of the file.
#[derive(Default)]
pub struct ShellLog {
    partial: Vec<u8>,
    shells: Vec<ShellRecord>,
    index: HashMap<String, usize>,
    /// Background `tool_use` blocks whose `tool_result` has not been seen.
    pending_starts: HashMap<String, PendingStart>,
    /// Results (shell id, timestamp) whose `tool_use` has not been seen.
    orphan_results: HashMap<String, (String, Option<SystemTime>)>,
}

impl ShellLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Shells in start order.
    pub fn records(&self) -> &[ShellRecord] {
        &self.shells
    }

    /// Forget everything (the file was rewritten).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Consume the next chunk. Returns whether any record changed.
    pub fn feed(&mut self, chunk: &[u8]) -> bool {
        let mut changed = false;
        let mut rest = chunk;
        while let Some(pos) = rest.iter().position(|b| *b == b'\n') {
            let (head, tail) = rest.split_at(pos);
            rest = &tail[1..];
            if self.partial.is_empty() {
                changed |= self.line(head);
            } else {
                self.partial.extend_from_slice(head);
                let line = std::mem::take(&mut self.partial);
                changed |= self.line(&line);
            }
        }
        self.partial.extend_from_slice(rest);
        changed
    }

    fn line(&mut self, line: &[u8]) -> bool {
        let has = |needle: &[u8]| line.windows(needle.len()).any(|w| w == needle);
        let bg = has(b"run_in_background");
        let result = has(b"backgroundTaskId");
        let notification = has(b"task-notification");
        if !(bg || result || notification) {
            return false;
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            return false;
        };
        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_time);
        let mut changed = false;
        if bg {
            changed |= self.tool_use(&value);
        }
        if result {
            changed |= self.tool_result(&value, timestamp);
        }
        if notification {
            for text in notification_texts(&value) {
                changed |= self.notifications(text, timestamp);
            }
        }
        changed
    }

    fn tool_use(&mut self, value: &Value) -> bool {
        let Some(blocks) = value.pointer("/message/content").and_then(Value::as_array) else {
            return false;
        };
        let mut changed = false;
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_use")
                || block.get("name").and_then(Value::as_str) != Some("Bash")
                || block
                    .pointer("/input/run_in_background")
                    .and_then(Value::as_bool)
                    != Some(true)
            {
                continue;
            }
            let Some(tool_use_id) = block.get("id").and_then(Value::as_str) else {
                continue;
            };
            let command = block
                .pointer("/input/command")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let description = block
                .pointer("/input/description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let start = PendingStart {
                command: command.to_string(),
                description: description.to_string(),
            };
            if let Some((shell_id, started_at)) = self.orphan_results.remove(tool_use_id) {
                changed |= self.start(shell_id, start, started_at);
            } else {
                self.pending_starts.insert(tool_use_id.to_string(), start);
            }
        }
        changed
    }

    fn tool_result(&mut self, value: &Value, timestamp: Option<SystemTime>) -> bool {
        let Some(shell_id) = value
            .pointer("/toolUseResult/backgroundTaskId")
            .and_then(Value::as_str)
        else {
            return false;
        };
        let Some(blocks) = value.pointer("/message/content").and_then(Value::as_array) else {
            return false;
        };
        let mut changed = false;
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(tool_use_id) = block.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            if let Some(start) = self.pending_starts.remove(tool_use_id) {
                changed |= self.start(shell_id.to_string(), start, timestamp);
            } else {
                self.orphan_results
                    .insert(tool_use_id.to_string(), (shell_id.to_string(), timestamp));
            }
        }
        changed
    }

    fn start(&mut self, id: String, start: PendingStart, at: Option<SystemTime>) -> bool {
        if self.index.contains_key(&id) {
            return false;
        }
        let description = if start.description.trim().is_empty() {
            start.command.lines().next().unwrap_or_default().to_string()
        } else {
            start.description
        };
        self.index.insert(id.clone(), self.shells.len());
        self.shells.push(ShellRecord {
            id,
            command: start.command,
            description,
            started_at: at,
            ended_at: None,
            status: ShellStatus::Running,
        });
        true
    }

    fn notifications(&mut self, text: &str, at: Option<SystemTime>) -> bool {
        let mut changed = false;
        let mut rest = text;
        while let Some(pos) = rest.find("<task-notification>") {
            rest = &rest[pos + "<task-notification>".len()..];
            let body = rest.split("</task-notification>").next().unwrap_or(rest);
            let (Some(id), Some(status)) = (tag(body, "task-id"), tag(body, "status")) else {
                continue;
            };
            let Some(&idx) = self.index.get(id.trim()) else {
                continue; // not a background shell (e.g. a subagent)
            };
            let shell = &mut self.shells[idx];
            if shell.ended() {
                continue; // duplicate notification
            }
            shell.status = match status.trim() {
                "failed" => ShellStatus::Failed,
                "killed" => ShellStatus::Killed,
                _ => ShellStatus::Done,
            };
            shell.ended_at = at;
            changed = true;
        }
        changed
    }
}

fn tag<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&format!("</{name}>"))? + start;
    Some(&body[start..end])
}

fn parse_time(text: &str) -> Option<SystemTime> {
    OffsetDateTime::parse(text, &Rfc3339)
        .ok()
        .map(SystemTime::from)
}

/// Places a notification can appear: a queue-operation `content`, an
/// attachment prompt, or a user message string/text block.
fn notification_texts(value: &Value) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    out.extend(value.get("content").and_then(Value::as_str));
    out.extend(value.pointer("/attachment/prompt").and_then(Value::as_str));
    match value.pointer("/message/content") {
        Some(Value::String(text)) => out.push(text),
        Some(Value::Array(blocks)) => {
            for block in blocks {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    out.extend(block.get("text").and_then(Value::as_str));
                }
            }
        }
        _ => {}
    }
    out.retain(|text| text.contains("<task-notification>"));
    out
}

fn ended_state(status: ShellStatus) -> Option<ShellState> {
    match status {
        ShellStatus::Running => None,
        ShellStatus::Done => Some(ShellState::Done),
        ShellStatus::Failed => Some(ShellState::Failed),
        ShellStatus::Killed => Some(ShellState::Killed),
    }
}

fn is_leftover(started_at: Option<SystemTime>, observed: Option<SystemTime>) -> bool {
    matches!((observed, started_at), (Some(t), Some(s)) if s < t)
}

fn sort_shells(list: &mut [BackgroundShell]) {
    list.sort_by(|a, b| {
        a.started_at
            .cmp(&b.started_at)
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Fold parsed records into the current list (design D10, rules as D3).
pub fn reconcile_shells(
    previous: &[BackgroundShell],
    records: &[ShellRecord],
    parent_live: bool,
    session_observed_at: Option<SystemTime>,
    now: Instant,
    wall_now: SystemTime,
) -> Vec<BackgroundShell> {
    if !parent_live {
        return Vec::new();
    }
    let prev: HashMap<&str, &BackgroundShell> =
        previous.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut out: Vec<BackgroundShell> = Vec::new();
    for rec in records {
        let previous = prev.get(rec.id.as_str());
        let (state, done_at) = match ended_state(rec.status) {
            Some(state) => match previous {
                Some(p) if p.state == ShellState::Running => (state, Some(now)),
                Some(p) if p.done_at.is_some() => (state, p.done_at),
                // Ended at first sight (often a shell that ends within the
                // debounce window): listed only if it ran during this run and
                // ended within the linger window, lingering from its real end.
                _ => match recent_end(rec, session_observed_at, now, wall_now) {
                    Some(done_at) => (state, Some(done_at)),
                    None => continue,
                },
            },
            None => {
                if is_leftover(rec.started_at, session_observed_at) {
                    continue;
                }
                (ShellState::Running, None)
            }
        };
        out.push(BackgroundShell {
            id: rec.id.clone(),
            command: rec.command.clone(),
            description: rec.description.clone(),
            state,
            started_at: rec.started_at,
            ended_at: rec.ended_at.filter(|_| done_at.is_some()),
            done_at,
        });
    }
    // Lingering entries whose record vanished (transcript rewritten) stay until pruned.
    for p in previous {
        if p.done_at.is_some() && !records.iter().any(|r| r.id == p.id) {
            out.push(p.clone());
        }
    }
    sort_shells(&mut out);
    out
}

/// The linger start of a shell first seen already ended: its end time mapped
/// onto the monotonic clock, if it ended less than [`DONE_LINGER`] ago and
/// was not a leftover of an earlier run.
fn recent_end(
    rec: &ShellRecord,
    session_observed_at: Option<SystemTime>,
    now: Instant,
    wall_now: SystemTime,
) -> Option<Instant> {
    if is_leftover(rec.started_at, session_observed_at) {
        return None;
    }
    let age = wall_now
        .duration_since(rec.ended_at?)
        .unwrap_or(Duration::ZERO);
    (age < super::DONE_LINGER).then(|| now.checked_sub(age).unwrap_or(now))
}

/// Fold parsed records into the session history list: every shell, no linger.
pub fn reconcile_shell_history(
    records: &[ShellRecord],
    parent_live: bool,
    session_observed_at: Option<SystemTime>,
) -> Vec<BackgroundShell> {
    if !parent_live {
        return Vec::new();
    }
    let mut out: Vec<BackgroundShell> = records
        .iter()
        .map(|rec| {
            let state = ended_state(rec.status).unwrap_or_else(|| {
                if is_leftover(rec.started_at, session_observed_at) {
                    ShellState::Interrupted
                } else {
                    ShellState::Running
                }
            });
            BackgroundShell {
                id: rec.id.clone(),
                command: rec.command.clone(),
                description: rec.description.clone(),
                state,
                started_at: rec.started_at,
                ended_at: rec.ended_at.filter(|_| rec.ended()),
                done_at: None,
            }
        })
        .collect();
    sort_shells(&mut out);
    out
}

/// Remove ended entries older than `linger`. Returns true if the list changed.
pub fn prune_done_shells(list: &mut Vec<BackgroundShell>, now: Instant, linger: Duration) -> bool {
    let before = list.len();
    list.retain(|s| {
        s.done_at
            .is_none_or(|done_at| now.saturating_duration_since(done_at) < linger)
    });
    list.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: &str = "2026-11-26T12:00:00.000Z";
    const T1: &str = "2026-11-26T12:01:00.000Z";

    fn use_line(tool_use_id: &str, command: &str, description: Option<&str>) -> String {
        let mut input = serde_json::json!({"command": command, "run_in_background": true});
        if let Some(d) = description {
            input["description"] = d.into();
        }
        serde_json::json!({"type":"assistant","message":{"content":[
            {"type":"tool_use","id":tool_use_id,"name":"Bash","input":input}]}})
        .to_string()
    }

    fn result_line(tool_use_id: &str, shell_id: &str, ts: &str) -> String {
        serde_json::json!({"type":"user","timestamp":ts,
            "message":{"content":[{"type":"tool_result","tool_use_id":tool_use_id}]},
            "toolUseResult":{"backgroundTaskId":shell_id}})
        .to_string()
    }

    fn note_text(id: &str, status: &str) -> String {
        format!(
            "<task-notification>\n<task-id>{id}</task-id>\n<tool-use-id>toolu_x</tool-use-id>\n<output-file>/x</output-file>\n<status>{status}</status>\n<summary>s</summary>\n</task-notification>"
        )
    }

    fn queue_line(id: &str, status: &str, ts: &str) -> String {
        serde_json::json!({"type":"queue-operation","operation":"enqueue","timestamp":ts,
            "content": note_text(id, status)})
        .to_string()
    }

    fn feed(log: &mut ShellLog, lines: &[String]) {
        for l in lines {
            log.feed(l.as_bytes());
            log.feed(b"\n");
        }
    }

    fn started(log: &mut ShellLog, id: &str) {
        feed(
            log,
            &[
                use_line(&format!("toolu_{id}"), "sleep 9\necho", Some("wait")),
                result_line(&format!("toolu_{id}"), id, T0),
            ],
        );
    }

    #[test]
    fn start_and_description_fallback() {
        let mut log = ShellLog::new();
        feed(
            &mut log,
            &[
                use_line("t1", "npm run dev\n--port 1", None),
                result_line("t1", "b1", T0),
                use_line("t2", "ls", Some("list")),
                result_line("t2", "b2", T1),
            ],
        );
        let r = log.records();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].id, "b1");
        assert_eq!(r[0].description, "npm run dev");
        assert_eq!(r[0].command, "npm run dev\n--port 1");
        assert_eq!(r[0].status, ShellStatus::Running);
        assert!(r[0].started_at.is_some() && r[0].ended_at.is_none());
        assert_eq!(r[1].description, "list");
        assert!(r[1].started_at > r[0].started_at);
    }

    #[test]
    fn statuses_and_unknown_status() {
        for (status, want) in [
            ("completed", ShellStatus::Done),
            ("failed", ShellStatus::Failed),
            ("killed", ShellStatus::Killed),
            ("weird", ShellStatus::Done),
        ] {
            let mut log = ShellLog::new();
            started(&mut log, "b1");
            feed(&mut log, &[queue_line("b1", status, T1)]);
            assert_eq!(log.records()[0].status, want, "{status}");
            assert!(log.records()[0].ended_at.is_some());
        }
    }

    #[test]
    fn subagent_notification_ignored_and_duplicates_idempotent() {
        let mut log = ShellLog::new();
        started(&mut log, "b1");
        feed(
            &mut log,
            &[queue_line("a14b14f700c724a0c", "completed", T1)],
        );
        assert_eq!(log.records()[0].status, ShellStatus::Running);
        assert_eq!(log.records().len(), 1);

        let attachment = serde_json::json!({"type":"attachment","timestamp":"2026-11-26T12:09:00Z",
            "attachment":{"type":"queued_command","prompt": note_text("b1","failed")}})
        .to_string();
        feed(&mut log, &[queue_line("b1", "completed", T1), attachment]);
        let r = &log.records()[0];
        assert_eq!(r.status, ShellStatus::Done); // first wins
        assert_eq!(r.ended_at, parse_time(T1));
    }

    #[test]
    fn user_message_string_and_text_block_notifications() {
        let mut log = ShellLog::new();
        started(&mut log, "b1");
        started(&mut log, "b2");
        let s = serde_json::json!({"type":"user","timestamp":T1,
            "message":{"content": note_text("b1","completed")}})
        .to_string();
        let blocks = serde_json::json!({"type":"user","timestamp":T1,
            "message":{"content":[{"type":"text","text": note_text("b2","killed")}]}})
        .to_string();
        feed(&mut log, &[s, blocks]);
        assert_eq!(log.records()[0].status, ShellStatus::Done);
        assert_eq!(log.records()[1].status, ShellStatus::Killed);
    }

    #[test]
    fn notification_for_never_started_shell_is_ignored() {
        let mut log = ShellLog::new();
        feed(&mut log, &[queue_line("ghost", "completed", T1)]);
        assert!(log.records().is_empty());
        // And a later start is not pre-ended.
        started(&mut log, "ghost");
        assert_eq!(log.records()[0].status, ShellStatus::Running);
    }

    #[test]
    fn result_before_tool_use_ordering() {
        let mut log = ShellLog::new();
        feed(
            &mut log,
            &[result_line("t1", "b1", T0), use_line("t1", "make", None)],
        );
        assert_eq!(log.records().len(), 1);
        assert_eq!(log.records()[0].description, "make");
        assert!(log.records()[0].started_at.is_some());
    }

    #[test]
    fn non_background_bash_is_not_a_shell() {
        let mut log = ShellLog::new();
        let fg =
            serde_json::json!({"message":{"content":[{"type":"tool_use","id":"t","name":"Bash",
            "input":{"command":"ls","run_in_background":false}}]}})
            .to_string();
        feed(&mut log, &[fg, result_line("t", "x", T0)]);
        assert!(log.records().is_empty());
    }

    #[test]
    fn line_split_across_chunks() {
        let mut log = ShellLog::new();
        let text = format!(
            "{}\n{}\n",
            use_line("t1", "sleep 1", Some("nap")),
            result_line("t1", "b1", T0)
        );
        let bytes = text.as_bytes();
        let mut changed = false;
        for chunk in bytes.chunks(7) {
            changed |= log.feed(chunk);
        }
        assert!(changed);
        assert_eq!(log.records().len(), 1);
        assert_eq!(log.records()[0].description, "nap");
        // A trailing partial line is kept, not parsed.
        let mut log = ShellLog::new();
        assert!(!log.feed(use_line("t1", "x", None).as_bytes()));
        assert!(log.records().is_empty());
    }

    #[test]
    fn prefilter_skips_and_garbage_is_tolerated() {
        let mut log = ShellLog::new();
        assert!(
            !log.feed(b"{\"type\":\"user\",\"message\":\"hello\"}\nnot json run_in_background\n\n")
        );
        assert!(log.records().is_empty());
    }

    #[test]
    fn reset_forgets_everything() {
        let mut log = ShellLog::new();
        started(&mut log, "b1");
        log.reset();
        assert!(log.records().is_empty());
        started(&mut log, "b1");
        assert_eq!(log.records().len(), 1);
    }

    // ---- reconcile ----

    fn rec(id: &str, status: ShellStatus, start: u64) -> ShellRecord {
        let at = |s| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        ShellRecord {
            id: id.into(),
            command: format!("cmd {id}"),
            description: format!("desc {id}"),
            started_at: Some(at(start)),
            ended_at: (status != ShellStatus::Running).then(|| at(start + 5)),
            status,
        }
    }

    fn at(s: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(s))
    }

    /// Wall clock far after every record's end.
    fn wall() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(10_000)
    }

    #[test]
    fn recently_ended_at_first_sight_lingers_from_its_end() {
        let t = Instant::now();
        // Started at 100, ended at 105; scanned at 115: lingers 20 s more.
        let records = [rec("a", ShellStatus::Failed, 100)];
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(115);
        let mut list = reconcile_shells(&[], &records, true, at(50), t, wall);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].state, ShellState::Failed);
        assert_eq!(list[0].done_at, t.checked_sub(Duration::from_secs(10)));
        assert!(!prune_done_shells(
            &mut list,
            t + Duration::from_secs(19),
            super::super::DONE_LINGER
        ));
        assert!(prune_done_shells(
            &mut list,
            t + Duration::from_secs(20),
            super::super::DONE_LINGER
        ));
        // Ended longer ago than the linger window, or before this run: hidden.
        let late = SystemTime::UNIX_EPOCH + Duration::from_secs(140);
        assert!(reconcile_shells(&[], &records, true, at(50), t, late).is_empty());
        assert!(reconcile_shells(&[], &records, true, at(101), t, wall).is_empty());
    }

    #[test]
    fn running_then_linger_then_prune() {
        let t = Instant::now();
        let live = reconcile_shells(
            &[],
            &[rec("a", ShellStatus::Running, 100)],
            true,
            at(50),
            t,
            wall(),
        );
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].state, ShellState::Running);

        let t1 = t + Duration::from_secs(1);
        let mut ended = reconcile_shells(
            &live,
            &[rec("a", ShellStatus::Failed, 100)],
            true,
            at(50),
            t1,
            wall(),
        );
        assert_eq!(ended[0].state, ShellState::Failed);
        assert_eq!(ended[0].done_at, Some(t1));
        assert!(ended[0].ended_at.is_some());

        // Stable on rescan; linger clock not restarted.
        let again = reconcile_shells(
            &ended,
            &[rec("a", ShellStatus::Failed, 100)],
            true,
            at(50),
            t1 + Duration::from_secs(10),
            wall(),
        );
        assert_eq!(again, ended);

        assert!(!prune_done_shells(
            &mut ended,
            t1 + Duration::from_secs(29),
            super::super::DONE_LINGER
        ));
        assert!(prune_done_shells(
            &mut ended,
            t1 + Duration::from_secs(30),
            super::super::DONE_LINGER
        ));
        assert!(ended.is_empty());
    }

    #[test]
    fn ended_at_first_sight_not_current_but_in_history() {
        let t = Instant::now();
        let records = [rec("a", ShellStatus::Done, 100)];
        assert!(reconcile_shells(&[], &records, true, at(50), t, wall()).is_empty());
        let hist = reconcile_shell_history(&records, true, at(50));
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].state, ShellState::Done);
        assert!(hist[0].done_at.is_none() && hist[0].ended_at.is_some());
    }

    #[test]
    fn resume_leftover_is_interrupted_in_history_only() {
        let t = Instant::now();
        let records = [
            rec("old", ShellStatus::Running, 10),
            rec("new", ShellStatus::Running, 100),
        ];
        let live = reconcile_shells(&[], &records, true, at(50), t, wall());
        assert_eq!(
            live.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["new"]
        );
        let hist = reconcile_shell_history(&records, true, at(50));
        assert_eq!(hist[0].state, ShellState::Interrupted);
        assert!(hist[0].ended_at.is_none());
        assert_eq!(hist[1].state, ShellState::Running);
        // No observed time disables the filter.
        assert_eq!(
            reconcile_shells(&[], &records, true, None, t, wall()).len(),
            2
        );
    }

    #[test]
    fn parent_dead_clears_both() {
        let t = Instant::now();
        let records = [rec("a", ShellStatus::Running, 100)];
        assert!(reconcile_shells(&[], &records, false, at(50), t, wall()).is_empty());
        assert!(reconcile_shell_history(&records, false, at(50)).is_empty());
    }
}
