//! Native delivery lifecycle. One reservation spans picker, startup, worker and
//! reaping. The snapshot/options/playhead stay frozen across an in-place retry.
use crate::session::Controller;
use gpui::Context;
use pm_promedia::{
    export_panel::{Format, Options},
    tasks::{BackgroundTasks, ProgressUnit, TaskKind, TaskSpec},
};
use pm_protocol::{
    common::SessionId,
    priority::{JobSpec, JobState},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Clone)]
pub struct Frozen {
    publication: Arc<pm_kernel::Published>,
    options: Options,
    media: pm_engine_service::MediaCatalog,
    resources: Arc<pm_engine_service::resources::ResourceSnapshot>,
    playhead: f64,
    pub destination: PathBuf,
    /// The person chose Export Without Them: offline clips are left out.
    without_offline: bool,
}
pub struct Run {
    pub handle: Mutex<Option<pm_engine_service::jobs::ExportHandle>>,
    pub cancel: AtomicBool,
}
pub type Job = Arc<Run>;
/// Every export of this process, the person's and agents', in one list: every
/// window's Background Tasks shows them all.
pub fn tasks() -> Arc<Mutex<BackgroundTasks>> {
    static TASKS: std::sync::OnceLock<Arc<Mutex<BackgroundTasks>>> = std::sync::OnceLock::new();
    TASKS.get_or_init(Arc::default).clone()
}
/// How to stop a running export: an agent's flag, or a window's job.
enum Canceller {
    Flag(Arc<AtomicBool>),
    Run(std::sync::Weak<Run>),
}
/// Every running export's canceller, by task: Cancel reaches exactly that export,
/// whichever window started it or shows the list.
fn cancellers() -> &'static Mutex<std::collections::HashMap<u64, Canceller>> {
    static CANCELS: std::sync::OnceLock<Mutex<std::collections::HashMap<u64, Canceller>>> =
        std::sync::OnceLock::new();
    CANCELS.get_or_init(Default::default)
}
/// Background Tasks ▸ Cancel. False if no running export has that task.
pub fn cancel_export(id: pm_promedia::tasks::TaskId) -> bool {
    match cancellers().lock().unwrap().get(&id.0) {
        Some(Canceller::Flag(flag)) => flag.store(true, Ordering::Release),
        Some(Canceller::Run(run)) => match run.upgrade() {
            Some(run) => run.cancel.store(true, Ordering::Release),
            None => return false,
        },
        None => return false,
    }
    true
}
/// An agent's export as a Background Task: listed beside the person's, with its
/// progress, and cancellable there.
pub struct AgentExport {
    ticket: pm_promedia::tasks::Ticket,
    pub cancel: Arc<AtomicBool>,
}
impl AgentExport {
    pub fn start(title: &str, project: &str, frames: u64) -> Result<Self, String> {
        Self::begin(
            TaskSpec {
                title: format!("{title} (Agent)"),
                kind: TaskKind::Export,
                unit: ProgressUnit::Frames,
                priority: 0,
                total: frames,
            },
            project,
        )
    }
    /// Any cancellable row run off main (Generate Captions too).
    pub fn begin(spec: TaskSpec, project: &str) -> Result<Self, String> {
        let tasks = tasks();
        let mut tasks = tasks.lock().unwrap();
        let id = tasks
            .enqueue(project.into(), project.into(), spec)
            .map_err(|e| format!("{e:?}"))?;
        // Enqueued and started under one lock: no other export waits queued.
        let ticket = tasks
            .start_next()
            .filter(|t| t.id == id)
            .ok_or("Export task did not start")?;
        let cancel = Arc::new(AtomicBool::new(false));
        cancellers()
            .lock()
            .unwrap()
            .insert(id.0, Canceller::Flag(cancel.clone()));
        Ok(Self { ticket, cancel })
    }
    pub fn progress(&self, frames: u64, total: u64) {
        let tasks = tasks();
        let mut tasks = tasks.lock().unwrap();
        let done = tasks.get(self.ticket.id).map_or(0, |t| t.completed);
        let _ = tasks.progress(self.ticket, frames.max(done).min(total), total);
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
    /// The row's task, as Background Tasks ▸ Cancel names it (`cancel_export`).
    #[cfg(test)]
    pub fn id(&self) -> pm_promedia::tasks::TaskId {
        self.ticket.id
    }
    /// Settles the row: completed, failed with its reason, or cancelled.
    pub fn finish(self, result: Result<(), String>) {
        cancellers().lock().unwrap().remove(&self.ticket.id.0);
        let tasks = tasks();
        let mut tasks = tasks.lock().unwrap();
        if self.cancel.load(Ordering::Acquire) {
            let _ = tasks
                .cancel(self.ticket.id)
                .and_then(|_| tasks.settle_control(self.ticket));
        } else {
            let _ = tasks.finish(self.ticket, result.map_err(|e| e.into()));
        }
    }
}
impl Controller {
    /// `without`: the person chose Export Without Them after a refusal naming
    /// offline clips; the same frozen export runs again, leaving them out.
    pub fn begin_builtin_export(
        &mut self,
        options: Value,
        retry: bool,
        without: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        if self.builtins.export_job.is_some() {
            // Preserve the actual in-flight run and its terminal ownership.
            return;
        }
        let saved = if retry {
            self.builtins.export_retry.clone().map(|mut frozen| {
                frozen.without_offline |= without;
                frozen
            })
        } else {
            None
        };
        let without_offline = saved.as_ref().is_some_and(|f| f.without_offline);
        let (p, typed, media, playhead) = if let Some(frozen) = &saved {
            (
                frozen.publication.clone(),
                frozen.options.clone(),
                frozen.media.clone(),
                frozen.playhead,
            )
        } else {
            let Some(p) = self.published.clone() else {
                return;
            };
            let typed = match pm_builtins::delivery::options(&options) {
                Ok(o) => o,
                Err(e) => {
                    let _ = runtime.observe_export(json!({"phase":"failed","message":e}));
                    return;
                }
            };
            (p, typed, self.media.clone(), self.time())
        };
        let filename = format!(
            "{}.{}",
            p.snapshot
                .root()
                .and_then(|r| r.get("name"))
                .and_then(crate::node_string)
                .unwrap_or("Untitled"),
            typed.format.extension()
        );
        let picker = if saved.is_none() {
            Some(crate::platform::export_destination(
                &self.host.borrow().window,
                typed.format == Format::Png,
                &filename,
            ))
        } else {
            None
        };
        let job = Arc::new(Run {
            handle: Mutex::new(None),
            cancel: AtomicBool::new(false),
        });
        self.builtins.export_job = Some(job.clone());
        let activity = self.engine.export_activity();
        let tasks = self.builtins.export_tasks.clone();
        let resources = self.extensions.resources.clone();
        cx.spawn(async move |this, cx| {
            let destination = if let Some(frozen) = saved {
                Some(frozen.destination)
            } else if let Some(picker) = picker {
                picker.recv().await.ok().flatten()
            } else {
                None
            };
            let Some(destination) = destination else {
                let _ = runtime.observe_export(json!({"phase":"idle"}));
                let _ = this.update(cx, |v, cx| {
                    if v.builtins
                        .export_job
                        .as_ref()
                        .is_some_and(|j| Arc::ptr_eq(j, &job))
                    {
                        v.builtins.export_job = None;
                    }
                    cx.notify();
                });
                return;
            };
            // A delivery started while the window's extensions load waits for
            // them: an effect still loading is never refused as not installed.
            // The status line says who it waits for; Cancel still cancels.
            let mut resources = resources;
            let mut shown = String::new();
            if resources.pending {
                // The destination is chosen: the panel shows the export as
                // preparing, the phase whose Cancel it sends (while it chose
                // the destination its Cancel was off, and a wait left in
                // that phase could not be cancelled from the panel).
                observe(&runtime, json!({"phase":"preparing"})).await;
            }
            while resources.pending && !job.cancel.load(Ordering::Acquire) {
                let text = crate::extensions::starts_once_loaded("Export", &resources);
                let _ = this.update(cx, |v, cx| {
                    if (shown.is_empty() || v.message == shown) && v.message != text {
                        v.message = text.clone();
                        cx.notify();
                    }
                });
                shown = text;
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                match this.update(cx, |v, _| v.extensions.resources.clone()) {
                    Ok(now) => resources = now,
                    Err(_) => return,
                }
            }
            if !shown.is_empty() {
                let _ = this.update(cx, |v, cx| {
                    if v.message == shown {
                        v.message.clear();
                        cx.notify();
                    }
                });
            }
            let frozen = Frozen {
                publication: p,
                options: typed,
                media,
                resources,
                playhead,
                destination,
                without_offline,
            };
            // The window's extensions as the job starts answer its overrides.
            let (current, overrides) = this
                .update(cx, |v, _| {
                    let current = v
                        .builtins
                        .runtime
                        .as_ref()
                        .is_some_and(|r| Arc::ptr_eq(r, &runtime));
                    if current {
                        v.builtins.export_retry = Some(frozen.clone());
                    }
                    (current, v.extensions.overrides.as_ref().map(|o| o.jobs()))
                })
                .unwrap_or((false, None));
            if !current {
                job.cancel.store(true, Ordering::Release);
            }
            let worker_job = job.clone();
            let executor = cx.background_executor().clone();
            let result = cx.background_executor().spawn(async move {
                execute(
                    frozen,
                    worker_job,
                    EngineJob {
                        activity,
                        overrides,
                    },
                    runtime.clone(),
                    executor,
                    tasks,
                )
                .await
            });
            // Retain a separate runtime reference for terminal publication.
            let result = result.await;
            let _ = this.update(cx, |v, cx| {
                if v.builtins
                    .export_job
                    .as_ref()
                    .is_some_and(|j| Arc::ptr_eq(j, &job))
                {
                    v.builtins.export_job = None;
                    match &result {
                        Err(error)
                            if !(job.cancel.load(Ordering::Acquire) && error == "cancelled") =>
                        {
                            v.message = error.clone();
                        }
                        Ok(path) => {
                            v.close_export_sheet(cx);
                            let name = std::path::Path::new(path)
                                .file_name()
                                .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
                            v.note(format!("Exported {name}"), cx);
                        }
                        Err(_) => v.close_export_sheet(cx),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
async fn observe(runtime: &pm_builtins::Builtins, run: Value) {
    if let Ok(ticket) = runtime.observe_export(run) {
        let _ = ticket.await;
    }
}
async fn execute(
    frozen: Frozen,
    job: Job,
    engine: EngineJob,
    runtime: Arc<pm_builtins::Builtins>,
    executor: gpui::BackgroundExecutor,
    tasks: Arc<Mutex<BackgroundTasks>>,
) -> Result<String, String> {
    let planned = pm_builtins::delivery::plan(
        &frozen.publication.snapshot,
        &frozen.options,
        frozen.playhead,
    );
    let mut ticket = None;
    let mut result: Result<String, Failure> = match planned {
        Err(error) => Err(error.into()),
        Ok(planned) => {
            let title = frozen
                .publication
                .snapshot
                .root()
                .and_then(|r| r.get("name"))
                .and_then(crate::node_string)
                .unwrap_or("Untitled");
            let queued = {
                let mut tasks = tasks.lock().unwrap();
                tasks
                    .enqueue(
                        frozen.publication.project.clone(),
                        frozen.publication.project.clone(),
                        TaskSpec {
                            title: title.into(),
                            kind: TaskKind::Export,
                            unit: if frozen.options.format == Format::Json {
                                ProgressUnit::Items
                            } else {
                                ProgressUnit::Frames
                            },
                            priority: 0,
                            total: planned.frames,
                        },
                    )
                    .map_err(|e| format!("{e:?}"))
                    .and_then(|_| tasks.start_next().ok_or("Export task did not start".into()))
            };
            match queued {
                Err(error) => Err(error.into()),
                Ok(started) => {
                    ticket = Some(started);
                    cancellers()
                        .lock()
                        .unwrap()
                        .insert(started.id.0, Canceller::Run(Arc::downgrade(&job)));
                    execute_inner(
                        &frozen,
                        &job,
                        engine,
                        &runtime,
                        &executor,
                        TaskAdmission {
                            tasks: &tasks,
                            ticket: started,
                        },
                        &planned,
                    )
                    .await
                }
            }
        }
    };
    // Always release/reap on this worker before publishing a terminal task/run.
    let cleanup = job.handle.lock().unwrap().take();
    if let Some(handle) = cleanup {
        handle.cancel();
        if let Err(error) = handle.wait() {
            let failed = result.err().map(|f| f.message).unwrap_or_default();
            result = Err(format!("{failed}; export cleanup failed: {error}").into());
        }
    }
    if let Some(ticket) = ticket {
        cancellers().lock().unwrap().remove(&ticket.id.0);
        let settled = {
            let mut tasks = tasks.lock().unwrap();
            if result.as_ref().is_err_and(|e| e.message == "cancelled") {
                tasks
                    .cancel(ticket.id)
                    .and_then(|_| tasks.settle_control(ticket))
            } else {
                tasks.finish(
                    ticket,
                    result
                        .as_ref()
                        .map(|_| ())
                        .map_err(|e| e.message.clone().into()),
                )
            }
        };
        if let Err(error) = settled {
            result = Err(format!("Export task could not settle: {error:?}").into());
        }
    }
    match &result {
        Err(error) if job.cancel.load(Ordering::Acquire) && error.message == "cancelled" => {
            observe(&runtime, json!({"phase":"cancelled"})).await
        }
        Err(error) => {
            let offline = !error.offline.is_empty();
            observe(
                &runtime,
                json!({"phase":"failed","message":error.message,"offline":offline}),
            )
            .await
        }
        Ok(path) => observe(&runtime, json!({"phase":"done","path":path})).await,
    }
    result.map_err(|f| f.message)
}
/// Why an export ended without a file: what the sheet says, and the offline
/// media it refused to export without (the sheet then offers Export Without
/// Them).
struct Failure {
    message: String,
    offline: Vec<pm_protocol::common::AssetId>,
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            message,
            offline: vec![],
        }
    }
}
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl From<pm_protocol::error::EngineError> for Failure {
    fn from(error: pm_protocol::error::EngineError) -> Self {
        use pm_protocol::error::{ErrorCode, ErrorDetail};
        let offline = match error.detail {
            Some(ErrorDetail::Offline { assets }) if error.code == ErrorCode::MediaOffline => {
                assets
            }
            _ => vec![],
        };
        Self {
            message: error.message,
            offline,
        }
    }
}
/// What the window lends its export's job: the viewer's activity, which
/// keeps precedence on the GPU, and its extensions, which answer the job's
/// raster overrides.
struct EngineJob {
    activity: pm_engine_service::jobs::Activity,
    overrides: Option<crate::raster_overrides::JobOverrides>,
}
struct TaskAdmission<'a> {
    tasks: &'a Arc<Mutex<BackgroundTasks>>,
    ticket: pm_promedia::tasks::Ticket,
}

/// Caption files beside the exported video (today's delivery sidecars):
/// "Film.mp4" gets "Film.srt", or "Film.en.srt", "Film.fr.srt" with several
/// captions layers, in times from the start of the exported range.
fn write_sidecars(
    frozen: &Frozen,
    format: pm_captions::CaptionFormat,
    video: &std::path::Path,
) -> Result<(), Failure> {
    let Some(doc) = crate::captions_panel::read(&frozen.publication) else {
        return Ok(());
    };
    let range = match doc.work.filter(|_| frozen.options.range == "work") {
        Some(work) => work,
        None => [0., doc.duration],
    };
    let extension = if format == pm_captions::CaptionFormat::Vtt {
        "vtt"
    } else {
        "srt"
    };
    let base = video
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    for (suffix, text) in crate::captions_panel::sidecars(&doc, format, range) {
        let name = match suffix {
            Some(tag) => format!("{base}.{tag}.{extension}"),
            None => format!("{base}.{extension}"),
        };
        crate::captions_panel::write_file(&video.with_file_name(name), text.as_bytes()).map_err(
            |_| "The video was exported, but its caption file couldn’t be written.".to_owned(),
        )?;
    }
    Ok(())
}
async fn execute_inner(
    frozen: &Frozen,
    job: &Job,
    engine: EngineJob,
    runtime: &pm_builtins::Builtins,
    executor: &gpui::BackgroundExecutor,
    task: TaskAdmission<'_>,
    planned: &pm_builtins::delivery::Plan,
) -> Result<String, Failure> {
    let TaskAdmission { tasks, ticket } = task;
    if job.cancel.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    let intent = frozen.options.intent(planned.has_audio, planned.captions)?;
    let sidecar = match intent.sidecar.as_str() {
        "srt" => Some(pm_captions::CaptionFormat::Srt),
        "vtt" => Some(pm_captions::CaptionFormat::Vtt),
        _ => None,
    };
    observe(runtime, json!({"phase":"preparing"})).await;
    if frozen.options.format == Format::Json {
        let path = pm_builtins::delivery::portable(
            &frozen.publication.snapshot,
            &frozen.media.originals,
            &frozen.destination,
            &job.cancel,
        )?;
        return Ok(path.to_string_lossy().into());
    }
    let document = frozen.publication.snapshot.doc_id;
    let mut request = pm_engine_service::jobs::ExportRequest::new(
        pm_kernel::new_id("export").as_str().into(),
        &frozen.publication.snapshot,
        JobSpec::Export {
            session: SessionId::new(document).map_err(|e| e.to_string())?,
            composition: frozen.publication.snapshot.render_comp().as_str().into(),
            settings: planned.settings.clone().ok_or("Missing render settings")?,
        },
        frozen.media.clone(),
        frozen.destination.clone(),
    )
    .map_err(|e| e.to_string())?;
    request.resources = (*frozen.resources).clone();
    request.without_offline = frozen.without_offline;
    let mut session = pm_engine_service::jobs::JobSession::new(request);
    // Its raster overrides answer from this publication, whatever is edited meanwhile.
    if let Some(overrides) = engine.overrides {
        overrides.pin(&mut session, &frozen.publication, false);
    }
    let handle = pm_engine_service::jobs::ExportHandle::start_scheduled(
        &std::env::current_exe().map_err(|e| e.to_string())?,
        session,
        engine.activity,
    )
    .map_err(|e| e.to_string())?;
    *job.handle.lock().unwrap() = Some(handle);
    let started = std::time::Instant::now();
    let mut cancellation_sent = false;
    loop {
        if job.cancel.load(Ordering::Acquire) && !cancellation_sent {
            cancellation_sent = true;
            job.handle
                .lock()
                .unwrap()
                .as_ref()
                .ok_or("Export worker lost")?
                .cancel();
        }
        let status = job
            .handle
            .lock()
            .unwrap()
            .as_ref()
            .ok_or("Export worker lost")?
            .status();
        if matches!(
            status.state,
            JobState::Done | JobState::Failed | JobState::Cancelled
        ) {
            let handle = job
                .handle
                .lock()
                .unwrap()
                .take()
                .ok_or("Export worker lost")?;
            let final_status = handle.wait().map_err(|e| e.to_string())?;
            return match final_status.state {
                JobState::Done => {
                    let path = final_status
                        .path
                        .ok_or("Export completed without an output path")?;
                    if let Some(format) = sidecar {
                        write_sidecars(frozen, format, std::path::Path::new(&path))?;
                    }
                    Ok(path)
                }
                JobState::Cancelled => Err("cancelled".into()),
                _ => Err(final_status.error.map_or_else(
                    || "Powermove lost contact with the export engine. Try again.".into(),
                    Failure::from,
                )),
            };
        }
        let frame = (status.progress * planned.frames as f64).round() as u64;
        tasks
            .lock()
            .unwrap()
            .progress(ticket, frame, planned.frames)
            .map_err(|e| format!("{e:?}"))?;
        let run = if job.cancel.load(Ordering::Acquire) {
            json!({"phase":"cancelling"})
        } else if status.state == JobState::Running
            && let Some(checked) = status.checking
        {
            // Every frame is encoded: the written file is being checked.
            let checked = (checked * planned.frames as f64).round() as u64;
            json!({"phase":"checking","frame":checked,"total":planned.frames,"elapsed":started.elapsed().as_secs_f64()})
        } else if status.state == JobState::Running {
            json!({"phase":"rendering","frame":frame,"total":planned.frames,"elapsed":started.elapsed().as_secs_f64()})
        } else {
            json!({"phase":"preparing"})
        };
        observe(runtime, run).await;
        executor.timer(std::time::Duration::from_millis(50)).await;
    }
}
#[cfg(test)]
mod agent_tests {
    use super::*;
    use pm_promedia::tasks::TaskState;

    fn row(project: &str) -> pm_promedia::tasks::Task {
        tasks()
            .lock()
            .unwrap()
            .all()
            .into_iter()
            .rev()
            .find(|t| t.project == project)
            .expect("listed")
    }
    #[test]
    fn an_agent_export_is_a_background_task_the_person_can_cancel() {
        let export = AgentExport::start("Promo", "agent-test-cancel", 120).unwrap();
        let listed = row("agent-test-cancel");
        assert_eq!(listed.title, "Promo (Agent)");
        assert_eq!(listed.kind, TaskKind::Export);
        assert_eq!(listed.state, TaskState::Running);
        export.progress(30, 120);
        assert_eq!(row("agent-test-cancel").completed, 30);
        // Background Tasks ▸ Cancel reaches the agent's job, not the person's.
        assert!(cancel_export(listed.id));
        assert!(export.cancelled());
        export.finish(Err("cancelled".into()));
        assert_eq!(row("agent-test-cancel").state, TaskState::Cancelled);
        assert!(!cancel_export(listed.id), "a settled export has no cancel");
    }
    /// Two windows' exports run at once: Cancel on one row stops that export only.
    #[test]
    fn cancel_reaches_exactly_the_export_on_its_row() {
        let run = || {
            Arc::new(Run {
                handle: Mutex::new(None),
                cancel: AtomicBool::new(false),
            })
        };
        let (first, second) = (run(), run());
        for (id, job) in [(9_001, &first), (9_002, &second)] {
            cancellers()
                .lock()
                .unwrap()
                .insert(id, Canceller::Run(Arc::downgrade(job)));
        }
        assert!(cancel_export(pm_promedia::tasks::TaskId(9_002)));
        assert!(second.cancel.load(Ordering::Acquire));
        assert!(!first.cancel.load(Ordering::Acquire));
        drop(second);
        assert!(
            !cancel_export(pm_promedia::tasks::TaskId(9_002)),
            "a finished job"
        );
        cancellers().lock().unwrap().retain(|id, _| *id < 9_000);
    }
    /// Export Without Them follows the job's typed refusal, never its words.
    #[test]
    fn only_an_offline_refusal_offers_export_without_them() {
        use pm_protocol::error::{EngineError, ErrorCode, ErrorDetail, ErrorScope};
        let job = ErrorScope::Job { job: "j".into() };
        let refused = EngineError::new(ErrorCode::MediaOffline, job.clone(), "Any words")
            .with_detail(ErrorDetail::Offline {
                assets: vec!["gone".into()],
            });
        let failure = Failure::from(refused);
        assert_eq!(failure.message, "Any words");
        assert_eq!(failure.offline, vec!["gone".into()]);
        let worded = EngineError::new(
            ErrorCode::ExportFailed,
            job,
            "1 clip is offline: a.mov. Relink the originals, or export without them.",
        );
        assert!(Failure::from(worded).offline.is_empty());
    }
    #[test]
    fn an_agent_export_settles_as_done_or_failed() {
        let export = AgentExport::start("Done", "agent-test-done", 10).unwrap();
        export.finish(Ok(()));
        let done = row("agent-test-done");
        assert_eq!((done.state, done.completed), (TaskState::Completed, 10));
        let export = AgentExport::start("Failed", "agent-test-failed", 10).unwrap();
        export.finish(Err("Missing media: clip.mov".into()));
        assert_eq!(row("agent-test-failed").state, TaskState::Failed);
    }
}
