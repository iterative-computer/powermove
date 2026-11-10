//! Window ▸ Panels ▸ Notes (S104) over pm-services `surface_backing::projects`
//! `Notes`: the project's direction notes (`notes`, v1's field), in a native
//! multiline editor. Typing saves 250 ms after the last keystroke and at once
//! when the editor loses focus or the project changes; a write lands only
//! while the project still holds what the editor last saw, and, as in v1, it
//! is not an Undo step (`DirectWrite`).
use crate::session::Controller;
use gpui::{prelude::*, *};
use pm_kernel::{Call, Caller, KernelHandle, Reply, Request};
use pm_services::surface_backing::{
    Session,
    projects::{NOTES_DEBOUNCE_MS, NoteWrite, Notes, NotesHost},
};
use pm_shell::text::{TextEvent, TextField, TextKind};
use std::time::Duration;

#[derive(Default)]
pub struct State {
    field: Option<Entity<TextField>>,
    _events: Option<Subscription>,
    notes: Notes,
    /// The project revision the editor last observed.
    seen: Option<(Session, u64)>,
    timer: Option<Task<()>>,
    /// The last write's failure, if any.
    pub status: String,
}

/// The `notes` the published project holds.
pub fn published_notes(p: &pm_kernel::Published) -> String {
    p.snapshot
        .root()
        .and_then(|root| root.get("notes"))
        .and_then(|n| match n.kind() {
            pm_model::NodeKind::String(s) => Some(s.to_string_lossy()),
            _ => None,
        })
        .unwrap_or_default()
}
fn session(p: &pm_kernel::Published) -> Session {
    Session {
        project: p.project.clone(),
        incarnation: p.snapshot.doc_id,
    }
}
/// The kernel write: held to the document and to the text the editor saw.
pub struct Kernel<'a>(pub &'a KernelHandle);
impl NotesHost for Kernel<'_> {
    fn persist(&mut self, write: &NoteWrite) -> pm_services::Result<()> {
        let failed = |why: &str| {
            pm_services::ServiceError::message(pm_services::ServiceErrorCode::Unavailable, why)
        };
        let p = self
            .0
            .published(&write.session.project)
            .filter(|p| p.snapshot.doc_id == write.session.incarnation)
            .ok_or_else(|| failed("The project closed before its notes were saved."))?;
        if published_notes(&p) != write.expected {
            return Err(failed("The notes changed elsewhere. Your text is kept."));
        }
        let call = Call::DirectWrite {
            ops: vec![pm_model::ops::Op::Leaf(pm_model::patch::Patch::set(
                vec!["notes".into()],
                pm_model::Value::from(write.text.as_str()),
            ))],
        };
        match self
            .0
            .call(
                &write.session.project,
                Request::new(Caller::native(), call).with_document(write.session.incarnation),
            )
            .wait()
        {
            Some(Ok(ack)) => match ack.reply {
                Reply::Throw(e) => Err(failed(&e.to_string())),
                _ => Ok(()),
            },
            Some(Err(e)) => Err(failed(&e.to_string())),
            None => Err(failed("The project closed before its notes were saved.")),
        }
    }
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn panel(
    v: &mut Controller,
    id: &str,
    bounds: pm_shell::layout::Rect,
    window: &mut Window,
    cx: &mut Context<Controller>,
) -> Option<AnyElement> {
    (id == "notes").then(|| render(v, bounds, window, cx))
}

impl Controller {
    /// Writes the draft when it is due (or now, with `force`), on a worker.
    pub fn flush_notes(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(current) = self.notes.notes.current.clone() else {
            return;
        };
        let write = match self.notes.notes.flush(&current, now_ms(), force) {
            Ok(Some(write)) => write,
            _ => return,
        };
        let kernel = self.kernel.clone();
        let task = write.clone();
        let work = cx.background_spawn(async move { Kernel(&kernel).persist(&task) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |v, cx| {
                // Only a failure shows; the text stays in the editor to retry.
                v.notes.status = match v.notes.notes.settle(&write, result) {
                    Ok(()) => String::new(),
                    Err(e) => e.diagnostic().to_string(),
                };
                cx.notify();
            });
        })
        .detach();
    }
    fn notes_typed(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(current) = self.notes.notes.current.clone() else {
            return;
        };
        if self.notes.notes.input(&current, text, now_ms()).is_err() {
            return;
        }
        self.notes.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(NOTES_DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |v, cx| v.flush_notes(false, cx));
        }));
    }
}

fn render(
    v: &mut Controller,
    bounds: pm_shell::layout::Rect,
    window: &mut Window,
    cx: &mut Context<Controller>,
) -> AnyElement {
    let t = v.host.borrow().tokens.clone();
    let Some(p) = v.published.clone() else {
        return pm_shell::view::empty_state(&t, "No Notes", "Open a project to write its notes.")
            .size_full()
            .into_any_element();
    };
    let session = session(&p);
    let text = published_notes(&p);
    let field = match v.notes.field.clone() {
        Some(field) => field,
        None => {
            let tokens = t.clone();
            let field = cx.new(|cx| {
                let mut field = TextField::new(
                    TextKind::Multiline,
                    "Project notes",
                    &text,
                    tokens,
                    window,
                    cx,
                );
                field.set_placeholder(
                    "Direction notes — saved with this project as its creative brief.",
                );
                field.set_fill(true);
                field
            });
            v.notes._events = Some(cx.subscribe(&field, |v, _, event, cx| match event {
                TextEvent::Changed(text) => v.notes_typed(text.clone(), cx),
                TextEvent::Committed(text) => {
                    v.notes_typed(text.clone(), cx);
                    v.flush_notes(true, cx);
                }
                _ => {}
            }));
            v.notes.field = Some(field.clone());
            field
        }
    };
    // A new document: the old one's pending text is written first, then the
    // editor shows the new project's notes.
    if v.notes.notes.current.as_ref() != Some(&session) {
        v.flush_notes(true, cx);
        v.notes.notes.attach(session.clone(), text.clone());
        v.notes.seen = None;
        field.update(cx, |f, cx| f.set_value(&text, cx));
    }
    if v.notes.seen.as_ref() != Some(&(session.clone(), p.revision())) {
        v.notes.seen = Some((session.clone(), p.revision()));
        let _ = v.notes.notes.observe(&session, text);
        if let Some(draft) = v.notes.notes.draft(&session) {
            let shown = draft.text.clone();
            field.update(cx, |f, cx| {
                if f.value() != shown {
                    f.set_value(&shown, cx);
                }
            });
        }
    }
    let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
    div()
        .id("notes")
        .flex()
        .flex_col()
        .w_full()
        .h(px((bounds.h - m.header).max(0.)))
        .p(px(m.gap))
        .gap(px(m.border * 2.))
        .child(div().flex_1().min_h_0().child(field))
        .child(
            div()
                .flex_none()
                .h(px(ty.minimum * 1.4))
                .text_size(px(ty.minimum))
                .text_color(rgb(c.muted))
                .child(v.notes.status.clone()),
        )
        .into_any_element()
}
