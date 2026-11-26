//! App-side glue for subagent tracking: watcher registration, scheduled
//! linger pruning, and the single flush point for subagent-list changes.
//!
//! Nothing here runs from view computation or rendering. Registration and
//! pruning run from the existing scheduled-task pass of both event loops.

use std::time::Instant;

use super::App;
use crate::events::AppEvent;
use crate::subagents::watcher::{WatcherConfig, WatcherOutput};
use crate::subagents::SubagentWatcher;

impl App {
    /// Scheduled-task entry point: keep watch registrations in sync with the
    /// terminals' tracked transcripts and prune lingering `Done` subagents.
    /// Returns whether anything visible changed.
    pub(crate) fn run_subagent_tasks(&mut self, now: Instant) -> bool {
        self.sync_subagent_watches();
        let due = self
            .state
            .next_subagent_prune_deadline()
            .is_some_and(|deadline| now >= deadline);
        if !due {
            return false;
        }
        let changed = self.state.prune_subagents_at(now);
        self.flush_subagent_changes();
        changed
    }

    /// Register/unregister watcher targets so they mirror each terminal's
    /// `agent_session_transcript`. Steady state is a cheap scan over terminals
    /// with no allocation or I/O; the watcher thread does all filesystem work.
    pub(crate) fn sync_subagent_watches(&mut self) {
        let mut desired = 0usize;
        let mut to_track = Vec::new();
        for (terminal_id, terminal) in &self.state.terminals {
            let Some(path) = terminal.agent_session_transcript() else {
                continue;
            };
            desired += 1;
            if self.subagent_tracked.get(terminal_id).map(|p| p.as_path()) != Some(path) {
                to_track.push((terminal_id.clone(), path.to_path_buf()));
            }
        }
        if to_track.is_empty() && desired == self.subagent_tracked.len() {
            return;
        }

        let stale: Vec<_> = self
            .subagent_tracked
            .keys()
            .filter(|terminal_id| {
                self.state
                    .terminals
                    .get(*terminal_id)
                    .and_then(|terminal| terminal.agent_session_transcript())
                    .is_none()
            })
            .cloned()
            .collect();
        for terminal_id in stale {
            self.subagent_tracked.remove(&terminal_id);
            if let Some(watcher) = &self.subagent_watcher {
                watcher.untrack(terminal_id);
            }
        }

        if to_track.is_empty() {
            return;
        }
        let started = self.ensure_subagent_watcher();
        for (terminal_id, path) in to_track {
            // If the watcher could not start (logged), the list stays empty;
            // the target is still recorded so we do not retry every tick.
            if started {
                if let Some(watcher) = &self.subagent_watcher {
                    watcher.track(terminal_id.clone(), path.clone());
                }
            }
            self.subagent_tracked.insert(terminal_id, path);
        }
    }

    fn ensure_subagent_watcher(&mut self) -> bool {
        if self.subagent_watcher.is_none() {
            let event_tx = self.event_tx.clone();
            let sink = move |output: WatcherOutput| {
                let event = match output {
                    WatcherOutput::Subagents(scan) => AppEvent::SubagentsScanned {
                        terminal_id: scan.terminal_id,
                        transcript: scan.transcript,
                        records: scan.records,
                        parents: scan.parents,
                    },
                    WatcherOutput::Shells(scan) => AppEvent::BackgroundShellsScanned {
                        terminal_id: scan.terminal_id,
                        transcript: scan.transcript,
                        shells: scan.shells,
                    },
                };
                event_tx.blocking_send(event).is_ok()
            };
            match SubagentWatcher::spawn(sink, WatcherConfig::default()) {
                Ok(watcher) => self.subagent_watcher = Some(watcher),
                Err(err) => {
                    tracing::warn!(%err, "failed to start subagent watcher thread");
                }
            }
        }
        self.subagent_watcher.is_some()
    }

