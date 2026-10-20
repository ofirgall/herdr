//! Server-owned terminal panels.
//!
//! A panel is a long-lived process declared in the config (`[[panels]]`). Its
//! terminal lives in `AppState::terminals` and its runtime in the terminal
//! runtime registry, like the plugin popup, but it belongs to no workspace,
//! tab or pane layout. Agent detection is disabled for it, and it is not part
//! of saved session state, pane listings or live handoff.

use std::path::PathBuf;

use crate::api::schema::{PanelInfo, PanelRestartParams, PanelRunState, ResponseResult};
use crate::app::api::responses::{encode_error, encode_success};
use crate::app::state::{PanelSource, PanelSpec, PanelState, PanelStatus};
use crate::app::App;
use crate::config::PanelConfig;
use crate::layout::PaneId;
use crate::pane::PaneLaunchEnv;
use crate::terminal::{TerminalId, TerminalRuntime, TerminalState};

/// Size a panel terminal starts with, until a client reports the real one.
pub(crate) const PANEL_DEFAULT_COLS: u16 = 80;
pub(crate) const PANEL_DEFAULT_ROWS: u16 = 24;

/// What a config reload does to the running panels, by id.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PanelReconcilePlan {
    pub add: Vec<String>,
    pub remove: Vec<String>,
    pub respawn: Vec<String>,
    pub keep: Vec<String>,
}

fn panel_not_found_message(id: &str) -> String {
    format!("no panel with id \"{id}\" is configured")
}

pub(crate) fn panel_spec_from_config(config: &PanelConfig) -> PanelSpec {
    PanelSpec {
        argv: config.command.clone(),
        cwd: config.cwd.clone(),
    }
}

