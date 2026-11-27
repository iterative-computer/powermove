//! Menus opened by an agent's panel action. While an MCP panel action runs on a
//! window, a menu that opens there (or in one of its child windows) is never
//! shown: the item the agent named is chosen by its label, or the menu closes
//! with nothing chosen and its items go back to the agent. No call waits on a
//! person to dismiss a menu.
//!
//! Items are JSON objects `{label, enabled?, checked?, children?}`; a separator
//! is `null`, `{"separator":true}` or an empty label. Choices are preorder
//! indices over every entry, separators and submenu headers included, which is
//! how both the app's menus and the shell's context menus number their rows.
use serde_json::{Value, json};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// How long a menu may still arrive after an action's call ends (a panel's
/// script can answer a click after the call stopped waiting). It is closed,
/// never shown, and the next action on that window reports it.
const LATE: Duration = Duration::from_secs(1);

struct Request {
    id: u64,
    window: usize,
    want: Option<String>,
    outcome: Option<Value>,
    ended: Option<Instant>,
    wake: Box<dyn Fn()>,
}

thread_local! {
    static ACTIVE: RefCell<Vec<Request>> = const { RefCell::new(Vec::new()) };
    static NEXT: RefCell<u64> = const { RefCell::new(0) };
}

/// One agent action on `window`: menus it opens are resolved while this lives.
pub struct Scope(u64);

/// Starts an agent action on `window` (an `NSWindow` address). `want` is the
/// item's label, or `"Submenu ▸ Item"` (`>` also separates). `wake` runs when
/// a menu is resolved, so a waiting call can stop waiting.
pub fn begin(window: usize, want: Option<String>, wake: impl Fn() + 'static) -> Scope {
    let id = NEXT.with(|n| {
        let mut n = n.borrow_mut();
        *n += 1;
        *n
    });
    ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        // A newer action on the window owns its menus; an earlier action's
        // late window ends (callers take its late report first).
        a.retain(|r| r.window != window);
        a.push(Request {
            id,
            window,
            want: want.filter(|w| !w.trim().is_empty()),
            outcome: None,
            ended: None,
            wake: Box::new(wake),
        });
    });
    Scope(id)
}

impl Scope {
    /// What happened to the menu this action opened, if one opened.
    pub fn outcome(&self) -> Option<Value> {
        ACTIVE.with(|a| {
            a.borrow()
                .iter()
                .find(|r| r.id == self.0)
                .and_then(|r| r.outcome.clone())
        })
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|a| {
            let mut a = a.borrow_mut();
            if let Some(i) = a.iter().position(|r| r.id == self.0) {
                if a[i].outcome.is_some() {
                    a.remove(i);
                } else {
                    a[i].ended = Some(Instant::now());
                }
            }
        });
    }
}

/// A menu that opened on `window` after its action's call ended, if any:
/// reported once by the next action there.
pub fn take_late(window: usize) -> Option<Value> {
    ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let i = a
            .iter()
            .position(|r| r.window == window && r.ended.is_some() && r.outcome.is_some())?;
        a.remove(i).outcome
    })
}

/// Called by every menu before it shows. `None`: no agent action owns the
/// window, show the menu. `Some(choice)`: don't show it; `choice` is the
/// preorder index to act on, or `None` to close it with nothing chosen.
pub fn claim(windows: &[usize], items: &[Value]) -> Option<Option<usize>> {
    ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        a.retain(|r| r.ended.is_none_or(|t| t.elapsed() < LATE));
        let request = a
            .iter_mut()
            .find(|r| windows.contains(&r.window) && r.outcome.is_none())?;
        let listed = list(items);
        let (choice, outcome) = match request.want.as_deref() {
            None => (
                None,
                json!({"opened": true, "items": listed,
                "note": "The action opened a menu; it closed with nothing chosen. To choose an item, repeat the click with the item's label as value (interact_panel) or text (computer_use_panel); a nested item is \"Submenu ▸ Item\"."}),
            ),
            Some(want) => match find(items, want) {
                Ok((index, label)) => (Some(index), json!({"opened": true, "chose": label})),
                Err(reason) => (
                    None,
                    json!({"opened": true, "chose": null, "error": reason, "items": listed}),
                ),
            },
        };
        let mut outcome = outcome;
        if request.ended.is_some() {
            outcome["late"] = json!(true);
        }
        request.outcome = Some(outcome);
        (request.wake)();
        Some(choice)
    })
}

fn is_separator(item: &Value) -> bool {
    item.is_null() || item.get("separator") == Some(&Value::Bool(true)) || label(item).is_empty()
}
fn label(item: &Value) -> &str {
    item.get("label")
        .and_then(Value::as_str)
        .or_else(|| item.as_str())
        .unwrap_or("")
}
fn children(item: &Value) -> &[Value] {
    item.get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}
fn enabled(item: &Value) -> bool {
    item.get("enabled") != Some(&Value::Bool(false))
        && item.get("disabled") != Some(&Value::Bool(true))
}

