//! Subagent tracking: pure data model, source trait, and reconciliation.
//!
//! This module is standalone: no terminal, app-state, or watcher coupling.
//! A [`SubagentSource`] reads agent-specific on-disk metadata into
//! [`SubagentRecord`]s, [`attribute_parents`] works out nesting, and
//! [`reconcile`] folds records plus pane facts into the displayed list.

mod attribution;
mod claude;
pub mod watcher;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

pub use attribution::attribute_parents;
pub use claude::ClaudeSubagentSource;
pub use watcher::SubagentWatcher;

/// How long a finished subagent stays listed after Herdr observed it finish.
pub const DONE_LINGER: Duration = Duration::from_secs(30);

/// One subagent as described by a source's on-disk metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentRecord {
    pub id: String,
    /// Agent type (Claude `agentType`).
    pub kind: String,
    pub description: String,
    /// Id of the tool-use block that spawned this subagent.
    pub tool_use_id: Option<String>,
    pub spawn_depth: u32,
    /// `requestShape == "background"`.
    pub background: bool,
    pub finished: bool,
    /// File birth time, falling back to mtime.
    pub created_at: Option<SystemTime>,
    /// When the subagent started (transcript birth time; design D9).
    pub started_at: Option<SystemTime>,
    /// Finish time when finished, otherwise last activity.
    pub ended_at: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentState {
    Running,
    Done {
        done_at: Instant,
    },
    /// History view only: the subagent finished. Never in the current list.
    Finished,
    /// History view only: unfinished, but created before this session was
    /// observed (an earlier run of a resumed session that never completed).
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subagent {
    pub id: String,
    pub kind: String,
    pub description: String,
    /// Id of the parent subagent; `None` means a child of the root agent.
    pub parent: Option<String>,
    pub background: bool,
    pub state: SubagentState,
    /// Wall-clock start, for client-side elapsed display.
    pub started_at: Option<SystemTime>,
    /// Wall-clock end (finished) or last activity (interrupted). Always
    /// `None` while `Running`.
    pub ended_at: Option<SystemTime>,
}

pub trait SubagentSource: Send + Sync {
    /// Directory to watch/scan for this session transcript path,
    /// e.g. `<dir>/<sid>/subagents`.
    fn subagent_dir(&self, transcript: &Path) -> Option<PathBuf>;
    /// Read all records in `dir`. Malformed or partial files are skipped
    /// silently; a missing directory yields an empty vec.
    fn scan(&self, dir: &Path) -> Vec<SubagentRecord>;
}

pub struct ReconcileInput<'a> {
    pub previous: &'a [Subagent],
    pub records: &'a [SubagentRecord],
    /// The pane has a live agent whose session matches the watched one.
    pub parent_live: bool,
    /// When Herdr observed the current session start; `None` disables the
    /// resume-leftover filter.
    pub session_observed_at: Option<SystemTime>,
    /// Subagent id -> parent subagent id, from [`attribute_parents`].
    pub parents: &'a HashMap<String, Option<String>>,
    pub now: Instant,
}

/// Fold scanned records into the displayed subagent list (design D3).
///
/// Order is deterministic: by record `created_at` ascending (entries without
/// a known creation time, including lingering `Done` entries whose record
/// vanished, sort first), then by id.
pub fn reconcile(input: ReconcileInput<'_>) -> Vec<Subagent> {
    if !input.parent_live {
        return Vec::new();
    }
    let prev: HashMap<&str, &Subagent> =
        input.previous.iter().map(|s| (s.id.as_str(), s)).collect();

    // (created_at, subagent) pairs, sorted below.
    let mut out: Vec<(Option<SystemTime>, Subagent)> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();

    for rec in input.records {
        if !seen.insert(rec.id.as_str()) {
            continue; // duplicate record id; first wins
        }
        let previous = prev.get(rec.id.as_str());
        let mut ended_at = None;
        let state = if rec.finished {
            ended_at = rec.ended_at.or_else(|| previous.and_then(|p| p.ended_at));
            match previous.map(|p| &p.state) {
                Some(SubagentState::Running) => SubagentState::Done { done_at: input.now },
                Some(done @ SubagentState::Done { .. }) => done.clone(),
                // Finished at first sight: never listed. (History-only states
                // never occur in the current list.)
                _ => continue,
            }
        } else {
            let leftover = matches!(
                (input.session_observed_at, rec.created_at),
                (Some(t), Some(c)) if c < t
            );
            if leftover {
                continue;
            }
            SubagentState::Running
        };
        out.push((
            rec.created_at,
            Subagent {
                id: rec.id.clone(),
                kind: rec.kind.clone(),
                description: rec.description.clone(),
                parent: None,
                background: rec.background,
                state,
                started_at: rec.started_at,
                ended_at,
            },
        ));
    }

    // Lingering Done entries whose record vanished stay until pruned;
    // Running entries whose record vanished are dropped.
    for p in input.previous {
        if seen.contains(p.id.as_str()) {
            continue;
        }
        if matches!(p.state, SubagentState::Done { .. }) && seen.insert(p.id.as_str()) {
            out.push((None, p.clone()));
        }
    }

    out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let mut list: Vec<Subagent> = out.into_iter().map(|(_, s)| s).collect();

    resolve_parents(&mut list, input.parents);
    list
}

