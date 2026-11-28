use crate::{BUILTIN_PANELS, LockShared};
use pm_runtime_v8::{Error, Result};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Mutex};

pub(crate) const TOOLBAR_TARGETS: [&str; 7] = [
    "title",
    "tools",
    "add",
    "tasks",
    "agent",
    "export",
    "panelLibrary",
];
const TOOLBAR_ZONES: [&str; 4] = ["leading", "center", "after-add", "trailing"];
const STATUS_BAR_TARGETS: [&str; 4] = ["state", "offline", "dimensions", "workspace"];
const STATUS_BAR_SIDES: [&str; 3] = ["left", "center", "right"];
const CONTEXT_MENUS: [&str; 10] = [
    "panel:context",
    "layer:context",
    "timeline:context",
    "viewer:context",
    "assets:context",
    "inspector:context",
    "effects:context",
    "home:context",
    "toolbar:context",
    "statusBar:context",
];

/// Named slots where extensions can publish live UI.
pub const BUILTIN_SLOTS: [&str; 44] = [
    "inspector.top",
    "inspector.bottom",
    "timeline.toolbar",
    "timeline.footer",
    "viewer.hud",
    "agent.toolbar",
    "assets.toolbar",
    "toolbar.leading",
    "toolbar.center",
    "toolbar.trailing",
    "statusBar.left",
    "statusBar.center",
    "statusBar.right",
    "viewer.top",
    "viewer.bottom",
    "viewer.controls.left",
    "viewer.controls.right",
    "timeline.header",
    "timeline.top",
    "timeline.bottom",
    "inspector.header",
    "assets.top",
    "assets.bottom",
    "effects.top",
    "effects.bottom",
    "layerEffects.top",
    "layerEffects.bottom",
    "agent.top",
    "agent.bottom",
    "easing.top",
    "easing.bottom",
    "color.top",
    "color.bottom",
    "export.top",
    "export.bottom",
    "home.header",
    "home.top",
    "home.bottom",
    "panelLibrary.top",
    "panelLibrary.bottom",
    "welcome.top",
    "welcome.bottom",
    "store.top",
    "store.bottom",
];

