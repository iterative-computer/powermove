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
    path::{Path, PathBuf},
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
    /// The person said Replace: the files there are replaced once the export
    /// succeeded, the old ones archived. A retry keeps the answer.
    replace: bool,
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
        // NSSavePanel asks before choosing a file that exists; the folder
        // panel (and a development review folder) never does.
        let panel_asks = typed.format != Format::Png && !crate::platform::reviewing_saves();
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
                Some((frozen.destination, frozen.replace))
            } else if let Some(picker) = picker
                && let Some(destination) = picker.recv().await.ok().flatten()
            {
                // What the export would replace is one decision: the save
                // panel's Replace answers for the video alone; caption files
                // (or no panel) take one question listing every file.
                let files = sidecar_format(&typed)
                    .map(|format| sidecar_files(&p, &typed.range, format, &destination))
                    .unwrap_or_default();
                let existing = existing_outputs(&destination, &files);
                if existing.is_empty() {
                    Some((destination, false))
                } else if existing == [destination.clone()] && panel_asks {
                    Some((destination, true))
                } else {
                    let asked = this.update(cx, |v, _| {
                        replace_question(&existing).sheet(&v.host.borrow().window)
                    });
                    match asked {
                        Ok(rx) if rx.recv().await == Ok(Some(true)) => Some((destination, true)),
                        _ => None,
                    }
                }
            } else {
                None
            };
            let Some((destination, replace)) = destination else {
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
                replace,
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
            let began = std::time::Instant::now();
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
                        Ok(finished) => {
                            v.close_export_sheet(cx);
                            let line = finished.line();
                            // A long export ends while the user is
                            // elsewhere, and a file with something to say
                            // (its sound clipped) is worth reading: the line
                            // stays until something replaces it, and the
                            // Dock icon asks once.
                            if began.elapsed() > std::time::Duration::from_secs(30) {
                                v.message = line;
                                crate::platform::request_attention();
                            } else if !finished.notes.is_empty() {
                                v.message = line;
                            } else {
                                v.note(line, cx);
                            }
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
) -> Result<Finished, String> {
    let planned = pm_builtins::delivery::plan(
        &frozen.publication.snapshot,
        &frozen.options,
        frozen.playhead,
    );
    let mut ticket = None;
    let mut result: Result<Finished, Failure> = match planned {
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
                json!({"phase":"failed","message":plain_failure(&error.message),"offline":offline}),
            )
            .await
        }
        Ok(finished) => {
            observe(
                &runtime,
                json!({"phase":"done","path":finished.path,"notes":finished.notes}),
            )
            .await
        }
    }
    result.map_err(|f| f.message)
}
/// A finished export: its file, and what the person is told with it (the
/// sound clipped; fine detail the data rate did not keep).
struct Finished {
    path: String,
    notes: Vec<String>,
}
impl Finished {
    /// "Exported Film.mov", or "Exported Film.mov." with each note after it.
    fn line(&self) -> String {
        let name = std::path::Path::new(&self.path)
            .file_name()
            .map_or_else(|| self.path.clone(), |n| n.to_string_lossy().into_owned());
        if self.notes.is_empty() {
            return format!("Exported {name}");
        }
        std::iter::once(format!("Exported {name}."))
            .chain(self.notes.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
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
/// captions layers, in times from the start of the exported range. Each
/// path with its text; none when the project has no captions.
pub(crate) fn sidecar_files(
    publication: &pm_kernel::Published,
    exported_range: &str,
    format: pm_captions::CaptionFormat,
    video: &Path,
) -> Vec<(PathBuf, String)> {
    let Some(doc) = crate::captions_panel::read(publication) else {
        return vec![];
    };
    let range = match doc.work.filter(|_| exported_range == "work") {
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
    crate::captions_panel::sidecars(&doc, format, range)
        .into_iter()
        .map(|(suffix, text)| {
            let name = match suffix {
                Some(tag) => format!("{base}.{tag}.{extension}"),
                None => format!("{base}.{extension}"),
            };
            (video.with_file_name(name), text)
        })
        .collect()
}
/// The caption format the sheet's options write beside a video, if any.
fn sidecar_format(options: &Options) -> Option<pm_captions::CaptionFormat> {
    match options.captions_sidecar.as_str() {
        "srt" if options.format.video() => Some(pm_captions::CaptionFormat::Srt),
        "vtt" if options.format.video() => Some(pm_captions::CaptionFormat::Vtt),
        _ => None,
    }
}
/// What an export to `video` would replace: the video, then its caption
/// files, those that are already there.
pub(crate) fn existing_outputs(video: &Path, sidecars: &[(PathBuf, String)]) -> Vec<PathBuf> {
    std::iter::once(video)
        .chain(sidecars.iter().map(|(path, _)| path.as_path()))
        .filter(|path| std::fs::symlink_metadata(path).is_ok())
        .map(Path::to_path_buf)
        .collect()
}
/// The file's name, as a question or a refusal names it.
pub(crate) fn shown_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}
/// Publishes the caption files as the video is: each through a temporary
/// beside it, never over a file the export was not told to replace (one
/// that appeared meanwhile included); a replaced file is kept in the export
/// archive.
pub(crate) fn write_sidecars(files: &[(PathBuf, String)], replace: bool) -> Result<(), String> {
    let overwrite = if replace {
        pm_export::Overwrite::Replace
    } else {
        pm_export::Overwrite::Fail
    };
    for (path, text) in files {
        let refused = |e: pm_export::Error| match e {
            pm_export::Error::Exists(_) => format!(
                "The video was exported, but “{}” is already there, so its captions weren’t written over it.",
                shown_name(path)
            ),
            _ => "The video was exported, but its caption file couldn’t be written.".to_owned(),
        };
        let output =
            pm_export::atomic::AtomicOutput::new(path, overwrite, None).map_err(refused)?;
        std::fs::write(output.temp_path(), text.as_bytes())
            .map_err(|e| refused(pm_export::Error::Io(e.to_string())))?;
        output.commit().map_err(refused)?;
    }
    Ok(())
}
/// One question for every file an export would replace, the video and its
/// caption files together; Cancel is the default.
fn replace_question(existing: &[PathBuf]) -> crate::alert::Alert<bool> {
    use crate::alert::{Alert, Button};
    let names: Vec<String> = existing
        .iter()
        .map(|p| format!("“{}”", shown_name(p)))
        .collect();
    let (title, list) = match names.as_slice() {
        [one] => (format!("Replace {one}?"), format!("{one} already exists.")),
        [rest @ .., last] => (
            format!("Replace {} files?", names.len()),
            format!("{} and {last} already exist.", rest.join(", ")),
        ),
        [] => (String::new(), String::new()),
    };
    Alert::new(
        title,
        format!("{list} The ones replaced are kept in the folder’s export archive."),
        vec![
            Button::new("Cancel", false).default().cancel(),
            Button::new("Replace", true).destructive(),
        ],
    )
}
async fn execute_inner(
    frozen: &Frozen,
    job: &Job,
    engine: EngineJob,
    runtime: &pm_builtins::Builtins,
    executor: &gpui::BackgroundExecutor,
    task: TaskAdmission<'_>,
    planned: &pm_builtins::delivery::Plan,
) -> Result<Finished, Failure> {
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
            frozen.replace,
            &job.cancel,
        )?;
        return Ok(Finished {
            path: path.to_string_lossy().into(),
            notes: vec![],
        });
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
    request.replace = frozen.replace;
    // The sheet's per-export 3D quality; the composition's saved one otherwise.
    request.studio_quality = frozen
        .options
        .opaque
        .get("studioQuality")
        .filter(|q| q.as_str().is_some_and(|q| q != "composition"))
        .cloned();
    request.studio_render_scale = frozen
        .options
        .opaque
        .get("studioReducedSize")
        .and_then(serde_json::Value::as_bool)
        .filter(|v| *v)
        .map(|_| 0.75);
    let mut session = pm_engine_service::jobs::JobSession::new(request);
    // Its raster overrides answer from this publication, whatever is edited
    // meanwhile; while their extensions are slow, the sheet says what for.
    let waiting = engine
        .overrides
        .map(|overrides| overrides.pin(&mut session, &frozen.publication, false));
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
                        let files = sidecar_files(
                            &frozen.publication,
                            &frozen.options.range,
                            format,
                            Path::new(&path),
                        );
                        write_sidecars(&files, frozen.replace)?;
                    }
                    Ok(Finished {
                        path,
                        notes: final_status.notes,
                    })
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
            let elapsed = started.elapsed().as_secs_f64();
            let mut run =
                json!({"phase":"rendering","frame":frame,"total":planned.frames,"elapsed":elapsed});
            if let Some(note) = waiting.as_ref().and_then(|w| w.lock().unwrap().clone()) {
                let mut note = note.chars();
                run["extra"] = json!(
                    note.next()
                        .map(|c| c.to_uppercase().chain(note).collect::<String>())
                        .unwrap_or_default()
                );
            }
            // The sheet says "About N min left" from the pace so far (it
            // waits for a few frames before trusting it).
            if frame > 0 && frame < planned.frames {
                run["eta"] = json!(elapsed / frame as f64 * (planned.frames - frame) as f64);
            }
            run
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

    #[test]
    fn a_finished_export_line_carries_what_the_person_is_told() {
        let plain = Finished {
            path: "/out/Film.mov".into(),
            notes: vec![],
        };
        assert_eq!(plain.line(), "Exported Film.mov");
        let clipped = Finished {
            path: "/out/Film.mov".into(),
            notes: vec!["Audio clipped: 1,204 samples over full scale, peak +2.1 dB.".into()],
        };
        assert_eq!(
            clipped.line(),
            "Exported Film.mov. Audio clipped: 1,204 samples over full scale, peak +2.1 dB."
        );
    }
    /// The archived copies of `name` in `folder`'s export archive.
    fn archived(folder: &Path, name: &str) -> Vec<String> {
        std::fs::read_dir(folder.join(".powermove-export-archive"))
            .map(|entries| {
                entries
                    .map(|e| e.unwrap().path())
                    .filter(|p| p.to_string_lossy().ends_with(name))
                    .map(|p| std::fs::read_to_string(p).unwrap())
                    .collect()
            })
            .unwrap_or_default()
    }
    /// A hand-corrected Film.srt beside the export is never written over
    /// unless the person said Replace; then the old one is archived.
    #[test]
    fn caption_files_replace_nothing_they_were_not_told_to() {
        let dir = tempfile::tempdir().unwrap();
        let (video, srt) = (dir.path().join("Film.mp4"), dir.path().join("Film.srt"));
        let fresh = dir.path().join("Film.en.srt");
        std::fs::write(&srt, "1\nhand corrected\n").unwrap();
        let files = vec![
            (fresh.clone(), "1\nnew en\n".to_owned()),
            (srt.clone(), "1\nnew\n".to_owned()),
        ];
        assert_eq!(existing_outputs(&video, &files), vec![srt.clone()]);
        let refused = write_sidecars(&files, false).unwrap_err();
        assert!(refused.contains("“Film.srt” is already there"), "{refused}");
        assert_eq!(
            std::fs::read_to_string(&srt).unwrap(),
            "1\nhand corrected\n"
        );
        assert!(archived(dir.path(), "Film.srt").is_empty());
        write_sidecars(&files, true).unwrap();
        assert_eq!(std::fs::read_to_string(&srt).unwrap(), "1\nnew\n");
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "1\nnew en\n");
        assert_eq!(
            archived(dir.path(), "Film.srt"),
            vec!["1\nhand corrected\n"]
        );
        // Nothing but the files and the archive is left beside them.
        let mut names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [".powermove-export-archive", "Film.en.srt", "Film.srt"]
        );
    }
    /// The video and its caption files are one question; Cancel is the default.
    #[test]
    fn one_question_names_every_file_an_export_would_replace() {
        let files = [PathBuf::from("/d/Film.mp4"), PathBuf::from("/d/Film.srt")];
        let alert = replace_question(&files);
        assert_eq!(alert.message, "Replace 2 files?");
        assert!(
            alert
                .informative
                .starts_with("“Film.mp4” and “Film.srt” already exist."),
            "{}",
            alert.informative
        );
        let one = replace_question(&files[1..]);
        assert_eq!(one.message, "Replace “Film.srt”?");
        let default = one.buttons.iter().find(|b| b.default);
        assert_eq!(
            default.map(|b| (b.label, b.action)),
            Some(("Cancel", false))
        );
        let replace = one.buttons.iter().find(|b| b.action);
        assert!(replace.is_some_and(|b| b.destructive && !b.default));
    }
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

/// The sheet says what happened in words; the engine's own wording stays in
/// the log. A render worker that dies (crash, out of memory, killed) reports
/// only that it ended without a result.
fn plain_failure(message: &str) -> String {
    if message.contains("without a terminal status") {
        eprintln!("Export: {message}");
        return "The renderer stopped unexpectedly. Try again; if it stops again, \
                use a lower 3D Quality or a smaller size."
            .into();
    }
    message.into()
}
