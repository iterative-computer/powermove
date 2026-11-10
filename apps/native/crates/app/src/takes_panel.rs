//! Window ▸ Panels ▸ Takes (S103) over pm-services `surface_backing::projects`:
//! named versions of the open project, newest first. Save Take records the
//! project as it stands; clicking a take restores it as one Undo step;
//! right-click offers Restore and Delete. Takes live in the library's `takes`
//! store (v1's), which the kernel reads and replaces only while it still
//! holds what this panel read. Reads, snapshots and writes run off the main
//! thread.
use crate::session::Controller;
use gpui::{prelude::*, *};
use pm_kernel::{
    Call, KernelHandle, Published,
    session::{ShellCall, ShellReply},
};
use pm_services::surface_backing::{
    RecordHost, Records, Session,
    projects::{self, TakeRestoreHost},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// One take of the open project, as listed.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub label: String,
    pub at: f64,
}
#[derive(Default)]
pub struct State {
    /// The project these rows belong to, and the whole store as read.
    project: Option<String>,
    rows: Vec<Row>,
    all: Arc<Vec<Value>>,
    loading: bool,
    busy: bool,
    pub status: String,
}

pub fn panel(
    v: &mut Controller,
    id: &str,
    bounds: pm_shell::layout::Rect,
    cx: &mut Context<Controller>,
) -> Option<AnyElement> {
    (id == "takes").then(|| render(v, bounds, cx))
}

/// The document's own `id` (what a take's snapshot carries), which is not
/// the kernel's session key for a file project.
pub fn document_id(p: &Published) -> Option<String> {
    let root = p.snapshot.root()?;
    match root.get("id")?.kind() {
        pm_model::NodeKind::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}
/// The open project's takes: a take names its project (`project`), or, for
/// takes v1 saved, its snapshot's `id` does.
pub fn rows_for(all: &[Value], project: &str) -> Vec<Row> {
    all.iter()
        .filter(|take| {
            take["project"].as_str().map_or_else(
                || {
                    take["json"]
                        .as_str()
                        .and_then(|json| serde_json::from_str::<Value>(json).ok())
                        .is_some_and(|p| p["id"] == project)
                },
                |owner| owner == project,
            )
        })
        .filter_map(|take| {
            Some(Row {
                id: take["id"].as_str()?.to_owned(),
                label: take["label"].as_str()?.to_owned(),
                at: take["at"].as_f64()?,
            })
        })
        .collect()
}

fn kernel_value(value: &pm_model::Value) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(value)).unwrap_or(Value::Null)
}
fn model_value(value: &Value) -> Option<pm_model::Value> {
    pm_model::json::parse(&value.to_string()).ok()
}
/// Records the project as it stands as a new take named `label`.
pub fn save(kernel: &KernelHandle, p: &Published, label: String) -> Result<String, String> {
    let root = p.snapshot.root().ok_or("No project is open.")?;
    let snapshot = pm_model::json::to_canonical_string(&root.to_value());
    let before = Records {
        revision: 0,
        entries: read(kernel)?,
    };
    let mut take =
        projects::take_record(format!("t{}", unique()), label.clone(), now_ms(), snapshot);
    take["project"] = json!(document_id(p).ok_or("This project has no id.")?);
    projects::save_take(&mut Store(kernel), &before, take)
        .map_err(|e| e.diagnostic().to_string())?;
    Ok(label)
}
/// Restores `row` over the project `p` shows, as one Undo step.
pub fn restore(
    kernel: &KernelHandle,
    p: &Published,
    all: &[Value],
    row: &Row,
) -> Result<String, String> {
    let session = Session {
        project: p.project.clone(),
        incarnation: p.snapshot.doc_id,
    };
    let records = Records {
        revision: 0,
        entries: all.to_vec(),
    };
    let mut host = Restorer {
        kernel,
        document: document_id(p).ok_or("This project has no id.")?,
        label: format!("Restore {}", row.label),
    };
    projects::restore_take(&mut host, &session, p.revision(), &records, &row.id)
        .map_err(|e| e.diagnostic().to_string())?;
    Ok(row.label.clone())
}
pub fn delete(kernel: &KernelHandle, row: &Row) -> Result<String, String> {
    let before = Records {
        revision: 0,
        entries: read(kernel)?,
    };
    projects::archive_take(&mut Store(kernel), &before, &row.id)
        .map_err(|e| e.diagnostic().to_string())?;
    Ok(row.label.clone())
}
pub fn read(kernel: &KernelHandle) -> Result<Vec<Value>, String> {
    match kernel.shell(ShellCall::Takes).wait() {
        Some(Ok(ShellReply::Recent(rows))) => Ok(rows.iter().map(kernel_value).collect()),
        Some(Err(e)) => Err(e.to_string()),
        _ => Err("The takes could not be read.".into()),
    }
}
/// The kernel's compare-and-replace of the `takes` store.
struct Store<'a>(&'a KernelHandle);
impl RecordHost for Store<'_> {
    fn replace_retaining(
        &mut self,
        _: &str,
        before: &Records,
        after: &[Value],
    ) -> pm_services::Result<u64> {
        let convert = |values: &[Value]| values.iter().filter_map(model_value).collect::<Vec<_>>();
        let failed = |why: &str| {
            pm_services::ServiceError::message(pm_services::ServiceErrorCode::Unavailable, why)
        };
        match self
            .0
            .shell(ShellCall::ReplaceTakes {
                expected: convert(&before.entries),
                next: convert(after),
            })
            .wait()
        {
            Some(Ok(ShellReply::Changed(true))) => Ok(before.revision + 1),
            Some(Ok(ShellReply::Changed(false))) => {
                Err(failed("The takes changed meanwhile. Try again."))
            }
            Some(Err(e)) => Err(failed(&e.to_string())),
            _ => Err(failed("The takes could not be saved.")),
        }
    }
}
/// Restore through the kernel: one Undo step, held to the document and
/// revision the person saw.
struct Restorer<'a> {
    kernel: &'a KernelHandle,
    /// The open document's own id.
    document: String,
    label: String,
}
impl TakeRestoreHost for Restorer<'_> {
    fn restore(
        &mut self,
        session: &Session,
        revision: u64,
        snapshot: &str,
    ) -> pm_services::Result<u64> {
        let failed = |why: String| {
            pm_services::ServiceError::message(pm_services::ServiceErrorCode::InvalidData, why)
        };
        let project = pm_model::json::parse(snapshot)
            .map_err(|_| failed("This take can't be read.".into()))?;
        if project.get("id").and_then(pm_model::Value::as_str) != Some(self.document.as_str()) {
            return Err(failed("This take belongs to another project.".into()));
        }
        let call = Call::Restore {
            project,
            label: self.label.clone(),
            expected_revision: revision,
            expected_document: session.incarnation,
        };
        match self
            .kernel
            .call(
                &session.project,
                pm_kernel::Request::new(pm_kernel::Caller::native(), call),
            )
            .wait()
        {
            Some(Ok(ack)) => match ack.reply {
                pm_kernel::Reply::Throw(e) => Err(failed(e.to_string())),
                _ => Ok(revision + 1),
            },
            Some(Err(e)) => Err(failed(e.to_string())),
            None => Err(failed("The project closed.".into())),
        }
    }
}