    /// Single flush point for subagent-list and background-shell-list changes.
    /// Every mutation (scan applied, cleared, pruned) queues the terminal in
    /// `AppState::subagent_changed_terminals` (subagent lists) or
    /// `AppState::background_shell_changed_terminals` (shell lists); this
    /// drains both queues and emits the matching event per attached pane.
    pub(crate) fn flush_subagent_changes(&mut self) {
        if self.state.subagent_changed_terminals.is_empty()
            && self.state.background_shell_changed_terminals.is_empty()
        {
            return;
        }
        let subagent_changed = std::mem::take(&mut self.state.subagent_changed_terminals);
        for terminal_id in subagent_changed {
            tracing::debug!(terminal = %terminal_id, "subagents changed");
            for (ws_idx, pane_id) in self.panes_attached_to_terminal(&terminal_id) {
                let Some(pane) = self.pane_info(ws_idx, pane_id) else {
                    continue;
                };
                self.emit_event(crate::api::schema::EventEnvelope {
                    event: crate::api::schema::EventKind::PaneSubagentsChanged,
                    data: crate::api::schema::EventData::PaneSubagentsChanged {
                        pane_id: pane.pane_id,
                        workspace_id: pane.workspace_id,
                        subagents: pane.subagents,
                    },
                });
            }
        }
        let shells_changed = std::mem::take(&mut self.state.background_shell_changed_terminals);
        for terminal_id in shells_changed {
            tracing::debug!(terminal = %terminal_id, "background shells changed");
            for (ws_idx, pane_id) in self.panes_attached_to_terminal(&terminal_id) {
                let Some(pane) = self.pane_info(ws_idx, pane_id) else {
                    continue;
                };
                self.emit_event(crate::api::schema::EventEnvelope {
                    event: crate::api::schema::EventKind::PaneBackgroundShellsChanged,
                    data: crate::api::schema::EventData::PaneBackgroundShellsChanged {
                        pane_id: pane.pane_id,
                        workspace_id: pane.workspace_id,
                        background_shells: pane.background_shells,
                    },
                });
            }
        }
    }

