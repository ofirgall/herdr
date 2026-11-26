//! Background filesystem watcher for subagent metadata (design D2).
//!
//! One worker thread owns a single `notify` watcher shared by every tracked
//! terminal. A single watcher keeps the thread/handle count constant no matter
//! how many Claude panes exist; watched paths are reference counted because
//! sessions of the same project share a parent directory.
//!
//! All filesystem I/O (existence checks, directory scans, parent attribution)
//! happens on this thread. Results are delivered through a caller-supplied
//! sink, so the app/render thread never touches the disk for subagents.
//!
//! Watch target per terminal, non-recursive:
//! `<sid>/subagents/` if it exists, else `<sid>/`, else the transcript's
//! directory. The target is upgraded when the deeper directory appears.
//! Only `.meta.json` changes (plus creation/removal of the directories on the
//! path) trigger work; subagent `.jsonl` writes are dropped before debouncing.
//!
//! Background shells (design D10) are tailed from the session transcript on
//! the same thread: the transcript's directory is watched too (ref counted),
//! only events for that exact path trigger a debounced incremental read, and
//! the safety-net pass tails as well.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::{
    attribute_parents, ClaudeSubagentSource, ShellLog, ShellRecord, SubagentRecord, SubagentSource,
};
use crate::terminal::TerminalId;

/// Coalescing window between the first relevant event and the scan.
pub const DEBOUNCE: Duration = Duration::from_millis(200);
/// Safety-net rescan period covering dropped or coalesced events.
pub const SAFETY_RESCAN: Duration = Duration::from_secs(30);

const META_SUFFIX: &str = ".meta.json";

#[derive(Debug, Clone, Copy)]
pub struct WatcherConfig {
    pub debounce: Duration,
    pub safety_rescan: Duration,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            debounce: DEBOUNCE,
            safety_rescan: SAFETY_RESCAN,
        }
    }
}

/// Result of one full directory scan for a tracked terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    pub terminal_id: TerminalId,
    /// The transcript path as given to [`SubagentWatcher::track`].
    pub transcript: PathBuf,
    pub records: Vec<SubagentRecord>,
    pub parents: HashMap<String, Option<String>>,
}

/// Result of tailing a tracked terminal's transcript for background shells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellsResult {
    pub terminal_id: TerminalId,
    /// The transcript path as given to [`SubagentWatcher::track`].
    pub transcript: PathBuf,
    /// Every shell parsed so far, in start order.
    pub shells: Vec<ShellRecord>,
}

/// What the watcher delivers to its sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatcherOutput {
    Subagents(ScanResult),
    Shells(ShellsResult),
}

enum Cmd {
    Track {
        terminal_id: TerminalId,
        transcript: PathBuf,
    },
    Untrack(TerminalId),
    Fs(Vec<PathBuf>),
    Shutdown,
}

/// Handle to the watcher thread. Dropping it stops the thread.
pub struct SubagentWatcher {
    tx: mpsc::Sender<Cmd>,
}

impl SubagentWatcher {
    /// Start the worker. `sink` receives every scan result and returns `false`
    /// once the consumer is gone, which stops the worker.
    pub fn spawn(
        sink: impl FnMut(WatcherOutput) -> bool + Send + 'static,
        config: WatcherConfig,
    ) -> io::Result<Self> {
        Self::spawn_with_source(Box::new(ClaudeSubagentSource), sink, config)
    }