impl Controller {
    /// Reads the takes on a worker; the panel shows them when they arrive.
    fn load_takes(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.published.as_deref().and_then(document_id) else {
            return;
        };
        self.takes.loading = true;
        self.takes.project = Some(project.clone());
        let kernel = self.kernel.clone();
        let work = cx.background_spawn(async move {
            let all = read(&kernel)?;
            let rows = rows_for(&all, &project);
            Ok::<_, String>((all, rows))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |v, cx| {
                v.takes.loading = false;
                match result {
                    Ok((all, rows)) => {
                        v.takes.all = Arc::new(all);
                        v.takes.rows = rows;
                    }
                    Err(e) => v.takes.status = e,
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// Save Take: the project as it stands, named `Take N` as v1 did.
    pub fn save_take(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.published.clone() else {
            return;
        };
        if self.takes.busy {
            return;
        }
        self.takes.busy = true;
        let kernel = self.kernel.clone();
        let label = format!("Take {}", self.takes.rows.len() + 1);
        let work = cx.background_spawn(async move { save(&kernel, &p, label) });
        self.after_take_write(work, |label| format!("Saved {label}"), cx);
    }
    pub fn restore_take(&mut self, id: &str, cx: &mut Context<Self>) {
        let (Some(p), Some(row)) = (
            self.published.clone(),
            self.takes.rows.iter().find(|r| r.id == id).cloned(),
        ) else {
            return;
        };
        if self.takes.busy {
            return;
        }
        self.takes.busy = true;
        let kernel = self.kernel.clone();
        let all = self.takes.all.clone();
        let work = cx.background_spawn(async move { restore(&kernel, &p, &all, &row) });
        self.after_take_write(work, |label| format!("Restored {label}"), cx);
    }
    pub fn delete_take(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(row) = self.takes.rows.iter().find(|r| r.id == id).cloned() else {
            return;
        };
        if self.takes.busy {
            return;
        }
        self.takes.busy = true;
        let kernel = self.kernel.clone();
        let work = cx.background_spawn(async move { delete(&kernel, &row) });
        self.after_take_write(work, |label| format!("Deleted {label}"), cx);
    }
    fn after_take_write(
        &mut self,
        work: Task<Result<String, String>>,
        done: impl FnOnce(String) -> String + 'static,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |v, cx| {
                v.takes.busy = false;
                v.takes.status = match result {
                    Ok(label) => done(label),
                    Err(e) => e,
                };
                v.load_takes(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
fn unique() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{:x}{:x}",
        now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed) ^ u64::from(std::process::id())
    )
}
/// Same-day takes read as a clock time, older ones as a short date (v1), in
/// the system's own formats; the tooltip gives both.
fn when(at: f64) -> (String, String) {
    use objc2_foundation::{NSDate, NSDateFormatter, NSDateFormatterStyle, NSString};
    let date = NSDate::dateWithTimeIntervalSince1970(at / 1000.);
    let formatter = NSDateFormatter::new();
    formatter.setDateFormat(Some(&NSString::from_str("yyyy-MM-dd")));
    let today = formatter.stringFromDate(&NSDate::now()).to_string();
    let same_day = formatter.stringFromDate(&date).to_string() == today;
    formatter.setLocalizedDateFormatFromTemplate(&NSString::from_str(if same_day {
        "jmm"
    } else {
        "MMMd"
    }));
    let short = formatter.stringFromDate(&date).to_string();
    formatter.setDateFormat(None);
    formatter.setDateStyle(NSDateFormatterStyle::MediumStyle);
    formatter.setTimeStyle(NSDateFormatterStyle::ShortStyle);
    (short, formatter.stringFromDate(&date).to_string())
}

fn render(
    v: &mut Controller,
    bounds: pm_shell::layout::Rect,
    cx: &mut Context<Controller>,
) -> AnyElement {
    let t = v.host.borrow().tokens.clone();
    let Some(project) = v.published.as_deref().and_then(document_id) else {
        return pm_shell::view::empty_state(&t, "No Takes", "Open a project to save takes.")
            .size_full()
            .into_any_element();
    };
    if v.takes.project.as_deref() != Some(project.as_str()) && !v.takes.loading {
        v.takes.rows.clear();
        v.takes.status.clear();
        v.load_takes(cx);
    }
    let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
    let height = (bounds.h - m.header).max(0.);
    let busy = v.takes.busy;
    let save = div()
        .id("takes-save")
        .role(Role::Button)
        .aria_label("Save Take")
        .flex_none()
        .h(px(m.control))
        .px(px(m.inset))
        .flex()
        .items_center()
        .rounded(px(m.control_radius))
        .bg(rgb(c.field))
        .when(!busy, |d| d.hover_style(|s| s.bg(rgb(c.hover))))
        .text_size(px(ty.reading))
        .text_color(rgb(if busy { c.muted } else { c.text }))
        .child("Save Take")
        .on_click(cx.listener(|v, _, _, cx| v.save_take(cx)));
    let mut body = div()
        .id("takes")
        .role(Role::List)
        .aria_label("Takes")
        .flex()
        .flex_col()
        .w_full()
        .h(px(height))
        .p(px(m.gap))
        .text_size(px(ty.body));
    if v.takes.rows.is_empty() && !v.takes.loading {
        body = body.child(
            pm_shell::view::empty_state(
                &t,
                "No Takes",
                "Save a take before exploring a new motion direction.",
            )
            .flex_1()
            .child(save.mt(px(m.gap))),
        );
    } else {
        let mut list = div()
            .id("takes-rows")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for row in &v.takes.rows {
            let (short, full) = when(row.at);
            let (restore, menu) = (row.id.clone(), row.id.clone());
            let tip = format!("{} · {full}", row.label);
            list = list.child(
                div()
                    .id(SharedString::from(format!("take-{}", row.id)))
                    .role(Role::Row)
                    .aria_label(format!("{}, {full}", row.label))
                    .flex()
                    .items_center()
                    .flex_none()
                    .h(px(m.row))
                    .px(px(m.gap))
                    .rounded(px(m.inner_radius))
                    .hover_style(|s| s.bg(rgb(c.hover)))
                    .tooltip(pm_shell::view::tooltip(&t, tip))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(c.text))
                            .child(row.label.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .pl(px(m.gap))
                            .text_color(rgb(c.muted))
                            .child(short),
                    )
                    .on_click(cx.listener(move |v, _, _, cx| v.restore_take(&restore, cx)))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |v, e: &MouseDownEvent, _, _| {
                            let entries = [
                                pm_shell::MenuEntry::item("Restore", format!("takeRestore:{menu}")),
                                pm_shell::MenuEntry::item(
                                    "Delete Take",
                                    format!("takeDelete:{menu}"),
                                ),
                            ];
                            v.host.borrow().context_menu(
                                &entries,
                                f32::from(e.position.x),
                                f32::from(e.position.y),
                            );
                        }),
                    ),
            );
        }
        body = body.child(list).child(
            div()
                .flex()
                .items_center()
                .gap(px(m.gap))
                .pt(px(m.gap))
                .child(save)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(ty.minimum))
                        .text_color(rgb(c.muted))
                        .child(v.takes.status.clone()),
                ),
        );
    }
    body.into_any_element()
}
