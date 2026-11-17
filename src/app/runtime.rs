use std::time::Instant;

#[cfg(test)]
use std::time::Duration;

use super::{
    background_update_check_enabled, App, AUTO_UPDATE_CHECK_INTERVAL, MIN_RENDER_INTERVAL,
};
fn retain_detached_process_after_wait(
    pid: u32,
    result: std::io::Result<Option<std::process::ExitStatus>>,
) -> bool {
    match result {
        Ok(None) => true,
        Ok(Some(_)) => false,
        Err(err) if err.kind() == std::io::ErrorKind::Interrupted => true,
        Err(err) => {
            tracing::warn!(pid, err = %err, "failed to reap detached process");
            false
        }
    }
}

/// Retired terminals finish process teardown away from the event loop. Keep
/// completion receivers so server exit still waits for every cleanup worker.
#[derive(Default)]
pub(super) struct TerminalShutdowns {
    pending: Vec<std::sync::mpsc::Receiver<()>>,
}

impl TerminalShutdowns {
    fn spawn(&mut self, shutdown: impl FnOnce() + Send + 'static) {
        self.pending
            .retain(|done| matches!(done.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
        let (done, finished) = std::sync::mpsc::channel::<()>();
        let spawned = crate::thread_spawn::spawn_named_with(
            "herdr-terminal-shutdown",
            (shutdown, done),
            |(shutdown, _done)| shutdown(),
        );
        match spawned {
            Ok(()) => self.pending.push(finished),
            Err(((shutdown, _done), err)) => {
                tracing::warn!(err = %err, "failed to start terminal cleanup worker; cleaning up inline");
                shutdown();
            }
        }
    }
}

impl Drop for TerminalShutdowns {
    fn drop(&mut self) {
        for done in self.pending.drain(..) {
            // Disconnect means the worker has finished (or unwound). Its sender
            // stays alive until shutdown has dropped the terminal runtime.
            let _ = done.recv();
        }
    }
}

impl App {
    pub(crate) fn reap_finished_detached_processes(&mut self) {
        self.detached_process_children
            .retain_mut(|child| retain_detached_process_after_wait(child.id(), child.try_wait()));
    }

    pub(crate) fn shutdown_terminal_runtime(&mut self, terminal_id: crate::terminal::TerminalId) {
        if let Some(runtime) = self.terminal_runtimes.remove(&terminal_id) {
            self.terminal_shutdowns.spawn(move || runtime.shutdown());
        }
    }

    pub(crate) fn shutdown_detached_terminal_runtimes(&mut self) {
        let terminal_ids = std::mem::take(&mut self.state.terminal_runtime_shutdowns);
        for terminal_id in terminal_ids {
            self.shutdown_terminal_runtime(terminal_id);
        }
    }

    pub(crate) fn sync_agent_metadata_deadline(&mut self) {
        self.agent_metadata_deadline = self.state.next_agent_metadata_expiry();
    }

    pub(crate) fn expire_due_metadata(&mut self, now: Instant) -> bool {
        let Some(deadline) = self
            .agent_metadata_deadline
            .filter(|deadline| now >= *deadline)
        else {
            return false;
        };
        self.expire_metadata_at(deadline, now);
        true
    }

    pub(crate) fn expire_metadata_at(&mut self, deadline: Instant, now: Instant) {
        let previous_toast = self.state.toast.clone();
        for update in self.state.expire_agent_metadata_at(deadline, now) {
            self.refresh_new_herdr_toast_context_for_update(&update, &previous_toast);
            self.emit_pane_state_update(&update);
        }
        let (panes, workspaces) = self.state.expire_metadata_tokens(now);
        for (ws_idx, pane_id) in panes {
            self.emit_pane_updated(ws_idx, pane_id);
        }
        for ws_idx in workspaces {
            self.emit_workspace_token_updated(ws_idx);
        }
        self.sync_agent_metadata_deadline();
    }

    pub(crate) fn can_render_now(&self, now: Instant) -> bool {
        match self.last_render_at {
            Some(last_render_at) => now.duration_since(last_render_at) >= MIN_RENDER_INTERVAL,
            None => true,
        }
    }

    pub(crate) fn can_present_now(&self, now: Instant) -> bool {
        match self.last_presentation_at {
            Some(last_presentation_at) => {
                now.duration_since(last_presentation_at) >= MIN_RENDER_INTERVAL
            }
            None => true,
        }
    }

    pub(crate) fn record_render_attempt(&mut self, now: Instant, presentation: bool) {
        self.last_render_at = Some(now);
        if presentation {
            self.last_presentation_at = Some(now);
        }
    }

    pub(crate) fn run_auto_update_check(&mut self) {
        if !background_update_check_enabled(
            self.policy.background_updates,
            self.update_version_check_enabled,
        ) {
            self.next_auto_update_check = None;
            return;
        }

        self.next_auto_update_check = self
            .state
            .update_available
            .is_none()
            .then_some(Instant::now() + AUTO_UPDATE_CHECK_INTERVAL);

        if self.state.update_available.is_some() {
            return;
        }

        let update_tx = self.event_tx.clone();
        if let Err(err) = crate::thread_spawn::spawn_named("herdr-update-check", move || {
            crate::update::auto_update(update_tx)
        }) {
            tracing::warn!(err = %err, "failed to spawn update check thread");
        }
    }

    pub(crate) fn run_agent_manifest_update_check(&mut self) {
        if !background_update_check_enabled(
            self.policy.background_updates,
            self.update_manifest_check_enabled,
        ) {
            self.next_agent_manifest_update_check = None;
            return;
        }

        self.next_agent_manifest_update_check = Some(Instant::now() + AUTO_UPDATE_CHECK_INTERVAL);

        let manifest_update_tx = self.event_tx.clone();
        if let Err(err) = crate::thread_spawn::spawn_named("herdr-manifest-check", move || {
            crate::detect::manifest_update::auto_update(manifest_update_tx)
        }) {
            tracing::warn!(err = %err, "failed to spawn agent manifest check thread");
        }
    }

    pub(crate) fn next_headless_loop_deadline_with_git_refresh(
        &self,
        now: Instant,
        needs_render: bool,
        include_git_refresh: bool,
    ) -> Option<Instant> {
        let render_deadline = if needs_render {
            self.last_render_at
                .map(|last_render_at| last_render_at + MIN_RENDER_INTERVAL)
                .filter(|deadline| *deadline > now)
        } else {
            None
        };

        [
            self.config_diagnostic_deadline,
            self.toast_deadline,
            self.state.next_pending_agent_notification_deadline(),
            self.state.next_managed_agent_deadline(),
            include_git_refresh
                .then(|| self.git_refresh_deadline())
                .flatten(),
            self.restored_worktree_validation_retry_at,
            self.next_auto_update_check,
            self.next_agent_manifest_update_check,
            self.agent_metadata_deadline,
            self.pending_agent_resume_deadline,
            self.session_save_deadline,
            self.next_tab_bar_status_deadline(),
            self.next_pull_request_deadline(),
            render_deadline,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    #[cfg(test)]
    pub(crate) fn drain_internal_events(&mut self) -> bool {
        self.drain_internal_events_up_to(super::APP_EVENT_DRAIN_LIMIT)
            .1
    }

    #[cfg(test)]
    pub(crate) fn drain_all_internal_events(&mut self) -> bool {
        let mut changed = false;
        loop {
            let (had_event, batch_changed) =
                self.drain_internal_events_up_to(super::APP_EVENT_DRAIN_LIMIT);
            changed |= batch_changed;
            if !had_event {
                break;
            }
        }
        changed
    }

    #[cfg(test)]
    fn drain_internal_events_up_to(&mut self, limit: usize) -> (bool, bool) {
        let mut had_event = false;
        let mut changed = false;
        for _ in 0..limit {
            let Ok(ev) = self.event_rx.try_recv() else {
                break;
            };
            had_event = true;
            changed |= self.handle_internal_event_with_render_impact(ev);
        }
        (had_event, changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;

    #[test]
    fn terminal_shutdown_runs_off_thread_and_owner_waits_for_cleanup() {
        let caller = std::thread::current().id();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let mut shutdowns = TerminalShutdowns::default();
        shutdowns.spawn(move || {
            started_tx.send(std::thread::current().id()).unwrap();
            release_rx.recv().unwrap();
            finished_tx.send(()).unwrap();
        });
        assert_ne!(
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            caller
        );
        assert!(finished_rx.try_recv().is_err());
        assert_eq!(shutdowns.pending.len(), 1);

        // Releasing cleanup from another thread lets owner destruction join it;
        // there is no performance threshold in this synchronization test.
        let release = std::thread::spawn(move || release_tx.send(()).unwrap());
        drop(shutdowns);
        finished_rx.try_recv().expect("owner waited for cleanup");
        release.join().unwrap();
    }

    #[test]
    fn terminal_shutdown_falls_back_inline_when_worker_cannot_start() {
        let caller = std::thread::current().id();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let mut shutdowns = TerminalShutdowns::default();
        crate::thread_spawn::test_hook::fail_next_spawns(1);
        shutdowns.spawn(move || done_tx.send(std::thread::current().id()).unwrap());
        assert_eq!(done_rx.try_recv().unwrap(), caller);
        assert!(shutdowns.pending.is_empty());
    }

    #[test]
    fn terminal_shutdown_reaps_finished_workers() {
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let mut shutdowns = TerminalShutdowns::default();
        shutdowns.spawn(move || drop(done_tx));
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap_err();
        // The worker's completion sender is released just after the closure's
        // captures, so use the same completion channel as owner destruction.
        shutdowns.pending[0]
            .recv_timeout(Duration::from_secs(5))
            .unwrap_err();
        shutdowns.spawn(|| {});
        assert_eq!(shutdowns.pending.len(), 1);
    }

    #[tokio::test]
    async fn terminal_shutdown_removes_runtime_and_closes_input() {
        let (mut app, _) = test_app_with_pane();
        let terminal_id = crate::terminal::TerminalId::alloc();
        let (runtime, mut input) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        app.terminal_runtimes.insert(terminal_id.clone(), runtime);
        app.shutdown_terminal_runtime(terminal_id.clone());
        assert!(app.terminal_runtimes.get(&terminal_id).is_none());
        assert!(tokio::time::timeout(Duration::from_secs(5), input.recv())
            .await
            .expect("cleanup closes terminal input")
            .is_none());
    }

    #[test]
    fn hidden_render_attempt_keeps_presentation_cadence_available() {
        let (mut app, _) = test_app_with_pane();
        let initial_presentation = Instant::now();
        app.record_render_attempt(initial_presentation, true);

        let hidden_attempt = initial_presentation + MIN_RENDER_INTERVAL;
        app.record_render_attempt(hidden_attempt, false);
        let foreground_echo = hidden_attempt + Duration::from_millis(1);

        assert!(!app.can_render_now(foreground_echo));
        assert!(app.can_present_now(foreground_echo));
    }

    #[test]
    fn interrupted_detached_process_wait_keeps_child_for_retry() {
        let interrupted = std::io::Error::new(std::io::ErrorKind::Interrupted, "test interrupt");

        assert!(retain_detached_process_after_wait(42, Err(interrupted)));
    }

    fn test_app_with_pane() -> (super::super::App, crate::layout::PaneId) {
        let mut app = super::super::App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            tokio::sync::mpsc::unbounded_channel().1,
            crate::api::EventHub::default(),
        );
        let ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        app.state.workspaces.push(ws);
        app.state.active = Some(0);
        app.state.view.pane_infos.push(crate::layout::PaneInfo {
            id: pane_id,
            rect: ratatui::layout::Rect::new(0, 0, 80, 24),
            inner_rect: ratatui::layout::Rect::new(0, 0, 80, 24),
            scrollbar_rect: None,
            borders: ratatui::widgets::Borders::NONE,
            is_focused: true,
        });
        (app, pane_id)
    }
}