    fn panes_attached_to_terminal(
        &self,
        terminal_id: &crate::terminal::TerminalId,
    ) -> Vec<(usize, crate::layout::PaneId)> {
        self.state
            .workspaces
            .iter()
            .enumerate()
            .flat_map(|(ws_idx, ws)| {
                ws.tabs.iter().flat_map(move |tab| {
                    tab.layout
                        .pane_ids()
                        .into_iter()
                        .filter_map(move |pane_id| {
                            (&tab.panes.get(&pane_id)?.attached_terminal_id == terminal_id)
                                .then_some((ws_idx, pane_id))
                        })
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::detect::{Agent, AgentState};
    use crate::layout::PaneId;
    use crate::subagents::test_util::TempDir;
    use crate::workspace::Workspace;

    fn test_app() -> (App, PaneId, crate::terminal::TerminalId) {
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            tokio::sync::mpsc::unbounded_channel().1,
            crate::api::EventHub::default(),
        );
        let ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        app.state.workspaces.push(ws);
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        let terminal_id = app
            .state
            .terminal_id_for_pane(0, pane_id)
            .expect("terminal");
        (app, pane_id, terminal_id)
    }

    fn start_claude_session(app: &mut App, pane_id: PaneId, sid: &str, transcript: &Path) {
        app.handle_internal_event(AppEvent::AgentProcessDetected {
            pane_id,
            agent: Agent::Claude,
            observed_at: Instant::now(),
        });
        app.handle_internal_event(AppEvent::AgentSessionReported {
            pane_id,
            source: "herdr:claude".into(),
            agent_label: "claude".into(),
            seq: Some(1),
            session_ref: crate::agent_resume::AgentSessionRef::id(sid),
            session_start_source: Some("startup".into()),
            session_transcript: Some(transcript.to_path_buf()),
        });
    }

    /// Feed scan events from the watcher into the app until `done` holds.
    fn pump_until(app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(app) {
            assert!(Instant::now() < deadline, "timed out: {what}");
            match app.event_rx.try_recv() {
                Ok(ev) => {
                    app.handle_internal_event_with_render_impact(ev);
                }
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
        }
    }

    fn write_meta(dir: &Path, id: &str, finished: bool) {
        let body = if finished {
            r#"{"agentType":"explore","description":"look","finished":true}"#
        } else {
            r#"{"agentType":"explore","description":"look"}"#
        };
        std::fs::write(dir.join(format!("agent-{id}.meta.json")), body).expect("write meta");
    }

    #[test]
    fn watcher_events_reach_terminal_state_and_untrack_on_release() {
        let tmp = TempDir::new();
        let project = tmp.path().join("proj");
        std::fs::create_dir_all(&project).expect("mkdir");
        let transcript: PathBuf = project.join("sidA.jsonl");
        std::fs::write(&transcript, "").expect("transcript");
        let (mut app, pane_id, terminal_id) = test_app();
        start_claude_session(&mut app, pane_id, "sidA", &transcript);

        app.run_subagent_tasks(Instant::now());
        assert_eq!(app.subagent_tracked.get(&terminal_id), Some(&transcript));

        // subagents/ is created after tracking started.
        let sub_dir = project.join("sidA").join("subagents");
        std::fs::create_dir_all(&sub_dir).expect("mkdir subagents");
        write_meta(&sub_dir, "a1", false);
        pump_until(&mut app, "running subagent", |app| {
            app.state.terminals[&terminal_id].subagents().len() == 1
        });

        // Agent exits: list empties at once and the watch is dropped.
        app.handle_internal_event(AppEvent::StateChanged {
            pane_id,
            agent: Some(Agent::Claude),
            state: AgentState::Idle,
            visible_blocker: false,
            visible_working: false,
            process_exited: true,
            observed_at: Instant::now(),
        });
        assert!(app.state.terminals[&terminal_id].subagents().is_empty());
        app.run_subagent_tasks(Instant::now());
        assert!(app.subagent_tracked.is_empty());
    }

    #[test]
    fn watch_on_missing_directories_logs_and_leaves_list_empty() {
        let tmp = TempDir::new();
        let transcript = tmp.path().join("missing").join("proj").join("sidB.jsonl");
        let (mut app, pane_id, terminal_id) = test_app();
        start_claude_session(&mut app, pane_id, "sidB", &transcript);
        app.run_subagent_tasks(Instant::now());
        assert!(app.subagent_tracked.contains_key(&terminal_id));

        // The initial (empty) scan still arrives and changes nothing.
        let deadline = Instant::now() + Duration::from_secs(20);
        let ev = loop {
            assert!(Instant::now() < deadline, "no initial scan event");
            if let Ok(ev) = app.event_rx.try_recv() {
                break ev;
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        assert!(matches!(ev, AppEvent::SubagentsScanned { .. }));
        assert!(!app.handle_internal_event_with_render_impact(ev));
        assert!(app.state.terminals[&terminal_id].subagents().is_empty());
    }

    #[test]
    fn scheduled_tick_prunes_done_subagents_after_linger() {
        let (mut app, pane_id, terminal_id) = test_app();
        let transcript = PathBuf::from("/t/sidC.jsonl");
        start_claude_session(&mut app, pane_id, "sidC", &transcript);
        let scan = |app: &mut App, finished: bool| {
            let record = crate::subagents::SubagentRecord {
                id: "a".into(),
                kind: "explore".into(),
                description: "look".into(),
                tool_use_id: None,
                spawn_depth: 1,
                background: false,
                finished,
                created_at: None,
                started_at: None,
                ended_at: None,
            };
            app.handle_internal_event_with_render_impact(AppEvent::SubagentsScanned {
                terminal_id: terminal_id.clone(),
                transcript: transcript.clone(),
                records: vec![record],
                parents: Default::default(),
            })
        };
        assert!(scan(&mut app, false));
        assert!(scan(&mut app, true)); // running -> done, lingering
        let deadline = app.state.next_subagent_prune_deadline().expect("deadline");
        assert!(deadline > Instant::now() + Duration::from_secs(20));

        assert!(!app.run_subagent_tasks(deadline - Duration::from_millis(1)));
        assert_eq!(app.state.terminals[&terminal_id].subagents().len(), 1);
        assert!(app.run_subagent_tasks(deadline + crate::subagents::DONE_LINGER));
        assert!(app.state.terminals[&terminal_id].subagents().is_empty());
        assert!(app.state.subagent_changed_terminals.is_empty()); // flushed
    }

    fn sample_subagent(
        id: &str,
        state: crate::subagents::SubagentState,
    ) -> crate::subagents::Subagent {
        crate::subagents::Subagent {
            id: id.into(),
            kind: "explore".into(),
            description: "look around".into(),
            parent: Some("root_child".into()),
            background: true,
            state,
            started_at: None,
            ended_at: None,
        }
    }

    #[test]
    fn pane_info_exposes_subagents_and_omits_empty_list() {
        let (mut app, pane_id, terminal_id) = test_app();

        let empty = app.pane_info(0, pane_id).expect("pane info");
        assert!(empty.subagents.is_empty());
        let json = serde_json::to_value(&empty).expect("json");
        assert!(json.get("subagents").is_none());

        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("terminal")
            .set_subagents(vec![
                sample_subagent("a1", crate::subagents::SubagentState::Running),
                sample_subagent(
                    "a2",
                    crate::subagents::SubagentState::Done {
                        done_at: Instant::now(),
                    },
                ),
            ]);
        let info = app.pane_info(0, pane_id).expect("pane info");
        let json = serde_json::to_value(&info).expect("json");
        assert_eq!(
            json["subagents"],
            serde_json::json!([
                {"id": "a1", "kind": "explore", "description": "look around",
                 "parent_id": "root_child", "background": true, "state": "running"},
                {"id": "a2", "kind": "explore", "description": "look around",
                 "parent_id": "root_child", "background": true, "state": "done"},
            ])
        );
    }

    fn sample_shell(
        id: &str,
        state: crate::subagents::ShellState,
    ) -> crate::subagents::BackgroundShell {
        crate::subagents::BackgroundShell {
            id: id.into(),
            command: "sleep 9".into(),
            description: "nap".into(),
            state,
            started_at: None,
            ended_at: None,
            done_at: None,
        }
    }

    #[test]
    fn pane_info_exposes_background_shells_and_omits_empty_list() {
        let (mut app, pane_id, terminal_id) = test_app();
        let empty = app.pane_info(0, pane_id).expect("pane info");
        assert!(empty.background_shells.is_empty());
        let json = serde_json::to_value(&empty).expect("json");
        assert!(json.get("background_shells").is_none());

        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("terminal")
            .set_background_shells(
                vec![sample_shell("b1", crate::subagents::ShellState::Running)],
                Vec::new(),
            );
        let info = app.pane_info(0, pane_id).expect("pane info");
        let json = serde_json::to_value(&info).expect("json");
        assert_eq!(
            json["background_shells"],
            serde_json::json!([
                {"id": "b1", "command": "sleep 9", "description": "nap", "state": "running"},
            ])
        );
    }

    #[test]
    fn shell_change_emits_only_background_shells_changed_and_vice_versa() {
        use crate::api::schema::EventData;
        let (mut app, pane_id, terminal_id) = test_app();
        let public_pane = app.public_pane_id(0, pane_id).expect("public id");

        // Shell-only change.
        let before = app.event_hub.current_sequence();
        let terminal = app.state.terminals.get_mut(&terminal_id).expect("terminal");
        assert!(terminal.set_background_shells(
            vec![sample_shell("b1", crate::subagents::ShellState::Running)],
            Vec::new(),
        ));
        assert_eq!(terminal.subagents_revision(), 0);
        app.state
            .background_shell_changed_terminals
            .push(terminal_id.clone());
        app.flush_subagent_changes();
        let events = app.event_hub.events_after(before);
        assert!(events
            .iter()
            .all(|(_, e)| !matches!(e.data, EventData::PaneSubagentsChanged { .. })));
        let shells: Vec<_> = events
            .iter()
            .filter_map(|(_, e)| match &e.data {
                EventData::PaneBackgroundShellsChanged {
                    pane_id,
                    background_shells,
                    ..
                } => Some((e.event, pane_id.clone(), background_shells.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(shells.len(), 1);
        assert_eq!(
            shells[0].0,
            crate::api::schema::EventKind::PaneBackgroundShellsChanged
        );
        assert_eq!(shells[0].1, public_pane);
        assert_eq!(shells[0].2.len(), 1);
        assert_eq!(
            shells[0].2[0].state,
            crate::api::schema::BackgroundShellStatus::Running
        );

        // Subagent-only change.
        let before = app.event_hub.current_sequence();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("terminal")
            .set_subagents(vec![sample_subagent(
                "a1",
                crate::subagents::SubagentState::Running,
            )]);
        app.state.subagent_changed_terminals.push(terminal_id);
        app.flush_subagent_changes();
        let events = app.event_hub.events_after(before);
        assert!(events
            .iter()
            .any(|(_, e)| matches!(e.data, EventData::PaneSubagentsChanged { .. })));
        assert!(events
            .iter()
            .all(|(_, e)| !matches!(e.data, EventData::PaneBackgroundShellsChanged { .. })));
    }

    #[test]
    fn flush_emits_pane_subagents_changed_for_attached_pane() {
        let (mut app, pane_id, terminal_id) = test_app();
        let before = app.event_hub.current_sequence();

        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("terminal")
            .set_subagents(vec![sample_subagent(
                "a1",
                crate::subagents::SubagentState::Running,
            )]);
        app.state.subagent_changed_terminals.push(terminal_id);
        app.flush_subagent_changes();

        let public_pane = app.public_pane_id(0, pane_id).expect("public id");
        let events = app.event_hub.events_after(before);
        let emitted: Vec<_> = events
            .iter()
            .filter_map(|(_, event)| match &event.data {
                crate::api::schema::EventData::PaneSubagentsChanged {
                    pane_id, subagents, ..
                } => Some((event.event, pane_id.clone(), subagents.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(emitted.len(), 1);
        assert_eq!(
            emitted[0].0,
            crate::api::schema::EventKind::PaneSubagentsChanged
        );
        assert_eq!(emitted[0].1, public_pane);
        assert_eq!(emitted[0].2.len(), 1);
        assert_eq!(
            emitted[0].2[0].state,
            crate::api::schema::SubagentStatus::Running
        );
    }
}
