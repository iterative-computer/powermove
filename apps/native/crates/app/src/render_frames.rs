//! View › Render Selection, Keep Rendered and Delete Rendered Frames: the
//! disk render cache's commands (`pm_engine_service::render_cache`). A
//! Render Selection is a Background Tasks row with frame progress and Cancel.
use crate::session::Controller;
use gpui::Context;
use pm_promedia::tasks::{ProgressUnit, TaskKind, TaskSpec, TaskState};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// How long a Render Selection asked during loading waits, once extensions
/// have settled, for the viewer to draw the settled composition.
const SETTLED_FRAME_WAIT: Duration = Duration::from_secs(10);
/// What a waiting Render Selection was asked for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Asked {
    doc_id: u64,
    composition: String,
    range: (f64, f64),
}
/// One look at a waiting Render Selection.
#[derive(Debug, PartialEq)]
enum Wait {
    /// Still waiting, with the status line that says for what.
    Keep(String),
    /// The window shows another project or composition now (or the wait was
    /// cancelled): it never starts.
    Drop,
    Start,
}
/// `now`: what a Render Selection would be asked for at this moment (`None`
/// once the wait was cancelled or nothing is open). `settled_for`: how long
/// the window's extensions have been loaded.
fn wait_step(
    asked: &Asked,
    now: Option<&Asked>,
    resources: &pm_engine_service::resources::ResourceSnapshot,
    observed: bool,
    settled_for: Option<Duration>,
) -> Wait {
    let same =
        now.is_some_and(|now| now.doc_id == asked.doc_id && now.composition == asked.composition);
    if !same {
        return Wait::Drop;
    }
    match settled_for {
        None => Wait::Keep(crate::extensions::starts_once_loaded(
            "Render Selection",
            resources,
        )),
        // Past the wait it starts and says what is in the way, if anything.
        Some(settled) if observed || settled >= SETTLED_FRAME_WAIT => Wait::Start,
        Some(_) => Wait::Keep("Reading the composition to render".to_owned()),
    }
}

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

/// Background rendering's fills: this executable's `--engine-job`, raster
/// overrides answered by the window whose document it renders, and none
/// while an export renders. Each window calls it; the first host stays.
pub(crate) fn serve_background(
    engine: &pm_engine_service::EngineService,
    windows: crate::raster_overrides::OverrideWindows,
) {
    // Unit tests' executables are not the app and render no jobs.
    if cfg!(test) {
        return;
    }
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    engine.render_cache().set_background_host(Some(
        pm_engine_service::render_cache::BackgroundHost {
            executable,
            pin: std::sync::Arc::new(move |doc_id, session| {
                if let Some(jobs) = windows.of(doc_id) {
                    jobs.pin_current(session);
                }
            }),
            busy: std::sync::Arc::new(pm_engine_service::jobs::exports_running),
        },
    ));
}

/// Wakes when the render bar changes.
pub(crate) fn watch() -> async_channel::Receiver<()> {
    let (tx, rx) = async_channel::bounded(1);
    WATCHERS.lock().unwrap().push(tx);
    rx
}