/// Attach parents: only keep one that is present in `list`; cycles are broken.
fn resolve_parents(list: &mut [Subagent], parents: &HashMap<String, Option<String>>) {
    // Resolve parents: only keep one that is present in the final list.
    let ids: HashSet<String> = list.iter().map(|s| s.id.clone()).collect();
    let resolved: HashMap<String, String> = list
        .iter()
        .filter_map(|s| {
            let p = parents.get(&s.id).cloned().flatten()?;
            (p != s.id && ids.contains(&p)).then(|| (s.id.clone(), p))
        })
        .collect();
    for s in list.iter_mut() {
        // Break cycles: walking up must not return to this subagent.
        let mut cur = resolved.get(&s.id);
        let mut steps = 0;
        let mut cyclic = false;
        while let Some(p) = cur {
            if *p == s.id || steps > resolved.len() {
                cyclic = true;
                break;
            }
            steps += 1;
            cur = resolved.get(p);
        }
        s.parent = if cyclic {
            None
        } else {
            resolved.get(&s.id).cloned()
        };
    }
}

/// Fold scanned records into the session history list (design D8).
///
/// Unlike [`reconcile`] this keeps every parseable record, needs no previous
/// state, and is never linger-pruned. Same ordering and parent attribution.
pub fn reconcile_history(
    records: &[SubagentRecord],
    parent_live: bool,
    session_observed_at: Option<SystemTime>,
    parents: &HashMap<String, Option<String>>,
) -> Vec<Subagent> {
    if !parent_live {
        return Vec::new();
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out: Vec<(Option<SystemTime>, Subagent)> = Vec::new();
    for rec in records {
        if !seen.insert(rec.id.as_str()) {
            continue;
        }
        let state = if rec.finished {
            SubagentState::Finished
        } else if matches!(
            (session_observed_at, rec.created_at),
            (Some(t), Some(c)) if c < t
        ) {
            SubagentState::Interrupted
        } else {
            SubagentState::Running
        };
        let ended_at = if state == SubagentState::Running {
            None
        } else {
            rec.ended_at
        };
        out.push((
            rec.created_at,
            Subagent {
                id: rec.id.clone(),
                kind: rec.kind.clone(),
                description: rec.description.clone(),
                parent: None,
                background: rec.background,
                state,
                started_at: rec.started_at,
                ended_at,
            },
        ));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let mut list: Vec<Subagent> = out.into_iter().map(|(_, s)| s).collect();
    resolve_parents(&mut list, parents);
    list
}

/// Remove `Done` entries older than `linger`. Returns true if the list changed.
pub fn prune_done(list: &mut Vec<Subagent>, now: Instant, linger: Duration) -> bool {
    let before = list.len();
    list.retain(|s| match s.state {
        SubagentState::Done { done_at } => now.saturating_duration_since(done_at) < linger,
        _ => true,
    });
    list.len() != before
}

#[cfg(test)]
pub(crate) mod test_util {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Minimal temp dir (no `tempfile` dev-dependency); removed on drop.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!(
                "herdr-subagents-test-{}-{}",
                std::process::id(),
                n
            ));
            std::fs::create_dir_all(&p).expect("create temp dir");
            TempDir(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, finished: bool) -> SubagentRecord {
        SubagentRecord {
            id: id.into(),
            kind: "general-purpose".into(),
            description: format!("desc {id}"),
            tool_use_id: None,
            spawn_depth: 1,
            background: false,
            finished,
            created_at: None,
            started_at: None,
            ended_at: None,
        }
    }

    fn sub(id: &str, state: SubagentState) -> Subagent {
        Subagent {
            id: id.into(),
            kind: "general-purpose".into(),
            description: format!("desc {id}"),
            parent: None,
            background: false,
            state,
            started_at: None,
            ended_at: None,
        }
    }

    fn run(
        previous: &[Subagent],
        records: &[SubagentRecord],
        live: bool,
        observed: Option<SystemTime>,
        parents: &HashMap<String, Option<String>>,
        now: Instant,
    ) -> Vec<Subagent> {
        reconcile(ReconcileInput {
            previous,
            records,
            parent_live: live,
            session_observed_at: observed,
            parents,
            now,
        })
    }

    #[test]
    fn running_has_no_end_and_done_carries_record_end() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let t1 = SystemTime::UNIX_EPOCH + Duration::from_secs(160);
        let now = Instant::now();
        let mut r = rec("a", false);
        r.started_at = Some(t0);
        r.ended_at = Some(t1); // last activity: must not leak while running
        let out = run(&[], &[r.clone()], true, None, &HashMap::new(), now);
        assert_eq!(out[0].started_at, Some(t0));
        assert_eq!(out[0].ended_at, None);

        r.finished = true;
        let out = run(&out, &[r.clone()], true, None, &HashMap::new(), now);
        assert!(matches!(out[0].state, SubagentState::Done { .. }));
        assert_eq!(out[0].ended_at, Some(t1));

        let hist = reconcile_history(&[r.clone()], true, None, &HashMap::new());
        assert_eq!(hist[0].ended_at, Some(t1));
        r.finished = false;
        let hist = reconcile_history(&[r.clone()], true, None, &HashMap::new());
        assert_eq!(hist[0].state, SubagentState::Running);
        assert_eq!(hist[0].ended_at, None);
        let observed = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(500));
        r.created_at = Some(t0);
        let hist = reconcile_history(&[r], true, observed, &HashMap::new());
        assert_eq!(hist[0].state, SubagentState::Interrupted);
        assert_eq!(hist[0].ended_at, Some(t1));
    }

    #[test]
    fn not_live_yields_empty_even_with_lingering_done() {
        let now = Instant::now();
        let prev = [sub("a", SubagentState::Done { done_at: now })];
        let out = run(&prev, &[rec("b", false)], false, None, &HashMap::new(), now);
        assert!(out.is_empty());
    }

    #[test]
    fn unfinished_is_running() {
        let out = run(
            &[],
            &[rec("a", false)],
            true,
            None,
            &HashMap::new(),
            Instant::now(),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].state, SubagentState::Running);
    }

    #[test]
    fn resume_leftover_is_filtered() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let mut old = rec("old", false);
        old.created_at = Some(t - Duration::from_secs(10));
        let mut new = rec("new", false);
        new.created_at = Some(t + Duration::from_secs(1));
        let mut at = rec("at", false);
        at.created_at = Some(t);
        let unknown = rec("unknown", false);
        let out = run(
            &[],
            &[old.clone(), new, at, unknown],
            true,
            Some(t),
            &HashMap::new(),
            Instant::now(),
        );
        let ids: Vec<_> = out.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["unknown", "at", "new"]);
        // No observed time: the filter is disabled.
        let out = run(&[], &[old], true, None, &HashMap::new(), Instant::now());
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn finished_at_first_sight_is_hidden() {
        let out = run(
            &[],
            &[rec("a", true)],
            true,
            None,
            &HashMap::new(),
            Instant::now(),
        );
        assert!(out.is_empty());
    }

    #[test]
    fn running_to_done_transition_records_now() {
        let now = Instant::now();
        let prev = [sub("a", SubagentState::Running)];
        let out = run(&prev, &[rec("a", true)], true, None, &HashMap::new(), now);
        assert_eq!(out[0].state, SubagentState::Done { done_at: now });
    }

    #[test]
    fn done_keeps_original_done_at() {
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(5);
        let prev = [sub("a", SubagentState::Done { done_at: t0 })];
        let out = run(&prev, &[rec("a", true)], true, None, &HashMap::new(), later);
        assert_eq!(out[0].state, SubagentState::Done { done_at: t0 });
    }

    #[test]
    fn vanished_records_running_dropped_done_kept() {
        let now = Instant::now();
        let prev = [
            sub("r", SubagentState::Running),
            sub("d", SubagentState::Done { done_at: now }),
        ];
        let out = run(&prev, &[], true, None, &HashMap::new(), now);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "d");
    }

    #[test]
    fn done_linger_kept_then_pruned() {
        let t0 = Instant::now();
        let mut list = vec![
            sub("d", SubagentState::Done { done_at: t0 }),
            sub("r", SubagentState::Running),
        ];
        let linger = Duration::from_secs(30);
        assert!(!prune_done(&mut list, t0 + Duration::from_secs(29), linger));
        assert_eq!(list.len(), 2);
        assert!(prune_done(&mut list, t0 + linger, linger));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "r");
        assert!(!prune_done(
            &mut list,
            t0 + Duration::from_secs(999),
            linger
        ));
    }

    #[test]
    fn history_keeps_finished_running_and_interrupted() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let mut fin = rec("fin", true);
        fin.created_at = Some(t - Duration::from_secs(20));
        let mut old = rec("old", false);
        old.created_at = Some(t - Duration::from_secs(10));
        let mut new = rec("new", false);
        new.created_at = Some(t + Duration::from_secs(1));
        let unknown = rec("unknown", false);
        let out = reconcile_history(&[new, old, fin, unknown], true, Some(t), &HashMap::new());
        let got: Vec<_> = out
            .iter()
            .map(|s| (s.id.as_str(), s.state.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("unknown", SubagentState::Running),
                ("fin", SubagentState::Finished),
                ("old", SubagentState::Interrupted),
                ("new", SubagentState::Running),
            ]
        );
        // Unknown observation time: nothing is interrupted.
        let out = reconcile_history(&[rec("a", false)], true, None, &HashMap::new());
        assert_eq!(out[0].state, SubagentState::Running);
    }

    #[test]
    fn history_empty_when_parent_not_live() {
        let out = reconcile_history(&[rec("a", true)], false, None, &HashMap::new());
        assert!(out.is_empty());
    }

    #[test]
    fn history_attributes_parents() {
        let mut parents = HashMap::new();
        parents.insert("child".to_string(), Some("parent".to_string()));
        let out = reconcile_history(
            &[rec("parent", true), rec("child", false)],
            true,
            None,
            &parents,
        );
        let c = out.iter().find(|s| s.id == "child").unwrap();
        assert_eq!(c.parent.as_deref(), Some("parent"));
    }

    #[test]
    fn parent_kept_only_when_present() {
        let mut parents = HashMap::new();
        parents.insert("child".to_string(), Some("parent".to_string()));
        parents.insert("orphan".to_string(), Some("missing".to_string()));
        parents.insert("selfref".to_string(), Some("selfref".to_string()));
        parents.insert("root".to_string(), None);
        let recs = [
            rec("parent", false),
            rec("child", false),
            rec("orphan", false),
            rec("selfref", false),
            rec("root", false),
        ];
        let out = run(&[], &recs, true, None, &parents, Instant::now());
        let p = |id: &str| out.iter().find(|s| s.id == id).unwrap().parent.clone();
        assert_eq!(p("child").as_deref(), Some("parent"));
        assert_eq!(p("orphan"), None);
        assert_eq!(p("selfref"), None);
        assert_eq!(p("root"), None);
        assert_eq!(p("parent"), None);
    }

    #[test]
    fn parent_cycle_is_broken() {
        let mut parents = HashMap::new();
        parents.insert("a".to_string(), Some("b".to_string()));
        parents.insert("b".to_string(), Some("a".to_string()));
        let out = run(
            &[],
            &[rec("a", false), rec("b", false)],
            true,
            None,
            &parents,
            Instant::now(),
        );
        assert!(out.iter().all(|s| s.parent.is_none()));
    }

    #[test]
    fn order_is_created_at_then_id() {
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let mut a = rec("z", false);
        a.created_at = Some(base);
        let mut b = rec("a", false);
        b.created_at = Some(base + Duration::from_secs(1));
        let mut c = rec("m", false);
        c.created_at = Some(base);
        let out = run(&[], &[b, a, c], true, None, &HashMap::new(), Instant::now());
        let ids: Vec<_> = out.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["m", "z", "a"]);
    }

    #[test]
    fn done_child_with_lingering_parent_keeps_parent() {
        let now = Instant::now();
        let prev = [
            sub("p", SubagentState::Done { done_at: now }),
            sub("c", SubagentState::Running),
        ];
        let mut parents = HashMap::new();
        parents.insert("c".to_string(), Some("p".to_string()));
        let out = run(&prev, &[rec("c", false)], true, None, &parents, now);
        assert_eq!(out.len(), 2);
        assert_eq!(
            out.iter().find(|s| s.id == "c").unwrap().parent.as_deref(),
            Some("p")
        );
    }
}
