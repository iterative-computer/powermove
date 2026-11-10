//! The shell's workspace store is the kernel's: the `workspaces`,
//! `workspace`, `defaultWorkspace` settings and the project's saved
//! workspace (`pm_shell::workspace`). Reads come from published state and
//! never wait; writes go to the kernel without waiting for its answer.
use crate::session::Controller;
use pm_kernel::{
    Caller, Request, WorkspaceCall,
    state::{StateOp, StateTransaction},
};
use pm_shell::workspace::{Store, Write, seeded};
use std::{cell::RefCell, rc::Rc};

/// The last store built and what it was built from: the kernel state
/// revision and the project's published workspace.
#[derive(Default)]
pub struct Cache(RefCell<Option<Built>>);
/// The state revision and project workspace a store was built from.
type Built = (u64, Option<pm_model::Value>, Rc<Store>);

fn json(v: &pm_model::Value) -> serde_json::Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(v)).unwrap_or_default()
}
fn model(v: &serde_json::Value) -> Option<pm_model::Value> {
    pm_model::json::parse(&v.to_string()).ok()
}

impl Controller {
    /// The store as this window sees it; the same `Rc` until it changes.
    pub fn workspace_store(&self) -> Rc<Store> {
        let state = self.kernel.state();
        let project = self.published.as_ref().map(|p| p.workspace.clone());
        let mut cache = self.workspace_store.0.borrow_mut();
        if let Some((revision, seen, store)) = &*cache
            && *revision == state.revision
            && *seen == project
        {
            return store.clone();
        }
        let setting = |key: &str| {
            state
                .settings
                .get(key)
                .and_then(pm_model::Value::as_str)
                .map(str::to_owned)
        };
        let stored = state
            .settings
            .get("workspaces")
            .map(json)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let store = Rc::new(Store {
            records: seeded(&stored),
            current: setting("workspace"),
            default: setting("defaultWorkspace"),
            project: project.as_ref().map(json),
            project_id: self.session.as_ref().map(|s| s.project.clone()),
        });
        *cache = Some((state.revision, project, store.clone()));
        store
    }
    /// Sends `write` to the kernel. A window with a project writes its
    /// workspace there (the kernel files it in `workspaces` and makes it the
    /// active one); a window without one writes the setting.
    pub fn write_workspace(&self, write: Write) {
        let ops = match write {
            Write::Active { before, after } => {
                let (Some(expected), Some(next)) = (model(&before), model(&after)) else {
                    return;
                };
                if let Some(session) = &self.session {
                    let call = WorkspaceCall::Replace { expected, next };
                    drop(
                        self.kernel
                            .call(&session.project, Request::new(Caller::native(), call)),
                    );
                    return;
                }
                let id = after["id"].as_str().map(pm_model::Value::from);
                vec![
                    StateOp::Workspace {
                        record: next,
                        base: Some(expected),
                    },
                    StateOp::Setting {
                        key: "workspace".into(),
                        value: id,
                    },
                ]
            }
            Write::Upsert(record) => match model(&record) {
                Some(record) => vec![StateOp::Workspace { record, base: None }],
                None => return,
            },
            Write::Archive(id) => vec![StateOp::ArchiveWorkspace {
                id,
                deleted_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0., |d| d.as_millis() as f64),
            }],
            Write::Default(id) => vec![StateOp::Setting {
                key: "defaultWorkspace".into(),
                value: Some(id.as_str().into()),
            }],
        };
        drop(self.kernel.state_transaction(StateTransaction {
            expected_revision: None,
            ops,
        }));
    }
}