    pub fn spawn_with_source(
        source: Box<dyn SubagentSource>,
        sink: impl FnMut(WatcherOutput) -> bool + Send + 'static,
        config: WatcherConfig,
    ) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let fs_tx = tx.clone();
        std::thread::Builder::new()
            .name("herdr-subagent-watch".into())
            .spawn(move || {
                let watcher = match notify::recommended_watcher(
                    move |res: notify::Result<notify::Event>| {
                        match res {
                            Ok(event) => {
                                if matches!(event.kind, EventKind::Access(_)) {
                                    return;
                                }
                                // Subagent transcripts are written constantly; drop
                                // events that only touch their `.jsonl` files up
                                // front. Other `.jsonl` events (session transcripts)
                                // pass and are path-filtered by the worker.
                                if event.paths.iter().all(|p| is_subagent_jsonl(p)) {
                                    return;
                                }
                                let _ = fs_tx.send(Cmd::Fs(event.paths));
                            }
                            Err(err) => tracing::debug!(%err, "subagent watcher backend error"),
                        }
                    },
                ) {
                    Ok(watcher) => Some(watcher),
                    Err(err) => {
                        // Degraded mode: only the safety-net rescan runs.
                        tracing::warn!(%err, "failed to create filesystem watcher for subagents");
                        None
                    }
                };
                Worker {
                    source,
                    sink: Box::new(sink),
                    config,
                    watcher,
                    watch_refs: HashMap::new(),
                    tracked: HashMap::new(),
                }
                .run(rx);
            })?;
        Ok(Self { tx })
    }

    /// Start (or restart) tracking `terminal_id` at `transcript`.
    pub fn track(&self, terminal_id: TerminalId, transcript: PathBuf) {
        let _ = self.tx.send(Cmd::Track {
            terminal_id,
            transcript,
        });
    }

    pub fn untrack(&self, terminal_id: TerminalId) {
        let _ = self.tx.send(Cmd::Untrack(terminal_id));
    }
}

impl Drop for SubagentWatcher {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

fn is_subagent_jsonl(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "jsonl")
        && path.components().any(|c| c.as_os_str() == "subagents")
}

fn is_meta_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(META_SUFFIX))
}

/// Canonicalize the longest existing ancestor and re-append the rest, so paths
/// that do not exist yet still match the (symlink-resolved) paths backends
/// report, e.g. `/var` vs `/private/var` on macOS.
fn canonicalize_lenient(path: &Path) -> PathBuf {
    let mut tail = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(mut resolved) = cur.canonicalize() {
            for part in tail.iter().rev() {
                resolved.push(part);
            }
            return resolved;
        }
        match (cur.file_name(), cur.parent()) {
            (Some(name), Some(parent)) => {
                let name = name.to_os_string();
                let parent = parent.to_path_buf();
                tail.push(name);
                cur = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Incremental tail state for one transcript.
struct ShellTail {
    /// Canonicalized transcript path events are matched against.
    path: PathBuf,
    log: ShellLog,
    offset: u64,
    due: Option<Instant>,
    /// Directory watch registered for the transcript's parent.
    watched: Option<PathBuf>,
    /// Last list sent, to suppress no-op deliveries.
    sent: Option<Vec<ShellRecord>>,
}

impl ShellTail {
    /// Read appended bytes (the whole file on first call). Returns whether a
    /// result should be delivered.
    fn read(&mut self) -> bool {
        let Ok(mut file) = File::open(&self.path) else {
            return false;
        };
        let Ok(len) = file.metadata().map(|m| m.len()) else {
            return false;
        };
        if len < self.offset {
            self.log.reset();
            self.offset = 0;
        }
        if len > self.offset && file.seek(SeekFrom::Start(self.offset)).is_ok() {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match file.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        self.log.feed(&buf[..n]);
                        self.offset += n as u64;
                    }
                    Err(err) => {
                        tracing::debug!(%err, "failed to read transcript for background shells");
                        break;
                    }
                }
            }
        }
        self.sent.as_deref() != Some(self.log.records())
    }
}

struct Tracked {
    transcript: PathBuf,
    shells: ShellTail,
    sub_dir: PathBuf,
    session_dir: PathBuf,
    project_dir: PathBuf,
    /// Parent of the project dir (Claude's `projects/`); the highest watch bound.
    projects_dir: PathBuf,
    watched: Option<PathBuf>,
    watch_failure_logged: bool,
    debounce_due: Option<Instant>,
    next_safety: Instant,
    parents: HashMap<String, Option<String>>,
}

impl Tracked {
    fn is_relevant(&self, path: &Path) -> bool {
        if path == self.project_dir || path == self.session_dir || path == self.sub_dir {
            return true;
        }
        path.starts_with(&self.sub_dir) && is_meta_file(path)
    }

    fn is_shell_relevant(&self, path: &Path) -> bool {
        path == self.shells.path || path == self.project_dir
    }

    fn desired_target(&self) -> &Path {
        if self.sub_dir.is_dir() {
            &self.sub_dir
        } else if self.session_dir.is_dir() {
            &self.session_dir
        } else if self.project_dir.is_dir() {
            &self.project_dir
        } else {
            &self.projects_dir
        }
    }
}

