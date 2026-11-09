//! View › Render Selection, Keep Rendered and Delete Rendered Frames: the
//! disk render cache's commands (`pm_engine_service::render_cache`). A
//! Render Selection is a Background Tasks row with frame progress and Cancel.
use crate::session::Controller;
use gpui::Context;
use pm_promedia::tasks::{ProgressUnit, TaskKind, TaskSpec, TaskState};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// The engine's render cache, for the timeline's render bar.
static DISK: OnceLock<pm_engine_service::render_cache::DiskFrames> = OnceLock::new();
/// Timelines waiting for the render bar to change.
static WATCHERS: Mutex<Vec<async_channel::Sender<()>>> = Mutex::new(Vec::new());

/// The session's engine cache; its changes wake every [`watch`]er.
pub(crate) fn install(disk: pm_engine_service::render_cache::DiskFrames) {
    disk.set_notify(Some(std::sync::Arc::new(|| {
        WATCHERS
            .lock()
            .unwrap()
            .retain(|tx| !tx.is_closed() && (tx.try_send(()).is_ok() || tx.is_full()));
    })));
    let _ = DISK.set(disk);
}

/// Wakes when the render bar changes.
pub(crate) fn watch() -> async_channel::Receiver<()> {
    let (tx, rx) = async_channel::bounded(1);
    WATCHERS.lock().unwrap().push(tx);
    rx
}

/// The render bar of `composition` (`None`: the open root composition), as
/// the timeline paints it.
pub(crate) fn bars(composition: Option<&str>) -> Vec<pm_panel_timeline::render::Bar> {
    use pm_panel_timeline::render::{Bar, BarState};
    let Some(bars) = DISK.get().map(|d| d.bars()) else {
        return vec![];
    };
    if composition.is_some() && bars.composition.as_deref() != composition {
        return vec![];
    }
    bars.bars
        .iter()
        .map(|&(from, to, state)| Bar {
            from,
            to,
            state: match state {
                pm_promedia::cache::BarState::Missing => BarState::Missing,
                pm_promedia::cache::BarState::Rendering => BarState::Rendering,
                pm_promedia::cache::BarState::Rendered => BarState::Rendered,
                pm_promedia::cache::BarState::Kept => BarState::Kept,
            },
        })
        .collect()
}

impl Controller {
    /// The selected layer's span in the open composition, or its work area.
    fn render_range(&self) -> Option<(f64, f64)> {
        let p = self.published.as_ref()?;
        let root = p.snapshot.root()?;
        let comp = pm_kernel::model::comp_node(root, p.snapshot.open_comp().as_deref())?;
        let number = |n: Option<&pm_model::Node>| n.and_then(|n| n.to_value().as_f64());
        if let Some(id) = &self.selected
            && let Some(layer) = pm_kernel::model::layers_of(comp)
                .find(|l| l.get("id").and_then(crate::node_string) == Some(id.as_str()))
        {
            let from = number(layer.get("from")).unwrap_or(0.);
            let dur = number(layer.get("dur")).unwrap_or(0.);
            return (dur > 0.).then_some((from, from + dur));
        }
        let duration = number(comp.get("dur")).unwrap_or(0.);
        let work = comp.get("work");
        let from = number(work.and_then(|w| w.at(0))).unwrap_or(0.);
        let to = number(work.and_then(|w| w.at(1))).unwrap_or(duration);
        (to > from).then_some((from, to))
    }

    pub(crate) fn render_selection(&mut self, cx: &mut Context<Self>) {
        let Some((from, to)) = self.render_range() else {
            self.message = "Nothing to render".into();
            cx.notify();
            return;
        };
        let Some(p) = self.published.clone() else {
            return;
        };
        let disk = self.engine.render_cache();
        let activity = self.engine.export_activity();
        let tasks = self.builtins.export_tasks.clone();
        let title = p
            .snapshot
            .root()
            .and_then(|r| r.get("name"))
            .and_then(crate::node_string)
            .map_or("Render".to_owned(), |name| format!("Render {name}"));
        // Spawning the child serializes the document: off the main thread.
        let fill = disk.clone();
        let started = cx.background_executor().spawn(async move {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let frozen = (
                p.revision(),
                p.clone() as std::sync::Arc<dyn std::any::Any + Send + Sync>,
            );
            let queued = fill.start_fill(from, to, &exe, activity, Some(frozen))?;
            if queued > 0 {
                follow(fill, tasks, p.project.clone(), title, queued);
            }
            Ok::<u64, String>(queued)
        });
        cx.spawn(async move |this, cx| {
            let queued = match started.await {
                Ok(0) => Err("Already rendered".to_owned()),
                Ok(n) => Ok(n),
                Err(e) => Err(e),
            };
            let total = match queued {
                Ok(total) => total,
                Err(message) => {
                    let _ = this.update(cx, |v, cx| {
                        v.message = message;
                        cx.notify();
                    });
                    return;
                }
            };
            // The status line counts the frames as they render and clears when
            // the render ends (finished, cancelled or failed; Background Tasks
            // keeps the outcome), unless something else has spoken since.
            let mut shown = String::new();
            loop {
                let bars = disk.bars();
                let next = if bars.running {
                    format!("Rendering {} of {total} frames", bars.done.min(total))
                } else {
                    String::new()
                };
                let ours = this
                    .update(cx, |v, cx| {
                        let ours = shown.is_empty() || v.message == shown;
                        if ours && v.message != next {
                            v.message = next.clone();
                            cx.notify();
                        }
                        ours
                    })
                    .unwrap_or(false);
                if !ours || !bars.running {
                    return;
                }
                shown = next;
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
            }
        })
        .detach();
    }

