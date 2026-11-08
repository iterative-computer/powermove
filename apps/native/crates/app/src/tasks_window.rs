//! Window ▸ Background Tasks (⌘9; design surfaces/tasks-storage.md and the
//! pro-media review): exports and proxy/optimized media work, one row each,
//! with progress, Pause/Resume and Cancel. Right-click sets a task's
//! priority (an NSMenu), as plan section 1 asks; the row order stays the
//! queue's, so priority never reorders what a person is reading.
use crate::pro_media::{ProMedia, TaskAction};
use gpui::{prelude::*, *};
use pm_promedia::tasks::{BackgroundTasks, ProgressUnit, Task, TaskProgress, TaskState};
use pm_shell::tokens;
use std::sync::{Arc, Mutex};

pub struct TasksWindow {
    pro: Arc<ProMedia>,
    exports: Arc<Mutex<BackgroundTasks>>,
    /// Cancels the running export through its own job (the export worker
    /// settles its task; this window never moves the export queue).
    controller: WeakEntity<crate::session::Controller>,
}

pub const TITLE: &str = "Background Tasks";
const EMPTY_TITLE: &str = "No Background Tasks";
const EMPTY_LINE: &str = "Exports, renders and proxy media appear here.";
/// Priority choices on a task's context menu.
const PRIORITIES: [(&str, i32); 3] = [("High", 10), ("Normal", 0), ("Low", -10)];

fn units(unit: Option<ProgressUnit>) -> &'static str {
    match unit {
        Some(ProgressUnit::Frames) => "frames",
        Some(ProgressUnit::Items) => "items",
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
    /// Media tasks act through the service. An export can only be
    /// cancelled, through its job: it has no pause, and one runs at a time.
    fn act(&self, media: bool, id: pm_promedia::tasks::TaskId, action: TaskAction, cx: &mut App) {
        if media {
            let _ = self.pro.act(id, action);
        } else if action == TaskAction::Cancel {
            let _ = self.controller.update(cx, |v, _| {
                if let Some(job) = &v.builtins.export_job {
                    job.cancel.store(true, std::sync::atomic::Ordering::Release);
                }
            });
        }
    }

    fn row(
        &self,
        task: &Task,
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
                div()
                    .id(SharedString::from(format!("{label}-{}-{}", media, id.0)))
                    .px(px(m.gap))
                    .text_color(rgb(if enabled { c.accent_text } else { c.muted }))
                    .when(enabled, |d| {
                        d.on_click(cx.listener(move |v, _, _, cx| {
                            v.act(media, id, action, cx);
                            cx.notify();
                        }))
                    })
                    .child(label)
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
                                    v.act(media, id, TaskAction::Priority(*priority), cx);
                                    cx.notify();
                                });
                            }
                        });
                    }),
                )
            })
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
                        format!("{} · {}", p.kind_label, p.title)
                    }))
                    .when_some(share, |d, share| {
                        d.child(
                            div()
                                .h(px(m.border * 3.))
                                .w_full()
                                .rounded(px(m.border * 1.5))
                                .bg(rgb(c.separator))
                                .child(
                                    div()
                                        .h_full()
                                        .w(relative(share.clamp(0., 1.)))
                                        .rounded(px(m.border * 1.5))
                                        .bg(rgb(c.accent)),
                                ),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(t.typography.minimum))
                            .text_color(rgb(if failed { c.danger } else { c.secondary }))
                            .child(
                                media
                                    .then(|| self.pro.waiting(id))
                                    .flatten()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| detail(task)),
                            ),
                    ),
            )
            .child(trailing);
        window_rows.push(row.into_any_element());
    }
}

impl Render for TasksWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = tokens();
        let (c, m) = (&t.colors, &t.metrics);
        let mut body = div()
            .id("background-tasks")
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .p(px(m.inset))
            .bg(rgb(c.panel))
            .text_color(rgb(c.text))
            .text_size(px(t.typography.body))
            .font_family(pm_design::TEXT_FONT);
        let exports = self.exports.lock().unwrap().all();
        let media = self.pro.snapshot();
        let mut rows = Vec::new();
        for task in &exports {
            self.row(task, false, &mut rows, cx);
        }
        for task in &media {
            self.row(task, true, &mut rows, cx);
        }
        if rows.is_empty() {
            return body
                .items_center()
                .justify_center()
                .gap(px(m.gap))
                .child(div().text_size(px(t.typography.title)).child(EMPTY_TITLE))
                .child(div().text_color(rgb(c.secondary)).child(EMPTY_LINE));
        }
        for row in rows {
            body = body.child(row);
        }
        body
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
        let controller = cx.entity().downgrade();
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
                let view = cx.new(|_| TasksWindow {
                    pro,
                    exports,
                    controller,
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
