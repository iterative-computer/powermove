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
}
pub struct Run {
    pub handle: Mutex<Option<pm_engine_service::jobs::ExportHandle>>,
    pub cancel: AtomicBool,
}
pub type Job = Arc<Run>;
impl Controller {
    pub fn begin_builtin_export(&mut self, options: Value, retry: bool, cx: &mut Context<Self>) {
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        if self.builtins.export_job.is_some() {
            // Preserve the actual in-flight run and its terminal ownership.
            return;
        }
        let saved = if retry {
            self.builtins.export_retry.clone()
        } else {
            None
        };
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
            let frozen = Frozen {
                publication: p,
                options: typed,
                media,
                resources,
                playhead,
                destination,
            };
            let current = this
                .update(cx, |v, _| {
                    let current = v
                        .builtins
                        .runtime
                        .as_ref()
                        .is_some_and(|r| Arc::ptr_eq(r, &runtime));
                    if current {
                        v.builtins.export_retry = Some(frozen.clone());
                    }
                    current
                })
                .unwrap_or(false);
            if !current {
                job.cancel.store(true, Ordering::Release);
            }
            let worker_job = job.clone();
            let executor = cx.background_executor().clone();
            let result = cx.background_executor().spawn(async move {
                execute(
                    frozen,
                    worker_job,
                    activity,
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
                    if let Err(error) = &result {
                        v.message = error.clone();
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
    activity: Arc<AtomicBool>,
    runtime: Arc<pm_builtins::Builtins>,
    executor: gpui::BackgroundExecutor,
    tasks: Arc<Mutex<BackgroundTasks>>,
) -> Result<(), String> {
    let planned = pm_builtins::delivery::plan(
        &frozen.publication.snapshot,
        &frozen.options,
        frozen.playhead,
    );
    let mut ticket = None;
    let mut result = match planned {
        Err(error) => Err(error),
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
                Err(error) => Err(error),
                Ok(started) => {
                    ticket = Some(started);
                    execute_inner(
                        &frozen,
                        &job,
                        activity,
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
    if let Some(mut handle) = cleanup {
        let cancelled = handle.cancel().map_err(|e| e.to_string());
        let reaped = handle.wait().map_err(|e| e.to_string());
        if let Err(error) = cancelled.and(reaped.map(|_| ())) {
            result = Err(format!(
                "{}; export cleanup failed: {error}",
                result.err().unwrap_or_default()
            ));
        }
    }
    if let Some(ticket) = ticket {
        let settled = {
            let mut tasks = tasks.lock().unwrap();
            if result.as_ref().is_err_and(|e| e == "cancelled") {
                tasks
                    .cancel(ticket.id)
                    .and_then(|_| tasks.settle_control(ticket))
            } else {
                tasks.finish(
                    ticket,
                    result.as_ref().map(|_| ()).map_err(|e| e.clone().into()),
                )
            }
        };
        if let Err(error) = settled {
            result = Err(format!("Export task could not settle: {error:?}"));
        }
    }
    match &result {
        Err(error) if job.cancel.load(Ordering::Acquire) && error == "cancelled" => {
            observe(&runtime, json!({"phase":"cancelled"})).await
        }
        Err(error) => observe(&runtime, json!({"phase":"failed","message":error})).await,
        Ok(path) => observe(&runtime, json!({"phase":"done","path":path})).await,
    }
    result.map(|_| ())
}
struct TaskAdmission<'a> {
    tasks: &'a Arc<Mutex<BackgroundTasks>>,
    ticket: pm_promedia::tasks::Ticket,
}

async fn execute_inner(
    frozen: &Frozen,
    job: &Job,
    activity: Arc<AtomicBool>,
    runtime: &pm_builtins::Builtins,
    executor: &gpui::BackgroundExecutor,
    task: TaskAdmission<'_>,
    planned: &pm_builtins::delivery::Plan,
) -> Result<String, String> {
    let TaskAdmission { tasks, ticket } = task;
    if job.cancel.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    let intent = frozen.options.intent(planned.has_audio, planned.captions)?;
    if intent.sidecar != "none" {
        return Err("Caption sidecar delivery is not connected".into());
    }
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
            composition: frozen
                .publication
                .snapshot
                .open_comp()
                .unwrap_or_default()
                .as_str()
                .into(),
            settings: planned.settings.clone().ok_or("Missing render settings")?,
        },
        frozen.media.clone(),
        frozen.destination.clone(),
    )
    .map_err(|e| e.to_string())?;
    request.resources = (*frozen.resources).clone();
    let handle = pm_engine_service::jobs::ExportHandle::start_scheduled(
        &std::env::current_exe().map_err(|e| e.to_string())?,
        request,
        activity,
    )
    .map_err(|e| e.to_string())?;
    *job.handle.lock().unwrap() = Some(handle);
    let started = std::time::Instant::now();
    let mut cancellation_sent = false;
    loop {
        if job.cancel.load(Ordering::Acquire) && !cancellation_sent {
            cancellation_sent = true;
            let mut guard = job.handle.lock().unwrap();
            guard
                .as_mut()
                .ok_or("Export worker lost")?
                .cancel()
                .map_err(|e| e.to_string())?;
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
                JobState::Done => final_status
                    .path
                    .ok_or("Export completed without an output path".into()),
                JobState::Cancelled => Err("cancelled".into()),
                _ => {
                    let error = final_status.error.map(|e| e.message).unwrap_or_else(|| {
                        "Powermove lost contact with the export engine. Try again.".into()
                    });
                    Err(error)
                }
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
        } else if status.state == JobState::Running {
            json!({"phase":"rendering","frame":frame,"total":planned.frames,"elapsed":started.elapsed().as_secs_f64()})
        } else {
            json!({"phase":"preparing"})
        };
        observe(runtime, run).await;
        executor.timer(std::time::Duration::from_millis(50)).await;
    }
}