    pub(crate) fn keep_rendered(&mut self, cx: &mut Context<Self>) {
        let Some((from, to)) = self.render_range() else {
            return;
        };
        let disk = self.engine.render_cache();
        // Disk work: off the main thread.
        let done = cx
            .background_executor()
            .spawn(async move { disk.keep_range(from, to, true) });
        cx.spawn(async move |this, cx| {
            let message = match done.await {
                Ok(0) => "Render the selection first, then keep it".to_owned(),
                Ok(n) => format!("Kept {n} rendered frames"),
                Err(e) => format!("Couldn't keep the rendered frames: {e}"),
            };
            let _ = this.update(cx, |v, cx| {
                v.message = message;
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn delete_rendered(&mut self, cx: &mut Context<Self>) {
        let disk = self.engine.render_cache();
        let done = cx
            .background_executor()
            .spawn(async move { disk.delete_all() });
        cx.spawn(async move |this, cx| {
            let message = match done.await {
                Ok(0) => "No rendered frames".to_owned(),
                Ok(n) => format!("Deleted {n} rendered frames"),
                Err(e) => format!("Couldn't delete the rendered frames: {e}"),
            };
            let _ = this.update(cx, |v, cx| {
                v.message = message;
                cx.notify();
            });
        })
        .detach();
    }
}

/// A Render Selection's Background Tasks row: frame progress from the
/// render cache, Cancel stopping its child, and a failure (a full disk or
/// limit, a frame that could not render) reported as one.
fn follow(
    disk: pm_engine_service::render_cache::DiskFrames,
    tasks: std::sync::Arc<Mutex<pm_promedia::tasks::BackgroundTasks>>,
    project: String,
    title: String,
    total: u64,
) {
    use pm_promedia::tasks::{TaskFailure, TaskFailureCode};
    let ticket = {
        let mut tasks = tasks.lock().unwrap();
        tasks
            .enqueue(
                project.clone(),
                project,
                TaskSpec {
                    title,
                    kind: TaskKind::Render,
                    unit: ProgressUnit::Frames,
                    priority: 0,
                    total,
                },
            )
            .ok()
            .and_then(|id| tasks.start_next().filter(|t| t.id == id))
    };
    let Some(ticket) = ticket else { return };
    std::thread::Builder::new()
        .name("pm-render-task".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let state = tasks.lock().unwrap().worker_state(ticket);
                match state {
                    Ok(TaskState::CancelRequested | TaskState::PauseRequested) => {
                        disk.cancel_fill();
                        let _ = tasks.lock().unwrap().settle_control(ticket);
                        return;
                    }
                    Ok(TaskState::Running) => {}
                    _ => return,
                }
                let bars = disk.bars();
                let mut tasks = tasks.lock().unwrap();
                let _ = tasks.progress(ticket, bars.done.min(total), total);
                if bars.running {
                    continue;
                }
                let result = match &bars.error {
                    None if bars.done >= total => Ok(()),
                    None => Err(TaskFailure {
                        code: TaskFailureCode::Unknown,
                        diagnostic: "The render stopped before every frame was rendered".into(),
                    }),
                    Some(error) => Err(TaskFailure {
                        code: if bars.disk_full {
                            TaskFailureCode::DiskFull
                        } else {
                            TaskFailureCode::Unknown
                        },
                        diagnostic: error.clone(),
                    }),
                };
                let _ = tasks.finish(ticket, result);
                return;
            }
        })
        .ok();
}