/// The agent's view of a menu: labels, nested submenus, disabled and checked
/// marks; separators left out.
fn list(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .filter(|i| !is_separator(i))
        .map(|item| {
            let mut out = json!({"label": label(item)});
            if !enabled(item) {
                out["enabled"] = json!(false);
            }
            if item.get("checked") == Some(&Value::Bool(true)) {
                out["checked"] = json!(true);
            }
            if !children(item).is_empty() {
                out["items"] = json!(list(children(item)));
            }
            out
        })
        .collect()
}

/// The enabled leaf `want` names: a path from the top (`A ▸ B`), or a single
/// label matched at the top level first, then anywhere if only one leaf has it.
fn find(items: &[Value], want: &str) -> Result<(usize, String), String> {
    let path: Vec<&str> = want
        .split(['▸', '>'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let mut leaves = Vec::new();
    walk(items, &mut 0, &mut Vec::new(), &mut leaves);
    let pick = |matches: Vec<&(usize, Vec<String>, bool)>| match matches.as_slice() {
        [(index, names, true)] => Ok((*index, names.join(" ▸ "))),
        [(_, names, false)] => Err(format!("\"{}\" is disabled.", names.join(" ▸ "))),
        [] => Err(format!("No menu item is named \"{want}\".")),
        _ => Err(format!(
            "More than one menu item is named \"{want}\"; name its submenu too (\"Submenu ▸ Item\")."
        )),
    };
    let exact: Vec<_> = leaves.iter().filter(|(_, n, _)| *n == path).collect();
    if !exact.is_empty() || path.len() != 1 {
        return pick(exact);
    }
    pick(
        leaves
            .iter()
            .filter(|(_, n, _)| n.last().map(String::as_str) == Some(path[0]))
            .collect(),
    )
}
fn walk(
    items: &[Value],
    next: &mut usize,
    trail: &mut Vec<String>,
    out: &mut Vec<(usize, Vec<String>, bool)>,
) {
    for item in items {
        let index = *next;
        *next += 1;
        if is_separator(item) {
            continue;
        }
        trail.push(label(item).to_owned());
        if children(item).is_empty() {
            out.push((index, trail.clone(), enabled(item)));
        } else if enabled(item) {
            walk(children(item), next, trail, out);
        } else {
            // A disabled submenu's rows still take their indices.
            walk(children(item), next, &mut trail.clone(), &mut Vec::new());
        }
        trail.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn menu() -> Vec<Value> {
        vec![
            json!({"label": "Copy"}),
            Value::Null,
            json!({"label": "Paste", "enabled": false}),
            json!({"label": "Effects", "children": [
                {"label": "Blur"}, {"label": "Glow", "checked": true}
            ]}),
            json!({"label": "Generate", "children": [{"label": "Blur"}]}),
        ]
    }

    #[test]
    fn a_named_item_is_chosen_by_label_or_path_and_never_shown() {
        let woke = Rc::new(RefCell::new(0));
        let w = woke.clone();
        let scope = begin(7, Some("Effects ▸ Glow".into()), move || {
            *w.borrow_mut() += 1
        });
        assert_eq!(claim(&[9], &menu()), None, "another window's menu shows");
        assert_eq!(
            claim(&[9, 7], &menu()),
            Some(Some(5)),
            "child window's menu"
        );
        assert_eq!(*woke.borrow(), 1);
        assert_eq!(scope.outcome().unwrap()["chose"], "Effects ▸ Glow");
        // One menu per action: a second one shows.
        assert_eq!(claim(&[7], &menu()), None);
        drop(scope);
        let scope = begin(7, Some("Copy".into()), || {});
        assert_eq!(claim(&[7], &menu()), Some(Some(0)));
        drop(scope);
        let scope = begin(7, Some("Generate > Blur".into()), || {});
        assert_eq!(claim(&[7], &menu()), Some(Some(7)));
        drop(scope);
    }

    #[test]
    fn without_a_name_the_menu_closes_and_its_items_come_back() {
        let scope = begin(3, None, || {});
        assert_eq!(claim(&[3], &menu()), Some(None));
        let out = scope.outcome().unwrap();
        assert_eq!(
            out["items"],
            json!([{"label":"Copy"},{"label":"Paste","enabled":false},
                {"label":"Effects","items":[{"label":"Blur"},{"label":"Glow","checked":true}]},
                {"label":"Generate","items":[{"label":"Blur"}]}])
        );
    }

    #[test]
    fn ambiguous_disabled_or_unknown_names_choose_nothing() {
        for (want, error) in [
            ("Blur", "More than one"),
            ("Paste", "disabled"),
            ("Cut", "No menu item"),
        ] {
            let scope = begin(4, Some(want.into()), || {});
            assert_eq!(claim(&[4], &menu()), Some(None), "{want}");
            let out = scope.outcome().unwrap();
            assert!(out["error"].as_str().unwrap().contains(error), "{out}");
            assert!(out["items"].is_array());
        }
    }

    #[test]
    fn a_late_menu_is_closed_and_reported_once_then_menus_show_again() {
        let scope = begin(5, None, || {});
        drop(scope);
        assert_eq!(claim(&[5], &menu()), Some(None), "late menu never shows");
        let late = take_late(5).unwrap();
        assert_eq!(late["late"], true);
        assert!(take_late(5).is_none());
        assert_eq!(claim(&[5], &menu()), None, "a person's menu shows");
    }
}