/// Settings and its descendants are reserved for Powermove.
pub fn is_protected_surface(id: &str) -> bool {
    id.eq_ignore_ascii_case("settings")
        || ["settings.", "settings:", "panel:settings"]
            .iter()
            .any(|prefix| {
                id.get(..prefix.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
            })
}

/// Accepts a named slot or a safe panel id followed by `.top` or `.bottom`.
pub fn is_valid_slot(slot: &str) -> bool {
    if is_protected_surface(slot) {
        return false;
    }
    if BUILTIN_SLOTS.contains(&slot) {
        return true;
    }
    let Some((panel, "top" | "bottom")) =
        slot.strip_prefix("panel:").and_then(|s| s.rsplit_once('.'))
    else {
        return false;
    };
    !is_protected_surface(panel)
        && panel.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

pub(crate) fn checked_slot(input: &Value) -> Result<&str> {
    let slot = target(input, "slot")?;
    if is_valid_slot(slot) {
        Ok(slot)
    } else {
        Err(Error::Invalid("unknown built-in panel slot"))
    }
}

pub(crate) fn surfaces() -> Value {
    let slots: Vec<_> = BUILTIN_SLOTS
        .into_iter()
        .chain(["panel:<panelId>.top", "panel:<panelId>.bottom"])
        .collect();
    let replaceable_panels: Vec<_> = BUILTIN_PANELS
        .iter()
        .map(|p| p.id.as_str())
        .filter(|id| !is_protected_surface(id))
        .collect();
    json!({
        "toolbarTargets": TOOLBAR_TARGETS,
        "toolbarZones": TOOLBAR_ZONES,
        "statusBarTargets": STATUS_BAR_TARGETS,
        "statusBarSides": STATUS_BAR_SIDES,
        "slots": slots,
        "contextMenus": CONTEXT_MENUS,
        "replaceablePanels": replaceable_panels,
        "excluded": ["settings"],
    })
}

#[derive(Default)]
pub(crate) struct Contributions(Mutex<(u64, BTreeMap<String, Vec<Value>>)>);

impl Contributions {
    pub(crate) fn set(&self, owner: &str, list: Vec<Value>) {
        let mut state = self.0.shared();
        state.0 += 1;
        if list.is_empty() {
            state.1.remove(owner);
        } else {
            state.1.insert(owner.to_owned(), list);
        }
    }

    pub(crate) fn clear_owner(&self, owner: &str) {
        let mut state = self.0.shared();
        if state.1.remove(owner).is_some() {
            state.0 += 1;
        }
    }

    pub(crate) fn since(&self, seen: u64, owner_field: &str) -> Option<(u64, Vec<Value>)> {
        let state = self.0.shared();
        (state.0 != seen).then(|| {
            let list = state
                .1
                .iter()
                .flat_map(|(owner, list)| {
                    list.iter().cloned().map(move |mut item| {
                        item[owner_field] = json!(owner);
                        item
                    })
                })
                .collect();
            (state.0, list)
        })
    }
}

pub(crate) fn validate(operation: &str, input: &Value) -> Result<Vec<Value>> {
    let (limit, error) = match operation {
        "toolbar.set" => (
            32,
            "invalid toolbar item: needs an id, a label and valid options",
        ),
        "toolbar.modify.set" => (
            64,
            "invalid toolbar patch: use a known toolbar target and valid options",
        ),
        "toolbar.addMenu.set" => (
            64,
            "invalid Add menu item: needs an id, a label and valid options",
        ),
        "statusBar.set" => (
            64,
            "invalid status bar item: needs an id, a side, text and valid options",
        ),
        "statusBar.modify.set" => (
            64,
            "invalid status bar patch: use a known status bar target and valid options",
        ),
        "panelHeader.modify.set" => (
            64,
            "invalid panel header patch: needs a panelId and valid options",
        ),
        _ => unreachable!("unregistered chrome operation"),
    };
    let list = input
        .as_array()
        .filter(|list| list.len() <= limit)
        .ok_or(Error::Invalid(error))?;
    for item in list {
        let valid = match operation {
            "toolbar.set" => {
                short_text(&item["id"])
                    && short_text(&item["label"])
                    && optional(item, "zone", |v| one_of(v, &TOOLBAR_ZONES))
                    && optional(item, "variant", |v| {
                        one_of(v, &["default", "accent", "subtle"])
                    })
                    && optional(item, "order", integer)
                    && optional(item, "disabled", Value::is_boolean)
                    && optional(item, "menu", |v| valid_menu(v, 0))
            }
            "toolbar.modify.set" => {
                TOOLBAR_TARGETS.contains(&target(item, "target")?)
                    && strings(item, &["label", "icon", "tooltip", "badge"])
                    && optional(item, "hidden", Value::is_boolean)
                    && optional(item, "order", integer)
            }
            "toolbar.addMenu.set" => {
                short_text(&item["id"])
                    && short_text(&item["label"])
                    && strings(item, &["icon", "section", "tooltip"])
            }
            "statusBar.set" => {
                short_text(&item["id"])
                    && one_of(&item["side"], &STATUS_BAR_SIDES)
                    && item["text"].is_string()
                    && strings(item, &["icon", "tooltip"])
                    && optional(item, "order", integer)
                    && optional(item, "tone", |v| {
                        one_of(v, &["default", "accent", "warning", "danger"])
                    })
                    && optional(item, "clickable", Value::is_boolean)
            }
            "statusBar.modify.set" => {
                STATUS_BAR_TARGETS.contains(&target(item, "target")?)
                    && optional(item, "text", Value::is_string)
                    && optional(item, "hidden", Value::is_boolean)
            }
            "panelHeader.modify.set" => {
                !target(item, "panelId")?.is_empty()
                    && strings(item, &["title", "icon", "badge"])
                    && optional(item, "hidden", Value::is_boolean)
            }
            _ => unreachable!(),
        };
        if !valid {
            return Err(Error::Invalid(error));
        }
    }
    let mut list = list.clone();
    if operation == "toolbar.set" {
        for item in &mut list {
            let fields = item.as_object_mut().expect("validated toolbar object");
            fields.entry("zone").or_insert(json!("after-add"));
            fields.entry("order").or_insert(json!(0));
        }
    }
    Ok(list)
}

pub(crate) fn target<'a>(item: &'a Value, key: &str) -> Result<&'a str> {
    let id = item[key]
        .as_str()
        .ok_or(Error::Invalid("missing UI surface id"))?;
    if is_protected_surface(id) {
        return Err(Error::Invalid("Settings cannot be modified by extensions"));
    }
    Ok(id)
}

fn short_text(value: &Value) -> bool {
    value.as_str().is_some_and(|s| (1..=128).contains(&s.len()))
}

fn integer(value: &Value) -> bool {
    value.as_i64().is_some() || value.as_u64().is_some()
}

fn one_of(value: &Value, choices: &[&str]) -> bool {
    value.as_str().is_some_and(|v| choices.contains(&v))
}

fn optional(item: &Value, field: &str, valid: impl FnOnce(&Value) -> bool) -> bool {
    item.get(field).is_none_or(valid)
}

fn strings(item: &Value, fields: &[&str]) -> bool {
    fields
        .iter()
        .all(|field| optional(item, field, Value::is_string))
}

fn valid_menu(value: &Value, depth: usize) -> bool {
    depth <= 8
        && value.as_array().is_some_and(|list| {
            list.len() <= 64
                && list.iter().all(|item| {
                    item["label"].is_string()
                        && strings(item, &["id", "icon", "key", "actionKey"])
                        && ["checked", "disabled", "separator"]
                            .iter()
                            .all(|field| optional(item, field, Value::is_boolean))
                        && optional(item, "submenu", |v| valid_menu(v, depth + 1))
                })
        })
}
