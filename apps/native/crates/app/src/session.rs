use crate::{panels, platform};
use gpui::{prelude::*, *};
use pm_engine_service::{EngineService, MediaCatalog, Viewer, ViewerConfig};
use pm_kernel::{
    persistence::DiskFormat,
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

pub struct Controller {
    pub kernel: KernelHandle,
    pub engine: Arc<EngineService>,
    pub host: Rc<RefCell<Host>>,
    pub profile: PathBuf,
    pub session: Option<SessionHandle>,
    pub published: Option<Arc<Published>>,
    pub mirror: Option<QueryMirror>,
    pub viewer: Option<Viewer>,
    pub media: MediaCatalog,
    pub opening: bool,
    pub layers: Vec<LayerInfo>,
    pub selected: Option<String>,
    pub message: String,
    pub path: Option<PathBuf>,
    pub recent: Vec<(String, String)>,
    pub show_library: bool,
    pub rate: pm_time::Rate,
    pub duration: f64,
    pub dimensions: [f64; 2],
    pub zoom: Option<f64>,
    pub drag: Option<panels::NumericDrag>,
    pub recovery_pending: bool,
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
        self.0
            .update(cx, |v, cx| panels::render(v, id, bounds, w, cx))
    }
    fn project_id(&self, cx: &App) -> Option<String> {
        self.0.read(cx).session.as_ref().map(|s| s.project.clone())
    }
    fn status(&self, cx: &App) -> String {
        self.0.read(cx).message.clone()
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
    pub fn new(
        kernel: KernelHandle,
        engine: Arc<EngineService>,
        profile: PathBuf,
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
            while let Ok(e) = rx.recv().await {
                if this
                    .update(cx, |v, cx| {
                        match e {
                            KernelEvent::Durability { message } => {
                                v.message = format!("Recovery warning: {message}")
                            }
                            KernelEvent::Notice { notice, .. } => {
                                v.message = format!("Kernel notice: {notice:?}")
                            }
                            _ => {}
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
            kernel,
            engine,
            profile,
            host,
            session: None,
            published: None,
            mirror: None,
            viewer: None,
            media: MediaCatalog::default(),
            opening: false,
            layers: vec![],
            selected: None,
            message: "Open a project to begin".into(),
            path: None,
            recent: vec![],
            show_library: false,
            rate: pm_time::Rate::FPS_30,
            duration: 1.,
            dimensions: [1920., 1080.],
            zoom: None,
            drag: None,
            recovery_pending: false,
        }
    }
    pub fn initialize(&mut self, launch: Option<PathBuf>, cx: &mut Context<Self>) {
        let pending = self.kernel.shell(ShellCall::AttachLibrary {
            profile: self.profile.clone(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        });
        cx.spawn(async move |this, cx| {
            let result = pending.await;
            let _ = this.update(cx, |v, cx| {
                match result {
                    Some(Ok(ShellReply::Library(pm_kernel::locks::OpenDecision::Writable))) => {
                        v.load_recent(cx);
                        if let Some(path) = launch {
                            v.open_file(path, cx);
                        }
                    }
                    other => v.message = format!("Could not attach isolated library: {other:?}"),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            cx.notify();
            return;
        };
        let Some(p) = self.kernel.published(&session.project) else {
            return;
        };
        if let Some(viewer) = &self.viewer
            && let Err(e) = viewer.publish(&p)
        {
            self.message = e.to_string();
        }
        if let Some(m) = &mut self.mirror {
            m.sync(&p);
        } else {
            self.mirror = Some(QueryMirror::new(&p));
        }
        self.layers = self.mirror.as_mut().unwrap().queries().layers(None);
        self.selected = p.selection.layers().first().cloned().or_else(|| {
            self.selected
                .clone()
                .filter(|id| self.layers.iter().any(|l| &l.id == id))
        });
        let title = p
            .snapshot
            .document
            .root()
            .and_then(|r| r.get("name"))
            .and_then(crate::node_string)
            .unwrap_or("Untitled");
        self.host
            .borrow()
            .set_document_metadata(title, p.dirty, self.path.as_deref());
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
        cx.notify();
    }
    pub fn call(&mut self, call: Call, cx: &mut Context<Self>) {
        let Some(session) = &self.session else { return };
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
    pub fn edit(&mut self, command: pm_kernel::Command, cx: &mut Context<Self>) {
        self.call(
            Call::Transact {
                label: None,
                body: WriteBody::Commands(vec![command]),
                expected_revision: None,
            },
            cx,
        );
    }
    pub fn select(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected = Some(id.clone());
        cx.notify();
    }
    pub fn time(&self) -> f64 {
        self.viewer
            .as_ref()
            .and_then(|v| v.status().clock)
            .map_or(0., |c| c.media_time.get() as f64 / TICKS_PER_SECOND as f64)
    }
    pub fn seek(&mut self, seconds: f64) {
        if let Some(v) = &self.viewer {
            let frame = self
                .rate
                .frame_of((seconds.clamp(0., self.duration) * TICKS_PER_SECOND as f64) as i64);
            if let Ok(time) = self.rate.frame_start(frame)
                && let Err(e) = v.scrub(Ticks::new(time).unwrap_or(Ticks::ZERO))
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
        self.open(SessionSource::File(path.clone()), Some(path), cx);
    }
    pub fn open_library(&mut self, id: String, cx: &mut Context<Self>) {
        self.open(
            SessionSource::Library {
                profile: self.profile.clone(),
                project: id,
            },
            None,
            cx,
        );
    }
    fn open(&mut self, source: SessionSource, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.opening || self.recovery_pending || self.drag.is_some() {
            self.message = "Finish the active edit or recovery decision first".into();
            return;
        }
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
            let media_path = path.clone();
            let library = match &source { SessionSource::Library {profile,project} => Some((profile.clone(),project.clone())), _=>None };
            let media = cx.background_executor().spawn(async move {
                let media_path = media_path.or_else(|| library.and_then(|(profile,id)| {
                    LibraryStore::open(&profile).load(&id).ok().flatten()?.session?
                        .get("file")?.get("path")?.as_str().map(PathBuf::from)
                }));
                media_path.as_deref().map(|p| crate::media::for_file(p, &cache)).transpose()
            }).await;
            if let Err(error) = &media {
                let _ = this.update(cx, |v,cx| {v.opening=false;v.message=format!("Open failed: {error}");cx.notify();});
                return;
            }
            let open_id = match &source {
                SessionSource::Library {project,..}=>project.clone(),
                SessionSource::File(p)=>stable_id(p),
                SessionSource::New(_)=>pm_kernel::new_id("app"),
            };
            let result = kernel.shell(ShellCall::Open {id:open_id, source, electron_profile:None}).await;
            let mut previous_save_failed = false;
            if matches!(&result, Some(Ok(ShellReply::Opened {..})))
                && let Some(old)=old {
                previous_save_failed = !matches!(kernel.shell(ShellCall::Close(old)).await,Some(Ok(ShellReply::Closed{..})));
            }
            let _ = this.update(cx, |v, cx| {
                v.opening = false;
                match result {
                    Some(Ok(ShellReply::Opened { session, recovery, decision, info })) => {
                        if let Some(viewer)=v.viewer.take(){viewer.retire();}
                        v.path=path;
                        v.session = Some(session);
                        v.mirror = None;
                        v.selected = None;
                        v.refresh(cx);
                        let p = v.published.as_ref().unwrap();
                        let comp = p.snapshot.open_comp().unwrap_or_default();
                        let root = p.snapshot.document.root().unwrap();
                        let composition = pm_kernel::model::comp_node(root, Some(&comp)).unwrap_or(root);
                        let number = |key: &str, default| composition.get(key).and_then(crate::node_number).unwrap_or(default);
                        v.duration = number("dur", 1.).max(0.001);
                        v.dimensions = [number("w",1920.), number("h",1080.)];
                        let fps = number("fps",30.);
                        v.rate = pm_time::Rate::new((fps * 1000.).round() as i64, 1000).unwrap_or(pm_time::Rate::FPS_30);
                        let media = match media { Ok(m) => m.unwrap_or_default(), Err(e) => { v.message = format!("Media could not be resolved: {e}"); MediaCatalog::default() } };
                        v.media = media.clone();
                        v.message = format!("Opened · {decision:?}{}", if info.history_rejected { " · Saved undo history could not be imported" } else { "" });
                        match v.engine.viewer(ViewerConfig { snapshot: p.snapshot.clone(), composition: comp, media,
                            output: pm_engine_service::Output { width: 640, height: 360, display: 0, headroom: 1. },
                            layer: Some(v.host.borrow().viewer_layer()), rate: v.rate, end: Some((v.duration * TICKS_PER_SECOND as f64) as i64), audio: true }) {
                            Ok(viewer) => v.viewer = Some(viewer), Err(e) => v.message = format!("Viewer failed: {e}"),
                        }
                        if let Some(ui) = v.published.as_ref().map(|p| p.ui_state.clone()) {
                            let time = ui.transport.get("time").and_then(|v| v.as_f64()).unwrap_or(0.);
                            v.seek(time);
                        }
                        v.show_library = false;
                        if previous_save_failed { v.message.push_str(" · Previous project could not be saved; its recovery journal is retained"); }

                        v.recovery(recovery, cx);
                        v.load_recent(cx);
                    }
                    other => { v.message = format!("Open failed: {other:?}"); }
                }
                cx.notify();
            });
        }).detach();
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
        if self.recovery_pending {
            self.message = "Choose how to recover this project before saving".into();
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.persist_ui();
        let call = as_path.clone().map_or_else(
            || ShellCall::Save(session.clone()),
            |path| ShellCall::SaveAs {
                session: session.clone(),
                path,
            },
        );
        let pending = self.kernel.shell(call);
        cx.spawn(async move |this, cx| {
            let result = pending.await;
            let _ = this.update(cx, |v, cx| {
                match result {
                    Some(Ok(ShellReply::Saved {
                        session,
                        recovery_copy,
                    })) => {
                        v.session = Some(session);
                        if as_path.is_some() {
                            v.path = as_path;
                        }
                        v.message = recovery_copy.map_or("Saved".into(), |_| {
                            "Saved to a recovery copy; original was not overwritten".into()
                        });
                        v.load_recent(cx);
                    }
                    other => v.message = format!("Save failed; recovery data retained: {other:?}"),
                }
                v.refresh(cx);
            });
        })
        .detach();
    }
    fn recovery(&mut self, result: RecoveryResult, cx: &mut Context<Self>) {
        if result == RecoveryResult::None {
            return;
        }
        self.recovery_pending = true;
        let rx = platform::choice(
            &self.host.borrow().window,
            "Unsaved work was found",
            &format!("{result:?}\nRecovery data is kept until you choose what to do."),
            &["Recover", "Open as copy", "Discard"],
        );
        cx.spawn(async move |this, cx| {
            let choice = rx.recv().await.unwrap_or(0);
            let _ = this.update(cx, |v, cx| {
                v.recovery_pending = false;
                match (&result, choice) {
                    (RecoveryResult::OfferCopy {project,..}, 0 | 1) => {
                        v.open_library(project.clone(),cx);
                    }
                    (RecoveryResult::OfferCopy {project,..}, 2) => {
                        let pending=v.kernel.shell(ShellCall::Trash(project.clone()));
                        cx.spawn(async move|this,cx|{
                            let result=pending.await;
                            let _=this.update(cx,|v,cx|{v.message=format!("Recovery copy moved to library trash: {result:?}");cx.notify();});
                        }).detach();
                    }
                    (RecoveryResult::Recovered {..}, 1) => v.save_dialog(cx),
                    (RecoveryResult::Recovered {..}, 2) => {
                        v.recovery_pending=true;
                        v.message="Discard needs a kernel recovery-decision API. Unsaved recovery is retained; reopen to choose Recover or Open as copy.".into();
                    }
                    (RecoveryResult::Unrecoverable {..}, _) => {
                        v.message="Recovery could not be replayed. The original is open and all recovery files are retained.".into();
                    }
                    _=>v.message="Recovered work is ready; save to keep it".into(),
                }
                cx.notify();
            });
        }).detach();
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
    pub fn command(&mut self, cmd: &ShellCommand, _: &mut Window, cx: &mut Context<Self>) -> bool {
        match cmd.name.as_str() {
            "open" => {
                let rx = platform::paths(&self.host.borrow().window, false);
                cx.spawn(async move |this, cx| {
                    if let Ok(Some(path)) = rx.recv().await {
                        let _ = this.update(cx, |v, cx| v.open_file(path, cx));
                    }
                })
                .detach();
            }
            "openRecent" | "projects" => {
                self.show_library = !self.show_library;
                self.load_recent(cx);
            }
            "save" => self.save(None, cx),
            "saveAs" => self.save_dialog(cx),
            "export" => self.export(cx),
            "undo" | "contextUndo" => self.call(Call::History(HistoryCall::Undo), cx),
            "redo" | "contextRedo" => self.call(Call::History(HistoryCall::Redo), cx),
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
                            .layers
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
            // Kernel Close saves; a window close leaves journal recovery intact.
            _ => return false,
        }
        cx.notify();
        true
    }
}
fn stable_id(path: &std::path::Path) -> String {
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
    }
}
impl Controller {
    fn export(&mut self, cx: &mut Context<Self>) {
        let Some(p) = &self.published else { return };
        if self.recovery_pending {
            self.message = "Choose a recovery option before exporting".into();
            return;
        }
        let snapshot = p.snapshot.clone();
        let media = self.media.clone();
        let duration = self.duration;
        let rate = self.rate;
        let dimensions = self.dimensions;
        let rx = platform::paths_named(&self.host.borrow().window, true, "Frames");
        self.message = "Export: choose a new PNG sequence folder".into();
        cx.spawn(async move |this, cx| {
            let Ok(Some(destination)) = rx.recv().await else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    use pm_protocol::{
                        color::{OutputColor, Precision},
                        common::{FrameRate, SessionId, Size, TickRange},
                        priority::{AlphaOutput, ExportFormat, ExportSettings, JobSpec},
                    };
                    let request = pm_engine_service::jobs::ExportRequest::new(
                        pm_kernel::new_id("app").as_str().into(),
                        &snapshot,
                        JobSpec::Export {
                            session: SessionId::new(snapshot.doc_id).unwrap(),
                            composition: snapshot.open_comp().unwrap_or_default().as_str().into(),
                            settings: ExportSettings {
                                format: ExportFormat::PngSequence { sixteen_bit: false },
                                alpha: AlphaOutput::Straight,
                                quality: pm_protocol::priority::QualityTier::High,
                                size: Size::new(dimensions[0] as u32, dimensions[1] as u32),
                                frame_rate: FrameRate::new(rate.num() as u32, rate.den() as u32),
                                range: TickRange {
                                    start: Ticks::ZERO,
                                    end: Ticks::new((duration * TICKS_PER_SECOND as f64) as i64)?,
                                },
                                audio: false,
                                audio_index_playhead: None,
                                motion_blur: false,
                                color: OutputColor::SrgbImage,
                                precision: Precision::Float16,
                                dither: true,
                                draft_from_proxies: false,
                                delivery: Default::default(),
                            },
                        },
                        media,
                        destination,
                    )?;
                    pm_engine_service::jobs::ExportHandle::start(
                        &std::env::current_exe()?,
                        request,
                    )?
                    .wait()
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                v.message = match result {
                    Ok(status) => format!(
                        "PNG sequence export: {:?}{}",
                        status.state,
                        status.error.map_or(String::new(), |e| format!(" · {e:?}"))
                    ),
                    Err(e) => format!("Export failed: {e}"),
                };
                cx.notify();
            });
        })
        .detach();
    }
}
