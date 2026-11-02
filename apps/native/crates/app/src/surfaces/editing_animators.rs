//! S034: immutable inspector state derived from authored animator objects.
use super::Accessibility;
use pm_design::Tokens;
use pm_model::Value;
use std::collections::BTreeSet;

pub const HEADING: &str = "Text Animators";
pub const ADD: &str = "Add Text Animator";
pub const ADD_PROPERTY: &str = "Add Animator Property";
pub const STYLE: &str = "Style Selected Text";
pub const SELECT_FIRST: &str = "Select text on the canvas first";
pub const EMPTY: &str = "Add an animator to move, fade or style characters over time.";
pub const SELECTOR_ROWS: [&str; 10] = [
    "Start",
    "End",
    "Offset",
    "Unit",
    "Shape",
    "Softness",
    "Direction",
    "Order",
    "Stagger",
    "Mode",
];
pub const ACTIONS: [&str; 5] = ["Rename…", "Duplicate", "Move Up", "Move Down", "Remove"];

#[derive(Clone, Debug, PartialEq)]
pub struct Animator {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub animated: bool,
    /// The entire source is retained, including unknown extension fields.
    pub source: Value,
}
impl Animator {
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.into(),
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            enabled: value.get("enabled").and_then(Value::as_bool) != Some(false),
            animated: animated(value),
            source: value.clone(),
        })
    }
}
fn animated(value: &Value) -> bool {
    match value {
        Value::Object(o) => {
            o.get("kf")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
                || o.values().any(animated)
        }
        Value::Array(a) => a.iter().any(animated),
        _ => false,
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatorPlan {
    pub id: String,
    pub ax: Accessibility,
    pub enabled_label: String,
    pub expanded: bool,
    pub text_colour: u32,
    pub rows_editable: bool,
    pub half_diamond: bool,
    pub menu: Vec<(&'static str, bool, bool)>,
    pub remove_colour: u32,
    pub selector_heading: &'static str,
    pub properties_heading: &'static str,
}
#[derive(Clone, Debug, Default)]
pub struct Animators {
    pub items: Vec<Animator>,
    pub collapsed: BTreeSet<String>,
    pub locked: bool,
    pub text_selection: Option<(String, usize, usize)>,
    pub layer: String,
}
impl Animators {
    pub fn plans(&self, tokens: &Tokens) -> Vec<AnimatorPlan> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let expanded = !self.collapsed.contains(&a.id);
                AnimatorPlan {
                    id: a.id.clone(),
                    ax: Accessibility {
                        id: format!("animators/{}/{}", self.layer, a.id),
                        role: "AXGroup",
                        label: format!("{}, animator {} of {}", a.name, i + 1, self.items.len()),
                        selected: None,
                        enabled: !self.locked,
                    },
                    enabled_label: format!("{} enabled", a.name),
                    expanded,
                    text_colour: if a.enabled {
                        tokens.colors.text
                    } else {
                        tokens.colors.muted
                    },
                    rows_editable: !self.locked,
                    half_diamond: !expanded && a.animated,
                    menu: ACTIONS
                        .into_iter()
                        .enumerate()
                        .map(|(action, label)| {
                            (
                                label,
                                !self.locked
                                    && (action != 2 || i > 0)
                                    && (action != 3 || i + 1 < self.items.len()),
                                action == 4,
                            )
                        })
                        .collect(),
                    remove_colour: tokens.colors.danger,
                    selector_heading: "Range",
                    properties_heading: "Properties",
                }
            })
            .collect()
    }
    pub fn can_style(&self) -> bool {
        !self.locked
            && self
                .text_selection
                .as_ref()
                .is_some_and(|(layer, start, end)| layer == &self.layer && end > start)
    }
    pub fn empty_copy(&self) -> Option<&'static str> {
        self.items.is_empty().then_some(EMPTY)
    }
    pub fn toggle(&mut self, id: &str) -> bool {
        if !self.items.iter().any(|a| a.id == id) {
            return false;
        }
        if !self.collapsed.remove(id) {
            self.collapsed.insert(id.into());
        }
        true
    }
    pub fn available_properties<'a>(
        &self,
        id: &str,
        catalogue: &'a [(&str, &str)],
    ) -> Vec<(&'a str, &'a str)> {
        let Some(a) = self.items.iter().find(|a| a.id == id) else {
            return vec![];
        };
        catalogue
            .iter()
            .copied()
            .filter(|(key, _)| a.source.get("p").and_then(|p| p.get(key)).is_none())
            .collect()
    }
}
