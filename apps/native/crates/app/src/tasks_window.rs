//! Window ▸ Background Tasks (⌘9; design surfaces/tasks-storage.md and the
//! pro-media review): exports and proxy/optimized media work, one row each,
//! with progress, Pause/Resume and Cancel. Right-click sets a task's
//! priority (an NSMenu), as plan section 1 asks; the row order stays the
//! queue's, so priority never reorders what a person is reading.
use crate::pro_media::{ProMedia, TaskAction};
use gpui::{prelude::*, *};
use pm_promedia::tasks::{BackgroundTasks, ProgressUnit, Task, TaskProgress, TaskState};
use pm_shell::{
    kit::{Kit, Recipes},
    motion::Motion,
    tokens,
};
use std::sync::{Arc, Mutex};

pub struct TasksWindow {
    motion: Motion<u64>,
    pro: Arc<ProMedia>,
    exports: Arc<Mutex<BackgroundTasks>>,
}

pub const TITLE: &str = "Background Tasks";
const EMPTY_TITLE: &str = "No Background Tasks";
const EMPTY_LINE: &str = "Exports and media optimisation appear here.";
/// Priority choices on a task's context menu.
const PRIORITIES: [(&str, i32); 3] = [("High", 10), ("Normal", 0), ("Low", -10)];

fn units(unit: Option<ProgressUnit>) -> &'static str {
    match unit {
        Some(ProgressUnit::Frames) => "frames",
        Some(ProgressUnit::Items) => "items",
        Some(ProgressUnit::Seconds) => "seconds",
        Some(ProgressUnit::Bytes) | None => "",
    }
}

/// The detail line under a task's title.
pub fn detail(task: &Task) -> String {
    let p = task.presentation();
    if let Some(word) = p.transition_label {
        return word.into();
    }
    match task.state {
        TaskState::Queued => "Waiting…".into(),
        TaskState::Paused => "Paused".into(),
        TaskState::Cancelled => "Cancelled".into(),
        TaskState::Completed => match task.kind {
            pm_promedia::tasks::TaskKind::Export => "Exported".into(),
            pm_promedia::tasks::TaskKind::Render => "Rendered".into(),
            pm_promedia::tasks::TaskKind::Import => "Imported".into(),
            pm_promedia::tasks::TaskKind::Captions => "Added".into(),
            _ => "Created".into(),
        },
        TaskState::Failed => p.error_message.unwrap_or("").into(),
        _ => match p.progress {
            TaskProgress::Indeterminate { .. } => "Working…".into(),
            TaskProgress::Determinate {
                completed,
                total,
                unit: Some(ProgressUnit::Bytes),
            } => format!(
                "{} of {}",
                crate::pro_media::bytes(completed),
                crate::pro_media::bytes(total)
            ),
            TaskProgress::Determinate {
                completed,
                total,
                unit,
            } => format!("{completed} of {total} {}", units(unit)),
        },
    }
}

impl TasksWindow {
    fn motion(&mut self) -> &mut Motion<u64> {
        &mut self.motion
    }
    /// Media tasks act through the service. Every other row is cancelled by
    /// kind and task id, never "whichever job this window has": a Render
    /// Selection row asks its task to stop (its follower stops the child); an
    /// export, the person's or an agent's, cancels exactly that export. Neither
    /// has a pause.
    fn act(&self, media: bool, id: pm_promedia::tasks::TaskId, action: TaskAction) {
        if media {
            let _ = self.pro.act(id, action);
        } else if action == TaskAction::Cancel {
            let tasks = crate::export_coordinator::tasks();
            let mut tasks = tasks.lock().unwrap();
            match tasks.get(id).map(|t| t.kind.clone()) {
                Some(pm_promedia::tasks::TaskKind::Render) => {
                    let _ = tasks.cancel(id);
                }
                Some(
                    pm_promedia::tasks::TaskKind::Export | pm_promedia::tasks::TaskKind::Captions,
                ) => {
                    drop(tasks);
                    crate::export_coordinator::cancel_export(id);
                }
                _ => {}
            }
        }
    }

