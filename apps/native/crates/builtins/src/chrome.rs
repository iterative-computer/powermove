//! v4 `timeline.chrome` edits to the workspace's `chrome.timeline` (v1–3's
//! `workspace.chrome.timeline`). Edits apply in order on one worker thread, so
//! none is lost and the main thread never waits on the kernel: each batch reads
//! the kernel's current workspace, changes only the fields it names and writes
//! it back. Each field keeps a stack of owners: an extension leaving puts back
//! the next owner's value, or what was there before any extension, unless
//! someone else changed the field since.
use pm_kernel::{Caller, KernelHandle, Reply, Request, WorkspaceCall};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, Sender};

pub(crate) enum ChromeEdit {
    /// Fields from `timeline_patch` an extension's generation set.
    Set {
        panel: String,
        generation: u64,
        patch: Value,
    },
    /// The extension (one generation of it, or all) left.
    Forget {
        panel: String,
        generation: Option<u64>,
    },
}

/// Per field (`timeline.rowHeight`, `timelineSurfaceOrder`): the value before
/// any extension set it (`Null`: absent), then each owner's value in order.
#[derive(Default)]
pub(crate) struct ChromeStacks(BTreeMap<String, (Value, Vec<Owner>)>);
/// An extension's panel, its generation and the value it set.
type Owner = (String, u64, Value);

impl ChromeStacks {
    /// Applies `edit` to `workspace`; true when it changed.
    pub(crate) fn apply(&mut self, workspace: &mut Value, edit: ChromeEdit) -> bool {
        match edit {
            ChromeEdit::Set {
                panel,
                generation,
                patch,
            } => {
                let mut changed = false;
                for (key, value) in patch.as_object().into_iter().flatten() {
                    let mut fields = vec![format!("timeline.{key}")];
                    // `surfaceOrder` also sets the sibling field v1–3 read.
                    if key == "surfaceOrder" {
                        fields.push("timelineSurfaceOrder".into());
                    }
                    for field in fields {
                        let (_, owners) = self
                            .0
                            .entry(field.clone())
                            .or_insert_with(|| (read(workspace, &field), vec![]));
                        owners.retain(|(p, _, _)| *p != panel);
                        owners.push((panel.clone(), generation, value.clone()));
                        changed |= write(workspace, &field, value.clone());
                    }
                }
                changed
            }
            ChromeEdit::Forget { panel, generation } => {
                let leaving = |p: &str, g: u64| p == panel && generation.is_none_or(|n| n == g);
                let mut changed = false;
                self.0.retain(|field, (base, owners)| {
                    let top = owners
                        .last()
                        .filter(|(p, g, _)| leaving(p, *g))
                        .map(|(_, _, v)| v.clone());
                    owners.retain(|(p, g, _)| !leaving(p, *g));
                    // Only the shown value is put back, and only if nobody
                    // changed the field after this extension set it.
                    if let Some(top) = top
                        && same(&read(workspace, field), &top)
                    {
                        let next = owners.last().map_or(base.clone(), |(_, _, v)| v.clone());
                        changed |= write(workspace, field, next);
                    }
                    !owners.is_empty()
                });
                changed
            }
        }
    }
}

/// Equal as the workspace stores them: `340` and `340.0` are one number.
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(a), Some(b)) => a == b,
        _ => a == b,
    }
}

fn read(workspace: &Value, field: &str) -> Value {
    let chrome = &workspace["chrome"];
    match field.strip_prefix("timeline.") {
        Some(key) => chrome["timeline"][key].clone(),
        None => chrome[field].clone(),
    }
}

/// Sets (or, with `Null`, removes) `field`; true when it changed.
fn write(workspace: &mut Value, field: &str, value: Value) -> bool {
    if same(&read(workspace, field), &value) {
        return false;
    }
    if !workspace.is_object() {
        *workspace = json!({});
    }
    if !workspace["chrome"].is_object() {
        workspace["chrome"] = json!({});
    }
    let (parent, key) = match field.strip_prefix("timeline.") {
        Some(key) => {
            if !workspace["chrome"]["timeline"].is_object() {
                workspace["chrome"]["timeline"] = json!({});
            }
            (&mut workspace["chrome"]["timeline"], key)
        }
        None => (&mut workspace["chrome"], field),
    };
    let map = parent.as_object_mut().expect("made an object above");
    if value.is_null() {
        map.remove(key);
    } else {
        map.insert(key.into(), value);
    }
    true
}

/// The worker that applies chrome edits for one window's project. It ends when
/// the window's broker (the only sender) goes.
pub(crate) fn spawn_writer(kernel: KernelHandle, project: String) -> Sender<ChromeEdit> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("pm-timeline-chrome".into())
        .spawn(move || run(kernel, project, rx))
        .expect("chrome writer thread");
    tx
}

