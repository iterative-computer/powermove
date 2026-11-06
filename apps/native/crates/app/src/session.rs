use crate::{panels, platform};
use gpui::{prelude::*, *};
use pm_engine_service::{EngineService, MediaCatalog, Viewer, ViewerConfig};
use pm_kernel::{
    session::*,
    state::{StateOp, StateTransaction},
    *,
};
use pm_protocol::{
    clock::{TransportCommand, TransportState},
    common::{PlaybackRate, TICKS_PER_SECOND, Ticks},
};
use pm_shell::{Command as ShellCommand, Host};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

#[derive(Clone)]
struct CompositionView {
    time: f64,
    timeline: pm_model::Value,
    viewer: pm_model::Value,
}

pub struct Controller {
    pub builtins: crate::builtins::Host,
    /// Per-composition navigation view, scoped to the live document identity.
    composition_views: std::collections::BTreeMap<(u64, String), CompositionView>,
    pub kernel: KernelHandle,
    pub engine: Arc<EngineService>,
    pub host: Rc<RefCell<Host>>,
    pub profile: PathBuf,
    /// The original app's profile, when this Mac has one. A file it might hold
    /// opens as a copy while it runs. The native library is never affected.
    pub electron: Option<PathBuf>,
    pub session: Option<SessionHandle>,
    pub published: Option<Arc<Published>>,
    pub mirror: Option<QueryMirror>,
    pub viewer: Option<Viewer>,
    pub media: MediaCatalog,
    pub opening: bool,
    pub rows: crate::rows::LayerRows,
    /// The last refresh saw a save in flight (drives the status note).
    pub saving_note: bool,
    pub output_mode: crate::viewer2::OutputMode,
    pub safe_areas: bool,
    pub guides: bool,
    pub popout: Option<crate::viewer2::Popout>,
    /// The editor viewer's last configured output (size, display, headroom).
    pub main_output: Rc<std::cell::Cell<Option<pm_engine_service::Output>>>,
    pub selected: Option<String>,
    pub message: String,
    pub path: Option<PathBuf>,
    pub recent: Vec<(String, String)>,
    pub library: Option<Entity<crate::project_library::ProjectLibrary>>,
    pub errors: crate::error_log::Shared,
    pub rate: pm_time::Rate,
    pub duration: f64,
    pub dimensions: [f64; 2],
    pub zoom: Option<f64>,
    /// Composition background (`bg`); `None` when transparent.
    pub background: Option<u32>,
    /// Keyframes in the open composition, for the status bar.
    pub keyframes: usize,
    pub recovery_pending: bool,
    pub mcp: Arc<crate::mcp::Host>,
    pub close_pending: bool,
    pub close_window: Option<gpui::AnyWindowHandle>,
    pub close_operation: Option<crate::mcp::Operation>,
    pub save_pending: bool,
    pub close_after_save: Option<crate::lifecycle::Intent>,
}

/// Why a project opened as a copy, in user language (never the enum name).
/// A banner with a lock badge replaces this status copy in P3.
pub fn copy_notice(decision: &pm_kernel::locks::OpenDecision) -> Option<&'static str> {
    use pm_kernel::locks::{CopyReason::*, OpenDecision};
    let OpenDecision::AsCopy(reason) = decision else {
        return None;
    };
    Some(match reason {
        ForeignLibrary => "Opened as a copy because the project belongs to another library.",
        ChangedOnDisk => "Opened as a copy because the file changed on disk.",
        OfflineFile => "Opened as a copy because the file isn’t downloaded.",
        CloudFolder => "Opened as a copy because the file is in a syncing folder.",
        ReadOnly => "Opened as a copy because the file is read-only.",
        OlderElectron => "Opened as a copy because an earlier version of Powermove saved it.",
        Locked => "Opened as a copy because the project is open somewhere else.",
        UncertainLock => {
            "Opened as a copy because Powermove couldn’t confirm the project isn’t open elsewhere."
        }
    })
}