    fn row(
        &self,
        task: &Task,
        kit: &Kit,
        media: bool,
        window_rows: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let t = tokens();
        let (c, m) = (&t.colors, &t.metrics);
        let p = task.presentation();
        let id = task.id;
        let failed = task.state == TaskState::Failed;
        let share = match p.progress {
            TaskProgress::Determinate {
                completed, total, ..
            } if !task.state.terminal() => Some(completed as f32 / total.max(1) as f32),
            _ => None,
        };
        let button =
            |label: &'static str, enabled: bool, action: TaskAction, cx: &mut Context<Self>| {
                kit.icon_button(
                    format!("{label}-{}-{}", media, id.0),
                    match action {
                        TaskAction::Resume => "play.circle",
                        TaskAction::Pause => "pause.circle",
                        _ => "xmark.circle.fill",
                    },
                    label,
                    false,
                )
                .when(!enabled, |d| kit.disabled(d))
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |v, _, _, cx| {
                        v.act(media, id, action);
                        cx.notify();
                    }))
                })
            };
        let mut trailing = div().flex().flex_none().gap(px(m.gap));
        if media && let Some(label) = p.pause_resume_label {
            let action = if label == "Resume" {
                TaskAction::Resume
            } else {
                TaskAction::Pause
            };
            trailing = trailing.child(button(label, p.pause_resume_enabled, action, cx));
        }
        if !task.state.terminal() {
            trailing = trailing.child(button("Cancel", p.cancel_enabled, TaskAction::Cancel, cx));
        }
        let row = div()
            .id(SharedString::from(format!("task-{media}-{}", id.0)))
            .flex()
            .items_center()
            .gap(px(m.inset))
            .py(px(m.gap))
            .when(media, |row| {
                row.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |_, _, window, cx| {
                        let options: Vec<serde_json::Value> = PRIORITIES
                        .iter()
                        .map(|(label, _)| serde_json::json!({"label": format!("{label} Priority")}))
                        .collect();
                        let entity = cx.entity().downgrade();
                        crate::v4_native::menu(window, &options, cx, move |chosen, cx| {
                            if let Some(index) = chosen
                                && let Some((_, priority)) = PRIORITIES.get(index)
                            {
                                let _ = entity.update(cx, |v, cx| {
                                    v.act(media, id, TaskAction::Priority(*priority));
                                    cx.notify();
                                });
                            }
                        });
                    }),
                )
            })
            .child(kit.symbol(
                if matches!(
                    task.kind,
                    pm_promedia::tasks::TaskKind::Export | pm_promedia::tasks::TaskKind::Render
                ) {
                    "film"
                } else {
                    "bolt.horizontal"
                },
                m.symbol_panel,
                pm_shell::symbol::Weight::Regular,
                c.secondary,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(m.gap / 2.))
                    .child(div().truncate().child(if p.title.is_empty() {
                        p.kind_label.to_owned()
                    } else {
                        p.title.to_owned()
                    }))
                    .when(!task.state.terminal(), |d| d.child(kit.progress(share)))
                    .child(
                        div()
                            .text_role(&t, pm_design::Text::Caption)
                            .text_color(rgb(if failed { c.danger } else { c.secondary }))
                            .child(
                                media
                                    .then(|| self.pro.waiting(id))
                                    .flatten()
                                    .unwrap_or_else(|| detail(task)),
                            ),
                    ),
            )
            .child(trailing);
        window_rows.push(row.into_any_element());
    }
}

impl Render for TasksWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.motion.frame(window);
        let t = tokens();
        let kit = Kit::new(&t, &self.motion, Self::motion, window, cx);
        let exports = self.exports.lock().unwrap().all();
        let media = self.pro.snapshot();
        let mut rows = Vec::new();
        for task in &exports {
            self.row(task, &kit, false, &mut rows, cx);
        }
        for task in &media {
            self.row(task, &kit, true, &mut rows, cx);
        }
        let body = div()
            .id("background-tasks")
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .when(rows.is_empty(), |d| {
                d.items_center().justify_center().child(kit.empty_state(
                    "checkmark.circle",
                    EMPTY_TITLE,
                    EMPTY_LINE,
                ))
            })
            .children(rows);
        kit.utility_window(TITLE, None, vec![], body.into_any_element(), vec![])
    }
}