/// Window ▸ Panels ▸ Workspaces (S099): the workspaces, current one checked.
/// Click activates; right-click a saved one to delete it (asked first);
/// Save Workspace… and Edit Workspace JSON… act on the active one.
pub fn panel(
    v: &mut Controller,
    id: &str,
    bounds: pm_shell::layout::Rect,
    cx: &mut gpui::Context<Controller>,
) -> Option<gpui::AnyElement> {
    (id == "workspaces").then(|| render(v, bounds, cx))
}
fn render(
    v: &mut Controller,
    rect: pm_shell::layout::Rect,
    cx: &mut gpui::Context<Controller>,
) -> gpui::AnyElement {
    use gpui::{prelude::*, *};
    use pm_shell::workspace::{is_builtin, record_id, record_name};
    let t = v.host.borrow().tokens.clone();
    let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
    let store = v.workspace_store();
    let current = store
        .project
        .as_ref()
        .filter(|p| pm_shell::workspace::usable(p))
        .and_then(|p| record_id(p))
        .or(store.current.as_deref())
        .unwrap_or(pm_shell::workspace::FALLBACK)
        .to_owned();
    let button = |id: &'static str, label: &'static str, command: &'static str| {
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .flex_none()
            .h(px(m.control))
            .px(px(m.inset))
            .flex()
            .items_center()
            .rounded(px(m.control_radius))
            .bg(rgb(c.field))
            .hover_style(|s| s.bg(rgb(c.hover)))
            .text_size(px(ty.reading))
            .text_color(rgb(c.text))
            .child(label)
            .on_click(cx.listener(move |v, _, _, _| {
                v.host.borrow().send_command(command, vec![]);
            }))
    };
    let mut list = div()
        .id("workspaces-rows")
        .role(Role::List)
        .aria_label("Workspaces")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll();
    for r in store.offered() {
        let (Some(id), name) = (record_id(r), record_name(r)) else {
            continue;
        };
        let on = id == current;
        let (activate, menu, label) = (id.to_owned(), id.to_owned(), name.to_owned());
        let builtin = is_builtin(r);
        list = list.child(
            div()
                .id(SharedString::from(format!("workspace-{id}")))
                .role(Role::Row)
                .aria_label(if on {
                    format!("{name}, current")
                } else {
                    name.to_owned()
                })
                .flex()
                .items_center()
                .flex_none()
                .h(px(m.row))
                .px(px(m.gap))
                .gap(px(m.gap))
                .rounded(px(m.inner_radius))
                .hover_style(|s| s.bg(rgb(c.hover)))
                .child(
                    div()
                        .flex_none()
                        .w(px(ty.reading))
                        .text_color(rgb(c.text))
                        .child(if on { "✓" } else { "" }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(c.text))
                        .child(name.to_owned()),
                )
                .on_click(cx.listener(move |v, _, _, cx| {
                    v.host
                        .borrow()
                        .send_command(&format!("workspace:{activate}"), vec![]);
                    v.workspaces_status = format!("Activated {}", activate_name(v, &activate));
                    cx.notify();
                }))
                .when(!builtin, |d| {
                    d.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |v, _: &MouseDownEvent, _, cx| {
                            v.confirm_workspace_delete(menu.clone(), label.clone(), cx);
                        }),
                    )
                }),
        );
    }
    div()
        .id("workspaces")
        .flex()
        .flex_col()
        .w_full()
        .h(px((rect.h - m.header).max(0.)))
        .p(px(m.gap))
        .text_size(px(ty.body))
        .child(list)
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(m.gap))
                .pt(px(m.gap))
                .child(button("workspaces-save", "Save Workspace…", "layout:save"))
                .child(button(
                    "workspaces-json",
                    "Edit Workspace JSON…",
                    "layout:edit",
                )),
        )
        .child(
            div()
                .pt(px(m.gap))
                .text_size(px(ty.minimum))
                .text_color(rgb(c.muted))
                .child(v.workspaces_status.clone()),
        )
        .child(
            div()
                .text_size(px(ty.minimum))
                .text_color(rgb(c.muted))
                .child("Ask the Powermove agent to redesign or add an interface section."),
        )
        .into_any_element()
}
fn activate_name(v: &Controller, id: &str) -> String {
    v.workspace_store()
        .get(id)
        .map(|r| pm_shell::workspace::record_name(r).to_owned())
        .unwrap_or_else(|| id.to_owned())
}
impl Controller {
    /// S012: `Delete the workspace “{name}”?`, Cancel the default.
    pub fn confirm_workspace_delete(
        &mut self,
        id: String,
        name: String,
        cx: &mut gpui::Context<Self>,
    ) {
        use crate::alert::{Alert, Button};
        let alert = Alert::new(
            format!("Delete the workspace “{name}”?"),
            "This removes the saved workspace layout. Your project and its layers will not be deleted.",
            vec![
                Button::new("Cancel", false).default().cancel(),
                Button::new("Delete Workspace", true).destructive(),
            ],
        );
        let answer = alert.sheet(&self.host.borrow().window.clone());
        cx.spawn(async move |this, cx| {
            if answer.recv().await == Ok(Some(true)) {
                let _ = this.update(cx, |v, cx| {
                    v.host
                        .borrow()
                        .send_command(&format!("layout:delete:{id}"), vec![]);
                    v.workspaces_status = format!("Deleted {name}");
                    cx.notify();
                });
            }
        })
        .detach();
    }
}