/// Diff the running panels against the desired config entries.
pub(crate) fn plan_panel_reconcile(
    current: &[PanelState],
    desired: &[PanelConfig],
) -> PanelReconcilePlan {
    let mut plan = PanelReconcilePlan::default();
    for entry in desired {
        match current.iter().find(|panel| panel.id == entry.id) {
            None => plan.add.push(entry.id.clone()),
            Some(panel) if panel.spec != panel_spec_from_config(entry) => {
                plan.respawn.push(entry.id.clone());
            }
            Some(_) => plan.keep.push(entry.id.clone()),
        }
    }
    for panel in current {
        if !desired.iter().any(|entry| entry.id == panel.id) {
            plan.remove.push(panel.id.clone());
        }
    }
    plan
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PanelRestartError {
    NotFound,
    SpawnFailed,
}

impl App {
    /// Start the configured panels. Called once when the server boots.
    pub(crate) fn spawn_configured_panels(&mut self, config: &crate::config::Config) {
        for diagnostic in self.reconcile_panels(&config.valid_panels()) {
            tracing::warn!("{diagnostic}");
        }
    }

    /// Make the running panels match `desired`: start new ids, stop removed
    /// ones, respawn ones whose command or cwd changed, and leave unchanged
    /// ones (including exited ones) alone with the same terminal. Returns
    /// warnings for panels that failed to start.
    pub(crate) fn reconcile_panels(&mut self, desired: &[PanelConfig]) -> Vec<String> {
        let plan = plan_panel_reconcile(&self.state.panels, desired);
        let mut diagnostics = Vec::new();
        let mut previous = std::mem::take(&mut self.state.panels);
        let mut next = Vec::with_capacity(desired.len());
        for entry in desired {
            let existing = previous
                .iter()
                .position(|panel| panel.id == entry.id)
                .map(|index| previous.remove(index));
            if plan.keep.contains(&entry.id) {
                next.extend(existing);
                continue;
            }
            let (cols, rows) = existing
                .as_ref()
                .map_or((PANEL_DEFAULT_COLS, PANEL_DEFAULT_ROWS), |panel| {
                    (panel.cols, panel.rows)
                });
            if let Some(old) = existing {
                self.release_panel_terminal(&old);
            }
            match self.spawn_panel(
                &entry.id,
                PanelSource::Config,
                panel_spec_from_config(entry),
                cols,
                rows,
            ) {
                Ok(panel) => next.push(panel),
                Err(err) => {
                    diagnostics.push(format!(
                        "panel \"{}\" failed to start: {err}; panel is unavailable",
                        entry.id
                    ));
                }
            }
        }
        for removed in previous {
            self.release_panel_terminal(&removed);
        }
        self.state.panels = next;
        if !plan.add.is_empty() || !plan.remove.is_empty() || !plan.respawn.is_empty() {
            self.render_dirty.request_generic();
            self.render_notify.notify_one();
        }
        diagnostics
    }

    /// Start a fresh process for panel `id`, replacing the current one. Used by
    /// restarting an exited panel; also works on a running one.
    pub(crate) fn restart_panel(&mut self, id: &str) -> Result<(), PanelRestartError> {
        let index = self
            .state
            .panels
            .iter()
            .position(|panel| panel.id == id)
            .ok_or(PanelRestartError::NotFound)?;
        let old = self.state.panels[index].clone();
        self.release_panel_terminal(&old);
        match self.spawn_panel(&old.id, old.source, old.spec, old.cols, old.rows) {
            Ok(panel) => {
                self.state.panels[index] = panel;
                self.render_dirty.request_generic();
                self.render_notify.notify_one();
                Ok(())
            }
            Err(err) => {
                tracing::warn!(panel = id, err = %err, "failed to restart panel");
                self.state.panels.remove(index);
                self.render_dirty.request_generic();
                self.render_notify.notify_one();
                Err(PanelRestartError::SpawnFailed)
            }
        }
    }

    /// Resize a panel terminal and remember the size for restarts.
    pub(crate) fn resize_panel(
        &mut self,
        id: &str,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) -> bool {
        let Some(panel) = self.state.panels.iter_mut().find(|panel| panel.id == id) else {
            return false;
        };
        if cols == 0 || rows == 0 {
            return false;
        }
        panel.cols = cols;
        panel.rows = rows;
        if self
            .state
            .direct_attach_resize_locks
            .contains(&panel.terminal_id)
        {
            return true;
        }
        if let Some(runtime) = self.terminal_runtimes.get(&panel.terminal_id) {
            runtime.resize(rows, cols, cell_width_px, cell_height_px);
        }
        true
    }

    pub(crate) fn panel_info(panel: &PanelState) -> PanelInfo {
        let (status, exit_code) = match panel.status {
            PanelStatus::Running => (PanelRunState::Running, None),
            PanelStatus::Exited(code) => (PanelRunState::Exited, code),
        };
        PanelInfo {
            id: panel.id.clone(),
            status,
            exit_code,
        }
    }

    pub(crate) fn handle_panel_list(&self, id: String) -> String {
        let panels = self.state.panels.iter().map(Self::panel_info).collect();
        encode_success(id, ResponseResult::PanelList { panels })
    }

    pub(crate) fn handle_panel_restart(
        &mut self,
        id: String,
        params: PanelRestartParams,
    ) -> String {
        match self.restart_panel(&params.id) {
            Ok(()) => {
                let panel = self
                    .state
                    .panels
                    .iter()
                    .find(|panel| panel.id == params.id)
                    .map(Self::panel_info);
                match panel {
                    Some(panel) => encode_success(id, ResponseResult::PanelRestarted { panel }),
                    None => {
                        encode_error(id, "panel_not_found", panel_not_found_message(&params.id))
                    }
                }
            }
            Err(PanelRestartError::NotFound) => {
                encode_error(id, "panel_not_found", panel_not_found_message(&params.id))
            }
            Err(PanelRestartError::SpawnFailed) => encode_error(
                id,
                "panel_restart_failed",
                format!("panel \"{}\" could not be started", params.id),
            ),
        }
    }

    /// Record that the panel owning `pane_id` lost its process. Returns false
    /// when `pane_id` is not a panel.
    pub(crate) fn handle_panel_process_exit(
        &mut self,
        pane_id: PaneId,
        exit_code: Option<i32>,
    ) -> bool {
        let Some(panel) = self
            .state
            .panels
            .iter_mut()
            .find(|panel| panel.pane_id == pane_id)
        else {
            return false;
        };
        panel.status = PanelStatus::Exited(exit_code);
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        true
    }

    fn release_panel_terminal(&mut self, panel: &PanelState) {
        self.state
            .direct_attach_resize_locks
            .remove(&panel.terminal_id);
        self.state.terminals.remove(&panel.terminal_id);
        self.shutdown_terminal_runtime(panel.terminal_id.clone());
    }

    fn spawn_panel(
        &mut self,
        id: &str,
        source: PanelSource,
        spec: PanelSpec,
        cols: u16,
        rows: u16,
    ) -> std::io::Result<PanelState> {
        let pane_id = PaneId::alloc();
        let terminal_id = TerminalId::alloc();
        let cwd = spec
            .cwd
            .as_deref()
            .map(|cwd| crate::worktree::expand_tilde_path(&cwd.to_string_lossy()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
        let runtime = self.spawn_panel_runtime(pane_id, &spec, &cwd, cols, rows)?;
        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        self.state.terminals.insert(
            terminal_id.clone(),
            TerminalState::new(terminal_id.clone(), cwd).with_launch_argv(spec.argv.clone()),
        );
        Ok(PanelState {
            id: id.to_string(),
            source,
            pane_id,
            terminal_id,
            spec,
            status: PanelStatus::Running,
            cols,
            rows,
        })
    }

    /// The one place that starts a panel process. Tests replace it through
    /// `test_panel_runtime_factory`.
    fn spawn_panel_runtime(
        &mut self,
        pane_id: PaneId,
        spec: &PanelSpec,
        cwd: &std::path::Path,
        cols: u16,
        rows: u16,
    ) -> std::io::Result<TerminalRuntime> {
        #[cfg(test)]
        if let Some(factory) = self.test_panel_runtime_factory.as_mut() {
            return factory(spec, cols, rows);
        }
        let launch_env = PaneLaunchEnv::from_extra(Vec::new()).without_pane_identity();
        TerminalRuntime::spawn_argv_command(
            pane_id,
            rows,
            cols,
            cwd.to_path_buf(),
            &spec.argv,
            &launch_env,
            crate::pane::AgentDetection::Disabled,
            self.state.pane_scrollback_limit_bytes,
            self.state.host_terminal_theme,
            self.state.host_terminal_appearance,
            self.event_tx.clone(),
            self.render_notify.clone(),
            self.render_dirty.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type SpawnLog = Arc<Mutex<Vec<Vec<String>>>>;

    fn test_app() -> (App, SpawnLog) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        let log: SpawnLog = Arc::default();
        let factory_log = log.clone();
        app.test_panel_runtime_factory = Some(Box::new(move |spec, cols, rows| {
            if spec.argv.first().is_some_and(|program| program == "fail") {
                return Err(std::io::Error::other("boom"));
            }
            factory_log.lock().unwrap().push(spec.argv.clone());
            Ok(TerminalRuntime::test_with_screen_bytes(cols, rows, b""))
        }));
        (app, log)
    }

    fn entry(id: &str, argv: &[&str]) -> PanelConfig {
        PanelConfig {
            id: id.to_string(),
            command: argv.iter().map(|word| word.to_string()).collect(),
            cwd: None,
        }
    }

    fn panel<'a>(app: &'a App, id: &str) -> &'a PanelState {
        app.state.panels.iter().find(|p| p.id == id).unwrap()
    }

    #[tokio::test]
    async fn configured_panels_get_terminals_outside_workspaces() {
        let (mut app, log) = test_app();
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("one")];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        let panes_before = app.state.workspaces[0].tabs[0].panes.len();

        let diagnostics = app.reconcile_panels(&[
            entry("prs", &["gh", "dash"]),
            entry("logs", &["tail", "-f", "x"]),
        ]);

        assert!(diagnostics.is_empty());
        assert_eq!(app.state.panels.len(), 2);
        assert_eq!(log.lock().unwrap().len(), 2);
        for p in &app.state.panels {
            assert_eq!(p.status, PanelStatus::Running);
            assert!(app.state.terminals.contains_key(&p.terminal_id));
            assert!(app.terminal_runtimes.get(&p.terminal_id).is_some());
            assert!(app.find_pane(p.pane_id).is_none());
        }
        assert_eq!(app.state.workspaces[0].tabs[0].panes.len(), panes_before);
        app.state.assert_invariants_for_test();
    }

    #[tokio::test]
    async fn process_exit_marks_panel_exited_and_keeps_it() {
        let (mut app, _) = test_app();
        app.reconcile_panels(&[entry("prs", &["gh"])]);
        let pane_id = panel(&app, "prs").pane_id;

        app.handle_internal_event(crate::events::AppEvent::PaneDied {
            pane_id,
            exit_reason: crate::platform::ChildExitReason::Exited,
        });

        assert_eq!(panel(&app, "prs").status, PanelStatus::Exited(None));
        assert!(app
            .state
            .terminals
            .contains_key(&panel(&app, "prs").terminal_id));
        app.state.assert_invariants_for_test();
    }

    #[tokio::test]
    async fn unrelated_pane_exit_does_not_touch_panels() {
        let (mut app, _) = test_app();
        app.reconcile_panels(&[entry("prs", &["gh"])]);
        assert!(!app.handle_panel_process_exit(PaneId::alloc(), None));
        assert_eq!(panel(&app, "prs").status, PanelStatus::Running);
    }

    #[tokio::test]
    async fn reload_adds_removes_respawns_and_keeps_by_id() {
        let (mut app, log) = test_app();
        app.reconcile_panels(&[
            entry("keep", &["a"]),
            entry("change", &["b"]),
            entry("gone", &["c"]),
        ]);
        let keep_terminal = panel(&app, "keep").terminal_id.clone();
        let change_terminal = panel(&app, "change").terminal_id.clone();
        let gone_terminal = panel(&app, "gone").terminal_id.clone();
        log.lock().unwrap().clear();

        let plan = plan_panel_reconcile(
            &app.state.panels,
            &[
                entry("keep", &["a"]),
                entry("change", &["b", "--new"]),
                entry("fresh", &["d"]),
            ],
        );
        assert_eq!(plan.keep, vec!["keep"]);
        assert_eq!(plan.respawn, vec!["change"]);
        assert_eq!(plan.add, vec!["fresh"]);
        assert_eq!(plan.remove, vec!["gone"]);

        let diagnostics = app.reconcile_panels(&[
            entry("keep", &["a"]),
            entry("change", &["b", "--new"]),
            entry("fresh", &["d"]),
        ]);

        assert!(diagnostics.is_empty());
        let ids: Vec<_> = app.state.panels.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["keep", "change", "fresh"]);
        assert_eq!(panel(&app, "keep").terminal_id, keep_terminal);
        assert!(app.terminal_runtimes.get(&keep_terminal).is_some());
        assert_ne!(panel(&app, "change").terminal_id, change_terminal);
        assert_eq!(panel(&app, "change").spec.argv, vec!["b", "--new"]);
        assert!(!app.state.terminals.contains_key(&change_terminal));
        assert!(app.terminal_runtimes.get(&change_terminal).is_none());
        assert!(!app.state.terminals.contains_key(&gone_terminal));
        assert!(app.terminal_runtimes.get(&gone_terminal).is_none());
        assert_eq!(log.lock().unwrap().len(), 2, "only change and fresh spawn");
        app.state.assert_invariants_for_test();
    }

    #[tokio::test]
    async fn cwd_change_respawns_and_exited_unchanged_panel_stays_exited() {
        let (mut app, _) = test_app();
        app.reconcile_panels(&[entry("a", &["x"]), entry("b", &["y"])]);
        let a_pane = panel(&app, "a").pane_id;
        app.handle_panel_process_exit(a_pane, Some(1));
        let b_terminal = panel(&app, "b").terminal_id.clone();

        let mut b = entry("b", &["y"]);
        b.cwd = Some(PathBuf::from("/tmp"));
        app.reconcile_panels(&[entry("a", &["x"]), b]);

        assert_eq!(panel(&app, "a").status, PanelStatus::Exited(Some(1)));
        assert_ne!(panel(&app, "b").terminal_id, b_terminal);
        assert_eq!(panel(&app, "b").spec.cwd, Some(PathBuf::from("/tmp")));
    }

    #[tokio::test]
    async fn failed_spawn_is_reported_and_other_panels_still_start() {
        let (mut app, _) = test_app();
        let diagnostics = app.reconcile_panels(&[entry("bad", &["fail"]), entry("good", &["ok"])]);

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("bad"));
        assert_eq!(app.state.panels.len(), 1);
        assert_eq!(app.state.panels[0].id, "good");
        app.state.assert_invariants_for_test();
    }

    #[tokio::test]
    async fn restart_replaces_exited_panel_process_and_keeps_size() {
        let (mut app, log) = test_app();
        app.reconcile_panels(&[entry("prs", &["gh"])]);
        assert!(app.resize_panel("prs", 50, 12, 0, 0));
        let old = panel(&app, "prs").clone();
        app.handle_panel_process_exit(old.pane_id, Some(0));
        log.lock().unwrap().clear();

        assert_eq!(app.restart_panel("prs"), Ok(()));

        let new = panel(&app, "prs");
        assert_eq!(new.status, PanelStatus::Running);
        assert_ne!(new.terminal_id, old.terminal_id);
        assert_ne!(new.pane_id, old.pane_id);
        assert_eq!((new.cols, new.rows), (50, 12));
        assert_eq!(log.lock().unwrap().len(), 1);
        assert!(!app.state.terminals.contains_key(&old.terminal_id));
        assert_eq!(
            app.restart_panel("missing"),
            Err(PanelRestartError::NotFound)
        );
        app.state.assert_invariants_for_test();
    }

    #[tokio::test]
    async fn panel_terminals_are_excluded_from_session_snapshot() {
        let (mut app, _) = test_app();
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("one")];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        app.reconcile_panels(&[entry("prs", &["gh", "dash"])]);
        let panel_terminal = panel(&app, "prs").terminal_id.clone();

        let snapshot = crate::persist::capture(
            &app.state.workspaces,
            &app.state.terminals,
            &app.terminal_runtimes,
            app.state.active,
            app.state.selected,
        );
        let json = serde_json::to_string(&snapshot).unwrap();

        assert!(!json.contains(&panel_terminal.to_string()));
        assert!(!json.contains("dash"));
    }

    #[tokio::test]
    async fn panel_terminals_have_no_agent_detection_identity() {
        let (mut app, _) = test_app();
        app.reconcile_panels(&[entry("prs", &["gh"])]);
        let panel = panel(&app, "prs");
        // Detection results are keyed by pane id and resolved through workspace
        // panes, so a panel pane id must never resolve to a workspace pane.
        assert!(app.find_pane(panel.pane_id).is_none());
        let terminal = app.state.terminals.get(&panel.terminal_id).unwrap();
        assert!(terminal.pending_agent_resume_plan.is_none());
    }

    #[tokio::test]
    async fn live_config_reload_reconciles_panels_unless_section_invalid() {
        let (mut app, _) = test_app();
        let config = crate::config::Config {
            panels: vec![entry("prs", &["gh"])],
            ..Default::default()
        };
        app.apply_live_config(&config, &[], &[], false);
        assert_eq!(app.state.panels.len(), 1);
        let terminal = panel(&app, "prs").terminal_id.clone();

        app.apply_live_config(&config, &[], &[], false);
        assert_eq!(panel(&app, "prs").terminal_id, terminal);

        let empty = crate::config::Config::default();
        app.apply_live_config(&empty, &[], &["panels".to_string()], false);
        assert_eq!(app.state.panels.len(), 1, "invalid section keeps panels");

        app.apply_live_config(&empty, &[], &[], false);
        assert!(app.state.panels.is_empty());
        assert!(app.terminal_runtimes.get(&terminal).is_none());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn real_process_exit_is_reported_as_panel_exit() {
        let (mut app, _) = test_app();
        app.test_panel_runtime_factory = None;
        app.reconcile_panels(&[entry("quick", &["sh", "-c", "exit 0"])]);
        assert_eq!(app.state.panels.len(), 1);
        let pane_id = panel(&app, "quick").pane_id;

        let event = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                match app.event_rx.recv().await {
                    Some(crate::events::AppEvent::PaneDied {
                        pane_id: id,
                        exit_reason,
                    }) if id == pane_id => {
                        break crate::events::AppEvent::PaneDied {
                            pane_id: id,
                            exit_reason,
                        };
                    }
                    Some(_) => continue,
                    None => panic!("event channel closed"),
                }
            }
        })
        .await
        .expect("panel process should exit");
        app.handle_internal_event(event);

        assert!(matches!(
            panel(&app, "quick").status,
            PanelStatus::Exited(_)
        ));
        assert!(app.find_pane(pane_id).is_none());
    }

    fn api(app: &mut App, method: crate::api::schema::Method) -> serde_json::Value {
        let response = app.handle_api_request(crate::api::schema::Request {
            id: "r".into(),
            method,
        });
        serde_json::from_str(&response).unwrap()
    }

    #[tokio::test]
    async fn panel_list_reports_status_and_exit_code() {
        let (mut app, _) = test_app();
        app.reconcile_panels(&[entry("a", &["a"]), entry("b", &["b"])]);
        let pane_id = panel(&app, "b").pane_id;
        app.handle_panel_process_exit(pane_id, Some(2));

        let response = api(
            &mut app,
            crate::api::schema::Method::PanelList(crate::api::schema::EmptyParams::default()),
        );
        assert_eq!(response["result"]["type"], "panel_list");
        let panels = response["result"]["panels"].as_array().unwrap();
        assert_eq!(panels.len(), 2);
        assert_eq!(panels[0]["id"], "a");
        assert_eq!(panels[0]["status"], "running");
        assert!(panels[0].get("exit_code").is_none());
        assert_eq!(panels[1]["status"], "exited");
        assert_eq!(panels[1]["exit_code"], 2);
    }

    #[tokio::test]
    async fn panel_restart_api_restarts_exited_panel_and_rejects_unknown_id() {
        let (mut app, log) = test_app();
        app.reconcile_panels(&[entry("a", &["a"])]);
        let pane_id = panel(&app, "a").pane_id;
        app.handle_panel_process_exit(pane_id, Some(1));

        let response = api(
            &mut app,
            crate::api::schema::Method::PanelRestart(PanelRestartParams { id: "a".into() }),
        );
        assert_eq!(response["result"]["type"], "panel_restarted");
        assert_eq!(response["result"]["panel"]["status"], "running");
        assert_eq!(log.lock().unwrap().len(), 2);

        let response = api(
            &mut app,
            crate::api::schema::Method::PanelRestart(PanelRestartParams { id: "zzz".into() }),
        );
        assert_eq!(response["error"]["code"], "panel_not_found");
    }
}