fn run(kernel: KernelHandle, project: String, rx: Receiver<ChromeEdit>) {
    let mut stacks = ChromeStacks::default();
    let call = |call: WorkspaceCall| -> Option<Value> {
        let ack = kernel
            .call(&project, Request::new(Caller::native(), call))
            .wait()?
            .ok()?;
        match ack.reply {
            Reply::Workspace(w) => {
                serde_json::from_str(&pm_model::json::to_canonical_string(&w)).ok()
            }
            _ => None,
        }
    };
    while let Ok(first) = rx.recv() {
        // Everything queued so far goes in one read and one write.
        let edits: Vec<ChromeEdit> = std::iter::once(first).chain(rx.try_iter()).collect();
        let Some(mut workspace) = call(WorkspaceCall::Get) else {
            continue;
        };
        let mut changed = false;
        for edit in edits {
            changed |= stacks.apply(&mut workspace, edit);
        }
        if changed && let Ok(next) = pm_model::json::parse(&workspace.to_string()) {
            call(WorkspaceCall::Mutate { next, threw: None });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn set(panel: &str, patch: Value) -> ChromeEdit {
        ChromeEdit::Set {
            panel: panel.into(),
            generation: 1,
            patch,
        }
    }
    fn forget(panel: &str) -> ChromeEdit {
        ChromeEdit::Forget {
            panel: panel.into(),
            generation: None,
        }
    }
    #[test]
    fn owners_stack_per_field_and_leave_in_any_order() {
        let mut stacks = ChromeStacks::default();
        let mut w = json!({"chrome":{"timeline":{"rowHeight":26}}});
        // Patches carry floats; the kernel hands back integers.
        stacks.apply(&mut w, set("a", json!({"rowHeight":40.0})));
        w["chrome"]["timeline"]["rowHeight"] = json!(40);
        stacks.apply(&mut w, set("b", json!({"gutterWidth":340,"rowHeight":30})));
        assert_eq!(
            w["chrome"]["timeline"],
            json!({"rowHeight":30,"gutterWidth":340})
        );
        // A leaving under B keeps B's values; B leaving puts back the original,
        // not A's removed row height.
        stacks.apply(&mut w, forget("a"));
        assert_eq!(
            w["chrome"]["timeline"],
            json!({"rowHeight":30,"gutterWidth":340})
        );
        stacks.apply(&mut w, forget("b"));
        assert_eq!(w["chrome"]["timeline"], json!({"rowHeight":26}));
        // On top, leaving shows the owner below.
        stacks.apply(&mut w, set("a", json!({"rowHeight":40})));
        stacks.apply(&mut w, set("b", json!({"rowHeight":30})));
        stacks.apply(&mut w, forget("b"));
        assert_eq!(w["chrome"]["timeline"]["rowHeight"], 40);
        stacks.apply(&mut w, forget("a"));
        assert_eq!(w["chrome"]["timeline"]["rowHeight"], 26);
        assert!(stacks.0.is_empty());
    }
    #[test]
    fn surface_order_puts_back_its_sibling_and_later_edits_are_kept() {
        let mut stacks = ChromeStacks::default();
        let mut w = json!({"chrome":{"timelineSurfaceOrder":"normal"},"other":1});
        stacks.apply(
            &mut w,
            set("a", json!({"surfaceOrder":"reversed","keyframeSize":7.5})),
        );
        assert_eq!(w["chrome"]["timelineSurfaceOrder"], "reversed");
        assert_eq!(w["chrome"]["timeline"]["surfaceOrder"], "reversed");
        // Someone else changed the key size after A: A leaving keeps it.
        w["chrome"]["timeline"]["keyframeSize"] = json!(10);
        stacks.apply(&mut w, forget("a"));
        assert_eq!(
            w,
            json!({"chrome":{"timelineSurfaceOrder":"normal","timeline":{"keyframeSize":10}},"other":1})
        );
    }
    #[test]
    fn a_failed_generation_leaves_without_taking_the_next_one() {
        let mut stacks = ChromeStacks::default();
        let mut w = json!(null);
        stacks.apply(&mut w, set("a", json!({"rowHeight":40})));
        stacks.apply(
            &mut w,
            ChromeEdit::Set {
                panel: "a".into(),
                generation: 2,
                patch: json!({"rowHeight":44}),
            },
        );
        stacks.apply(
            &mut w,
            ChromeEdit::Forget {
                panel: "a".into(),
                generation: Some(1),
            },
        );
        assert_eq!(w["chrome"]["timeline"]["rowHeight"], 44);
        stacks.apply(
            &mut w,
            ChromeEdit::Forget {
                panel: "a".into(),
                generation: Some(2),
            },
        );
        assert_eq!(w["chrome"], json!({"timeline":{}}));
    }
}
