//! Additive panel recipes from design/panels/README §6 and the Settings/viewer reviews.
//! Missing pm-design tokens stay unresolved; no fallback visual literals are permitted.
use super::{Component, Recipe, Transition};
use crate::{Node, Surface};
use pm_design::{Scheme, Tokens};
use serde_json::Value;
use std::collections::BTreeMap;

pub const WHEEL_REST_TOKEN: &str = "motion.wheel_rest";
pub const TREE_INDENT_TOKEN: &str = "metrics.tree_indent";

/// An immutable pm-design serialization. The host can pass the newer token table
/// independently of this crate's compiled pm-design version. Never populate this
/// from extension metadata: it is a host-owned design input.
#[derive(Clone, Debug)]
pub struct DesignTokenTable(Value);
impl DesignTokenTable {
    pub fn from_design(tokens: &Tokens) -> Self {
        Self(serde_json::to_value(tokens).expect("finite pm-design tokens"))
    }
    pub fn from_host(table: Value) -> Self {
        Self(table)
    }
    pub fn metric(&self, name: &str) -> Option<f64> {
        let value = name
            .split('.')
            .try_fold(&self.0, |v, key| v.get(key))?
            .as_f64()?;
        (value.is_finite()
            && value > 0.
            && (!name.starts_with("metrics.") || value <= f64::from(f32::MAX)))
        .then_some(value)
    }
    pub(crate) fn value(&self) -> &Value {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TokenMetric {
    pub name: &'static str,
    pub value: Option<f64>,
}
impl TokenMetric {
    fn new(name: &'static str, table: &DesignTokenTable) -> Self {
        Self {
            name,
            value: table.metric(name),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoKind {
    Transform,
    Rotation,
    Anchor,
    PathVertex,
    Tangent,
}
#[derive(Clone, Debug, PartialEq)]
pub struct GizmoRecipe {
    pub kind: GizmoKind,
    pub size: TokenMetric,
    pub keyline: TokenMetric,
    pub stem: Option<TokenMetric>,
    pub position: Option<(f64, f64)>,
    pub fill: u32,
    pub stroke: u32,
    pub selected: bool,
    /// Rotation/anchor accent stroke is keyline + keyline/2 (viewer spec §2).
    pub accent_stroke: Option<f64>,
    pub inner_ring: Option<f64>,
    /// Logical display points, never scaled by composition zoom.
    pub display_scaled: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ColourPopoverRecipe {
    pub sb_height: f32,
    pub padding: f32,
    pub radius: f32,
    pub thumb: f32,
    pub ring: f32,
    pub slider_track: f32,
    pub slider_thumb: f32,
    pub row_gap: f32,
    pub checker: TokenMetric,
    pub fill_modes: bool,
    pub recent_limit: usize,
    pub sampler_symbol: &'static str,
    pub sampler_label: &'static str,
    /// The native picker must graft these into the window accessibility tree.
    pub axes: [ColourAxis; 3],
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColourAxis {
    pub label: &'static str,
    pub two_dimensional: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItemPresentation {
    pub separator: bool,
    pub value: Option<Value>,
    pub label: Option<String>,
    pub disabled: bool,
    pub key_equivalent: Option<String>,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Decoration {
    EmptyState {
        symbol_size: f32,
        symbol_color: u32,
    },
    PopUpButton {
        menu: Vec<MenuItemPresentation>,
        width_sample: Option<String>,
    },
    /// NSSwitch regular, trailing. Validator requires a Settings host root.
    Switch,
    /// NSSecureTextField; no text value or input event enters the surface.
    SecureField,
    ColourWell {
        minimal: bool,
        checker: TokenMetric,
    },
    ColourPopover(ColourPopoverRecipe),
    FormGroup {
        radius: TokenMetric,
        separator_inset: f32,
    },
    FormRow {
        minimum_height: TokenMetric,
        padding: f32,
        /// Native form layout centres the trailing control on the label line,
        /// regardless of wrapping detail text underneath it.
        control_on_label_line: bool,
    },
    ViewerGizmo(GizmoRecipe),
    ViewerOverlay {
        idle: TokenMetric,
        enter: f64,
        exit: f64,
        bottom_inset: f32,
    },
    Banner {
        dismiss_after: Option<std::time::Duration>,
        announcement: Announcement,
    },
    Tree {
        indent: TokenMetric,
        disclosure_size: f32,
        hit: f32,
        leading: f32,
    },
    PushButton {
        role: ButtonRole,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Announcement {
    Medium,
    High,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonRole {
    Normal,
    Default,
    Destructive,
}

pub(crate) fn options(node: &Node) -> Option<&Value> {
    node.props.get("metadata")?.get("pmKit")
}
pub(crate) fn bool_option(node: &Node, name: &str) -> bool {
    options(node)
        .and_then(|o| o.get(name))
        .and_then(Value::as_bool)
        == Some(true)
}
pub(crate) fn str_option<'a>(node: &'a Node, name: &str) -> Option<&'a str> {
    options(node)?.get(name)?.as_str()
}
pub(crate) fn over_image(surface: &Surface, node: &Node) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        if matches!(
            Component::of(n),
            Some(Component::ViewerOverlay | Component::ViewerGizmo)
        ) || (Component::of(n) == Some(Component::Banner)
            && str_option(n, "placement") != Some("inline"))
        {
            return true;
        }
        current = n.parent.and_then(|id| surface.node(id));
    }
    false
}
pub(crate) fn settings_host(surface: &Surface, node: &Node) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        if let Some(host) = n
            .props
            .get("metadata")
            .and_then(|m| m.get("hostWindow"))
            .and_then(Value::as_str)
        {
            // Only a top-level content root declares a host window; the shell
            // must validate that declaration against its actual window type.
            return n.parent == Some(0) && host == "settings";
        }
        current = n.parent.and_then(|id| surface.node(id));
    }
    false
}

pub(crate) fn decorate(node: &Node, t: &Tokens, table: &DesignTokenTable) -> Option<Decoration> {
    let metric = |name| TokenMetric::new(name, table);
    Some(match Component::of(node)? {
        Component::EmptyState => Decoration::EmptyState {
            symbol_size: t.metrics.symbol_panel + t.metrics.symbol_panel,
            symbol_color: t.colors.muted,
        },
        Component::PopUpButton => Decoration::PopUpButton {
            menu: options(node)
                .and_then(|o| o.get("menu"))
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            let separator =
                                item.get("separator").and_then(Value::as_bool) == Some(true);
                            MenuItemPresentation {
                                separator,
                                value: if separator {
                                    None
                                } else if item.is_string() {
                                    Some(item.clone())
                                } else {
                                    item.get("v").cloned()
                                },
                                label: item
                                    .as_str()
                                    .or_else(|| item.get("label").and_then(Value::as_str))
                                    .map(str::to_owned),
                                disabled: separator
                                    || item.get("disabled").and_then(Value::as_bool) == Some(true),
                                key_equivalent: item
                                    .get("keyEquivalent")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned),
                                detail: item
                                    .get("detail")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
            width_sample: str_option(node, "widthSample").map(str::to_owned),
        },
        Component::Switch => Decoration::Switch,
        Component::SecureField => Decoration::SecureField,
        Component::ColourWell => Decoration::ColourWell {
            minimal: bool_option(node, "minimal"),
            checker: metric("metrics.checker"),
        },
        Component::ColourPopover => Decoration::ColourPopover(ColourPopoverRecipe {
            sb_height: t.metrics.popover_width / 2.,
            padding: t.metrics.inset,
            radius: t.metrics.control_radius,
            thumb: t.metrics.inset,
            ring: t.metrics.control_radius - t.metrics.inner_radius,
            // Approved concentric math in inspector.md §7 (keyframe + 3;
            // track_row / 2 + 2; gap + 4) expressed using existing tokens.
            slider_track: t.metrics.keyframe + t.metrics.drag_threshold,
            slider_thumb: t.metrics.track_row / 2.
                + (t.metrics.control_radius - t.metrics.inner_radius),
            row_gap: t.metrics.gap + t.metrics.inner_radius,
            checker: metric("metrics.checker"),
            fill_modes: bool_option(node, "fill"),
            recent_limit: 8,
            sampler_symbol: "eyedropper",
            sampler_label: "Pick a Colour From the Screen",
            axes: [
                ColourAxis {
                    label: "Saturation and brightness",
                    two_dimensional: true,
                },
                ColourAxis {
                    label: "Hue",
                    two_dimensional: false,
                },
                ColourAxis {
                    label: "Opacity",
                    two_dimensional: false,
                },
            ],
        }),
        Component::FormGroup => Decoration::FormGroup {
            radius: metric("metrics.group_radius"),
            separator_inset: t.metrics.inset,
        },
        Component::FormRow => Decoration::FormRow {
            minimum_height: metric("metrics.form_row"),
            padding: t.metrics.inset,
            control_on_label_line: true,
        },
        Component::ViewerGizmo => {
            let selected = bool_option(node, "selected");
            let (kind, size, fill, stem) = match str_option(node, "kind") {
                Some("rotation") => (
                    GizmoKind::Rotation,
                    "metrics.keyframe",
                    t.colors.viewer,
                    Some(metric("metrics.rotation_stem")),
                ),
                Some("anchor") => (
                    GizmoKind::Anchor,
                    "metrics.anchor_mark",
                    t.colors.selected_text,
                    None,
                ),
                Some("path-vertex") => (
                    GizmoKind::PathVertex,
                    "metrics.gizmo_handle",
                    if selected {
                        t.colors.accent
                    } else {
                        t.colors.floating
                    },
                    None,
                ),
                Some("tangent") => (
                    GizmoKind::Tangent,
                    "metrics.tangent_end",
                    t.colors.secondary,
                    None,
                ),
                _ => (
                    GizmoKind::Transform,
                    "metrics.gizmo_handle",
                    t.colors.accent,
                    None,
                ),
            };
            Decoration::ViewerGizmo(GizmoRecipe {
                kind,
                size: metric(size),
                keyline: {
                    let mut line = metric("metrics.gizmo_keyline");
                    line.value = line.value.map(|v| v.max(f64::from(t.metrics.border)));
                    line
                },
                position: options(node)
                    .and_then(|o| o.get("position"))
                    .and_then(|v| Some((v.get("x")?.as_f64()?, v.get("y")?.as_f64()?)))
                    .filter(|(x, y)| x.is_finite() && y.is_finite()),
                stem,
                fill,
                stroke: if kind == GizmoKind::Transform {
                    t.colors.viewer
                } else {
                    t.colors.accent
                },
                selected,
                accent_stroke: table.metric("metrics.gizmo_keyline").map(|v| {
                    if matches!(kind, GizmoKind::Rotation | GizmoKind::Anchor) {
                        v + v / 2.
                    } else {
                        v
                    }
                }),
                inner_ring: (kind == GizmoKind::Anchor)
                    .then(|| table.metric("metrics.gizmo_keyline"))
                    .flatten(),
                display_scaled: true,
            })
        }
        Component::ViewerOverlay => Decoration::ViewerOverlay {
            idle: metric("motion.overlay_idle"),
            enter: if t.accessibility.reduce_motion {
                t.motion.reduced
            } else {
                t.motion.enter
            },
            exit: if t.accessibility.reduce_motion {
                t.motion.reduced
            } else {
                t.motion.exit
            },
            bottom_inset: t.metrics.inset,
        },
        Component::Banner => Decoration::Banner {
            // Interaction deadline specified in viewer-chrome.md, not a visual duration.
            dismiss_after: (!bool_option(node, "hasAction"))
                .then_some(std::time::Duration::from_secs(4)),
            announcement: if str_option(node, "severity") == Some("error") {
                Announcement::High
            } else {
                Announcement::Medium
            },
        },
        Component::Tree | Component::TreeItem => Decoration::Tree {
            indent: metric(TREE_INDENT_TOKEN),
            disclosure_size: t.metrics.symbol_inline,
            hit: t.metrics.hit,
            leading: t.metrics.inset - (t.metrics.hit - t.metrics.symbol_inline) / 2.,
        },
        Component::PushButton => Decoration::PushButton {
            role: match str_option(node, "buttonRole") {
                Some("default") => ButtonRole::Default,
                Some("destructive") => ButtonRole::Destructive,
                _ => ButtonRole::Normal,
            },
        },
        _ => return None,
    })
}
impl Decoration {
    pub fn unresolved_tokens(&self) -> Vec<&'static str> {
        let values: Vec<&TokenMetric> = match self {
            Self::ColourWell { checker, .. } => vec![checker],
            Self::ColourPopover(p) => vec![&p.checker],
            Self::FormGroup { radius, .. } => vec![radius],
            Self::FormRow { minimum_height, .. } => vec![minimum_height],
            Self::ViewerGizmo(g) => {
                let mut v = vec![&g.size, &g.keyline];
                v.extend(g.stem.as_ref());
                v
            }
            Self::ViewerOverlay { idle, .. } => vec![idle],
            Self::Tree { indent, .. } => vec![indent],
            _ => vec![],
        };
        values
            .into_iter()
            .filter(|v| v.value.is_none())
            .map(|v| v.name)
            .collect()
    }
}

pub(crate) fn customize(recipe: &mut Recipe, node: &Node, t: &Tokens, table: &DesignTokenTable) {
    match Component::of(node) {
        Some(Component::ColourWell) if bool_option(node, "minimal") => {
            // NSColorWell .minimal owns its geometry in Settings.
            recipe.width = None;
            recipe.visual_height = None;
            recipe.radius = None;
        }
        Some(Component::FormGroup) => {
            recipe.background = if t.scheme == Scheme::Light {
                t.colors.field
            } else {
                t.colors.floating
            };
            recipe.radius = table.metric("metrics.group_radius").map(|n| n as f32);
        }
        Some(Component::FormRow) => {
            recipe.visual_height = table.metric("metrics.form_row").map(|n| n as f32)
        }
        Some(Component::ViewerOverlay | Component::Banner) => {
            recipe.radius = Some(t.metrics.capsule_radius);
            recipe.motion = Transition::CrossFade {
                duration: if t.accessibility.reduce_motion {
                    t.motion.reduced
                } else {
                    t.motion.enter
                },
            };
            if str_option(node, "placement") == Some("inline") {
                recipe.background = t.colors.panel_secondary;
                recipe.material = pm_design::Material::Solid;
                recipe.radius = Some(t.metrics.control_radius);
            } else {
                recipe.background = t.colors.floating;
            }
        }
        Some(Component::PushButton) if str_option(node, "buttonRole") == Some("destructive") => {
            recipe.foreground = t.colors.danger
        }
        _ => {}
    }
}

/// Native-only secure editor boundary: host keeps its text buffer and Keychain
/// write entirely outside the inspectable tree/JS. Methods enqueue, never wait.
pub trait SecureFieldHost {
    fn focus(&mut self, id: crate::Id, binding: &str) -> Result<(), String>;
    fn commit(&mut self, id: crate::Id) -> Result<(), String>;
    fn cancel(&mut self, id: crate::Id) -> Result<(), String>;
}
/// macOS adapter owns the native picker and NSColorSampler. Authored colour
/// strings remain in the binding; it must not quantize them on open/dismiss.
pub trait ColourPopoverHost {
    fn open(&mut self, anchor: crate::Id, binding: &str, fill: bool) -> Result<(), String>;
    fn sample_screen(&mut self, anchor: crate::Id) -> Result<(), String>;
    fn close(&mut self, anchor: crate::Id, cancel: bool) -> Result<(), String>;
}

/// Inspectable decoration publication, keyed by accepted stable surface IDs.
pub type Decorations = BTreeMap<crate::Id, Decoration>;

/// Headless Linux test doubles expose only IDs, bindings and native commands.
#[derive(Clone, Debug, PartialEq)]
pub enum SecureFieldCommand {
    Focus { id: crate::Id, binding: String },
    Commit(crate::Id),
    Cancel(crate::Id),
}
#[derive(Default)]
pub struct InspectSecureFieldHost(pub Vec<SecureFieldCommand>);
impl SecureFieldHost for InspectSecureFieldHost {
    fn focus(&mut self, id: crate::Id, binding: &str) -> Result<(), String> {
        self.0.push(SecureFieldCommand::Focus {
            id,
            binding: binding.into(),
        });
        Ok(())
    }
    fn commit(&mut self, id: crate::Id) -> Result<(), String> {
        self.0.push(SecureFieldCommand::Commit(id));
        Ok(())
    }
    fn cancel(&mut self, id: crate::Id) -> Result<(), String> {
        self.0.push(SecureFieldCommand::Cancel(id));
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum ColourPopoverCommand {
    Open {
        anchor: crate::Id,
        binding: String,
        fill: bool,
    },
    SampleScreen(crate::Id),
    Close {
        anchor: crate::Id,
        cancel: bool,
    },
}
#[derive(Default)]
pub struct InspectColourPopoverHost(pub Vec<ColourPopoverCommand>);
impl ColourPopoverHost for InspectColourPopoverHost {
    fn open(&mut self, anchor: crate::Id, binding: &str, fill: bool) -> Result<(), String> {
        self.0.push(ColourPopoverCommand::Open {
            anchor,
            binding: binding.into(),
            fill,
        });
        Ok(())
    }
    fn sample_screen(&mut self, anchor: crate::Id) -> Result<(), String> {
        self.0.push(ColourPopoverCommand::SampleScreen(anchor));
        Ok(())
    }
    fn close(&mut self, anchor: crate::Id, cancel: bool) -> Result<(), String> {
        self.0.push(ColourPopoverCommand::Close { anchor, cancel });
        Ok(())
    }
}