/// Why background rendering is not rendering, when the disk is why.
pub(crate) fn background_paused_note() -> Option<String> {
    let bars = DISK.get()?.bars();
    (bars.background && bars.disk_full)
        .then(|| bars.error.clone())
        .flatten()
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
    /// The selected layer's span in the open composition (the part inside
    /// it), or its work area.
    fn render_range(&self) -> Option<(f64, f64)> {
        let p = self.published.as_ref()?;
        let root = p.snapshot.root()?;
        let comp = pm_kernel::model::comp_node(root, p.snapshot.open_comp().as_deref())?;
        let number = |n: Option<&pm_model::Node>| n.and_then(|n| n.to_value().as_f64());
        let duration = number(comp.get("dur")).unwrap_or(0.);
        if let Some(id) = &self.selected
            && let Some(layer) = pm_kernel::model::layers_of(comp)
                .find(|l| l.get("id").and_then(crate::node_string) == Some(id.as_str()))
        {
            let from = number(layer.get("from")).unwrap_or(0.);
            let dur = number(layer.get("dur")).unwrap_or(0.);
            return within_composition(from, from + dur, duration);
        }
        let work = comp.get("work");
        let from = number(work.and_then(|w| w.at(0))).unwrap_or(0.);
        let to = number(work.and_then(|w| w.at(1))).unwrap_or(duration);
        (to > from).then_some((from, to))
    }

    /// The project, composition and range a Render Selection is asked for now.
    fn render_ask(&self) -> Option<Asked> {
        let range = self.render_range()?;
        let p = self.published.as_ref()?;
        Some(Asked {
            doc_id: p.snapshot.doc_id,
            composition: p.snapshot.render_comp(),
            range,
        })
    }
    /// The viewer has drawn a settled frame of what `asked` names, as the
    /// window's project and extensions are now.
    fn render_observed(&self, asked: &Asked) -> bool {
        self.published.as_ref().is_some_and(|p| {
            self.engine.render_cache().observed(
                asked.doc_id,
                p.snapshot.revision,
                &asked.composition,
                &self.extensions.resources.definitions,
            )
        })
    }
    /// Asked while this window's extensions load, a Render Selection waits
    /// for them (its frames would otherwise be drawn without effects still
    /// loading, or find nothing to render), then for the viewer's settled
    /// frame of that composition, then starts. It is bound to the project,
    /// composition and range it was asked for: when the window shows
    /// another composition or project meanwhile it is dropped, never
    /// started on what is open then. Asking again replaces it.
    pub(crate) fn render_selection(&mut self, cx: &mut Context<Self>) {
        let Some(asked) = self.render_ask() else {
            self.message = "Nothing to render".into();
            cx.notify();
            return;
        };
        if !self.extensions.resources.pending && self.render_observed(&asked) {
            return self.start_render_selection(asked, cx);
        }
        self.render_wait += 1;
        let wait = self.render_wait;
        cx.spawn(async move |this, cx| {
            let mut shown = String::new();
            let mut settled_at = None;
            loop {
                let Ok(step) = this.update(cx, |v, cx| {
                    let resources = &v.extensions.resources;
                    let since = (!resources.pending)
                        .then(|| *settled_at.get_or_insert_with(std::time::Instant::now));
                    let step = wait_step(
                        &asked,
                        (v.render_wait == wait)
                            .then(|| v.render_ask())
                            .flatten()
                            .as_ref(),
                        resources,
                        v.render_observed(&asked),
                        since.map(|since| since.elapsed()),
                    );
                    // Only our own line is replaced; anything said since stays.
                    let ours = shown.is_empty() || v.message == shown;
                    match &step {
                        Wait::Keep(text) if ours && v.message != *text => {
                            v.message = text.clone();
                            cx.notify();
                        }
                        Wait::Drop | Wait::Start if v.message == shown => {
                            v.message.clear();
                            cx.notify();
                        }
                        _ => {}
                    }
                    if step == Wait::Start {
                        v.start_render_selection(asked.clone(), cx);
                    }
                    step
                }) else {
                    return;
                };
                match step {
                    Wait::Keep(text) => shown = text,
                    Wait::Drop | Wait::Start => return,
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
        })
        .detach();
    }
    fn start_render_selection(&mut self, asked: Asked, cx: &mut Context<Self>) {
        let (from, to) = asked.range;
        let Some(p) = self
            .published
            .clone()
            .filter(|p| p.snapshot.doc_id == asked.doc_id)
        else {
            return;
        };
        let disk = self.engine.render_cache();
        let activity = self.engine.export_activity();
        let overrides = self.extensions.overrides.as_ref().map(|o| o.jobs());
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
            // Its overrides answer from the window's publication, whatever is
            // edited meanwhile.
            let (doc_id, comp) = (asked.doc_id, asked.composition.as_str());
            let queued = fill.start_fill_of(doc_id, comp, from, to, &exe, activity, |session| {
                if let Some(overrides) = overrides {
                    overrides.pin(session, &p, false);
                }
            })?;
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
                Ok(kept) => keep_message(kept),
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

/// `[from, to)` cut to the composition's `[0, duration)`; `None` when no
/// part of it is inside (a clip longer than the composition renders only the
/// frames the composition shows).
fn within_composition(from: f64, to: f64, duration: f64) -> Option<(f64, f64)> {
    let (from, to) = (from.max(0.), to.min(duration));
    (to > from).then_some((from, to))
}

/// The status line after Keep Rendered.
fn keep_message(kept: pm_engine_service::render_cache::Kept) -> String {
    match (kept.changed, kept.already) {
        (0, 0) => "Render the selection first, then keep it".to_owned(),
        (0, n) => format!("Already kept: {n} rendered frames"),
        (n, _) => format!("Kept {n} rendered frames"),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A Render Selection asked while extensions load is of the project and
    /// composition it was asked for: it waits for the extensions, then for
    /// the viewer's settled frame of that composition, and is dropped (never
    /// started on something else) once the window shows another composition
    /// or project, or nothing.
    #[test]
    fn a_waiting_render_selection_is_bound_to_its_project_and_composition() {
        use pm_engine_service::resources::ResourceSnapshot;
        let asked = Asked {
            doc_id: 7,
            composition: "main".into(),
            range: (1., 4.),
        };
        let loading = ResourceSnapshot {
            pending: true,
            loading: vec![("glow".into(), "Glow".into())],
            ..Default::default()
        };
        let loaded = ResourceSnapshot::default();
        let second = Duration::from_secs(1);
        // Still what was asked for (the selection may have moved: the range
        // asked for is the one rendered).
        let same = Asked {
            range: (0., 9.),
            ..asked.clone()
        };
        assert_eq!(
            wait_step(&asked, Some(&same), &loading, false, None),
            Wait::Keep("Render Selection starts once Glow has loaded".into())
        );
        assert_eq!(
            wait_step(&asked, Some(&same), &loaded, false, Some(second)),
            Wait::Keep("Reading the composition to render".into())
        );
        assert_eq!(
            wait_step(&asked, Some(&same), &loaded, true, Some(second)),
            Wait::Start
        );
        // An observation left by another project is not this one's: without
        // its own the wait goes on, to its bound.
        assert_eq!(
            wait_step(
                &asked,
                Some(&same),
                &loaded,
                false,
                Some(SETTLED_FRAME_WAIT)
            ),
            Wait::Start
        );
        // Another composition, another project, nothing open, or replaced.
        let other_comp = Asked {
            composition: "precomp".into(),
            ..asked.clone()
        };
        let other_project = Asked {
            doc_id: 8,
            ..asked.clone()
        };
        for now in [Some(&other_comp), Some(&other_project), None] {
            for (resources, observed, settled) in [
                (&loading, false, None),
                (&loaded, true, Some(second)),
                (&loaded, false, Some(SETTLED_FRAME_WAIT)),
            ] {
                assert_eq!(
                    wait_step(&asked, now, resources, observed, settled),
                    Wait::Drop,
                    "{now:?}"
                );
            }
        }
    }

    #[test]
    fn a_selected_clip_renders_only_inside_the_composition() {
        assert_eq!(within_composition(0., 20., 10.), Some((0., 10.)));
        assert_eq!(within_composition(-2., 4., 10.), Some((0., 4.)));
        assert_eq!(within_composition(12., 15., 10.), None);
        assert_eq!(within_composition(3., 3., 10.), None);
    }

    #[test]
    fn keeping_kept_frames_again_says_so() {
        use pm_engine_service::render_cache::Kept;
        let kept = |changed, already| keep_message(Kept { changed, already });
        assert_eq!(kept(0, 0), "Render the selection first, then keep it");
        assert_eq!(kept(0, 300), "Already kept: 300 rendered frames");
        assert_eq!(kept(12, 288), "Kept 12 rendered frames");
    }
}
