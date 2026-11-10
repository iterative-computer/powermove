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
pub struct Cache(RefCell<Option<(u64, Option<pm_model::Value>, Rc<Store>)>>);

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
                    let _ = self
                        .kernel
                        .call(&session.project, Request::new(Caller::native(), call));
                    return;
                }
                let id = after["id"].as_str().map(pm_model::Value::from);
                vec![
                    StateOp::Workspace { record: next },
                    StateOp::Setting {
                        key: "workspace".into(),
                        value: id,
                    },
                ]
            }
            Write::Upsert(record) => match model(&record) {
                Some(record) => vec![StateOp::Workspace { record }],
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
        let _ = self.kernel.state_transaction(StateTransaction {
            expected_revision: None,
            ops,
        });
    }
}