struct Worker {
    source: Box<dyn SubagentSource>,
    sink: Box<dyn FnMut(WatcherOutput) -> bool + Send>,
    config: WatcherConfig,
    watcher: Option<RecommendedWatcher>,
    watch_refs: HashMap<PathBuf, usize>,
    tracked: HashMap<TerminalId, Tracked>,
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Cmd>) {
        loop {
            let now = Instant::now();
            let wake = self
                .tracked
                .values()
                .flat_map(|t| [t.debounce_due, t.shells.due, Some(t.next_safety)])
                .flatten()
                .min();
            let first = match wake {
                Some(wake) => rx.recv_timeout(wake.saturating_duration_since(now)),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match first {
                Ok(cmd) => {
                    if !self.handle(cmd) {
                        return;
                    }
                    while let Ok(cmd) = rx.try_recv() {
                        if !self.handle(cmd) {
                            return;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if !self.scan_due(Instant::now()) {
                return;
            }
        }
    }

    /// Returns false when the worker should stop.
    fn handle(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Shutdown => return false,
            Cmd::Track {
                terminal_id,
                transcript,
            } => self.track(terminal_id, transcript),
            Cmd::Untrack(terminal_id) => self.untrack(&terminal_id),
            Cmd::Fs(paths) => self.on_fs_event(&paths),
        }
        true
    }

    fn track(&mut self, terminal_id: TerminalId, transcript: PathBuf) {
        self.untrack(&terminal_id);
        let Some(sub_dir) = self.source.subagent_dir(&transcript) else {
            tracing::debug!(path = %transcript.display(), "no subagent directory for transcript");
            return;
        };
        let sub_dir = canonicalize_lenient(&sub_dir);
        let transcript_canon = canonicalize_lenient(&transcript);
        let (Some(session_dir), Some(project_dir), Some(projects_dir)) = (
            sub_dir.parent().map(Path::to_path_buf),
            sub_dir
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf),
            sub_dir
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .map(Path::to_path_buf),
        ) else {
            return;
        };
        let now = Instant::now();
        self.tracked.insert(
            terminal_id,
            Tracked {
                transcript,
                shells: ShellTail {
                    path: transcript_canon,
                    log: ShellLog::new(),
                    offset: 0,
                    due: Some(now),
                    watched: None,
                    sent: None,
                },
                sub_dir,
                session_dir,
                project_dir,
                projects_dir,
                watched: None,
                watch_failure_logged: false,
                // Initial scan right away: records may predate the watch.
                debounce_due: Some(now),
                next_safety: now + self.config.safety_rescan,
                parents: HashMap::new(),
            },
        );
    }

    fn untrack(&mut self, terminal_id: &TerminalId) {
        if let Some(tracked) = self.tracked.remove(terminal_id) {
            if let Some(path) = tracked.watched {
                self.unwatch(&path);
            }
            if let Some(path) = tracked.shells.watched {
                self.unwatch(&path);
            }
        }
    }

    fn on_fs_event(&mut self, paths: &[PathBuf]) {
        let due = Instant::now() + self.config.debounce;
        for tracked in self.tracked.values_mut() {
            if tracked.debounce_due.is_none() && paths.iter().any(|p| tracked.is_relevant(p)) {
                tracked.debounce_due = Some(due);
            }
            if tracked.shells.due.is_none() && paths.iter().any(|p| tracked.is_shell_relevant(p)) {
                tracked.shells.due = Some(due);
            }
        }
    }

    fn watch(&mut self, path: &Path) -> Result<(), notify::Error> {
        if let Some(count) = self.watch_refs.get_mut(path) {
            *count += 1;
            return Ok(());
        }
        if let Some(watcher) = self.watcher.as_mut() {
            watcher.watch(path, RecursiveMode::NonRecursive)?;
        }
        self.watch_refs.insert(path.to_path_buf(), 1);
        Ok(())
    }

    fn unwatch(&mut self, path: &Path) {
        let Some(count) = self.watch_refs.get_mut(path) else {
            return;
        };
        *count -= 1;
        if *count > 0 {
            return;
        }
        self.watch_refs.remove(path);
        if let Some(watcher) = self.watcher.as_mut() {
            // The path may already be gone; that is fine.
            let _ = watcher.unwatch(path);
        }
    }

    /// Move the watch to the deepest existing directory on the path, repeating
    /// while the target keeps deepening: a directory created between the
    /// existence check and the watch registration would otherwise be missed.
    fn retarget(&mut self, terminal_id: &TerminalId) {
        self.watch_transcript_dir(terminal_id);
        for _ in 0..4 {
            if !self.retarget_once(terminal_id) {
                break;
            }
        }
    }

    /// Watch the transcript's directory (once it exists) for transcript events.
    fn watch_transcript_dir(&mut self, terminal_id: &TerminalId) {
        let Some(tracked) = self.tracked.get(terminal_id) else {
            return;
        };
        if tracked.shells.watched.is_some() || !tracked.project_dir.is_dir() {
            return;
        }
        let dir = tracked.project_dir.clone();
        let result = self.watch(&dir);
        if let Some(tracked) = self.tracked.get_mut(terminal_id) {
            match result {
                Ok(()) => tracked.shells.watched = Some(dir),
                Err(err) => {
                    tracing::debug!(%err, path = %dir.display(), "failed to watch transcript directory");
                }
            }
        }
    }

    /// Returns true if the watch moved to a new, successfully watched target.
    fn retarget_once(&mut self, terminal_id: &TerminalId) -> bool {
        let Some(tracked) = self.tracked.get(terminal_id) else {
            return false;
        };
        let desired = tracked.desired_target().to_path_buf();
        let previous = tracked.watched.clone();
        if previous.as_deref() == Some(desired.as_path()) {
            return false;
        }
        if let Some(previous) = &previous {
            self.unwatch(previous);
        }
        let result = self.watch(&desired);
        let Some(tracked) = self.tracked.get_mut(terminal_id) else {
            return false;
        };
        match result {
            Ok(()) => {
                tracked.watched = Some(desired);
                tracked.watch_failure_logged = false;
                true
            }
            Err(err) => {
                tracked.watched = None;
                if tracked.watch_failure_logged {
                    tracing::debug!(%err, path = %desired.display(), "subagent watch still failing");
                } else {
                    tracked.watch_failure_logged = true;
                    tracing::warn!(%err, path = %desired.display(), "failed to watch subagent directory");
                }
                false
            }
        }
    }

    /// Scan every terminal whose debounce or safety deadline passed.
    /// Returns false when the sink is closed.
    fn scan_due(&mut self, now: Instant) -> bool {
        let due: Vec<TerminalId> = self
            .tracked
            .iter()
            .filter(|(_, t)| {
                t.debounce_due.is_some_and(|d| d <= now)
                    || t.shells.due.is_some_and(|d| d <= now)
                    || t.next_safety <= now
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.retarget(&id);
            let Some(tracked) = self.tracked.get_mut(&id) else {
                continue;
            };
            let safety = tracked.next_safety <= now;
            let meta_due = safety || tracked.debounce_due.is_some_and(|d| d <= now);
            let shells_due = safety || tracked.shells.due.is_some_and(|d| d <= now);
            if safety {
                tracked.next_safety = Instant::now() + self.config.safety_rescan;
            }
            if shells_due {
                tracked.shells.due = None;
                if tracked.shells.read() {
                    let shells = tracked.shells.log.records().to_vec();
                    tracked.shells.sent = Some(shells.clone());
                    let result = ShellsResult {
                        terminal_id: id.clone(),
                        transcript: tracked.transcript.clone(),
                        shells,
                    };
                    if !(self.sink)(WatcherOutput::Shells(result)) {
                        return false;
                    }
                }
            }
            if !meta_due {
                continue;
            }
            tracked.debounce_due = None;
            let records = self.source.scan(&tracked.sub_dir);
            attribute_parents(&tracked.sub_dir, &records, &mut tracked.parents);
            tracked
                .parents
                .retain(|id, _| records.iter().any(|r| r.id == *id));
            let parents = tracked.parents.clone();
            let result = ScanResult {
                terminal_id: id,
                transcript: tracked.transcript.clone(),
                records,
                parents,
            };
            if !(self.sink)(WatcherOutput::Subagents(result)) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subagents::test_util::TempDir;
    use std::sync::mpsc::Receiver;

    const WAIT: Duration = Duration::from_secs(20);

    fn start(config: WatcherConfig) -> (SubagentWatcher, Receiver<ScanResult>) {
        let (tx, rx) = mpsc::channel();
        let watcher = SubagentWatcher::spawn(
            move |out| match out {
                WatcherOutput::Subagents(scan) => tx.send(scan).is_ok(),
                WatcherOutput::Shells(_) => true,
            },
            config,
        )
        .expect("spawn watcher");
        (watcher, rx)
    }

    fn quiet_safety() -> WatcherConfig {
        WatcherConfig {
            debounce: DEBOUNCE,
            safety_rescan: Duration::from_secs(3600),
        }
    }

    /// Consume scans until `pred` matches; panics after `WAIT`.
    fn wait_for(rx: &Receiver<ScanResult>, what: &str, pred: impl Fn(&ScanResult) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(scan) if pred(&scan) => return,
                Ok(_) => {}
                Err(_) => panic!("timed out waiting for scan: {what}"),
            }
        }
    }

    fn meta(finished: bool) -> String {
        if finished {
            r#"{"agentType":"explore","description":"look around","finished":true}"#.into()
        } else {
            r#"{"agentType":"explore","description":"look around"}"#.into()
        }
    }

    fn write_meta(sub_dir: &Path, id: &str, finished: bool) {
        std::fs::write(
            sub_dir.join(format!("agent-{id}.meta.json")),
            meta(finished),
        )
        .expect("write meta");
    }

    #[test]
    fn create_finish_delete_and_late_subagents_dir() {
        let tmp = TempDir::new();
        let project = tmp.path().join("proj");
        std::fs::create_dir_all(&project).expect("mkdir");
        let transcript = project.join("sid1.jsonl");
        std::fs::write(&transcript, "").expect("write transcript");
        let id = TerminalId::alloc();

        let (watcher, rx) = start(quiet_safety());
        watcher.track(id.clone(), transcript.clone());
        // Initial scan: nothing there yet.
        wait_for(&rx, "initial empty scan", |s| {
            s.terminal_id == id && s.transcript == transcript && s.records.is_empty()
        });

        // subagents/ appears after the watch started (session dir missing too).
        let sub_dir = project.join("sid1").join("subagents");
        std::fs::create_dir_all(&sub_dir).expect("mkdir subagents");
        write_meta(&sub_dir, "a1", false);
        wait_for(&rx, "running record", |s| {
            s.records.len() == 1 && s.records[0].id == "a1" && !s.records[0].finished
        });

        // Finish it, and ignore .jsonl churn alongside.
        std::fs::write(sub_dir.join("agent-a1.jsonl"), "{}\n").expect("jsonl");
        write_meta(&sub_dir, "a1", true);
        wait_for(&rx, "finished record", |s| {
            s.records.len() == 1 && s.records[0].finished
        });

        // Delete it.
        std::fs::remove_file(sub_dir.join("agent-a1.meta.json")).expect("rm");
        wait_for(&rx, "deleted record", |s| s.records.is_empty());
    }

    #[test]
    fn existing_records_are_found_by_initial_scan() {
        let tmp = TempDir::new();
        let project = tmp.path().join("proj");
        let sub_dir = project.join("sid2").join("subagents");
        std::fs::create_dir_all(&sub_dir).expect("mkdir");
        write_meta(&sub_dir, "pre", false);
        let id = TerminalId::alloc();

        let (watcher, rx) = start(quiet_safety());
        watcher.track(id.clone(), project.join("sid2.jsonl"));
        wait_for(&rx, "pre-existing record", |s| {
            s.terminal_id == id && s.records.iter().any(|r| r.id == "pre")
        });
        // And later changes in the upgraded watch are seen.
        write_meta(&sub_dir, "later", false);
        wait_for(&rx, "second record", |s| s.records.len() == 2);
    }

    #[test]
    fn jsonl_writes_do_not_trigger_scans() {
        let tmp = TempDir::new();
        let project = tmp.path().join("proj");
        let sub_dir = project.join("sid3").join("subagents");
        std::fs::create_dir_all(&sub_dir).expect("mkdir");
        let id = TerminalId::alloc();

        let (watcher, rx) = start(quiet_safety());
        watcher.track(id, project.join("sid3.jsonl"));
        wait_for(&rx, "initial scan", |_| true);
        // Let any backend startup noise settle.
        std::thread::sleep(Duration::from_millis(500));
        while rx.try_recv().is_ok() {}

        for n in 0..5 {
            std::fs::write(sub_dir.join("agent-x.jsonl"), format!("{n}\n")).expect("jsonl");
            std::fs::write(project.join("sid3.jsonl"), format!("{n}\n")).expect("transcript");
        }
        assert!(
            rx.recv_timeout(Duration::from_millis(1500)).is_err(),
            ".jsonl writes must not trigger a scan"
        );
    }

    #[test]
    fn untracked_terminal_gets_no_more_scans() {
        let tmp = TempDir::new();
        let project = tmp.path().join("proj");
        let sub_dir = project.join("sid4").join("subagents");
        std::fs::create_dir_all(&sub_dir).expect("mkdir");
        let id = TerminalId::alloc();

        let (watcher, rx) = start(quiet_safety());
        watcher.track(id.clone(), project.join("sid4.jsonl"));
        wait_for(&rx, "initial scan", |_| true);
        watcher.untrack(id);
        std::thread::sleep(Duration::from_millis(500));
        while rx.try_recv().is_ok() {}
        write_meta(&sub_dir, "z", false);
        assert!(rx.recv_timeout(Duration::from_millis(1500)).is_err());
    }

    #[test]
    fn missing_directories_do_not_fail_and_safety_net_recovers() {
        let tmp = TempDir::new();
        // Neither the project dir nor the session dir exist: the watch cannot
        // be registered, which is logged; scans still succeed (empty).
        let project = tmp.path().join("nope").join("proj");
        let id = TerminalId::alloc();
        let (watcher, rx) = start(WatcherConfig {
            debounce: DEBOUNCE,
            safety_rescan: Duration::from_millis(300),
        });
        watcher.track(id.clone(), project.join("sid5.jsonl"));
        wait_for(&rx, "initial empty scan", |s| s.records.is_empty());

        // Later the directories appear; the safety-net rescan retargets the
        // watch and finds the record even without a delivered event.
        let sub_dir = project.join("sid5").join("subagents");
        std::fs::create_dir_all(&sub_dir).expect("mkdir");
        write_meta(&sub_dir, "late", false);
        wait_for(&rx, "record after safety rescan", |s| {
            s.terminal_id == id && s.records.iter().any(|r| r.id == "late")
        });
    }

    #[test]
    fn lazily_created_project_dir_is_picked_up_promptly() {
        let tmp = TempDir::new();
        // Only the projects dir exists at track time, as for a brand-new
        // Claude project whose transcript has not been written yet.
        let projects = tmp.path().join("projects");
        std::fs::create_dir_all(&projects).expect("mkdir projects");
        let project = projects.join("slug");
        let transcript = project.join("sid6.jsonl");
        let id = TerminalId::alloc();

        // Safety rescan far away: only events can deliver the records.
        let (watcher, rx) = start(quiet_safety());
        watcher.track(id.clone(), transcript.clone());
        wait_for(&rx, "initial empty scan", |s| s.records.is_empty());

        let started = Instant::now();
        std::fs::create_dir_all(&project).expect("mkdir project");
        std::fs::write(&transcript, "").expect("transcript");
        let sub_dir = project.join("sid6").join("subagents");
        std::fs::create_dir_all(&sub_dir).expect("mkdir subagents");
        write_meta(&sub_dir, "x", false);
        wait_for(&rx, "running record", |s| {
            s.terminal_id == id && s.records.len() == 1 && !s.records[0].finished
        });
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "records must arrive via events, not the safety rescan"
        );

        // Atomic replace (temp file + rename) marking it finished.
        let tmp_file = sub_dir.join("agent-x.meta.json.tmp");
        std::fs::write(&tmp_file, meta(true)).expect("write tmp");
        std::fs::rename(&tmp_file, sub_dir.join("agent-x.meta.json")).expect("rename");
        wait_for(&rx, "finished record after rename", |s| {
            s.records.len() == 1 && s.records[0].finished
        });
    }

    fn start_shells(config: WatcherConfig) -> (SubagentWatcher, Receiver<ShellsResult>) {
        let (tx, rx) = mpsc::channel();
        let watcher = SubagentWatcher::spawn(
            move |out| match out {
                WatcherOutput::Shells(res) => tx.send(res).is_ok(),
                WatcherOutput::Subagents(_) => true,
            },
            config,
        )
        .expect("spawn watcher");
        (watcher, rx)
    }

    fn wait_shells(rx: &Receiver<ShellsResult>, what: &str, pred: impl Fn(&ShellsResult) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(res) if pred(&res) => return,
                Ok(_) => {}
                Err(_) => panic!("timed out waiting for shells: {what}"),
            }
        }
    }

    fn append(path: &Path, text: &str) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open transcript");
        file.write_all(text.as_bytes()).expect("append");
    }

    #[test]
    fn transcript_appends_are_tailed_into_shell_results() {
        let tmp = TempDir::new();
        let project = tmp.path().join("proj");
        std::fs::create_dir_all(&project).expect("mkdir");
        let transcript = project.join("sid7.jsonl");
        std::fs::write(&transcript, "").expect("transcript");
        // A sibling session's transcript must not trigger anything.
        let other = project.join("other.jsonl");
        let id = TerminalId::alloc();

        let (watcher, rx) = start_shells(quiet_safety());
        watcher.track(id.clone(), transcript.clone());
        wait_shells(&rx, "initial empty", |r| {
            r.terminal_id == id && r.transcript == transcript && r.shells.is_empty()
        });
        std::thread::sleep(Duration::from_millis(500));
        while rx.try_recv().is_ok() {}
        append(&other, "{\"run_in_background\":1}\n");
        assert!(rx.recv_timeout(Duration::from_millis(1000)).is_err());

        let use_line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"sleep 5","description":"nap","run_in_background":true}}]}}"#;
        let result_line = r#"{"type":"user","timestamp":"2026-11-26T12:00:00Z","message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]},"toolUseResult":{"backgroundTaskId":"b1"}}"#;
        append(&transcript, &format!("{use_line}\n{result_line}\n"));
        wait_shells(&rx, "running shell", |r| {
            r.shells.len() == 1
                && r.shells[0].id == "b1"
                && r.shells[0].status == crate::subagents::ShellStatus::Running
        });

        let note = r#"{"type":"queue-operation","timestamp":"2026-11-26T12:01:00Z","content":"<task-notification>\n<task-id>b1</task-id>\n<status>failed</status>\n</task-notification>"}"#;
        append(&transcript, &format!("{note}\n"));
        wait_shells(&rx, "failed shell", |r| {
            r.shells.len() == 1 && r.shells[0].status == crate::subagents::ShellStatus::Failed
        });

        // Rewrite shorter than the offset: parser resets.
        std::fs::write(&transcript, "{}\n").expect("rewrite");
        wait_shells(&rx, "reset after rewrite", |r| r.shells.is_empty());
    }

    #[test]
    fn existing_transcript_is_read_whole_on_first_pass() {
        let tmp = TempDir::new();
        let project = tmp.path().join("proj");
        std::fs::create_dir_all(&project).expect("mkdir");
        let transcript = project.join("sid8.jsonl");
        let use_line = r#"{"message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"x","run_in_background":true}}]}}"#;
        let result_line = r#"{"timestamp":"2026-11-26T12:00:00Z","message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]},"toolUseResult":{"backgroundTaskId":"b9"}}"#;
        std::fs::write(&transcript, format!("{use_line}\n{result_line}\n")).expect("write");
        let id = TerminalId::alloc();
        let (watcher, rx) = start_shells(quiet_safety());
        watcher.track(id, transcript);
        wait_shells(&rx, "preexisting shell", |r| {
            r.shells.len() == 1 && r.shells[0].id == "b9"
        });
    }

    #[test]
    fn non_claude_transcript_is_ignored() {
        let tmp = TempDir::new();
        let id = TerminalId::alloc();
        let (watcher, rx) = start(quiet_safety());
        watcher.track(id, tmp.path().join("not-a-transcript.txt"));
        assert!(rx.recv_timeout(Duration::from_millis(800)).is_err());
    }
}