impl crate::session::Controller {
    /// Window ▸ Background Tasks.
    pub(crate) fn open_tasks_window(&mut self, cx: &mut Context<Self>) {
        if let Some(window) = cx
            .windows()
            .into_iter()
            .find(|w| w.downcast::<TasksWindow>().is_some())
        {
            let _ = window.update(cx, |_, w, _| w.activate());
            return;
        }
        let t = tokens();
        let pro = self.pro_media.clone();
        let exports = self.builtins.export_tasks.clone();
        let mut entity = None;
        let _ = cx.open_window(
            WindowOptions::new()
                .titlebar(Some(TitlebarOptions {
                    title: Some(TITLE.into()),
                    ..Default::default()
                }))
                .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                    point(px(t.metrics.toolbar), px(t.metrics.toolbar)),
                    size(px(t.metrics.palette_width), px(t.metrics.palette_height)),
                )))),
            |_, cx| {
                let watched = exports.clone();
                let view = cx.new(|cx| {
                    // Exports progress on workers (the person's and agents'):
                    // redraw when the list changes, while the window is open.
                    cx.spawn(async move |this, cx| {
                        let mut seen = None;
                        loop {
                            let now = watched.lock().unwrap().sequence();
                            if seen != Some(now) {
                                seen = Some(now);
                                if this.update(cx, |_, cx| cx.notify()).is_err() {
                                    return;
                                }
                            }
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(250))
                                .await;
                        }
                    })
                    .detach();
                    TasksWindow {
                        motion: Motion::default(),
                        pro,
                        exports,
                    }
                });
                entity = Some(view.clone());
                view
            },
        );
        self.tasks_window = entity;
    }
}

#[cfg(test)]
mod tests {
    use super::detail;
    use pm_promedia::tasks::{BackgroundTasks, ProgressUnit, TaskKind, TaskSpec};

    /// The detail line reads as the review specifies for each state.
    #[test]
    fn detail_lines_follow_the_task_states() {
        let mut q = BackgroundTasks::default();
        let id = q
            .enqueue(
                "main".into(),
                "p".into(),
                TaskSpec {
                    title: "Clip".into(),
                    kind: TaskKind::Proxy,
                    unit: ProgressUnit::Frames,
                    priority: 0,
                    total: 300,
                },
            )
            .unwrap();
        assert_eq!(detail(q.get(id).unwrap()), "Waiting…");
        let ticket = q.start_next().unwrap();
        q.progress(ticket, 128, 300).unwrap();
        assert_eq!(detail(q.get(id).unwrap()), "128 of 300 frames");
        q.pause(id).unwrap();
        assert_eq!(detail(q.get(id).unwrap()), "Pausing…");
        q.settle_control(ticket).unwrap();
        assert_eq!(detail(q.get(id).unwrap()), "Paused");
        q.resume(id).unwrap();
        let ticket = q.start_next().unwrap();
        q.finish(ticket, Ok(())).unwrap();
        assert_eq!(detail(q.get(id).unwrap()), "Created");
    }
}

#[::core::prelude::v1::test]
#[ignore = "writes synthetic review captures"]
fn capture_utility_tasks() {
    for empty in [false, true] {
        let profile = tempfile::tempdir().unwrap();
        let pro = Arc::new(ProMedia::new(
            profile.path().into(),
            profile.path().join("objects"),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        ));
        let mut exports = BackgroundTasks::default();
        if !empty {
            let _id = exports
                .enqueue(
                    "main".into(),
                    "synthetic".into(),
                    pm_promedia::tasks::TaskSpec {
                        title: "Opening Sequence".into(),
                        kind: pm_promedia::tasks::TaskKind::Export,
                        unit: ProgressUnit::Frames,
                        priority: 0,
                        total: 300,
                    },
                )
                .unwrap();
            let ticket = exports.start_next().unwrap();
            exports.progress(ticket, 128, 300).unwrap();
        }
        crate::utility_capture_tests::capture(
            if empty { "tasks-empty" } else { "tasks" },
            |_, cx| {
                cx.new(|_| TasksWindow {
                    motion: Motion::default(),
                    pro,
                    exports: Arc::new(Mutex::new(exports)),
                })
            },
        );
    }
}