pub struct Bridge(pub Entity<Controller>);
impl pm_shell::integration::Integration for Bridge {
    fn observe(&self, cx: &mut gpui::Context<pm_shell::view::Editor>) {
        cx.observe(&self.0, |_, _, cx| cx.notify()).detach();
    }
    fn command(&self, cmd: &ShellCommand, w: &mut Window, cx: &mut App) -> bool {
        self.0.update(cx, |v, cx| v.command(cmd, w, cx))
    }
    fn panel(
        &self,
        id: &str,
        bounds: pm_shell::layout::Rect,
        w: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.0.update(cx, |v, cx| {
            panels::library(v, id, bounds, cx).or_else(|| v.builtin_panel(id, bounds, w, cx))
        })
    }
    fn header(
        &self,
        id: &str,
        bounds: pm_shell::layout::Rect,
        w: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.0
            .update(cx, |v, cx| v.builtin_header(id, bounds, w, cx))
    }
    fn project_id(&self, cx: &App) -> Option<String> {
        self.0.read(cx).session.as_ref().map(|s| s.project.clone())
    }
    fn status(&self, cx: &App) -> String {
        self.0.read(cx).status_text()
    }
    fn drop_paths(&self, paths: Vec<PathBuf>, cx: &mut App) {
        if let Some(path) = paths
            .into_iter()
            .find(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("pmv" | "json")))
        {
            self.0.update(cx, |v, cx| v.open_file(path, cx));
        }
    }
}
impl Controller {
    /// Status bar copy (language §14): a pending message or error, else
    /// `Saved`/`Edited` with layer and keyframe counts.
    pub fn status_text(&self) -> String {
        // "Saved" is the state word the computed line already starts with.
        if (!self.message.is_empty() && self.message != "Saved") || self.session.is_none() {
            return self.message.clone();
        }
        let dirty = self.published.as_ref().is_some_and(|p| p.dirty);
        panels::status_line(
            dirty,
            self.path.is_some(),
            self.layers().len(),
            self.keyframes,
        )
    }
    pub fn new(
        kernel: KernelHandle,
        engine: Arc<EngineService>,
        profile: PathBuf,
        electron: Option<PathBuf>,
        mcp: Arc<crate::mcp::Host>,
        host: Rc<RefCell<Host>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let events = kernel.subscribe();
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            while let Ok(e) = events.recv() {
                if tx.send_blocking(e).is_err() {
                    break;
                }
            }
        });
        cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                // Coalesce a burst (an MCP run publishes several revisions):
                // one refresh reads the newest publication for all of them.
                let mut events = vec![first];
                while let Ok(e) = rx.try_recv() {
                    events.push(e);
                }
                if this
                    .update(cx, |v, cx| {
                        for e in events {
                            match e {
                                KernelEvent::Durability { message } => {
                                    v.message = format!("Recovery warning: {message}");
                                    v.errors.borrow_mut().push("Recovery", &message);
                                }
                                KernelEvent::Notice { notice, .. } => {
                                    v.message = format!("Kernel notice: {notice:?}")
                                }
                                _ => {}
                            }
                        }
                        v.refresh(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        for (chord, command) in [
            ("j", "shuttleJ"),
            ("k", "shuttleK"),
            ("l", "shuttleL"),
            ("ctrl+j", "shuttleReverse"),
        ] {
            host.borrow_mut()
                .add_bindings([pm_shell::keyboard::Binding {
                    id: format!("app.{command}"),
                    chord: chord.into(),
                    command: command.into(),
                    args: vec![],
                    priority: 200,
                    in_fields: false,
                    repeat: false,
                    loose: false,
                }]);
        }
        let layer = host.borrow().viewer_layer();
        pm_engine_service::layers::configure_layer(
            objc2::MainThreadMarker::new().unwrap(),
            &layer,
            engine.metal_device(),
        )
        .expect("viewer layer");
        host.borrow().set_viewer_label("Composition viewer");
        Self {
            builtins: Default::default(),
            composition_views: Default::default(),
            kernel,
            engine,
            profile,
            electron,
            host,
            session: None,
            published: None,
            mirror: None,
            viewer: None,
            media: MediaCatalog::default(),
            opening: false,
            rows: Default::default(),
            saving_note: false,
            output_mode: crate::viewer2::OutputMode::from_env(),
            safe_areas: false,
            guides: false,
            popout: None,
            main_output: Default::default(),
            selected: None,
            // The status bar holds facts, not instructions (review shell 2 #11).
            message: "No project".into(),
            path: None,
            recent: vec![],
            library: None,
            errors: crate::error_log::shared(cx),
            rate: pm_time::Rate::FPS_30,
            duration: 1.,
            dimensions: [1920., 1080.],
            zoom: None,
            background: None,
            keyframes: 0,
            recovery_pending: false,
            mcp,
            close_pending: false,
            close_window: None,
            close_operation: None,
            save_pending: false,
            close_after_save: None,
        }
    }
    pub fn initialize(&mut self, launch: Option<PathBuf>, cx: &mut Context<Self>) {
        self.load_recent(cx);
        if let Some(path) = launch {
            self.open_file(path, cx);
        }
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let _span = crate::diag::span("refresh");
        let Some(session) = &self.session else {
            cx.notify();
            return;
        };
        let Some(mut p) = self.kernel.published(&session.project) else {
            return;
        };
        let switched = self.published.as_ref().is_some_and(|old| {
            old.snapshot.doc_id == p.snapshot.doc_id
                && old.snapshot.open_comp() != p.snapshot.open_comp()
        });
        if switched {
            self.host.borrow_mut().close_flowchart();
            if let Some(old) = &self.published
                && let Some(id) = old.snapshot.open_comp()
            {
                self.composition_views.insert(
                    (old.snapshot.doc_id, id),
                    CompositionView {
                        time: self.time(),
                        timeline: old.ui_state.timeline.clone(),
                        viewer: old.ui_state.viewer.clone(),
                    },
                );
            }
            let key = (
                p.snapshot.doc_id,
                p.snapshot.open_comp().unwrap_or_default(),
            );
            let saved = self.composition_views.get(&key).cloned();
            let mut next = (*p).clone();
            let time = if let Some(saved) = saved {
                next.ui_state.timeline = saved.timeline;
                next.ui_state.viewer = saved.viewer;
                saved.time
            } else {
                0.
            };
            self.zoom = pm_panel_viewerchrome::Chrome::read(&next.ui_state)
                .ok()
                .and_then(|c| (!c.navigation.fit).then_some(c.navigation.zoom));
            p = Arc::new(next);
            self.published = Some(p.clone());
            self.bind_composition();
            self.apply_viewer_options();
            self.rebuild_popout_viewer();
            self.seek(time);
        }
        if let Some(viewer) = &self.viewer {
            let _span = crate::diag::span("refresh.publish");
            if let Err(e) = viewer.publish(&p) {
                self.message = e.to_string();
            }
        }
        if let Some(popout) = &self.popout
            && let Some(viewer) = &*popout.viewer.borrow()
            && let Err(e) = viewer.publish(&p)
        {
            self.message = format!("Popout viewer: {e}");
        }
        {
            let _span = crate::diag::span("refresh.mirror");
            if let Some(m) = &mut self.mirror {
                m.sync(&p);
            } else {
                self.mirror = Some(QueryMirror::new(&p));
            }
        }
        self.sync_rows(&p, cx);
        let saving = p.saving || self.save_pending;
        if let Some(note) = save_note(self.saving_note, saving, p.dirty, &self.message) {
            self.message = note.into();
        }
        self.saving_note = saving;
        self.selected = p.selection.primary().map(str::to_owned);
        if let Ok(chrome) = pm_panel_viewerchrome::Chrome::read(&p.ui_state) {
            self.zoom = (!chrome.navigation.fit).then_some(chrome.navigation.zoom);
        }
        self.path = p.session.as_ref().and_then(|s| match &s.destination {
            SessionDestination::File(path) => Some(path.clone()),
            _ => None,
        });
        let title = p
            .snapshot
            .document
            .root()
            .and_then(|r| r.get("name"))
            .and_then(crate::node_string)
            .unwrap_or("Untitled");
        // `Saving…` under the title while a save runs, then `Edited` or nothing.
        self.host.borrow().set_document_metadata(
            title,
            pm_shell::document_subtitle(p.dirty, saving),
            p.dirty,
            self.path.as_deref(),
        );
        platform::history_labels(
            p.history
                .undo_label
                .as_deref()
                .filter(|_| p.history.can_undo),
            p.history
                .redo_label
                .as_deref()
                .filter(|_| p.history.can_redo),
        );
        self.published = Some(p);
        self.update_builtins(cx);
        cx.notify();
    }
    pub fn layers(&self) -> &[LayerInfo] {
        &self.rows.rows
    }
    /// Rows follow the newest publication; large lists are built on a worker.
    fn sync_rows(&mut self, p: &Published, cx: &mut Context<Self>) {
        use crate::rows::{self, Plan};
        let key = rows::key(&p.snapshot);
        match self.rows.plan(key, rows::layer_count(&p.snapshot)) {
            Plan::Current => {}
            Plan::Inline => {
                let _span = crate::diag::span("refresh.rows");
                if self.rows.accept(key, rows::list(&p.snapshot), Some(key)) {
                    self.keyframes = rows::keyframe_count(&p.snapshot);
                }
            }
            Plan::Worker => {
                let snapshot = p.snapshot.clone();
                cx.spawn(async move |this, cx| {
                    let list = cx
                        .background_executor()
                        .spawn(
                            async move { (rows::list(&snapshot), rows::keyframe_count(&snapshot)) },
                        )
                        .await;
                    let _ = this.update(cx, |v, cx| {
                        let latest = v.published.as_ref().map(|p| rows::key(&p.snapshot));
                        let (list, keyframes) = list;
                        if v.rows.finish(key, list, latest) {
                            v.keyframes = keyframes;
                            cx.notify();
                        }
                        if let Some(p) = v.published.clone() {
                            v.sync_rows(&p, cx);
                        }
                    });
                })
                .detach();
            }
        }
    }
    pub fn call(&mut self, call: Call, cx: &mut Context<Self>) {
        self.call_at(call, Self::event_time(), cx);
    }
    pub fn call_at(&mut self, call: Call, input: f64, cx: &mut Context<Self>) {
        if self.close_pending {
            return;
        }
        let Some(session) = &self.session else { return };
        if matches!(
            call,
            Call::Transact { .. } | Call::Gesture(GestureCall::Step { .. })
        ) && let Some(viewer) = &self.viewer
        {
            viewer.input_event(input);
        }
        let pending = self
            .kernel
            .call(&session.project, Request::new(Caller::native(), call));
        cx.spawn(async move |this, cx| {
            let result = pending.await;
            let _ = this.update(cx, |v, cx| {
                match result {
                    Some(Ok(ack)) => {
                        if let Reply::Throw(e) = ack.reply {
                            v.message = e.to_string();
                        }
                    }
                    other => v.message = format!("Edit failed: {other:?}"),
                }
                v.refresh(cx);
            });
        })
        .detach();
    }
    pub fn time(&self) -> f64 {
        self.viewer
            .as_ref()
            .and_then(|v| v.status().clock)
            .map_or(0., |c| c.media_time.get() as f64 / TICKS_PER_SECOND as f64)
    }
    fn event_time() -> f64 {
        objc2_app_kit::NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap())
            .currentEvent()
            .map(|e| e.timestamp())
            .filter(|t| t.is_finite() && *t > 0.)
            .unwrap_or_else(pm_engine_service::trace::now)
    }
    pub fn seek(&mut self, seconds: f64) {
        self.seek_at(seconds, Self::event_time());
    }
    pub fn seek_at(&mut self, seconds: f64, input: f64) {
        if let Some(v) = &self.viewer {
            let frame = self
                .rate
                .frame_of((seconds.clamp(0., self.duration) * TICKS_PER_SECOND as f64) as i64);
            if let Ok(time) = self.rate.frame_start(frame)
                && let Err(e) = v.scrub_at(Ticks::new(time).unwrap_or(Ticks::ZERO), input)
            {
                self.message = e.to_string();
            }
        }
        self.persist_ui();
    }
    pub fn persist_ui(&self) {
        if let (Some(s), Some(p)) = (&self.session, &self.published) {
            let mut ui = p.ui_state.clone();
            if let pm_model::Value::Object(m) = &mut ui.transport {
                m.insert("time", pm_model::Value::from(self.time()));
                m.insert(
                    "playing",
                    pm_model::Value::from(
                        self.viewer
                            .as_ref()
                            .and_then(|v| v.status().clock)
                            .is_some_and(|c| c.state != TransportState::Stopped),
                    ),
                );
            }
            if let pm_model::Value::Object(m) = &mut ui.viewer {
                m.insert("zoom", pm_model::Value::from(self.zoom.unwrap_or(1.)));
                m.insert("fit", pm_model::Value::from(self.zoom.is_none()));
            }
            let _pending = self.kernel.state_transaction(StateTransaction {
                ops: vec![StateOp::Ui {
                    project: s.project.clone(),
                    next: ui,
                }],
                ..Default::default()
            });
        }
    }
    pub fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.open(SessionSource::File(path), cx);
    }
    pub fn open_library(&mut self, id: String, cx: &mut Context<Self>) {
        self.open(
            SessionSource::Library {
                profile: self.profile.clone(),
                project: id,
            },
            cx,
        );
    }
    fn open(&mut self, source: SessionSource, cx: &mut Context<Self>) {
        if self.opening || self.recovery_pending || self.close_pending || self.save_pending {
            self.message = "Finish the active edit or recovery decision first".into();
            return;
        }
        let Some(operation) = self.mcp.try_operation() else {
            self.message =
                "A project operation is in progress. Try opening again when it finishes.".into();
            cx.notify();
            return;
        };
        let open_id = match &source {
            SessionSource::Library { project, .. } => project.clone(),
            SessionSource::File(p) => stable_id(p),
            SessionSource::New(_) => pm_kernel::new_id("app"),
        };
        let name = match &source {
            SessionSource::File(path) => path
                .file_stem()
                .map_or("Untitled".into(), |n| n.to_string_lossy().into_owned()),
            SessionSource::Library { project, .. } => self
                .recent
                .iter()
                .find(|(id, _)| id == project)
                .map_or("Untitled".into(), |(_, name)| name.clone()),
            SessionSource::New(_) => "Untitled".into(),
        };
        let path = match &source {
            SessionSource::File(path) => Some(path.clone()),
            _ => None,
        };
        let electron_profile = path.is_some().then(|| self.electron.clone()).flatten();
        let pending = self.kernel.shell(ShellCall::PrepareOpen {
            id: open_id,
            source,
            electron_profile,
        });
        self.settle_open(operation, name, path, pending, cx);
    }
    /// A project the Library's worker already opened.
    pub fn adopt_reply(
        &mut self,
        reply: ShellReply,
        name: String,
        file: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let Some(operation) = self.mcp.try_operation() else {
            self.message =
                "A project operation is in progress. Try opening again when it finishes.".into();
            cx.notify();
            // Nothing will adopt this session, so it can't stay open in the kernel.
            if let ShellReply::Opened { session, .. } = reply {
                let closing = self.kernel.shell(ShellCall::Close(session));
                cx.spawn(async move |_, _| drop(closing.await)).detach();
            }
            return;
        };
        self.settle_open(operation, name, file, async move { Some(Ok(reply)) }, cx);
    }
    fn settle_open(
        &mut self,
        operation: crate::mcp::Operation,
        name: String,
        path: Option<PathBuf>,
        pending: impl std::future::Future<Output = Option<Result<ShellReply, KernelFailure>>> + 'static,
        cx: &mut Context<Self>,
    ) {
        // Switching is explicitly saved through the session API; never drop dirty work.
        let old = self.session.clone();
        let kernel = self.kernel.clone();
        let cache = self
            .profile
            .join("media-cache")
            .join(pm_kernel::new_id("app"));
        self.opening = true;
        self.message = "Opening…".into();
        cx.spawn(async move |this, cx| {
            let _operation = operation;
            let mut result = pending.await;
            if let Some(Ok(reply @ ShellReply::RecoveryRequired { .. })) = &result {
                let saved = path.as_ref().is_none_or(|p| p.exists());
                let recovery = crate::surfaces::recovery_kernel::PreparedRecovery::from_reply(
                    reply, &name, false, saved,
                );
                if let Some(recovery) = recovery {
                    use crate::surfaces::recovery::{Owner, Prompt, RecoverySurface};
                    if let RecoverySurface::Alert(alert) = recovery.surface.clone() {
                        let prompt = Prompt {
                            id: pm_kernel::new_id("recovery"),
                            revision: 0,
                            owner: Owner::Window(name.clone()),
                            alert,
                        };
                        let answer = this.update(cx, |v, _| {
                            if matches!(
                                reply,
                                ShellReply::RecoveryRequired {
                                    recovery: RecoveryResult::Unrecoverable { .. },
                                    ..
                                }
                            ) {
                                v.errors.borrow_mut().push("Project", &prompt.alert.message);
                            }
                            v.recovery_pending = true;
                            crate::surfaces::recovery_macos::sheet(
                                &prompt,
                                &v.host.borrow().window,
                            )
                        });
                        let response = match answer {
                            Ok(rx) => rx.recv().await.ok(),
                            Err(_) => None,
                        };
                        let call = response
                            .and_then(|r| r.action)
                            .and_then(|action| recovery.decision(action));
                        if let Some(call) = call {
                            result = kernel.shell(call).await;
                        }
                    }
                }
            }
            let media = if let Some(Ok(ShellReply::Opened { session, .. })) = &result {
                let published = kernel.published(&session.project).unwrap();
                cx.background_executor()
                    .spawn(async move {
                        crate::media::for_session(
                            &published.snapshot,
                            &published.media,
                            published.path.as_deref(),
                            &cache,
                        )
                    })
                    .await
                    .map(Some)
            } else {
                Ok(None)
            };
            let mut previous_save_failed = false;
            if matches!(&result, Some(Ok(ShellReply::Opened { .. })))
                && let Some(old) = old
            {
                previous_save_failed = !matches!(
                    kernel.shell(ShellCall::Close(old)).await,
                    Some(Ok(ShellReply::Closed { .. }))
                );
            }
            let _ = this.update(cx, |v, cx| {
                v.opening = false;
                v.recovery_pending = false;
                match result {
                    Some(Ok(ShellReply::Opened {
                        session,
                        recovery: _,
                        decision,
                        info,
                    })) => {
                        let media = match media {
                            Ok(m) => m,
                            Err(e) => {
                                v.message = format!("Media could not be resolved: {e}");
                                None
                            }
                        };
                        v.adopt(session, decision, info.history_rejected, media, cx);
                        if previous_save_failed {
                            v.message.push_str(
                                " · Previous project could not be saved; its recovery journal is retained",
                            );
                        }
                        v.load_recent(cx);
                    }
                    Some(Err(error)) => v.report_open_failure(&name, path, &error, cx),
                    Some(Ok(ShellReply::RecoveryRequired { .. })) => {
                        v.message = format!("“{name}” wasn’t opened. Its recovery data is kept.");
                    }
                    other => v.message = format!("Open failed: {other:?}"),
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// S004: an alert naming the file, with Show in Finder when it still exists.
    fn report_open_failure(
        &mut self,
        name: &str,
        path: Option<PathBuf>,
        error: &KernelFailure,
        cx: &mut Context<Self>,
    ) {
        use crate::surfaces::documents::{self, Action};
        let exists = path.as_ref().is_some_and(|p| p.exists());
        let alert = documents::open_error(name, open_failure_code(error), exists);
        self.errors.borrow_mut().push("Project", &alert.message);
        self.message = String::new();
        let labels: Vec<_> = alert.buttons.iter().map(|b| b.label).collect();
        let rx = platform::choice(
            &self.host.borrow().window,
            &alert.message,
            &alert.informative,
            &labels,
        );
        cx.spawn(async move |_, _| {
            let choice = rx.recv().await.ok();
            if alert.action_at(choice) == Some(Action::ShowInFinder)
                && let Some(path) = path
            {
                platform::reveal(&path);
            }
        })
        .detach();
    }
    fn adopt(
        &mut self,
        session: SessionHandle,
        decision: pm_kernel::locks::OpenDecision,
        history_rejected: bool,
        media: Option<MediaCatalog>,
        cx: &mut Context<Self>,
    ) {
        if let Some(viewer) = self.viewer.take() {
            viewer.retire();
        }
        self.host.borrow_mut().close_flowchart();
        self.composition_views.clear();
        self.session = Some(session);
        self.mirror = None;
        self.selected = None;
        self.refresh(cx);
        self.media = media.unwrap_or_default();
        // The status bar shows document state and counts; only exceptions get words.
        let notice = self.errors.borrow_mut().take_notice();
        self.message = [
            copy_notice(&decision),
            history_rejected.then_some("Undo history from the last session couldn’t be restored."),
            notice.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        self.bind_composition();
        self.apply_viewer_options();
        self.rebuild_popout_viewer();
        if let Some(ui) = self.published.as_ref().map(|p| p.ui_state.clone()) {
            let time = ui
                .transport
                .get("time")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.);
            self.seek(time);
        }
        self.mcp.bind(self.session.clone());
    }
    fn bind_composition(&mut self) {
        if let Some(viewer) = self.viewer.take() {
            viewer.retire();
        }
        let Some(p) = self.published.clone() else {
            return;
        };
        let comp = p.snapshot.open_comp().unwrap_or_default();
        let root = p.snapshot.document.root().unwrap();
        let composition = pm_kernel::model::comp_node(root, Some(&comp)).unwrap_or(root);
        let number = |key: &str, default| {
            composition
                .get(key)
                .and_then(crate::node_number)
                .unwrap_or(default)
        };
        self.duration = number("dur", 1.).max(0.001);
        self.dimensions = [number("w", 1920.), number("h", 1080.)];
        let fps = number("fps", 30.);
        self.rate =
            pm_time::Rate::new((fps * 1000.).round() as i64, 1000).unwrap_or(pm_time::Rate::FPS_30);
        let media = self.media.clone();
        self.background = composition
            .get("bg")
            .and_then(crate::node_string)
            .and_then(panels::parse_background);
        match self.engine.viewer(ViewerConfig {
            snapshot: p.snapshot.clone(),
            composition: comp,
            media,
            output: pm_engine_service::Output {
                width: 640,
                height: 360,
                display: 0,
                headroom: 1.,
            },
            layer: Some(self.host.borrow().viewer_layer()),
            rate: self.rate,
            end: Some((self.duration * TICKS_PER_SECOND as f64) as i64),
            audio: true,
        }) {
            Ok(viewer) => self.viewer = Some(viewer),
            Err(e) => self.message = format!("Viewer failed: {e}"),
        }
    }
    pub fn follow_mcp(&mut self, cx: &mut Context<Self>) {
        let current = self.mcp.current();
        if self.opening || self.close_pending || current == self.session {
            return;
        }
        let Some(session) = current else {
            return;
        };
        let Some(p) = self.kernel.published(&session.project) else {
            return;
        };
        let cache = self
            .profile
            .join("media-cache")
            .join(pm_kernel::new_id("app"));
        self.opening = true;
        cx.spawn(async move |this, cx| {
            let history_rejected = false;
            let media = cx
                .background_executor()
                .spawn(async move {
                    crate::media::for_session(&p.snapshot, &p.media, p.path.as_deref(), &cache)
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                v.opening = false;
                if v.mcp.current().as_ref() == Some(&session) {
                    match media {
                        Ok(media) => v.adopt(
                            session,
                            pm_kernel::locks::OpenDecision::Writable,
                            history_rejected,
                            Some(media),
                            cx,
                        ),
                        Err(e) => v.message = format!("Media could not be resolved: {e}"),
                    }
                    cx.notify();
                }
                v.follow_mcp(cx);
            });
        })
        .detach();
    }
    pub fn load_recent(&mut self, cx: &mut Context<Self>) {
        let pending = self.kernel.shell(ShellCall::Recent);
        cx.spawn(async move |this, cx| {
            if let Some(Ok(ShellReply::Recent(rows))) = pending.await {
                let _ = this.update(cx, |v, cx| {
                    v.recent = rows
                        .iter()
                        .filter_map(|r| {
                            Some((
                                r.get("id")?.as_str()?.into(),
                                r.get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Untitled")
                                    .into(),
                            ))
                        })
                        .collect();
                    cx.notify();
                });
            }
        })
        .detach();
    }
    pub fn save(&mut self, as_path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.recovery_pending || self.save_pending {
            self.message = "Finish the recovery decision or current save first".into();
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.save_pending = true;
        self.saving_note = true;
        self.message = SAVING.into();
        cx.notify();
        self.persist_ui();
        let call = as_path.clone().map_or_else(
            || ShellCall::Save(session.clone()),
            |path| ShellCall::SaveAs {
                session: session.clone(),
                path,
            },
        );
        let kernel = self.kernel.clone();
        let file_save = as_path.is_some()
            || self.published.as_ref().is_some_and(|p| {
                p.session
                    .as_ref()
                    .is_some_and(|s| matches!(s.destination, SessionDestination::File(_)))
            });
        cx.spawn(async move |this, cx| {
            let result = if file_save {
                match kernel.prepare_save(session, as_path.clone()).await {
                    Some(Ok(save)) => {
                        let completion = cx
                            .background_executor()
                            .spawn(async move { save.run() })
                            .await;
                        kernel.finish_save(completion).await
                    }
                    Some(Err(e)) => Some(Err(e)),
                    None => None,
                }
            } else {
                kernel.shell(call).await
            };
            let _ = this.update(cx, |v, cx| {
                v.save_pending = false;
                let close = v.close_after_save.take();
                let saved = matches!(
                    &result,
                    Some(Ok(ShellReply::Saved {
                        recovery_copy: None,
                        ..
                    }))
                );
                match result {
                    Some(Ok(ShellReply::Saved {
                        session,
                        recovery_copy,
                    })) => {
                        v.session = Some(session);
                        if as_path.is_some() {
                            v.path = as_path;
                        }
                        v.message = recovery_copy.map_or(String::new(), |_| {
                            "Saved to a recovery copy; original was not overwritten".into()
                        });
                        v.load_recent(cx);
                    }
                    other => v.message = format!("Save failed; recovery data retained: {other:?}"),
                }
                v.refresh(cx);
                if let Some(intent) = close {
                    if saved {
                        v.complete_close(intent, cx);
                    } else {
                        v.cancel_close(cx);
                    }
                }
            });
        })
        .detach();
    }
    pub fn save_dialog(&mut self, cx: &mut Context<Self>) {
        let rx = platform::paths(&self.host.borrow().window, true);
        cx.spawn(async move |this, cx| {
            if let Ok(Some(path)) = rx.recv().await {
                let _ = this.update(cx, |v, cx| v.save(Some(path), cx));
            }
        })
        .detach();
    }
    pub fn command(
        &mut self,
        cmd: &ShellCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match cmd.name.as_str() {
            "v4.platform" => self.builtin_platform(window, cx),
            "settings" | "agent" => self.show_builtin(&cmd.name, window, cx),
            "quit" => cx.defer(crate::lifecycle::begin_quit),
            "closeWindow" | "closeTab" => self.request_close(crate::lifecycle::Intent::Window, cx),
            "open" => {
                let rx = platform::paths(&self.host.borrow().window, false);
                cx.spawn(async move |this, cx| {
                    if let Ok(Some(path)) = rx.recv().await {
                        let _ = this.update(cx, |v, cx| v.open_file(path, cx));
                    }
                })
                .detach();
            }
            "save" => self.save(None, cx),
            "saveAs" => self.save_dialog(cx),
            "export" => self.show_builtin("export", window, cx),
            "undo" | "contextUndo" => self.call(Call::History(HistoryCall::Undo), cx),
            "redo" | "contextRedo" => self.call(Call::History(HistoryCall::Redo), cx),
            "composition.flowchart" => {
                let mut host = self.host.borrow_mut();
                if cmd.token.is_some()
                    && !host.composition_canvas_focused()
                    && !host.flowchart_open()
                {
                    return false;
                }
                let Some(published) = self.published.as_ref() else {
                    return false;
                };
                let Some(chart) = pm_kernel::navigation::Flowchart::read(&published.snapshot)
                else {
                    return false;
                };
                use pm_shell::FlowchartRow;
                let mut rows = vec![FlowchartRow::Heading("Used In".into())];
                let mut group = |items: Vec<pm_kernel::navigation::Composition>, empty: &str| {
                    if items.is_empty() {
                        rows.push(FlowchartRow::Empty(empty.into()));
                    }
                    for item in items {
                        rows.push(FlowchartRow::Composition {
                            id: item.id,
                            name: item.name,
                            current: false,
                        });
                    }
                };
                group(chart.parents, "Not nested anywhere");
                rows.push(FlowchartRow::Composition {
                    id: chart.current.id,
                    name: chart.current.name,
                    current: true,
                });
                rows.push(FlowchartRow::Heading("Contains".into()));
                if chart.children.is_empty() {
                    rows.push(FlowchartRow::Empty("No nested compositions".into()));
                }
                for child in chart.children {
                    rows.push(FlowchartRow::Composition {
                        id: child.id,
                        name: child.name,
                        current: false,
                    });
                }
                return host.toggle_flowchart(rows, published.snapshot.doc_id);
            }
            "composition.open" => {
                let Some(id) = cmd.args.first().and_then(|a| a.as_str()) else {
                    return false;
                };
                let Some(document) = cmd.args.get(1).and_then(|a| a.as_u64()) else {
                    return false;
                };
                if self
                    .published
                    .as_ref()
                    .is_none_or(|p| p.snapshot.doc_id != document)
                {
                    return false;
                }
                if self
                    .published
                    .as_ref()
                    .and_then(|p| p.snapshot.open_comp())
                    .as_deref()
                    == Some(id)
                {
                    return true;
                }
                if let Some(viewer) = &self.viewer {
                    let _ = viewer.command(TransportCommand::Pause);
                }
                self.call(
                    Call::OpenComposition {
                        comp_id: id.into(),
                        document,
                    },
                    cx,
                );
            }
            "toggleLayerStrips" => {
                let Some(runtime) = self.builtins.runtime.clone() else {
                    return false;
                };
                let Ok(ticket) = runtime.reveal_masks() else {
                    return false;
                };
                cx.spawn(async move |this, cx| {
                    let result = ticket.await;
                    let _ = this.update(cx, |v, cx| {
                        if !v
                            .builtins
                            .runtime
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &runtime))
                        {
                            return;
                        }
                        if let Err(error) = result {
                            v.message = format!("Mask properties could not be shown: {error}");
                        }
                        v.drain_builtins(cx);
                    });
                })
                .detach();
            }
            "fit" | "fitComp" | "viewer.fit" => {
                self.zoom = None;
                self.persist_ui();
            }
            "zoomIn" => {
                self.zoom = Some(self.zoom.unwrap_or(1.) * 1.25);
                self.persist_ui();
            }
            "zoomOut" => {
                self.zoom = Some(self.zoom.unwrap_or(1.) / 1.25);
                self.persist_ui();
            }
            "play" | "prevFrame" | "nextFrame" | "stepFrames" | "gotoStart" | "gotoEnd"
            | "gotoWorkIn" | "gotoWorkOut" | "shuttleJ" | "shuttleK" | "shuttleL"
            | "shuttleReverse" | "prevVisibleEvent" | "nextVisibleEvent" => {
                let Some(v) = &self.viewer else { return false };
                let command = match cmd.name.as_str() {
                    "play" => {
                        if v.status()
                            .clock
                            .is_some_and(|c| c.state != TransportState::Stopped)
                        {
                            TransportCommand::Pause
                        } else {
                            TransportCommand::Play {
                                rate: PlaybackRate::NORMAL,
                            }
                        }
                    }
                    "prevFrame" => TransportCommand::Step { frames: -1 },
                    "nextFrame" => TransportCommand::Step { frames: 1 },
                    "stepFrames" => TransportCommand::Step {
                        frames: cmd.args.first().and_then(|v| v.as_i64()).unwrap_or(1) as i32,
                    },
                    "gotoStart" | "gotoWorkIn" => {
                        self.seek(0.);
                        return true;
                    }
                    "gotoEnd" | "gotoWorkOut" => {
                        self.seek((self.duration - 1. / self.rate.to_f64()).max(0.));
                        return true;
                    }
                    "prevVisibleEvent" | "nextVisibleEvent" => {
                        let time = self.time();
                        let mut times: Vec<_> = self
                            .layers()
                            .iter()
                            .flat_map(|l| [l.in_time, l.out_time])
                            .collect();
                        times.sort_by(f64::total_cmp);
                        let t = if cmd.name == "prevVisibleEvent" {
                            times.into_iter().rev().find(|t| *t < time - 0.00001)
                        } else {
                            times.into_iter().find(|t| *t > time + 0.00001)
                        };
                        if let Some(t) = t {
                            self.seek(t);
                        }
                        return true;
                    }
                    name => {
                        use pm_engine_service::transport::ShuttleKey::*;
                        let key = if name == "shuttleK" {
                            K
                        } else if name == "shuttleL" {
                            L
                        } else {
                            J
                        };
                        return v.shuttle(key, name == "shuttleReverse").unwrap_or(false);
                    }
                };
                if let Err(e) = v.command(command) {
                    self.message = e.to_string();
                }
            }
            name => return self.surface_command(name, cx),
        }
        cx.notify();
        true
    }
}
/// The wire code the open alert's copy is keyed on.
fn open_failure_code(error: &KernelFailure) -> &'static str {
    let text = format!("{error:?}").to_lowercase();
    if text.contains("permission") {
        "permissionDenied"
    } else if text.contains("not found") || text.contains("no such file") {
        "notFound"
    } else {
        "invalidData"
    }
}
pub const SAVING: &str = "Saving…";
/// Status note for a save in flight, including saves started by MCP or the
/// kernel. Returns a replacement message on a transition only, so other
/// notes (refusals, errors) set during the save are kept until it ends.
pub fn save_note(was: bool, now: bool, dirty: bool, message: &str) -> Option<&'static str> {
    // Close/quit refusals explain why the window stayed open; keep them.
    let refusal = message.starts_with("Saving in progress")
        || message.starts_with("A project operation is in progress");
    match (was, now) {
        (false, true) if !refusal => Some(SAVING),
        (true, false) if message == SAVING || refusal => Some(if dirty {
            "Save did not complete; recovery data retained"
        } else {
            "Saved"
        }),
        _ => None,
    }
}
pub(crate) fn stable_id(path: &std::path::Path) -> String {
    // Stable across crashes; no random per-open journal namespace.
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .hash(&mut h);
    format!("file-{:016x}", h.finish())
}
impl Drop for Controller {
    fn drop(&mut self) {
        if let Some(v) = &self.viewer {
            v.retire();
        }
        if let Some(p) = self.popout.take() {
            p.shutdown();
        }
    }
}
#[cfg(test)]
mod save_note_tests {
    use super::*;
    // `gpui::*` above exports a `test` macro; name the built-in explicitly.
    #[::core::prelude::v1::test]
    fn save_note_shows_progress_keeps_refusals_and_reports_the_outcome() {
        assert_eq!(save_note(false, true, true, "Opened"), Some(SAVING));
        assert_eq!(save_note(true, true, true, SAVING), None);
        let refusal = "Saving in progress. Please wait for it to finish before closing.";
        assert_eq!(save_note(false, true, true, refusal), None);
        let fence = "A project operation is in progress. Try closing again when it finishes.";
        assert_eq!(save_note(false, true, true, fence), None);
        assert_eq!(save_note(true, false, false, refusal), Some("Saved"));
        assert_eq!(save_note(true, false, false, fence), Some("Saved"));
        assert_eq!(
            save_note(true, false, true, SAVING),
            Some("Save did not complete; recovery data retained")
        );
        // Other notes set during the save (errors, UI save results) stay.
        assert_eq!(save_note(true, false, false, "Saved"), None);
        assert_eq!(save_note(true, false, false, "Edit failed"), None);
        assert_eq!(save_note(false, false, false, "Opened"), None);
    }
}
