//! Publication-time compilation of accepted surfaces into immutable render facts.
//! Runs on a worker. Render reads these facts; it never parses JSON or token paths.
use gpui::{Styled, *};
use pm_design::Tokens;
use pm_surface::{
    Id, Node, Surface,
    kit::{Component, Decoration, Panel},
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

/// One flattened lookup per token set. Rebuilt only when the theme changes.
pub struct TokenTable {
    pub tokens: Tokens,
    values: HashMap<String, Value>,
}
impl TokenTable {
    pub fn new(tokens: Tokens) -> Self {
        fn flatten(prefix: String, value: Value, out: &mut HashMap<String, Value>) {
            if let Value::Object(map) = value {
                for (key, value) in map {
                    let path = if prefix.is_empty() {
                        key
                    } else {
                        format!("{prefix}.{key}")
                    };
                    flatten(path, value, out);
                }
            } else {
                out.insert(prefix, value);
            }
        }
        let mut values = HashMap::new();
        flatten(
            String::new(),
            serde_json::to_value(&tokens).expect("finite pm-design tokens"),
            &mut values,
        );
        Self { tokens, values }
    }
    fn resolve<'a>(&'a self, value: &'a Value) -> &'a Value {
        value
            .get("token")
            .and_then(Value::as_str)
            .map_or(value, |name| self.values.get(name).unwrap_or(&Value::Null))
    }
    fn number(&self, value: &Value) -> Option<f32> {
        self.resolve(value).as_f64().map(|n| n as f32)
    }
    fn color(&self, value: &Value) -> Option<u32> {
        match self.resolve(value) {
            Value::Number(n) => n.as_u64().map(|n| n as u32),
            Value::String(s) => parse_hex(s),
            _ => None,
        }
    }
}
/// A value well's fill: the ink at the wash step, so every well of a row
/// (a field, a pop-up, a colour, a menu trigger) is one value on a
/// translucent panel as on a solid one. An opaque `colors.field` nearly
/// vanished on the panel's composite.
pub fn well(t: &pm_design::Tokens) -> gpui::Rgba {
    pm_shell::kit::ink(t.colors.text, t.translucency.hover)
}
pub fn parse_hex(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#')?;
    let value = u32::from_str_radix(hex.get(..6)?, 16).ok()?;
    Some(value)
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Events {
    pub click: bool,
    pub double_click: bool,
    pub context_menu: bool,
    pub pointer_down: bool,
    pub pointer_move: bool,
    pub pointer_up: bool,
    pub hover: bool,
    pub wheel: bool,
    /// Takes files dropped from Finder (`data-pm-drop="files"`).
    pub file_drop: bool,
    pub key_down: bool,
}
impl Events {
    fn of(node: &Node) -> Self {
        let has = |name: &str| node.listeners.contains_key(name);
        Self {
            click: has("click"),
            double_click: has("doubleclick"),
            context_menu: has("contextmenu"),
            pointer_down: has("pointerdown"),
            pointer_move: has("pointermove"),
            pointer_up: has("pointerup"),
            hover: has("pointerenter") || has("pointerleave"),
            wheel: has("wheel"),
            file_drop: (has("drop") || has("dragover"))
                && node.props.get("data-pm-drop").and_then(Value::as_str) == Some("files"),
            key_down: has("keydown"),
        }
    }
    pub fn pointer(self) -> bool {
        self.pointer_down || self.pointer_move || self.pointer_up
    }
}
#[derive(Clone, Debug, Default)]
pub struct Ax {
    pub role: Option<Role>,
    pub label: Option<SharedString>,
    pub value: Option<SharedString>,
    pub disabled: bool,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    /// Checked state of radios, checkboxes and pressed buttons.
    pub toggled: Option<Toggled>,
}
/// Facts that vary by kit component and have no generic style expression.
#[derive(Clone, Debug, Default)]
pub enum Kit {
    #[default]
    None,
    Check(bool),
    Swatch {
        color: Option<u32>,
        keyline: u32,
    },
    Progress(Option<f32>),
    Anchored {
        anchor: Option<Id>,
        point: Option<(f32, f32)>,
        dismiss: bool,
    },
    Gizmo {
        position: (f32, f32),
        size: f32,
        fill: u32,
        stroke: u32,
        keyline: f32,
        round: bool,
    },
    Banner {
        symbol: Option<(SharedString, f32, u32)>,
    },
    Secure,
    Disclosure(bool),
    /// A colour wheel: its red, green and blue, where they are neutral, and
    /// the limits of one channel.
    Wheel {
        rgb: [f64; 3],
        neutral: f64,
        range: [f64; 2],
    },
    /// A curve editor: `[count, x0, y0, …]` and its channel's ink.
    Curve {
        values: Vec<f64>,
        channel: usize,
    },
}
#[derive(Clone, Debug)]
pub struct NodeView {
    /// A hosted control in a row's value column: field high, on the row's
    /// drawn well (a pop-up has no bezel of its own there).
    pub well: bool,
    /// A text field inside a well its row draws (a colour's hex).
    pub bare: bool,
    /// A text field for source text (`metadata.code`): the fixed-width
    /// face, no spelling or substitutions.
    pub code: bool,
    pub id: Id,
    pub parent: Option<Id>,
    pub children: Vec<Id>,
    pub component: Option<Component>,
    pub style: StyleRefinement,
    pub hidden: bool,
    pub scroll_y: bool,
    pub scroll_x: bool,
    pub semantic: Option<SharedString>,
    pub text: Option<SharedString>,
    pub symbol: Option<SharedString>,
    pub symbol_size: f32,
    pub disabled: bool,
    pub events: Events,
    pub ax: Option<Ax>,
    pub tab_index: Option<isize>,
    pub title: Option<SharedString>,
    pub row_count: Option<usize>,
    pub native: Option<SharedString>,
    pub kit: Kit,
    pub virtual_window: Option<pm_builtins::virtualization::Window>,
    pub is_plain_text: bool,
    pub binding: Option<SharedString>,
    pub number: Option<NumberFacts>,
    pub local_change: bool,
    /// Escape reports an empty change, so a transient editor can close.
    pub cancel_change: bool,
    pub timeline_scrub: Option<(SharedString, SharedString)>,
    pub outline_column: bool,
    pub outline_first: Option<SharedString>,
    pub outline_window: Option<(usize, usize)>,
    /// A number field's unit, drawn after the value in `muted` (language §5).
    pub unit: Option<SharedString>,
    /// `metadata.fixedWidthSample`: the node is at least as wide as this text in
    /// its own font, so a changing readout (timecode) never reflows its row.
    pub fixed_width: Option<SharedString>,
    /// `data-pm-slot="body"`: the part of a sheet between its title and its
    /// actions, which gives way while the sheet's height changes.
    pub sheet_body: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NumberFacts {
    pub value: f64,
    pub step: f64,
    pub speed: f64,
    pub min: f64,
    pub max: f64,
    pub precision: Option<u32>,
}
pub struct Prepared {
    pub nodes: HashMap<Id, NodeView>,
    pub semantics: HashMap<String, Id>,
    pub header: Option<Id>,
    pub has_keydown: bool,
    pub anchors: Vec<Id>,
}
impl NumberFacts {
    /// The shipped field's reading: the declared precision, else two places
    /// for fine steps, one under ten and none above; trailing zeros trimmed.
    /// Without a declared precision, a small value keeps two significant
    /// digits.
    pub fn text(self, value: f64) -> String {
        let mut places = self.precision.map_or(
            if self.step > 0. && self.step < 1. {
                2
            } else if value.abs() < 10. {
                1
            } else {
                0
            },
            |digits| digits as usize,
        );
        // A field without a declared precision keeps two significant digits
        // of a small value (fog density 0.002, not "0").
        if self.precision.is_none()
            && value != 0.
            && value.is_finite()
            && value.abs() * 10f64.powi(places as i32) < 10.
        {
            places = places.max(((-value.abs().log10()).ceil() as usize + 1).min(6));
        }
        let mut text = format!("{value:.places$}");
        if places > 0 && text.contains('.') {
            text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
        }
        if text == "-0" {
            text = "0".into();
        }
        text
    }
}
impl Prepared {
    pub fn new(surface: Arc<Surface>, table: &TokenTable) -> Self {
        let panel = Panel::prepare(
            surface.clone(),
            &table.tokens,
            &Default::default(),
            None,
            false,
        );
        let mut axes: HashMap<Id, Ax> = panel
            .accessibility
            .nodes
            .iter()
            .map(|(id, n)| {
                (
                    id.0,
                    Ax {
                        role: (n.role() != Role::GenericContainer).then(|| n.role()),
                        label: n.label().map(|s| s.to_owned().into()),
                        value: n.value().map(|s| s.to_owned().into()),
                        disabled: n.is_disabled(),
                        selected: n.is_selected(),
                        expanded: n.is_expanded(),
                        toggled: n.toggled(),
                    },
                )
            })
            .collect();
        let mut nodes = HashMap::with_capacity(surface.nodes().len());
        let mut semantics = HashMap::new();
        let mut anchors = vec![];
        let mut has_keydown = false;
        for node in surface.nodes().values() {
            let view = compile(&surface, node, &panel, table, axes.remove(&node.id));
            if let Some(semantic) = &view.semantic {
                semantics.entry(semantic.to_string()).or_insert(node.id);
            }
            if let Kit::Anchored {
                anchor: Some(id), ..
            } = view.kit
            {
                anchors.push(id);
            }
            // A control that opens a menu: the menu hangs from where it is.
            if node.props.contains_key("aria-haspopup") {
                anchors.push(node.id);
            }
            has_keydown |= view.events.key_down;
            nodes.insert(node.id, view);
        }
        let header = header(&surface);
        Self {
            nodes,
            semantics,
            header,
            has_keydown,
            anchors,
        }
    }
    pub fn node(&self, id: Id) -> Option<&NodeView> {
        self.nodes.get(&id)
    }
}
/// The root's child the panel declares as its header (`data-pm-slot="header"`):
/// the shell shows it in the panel's header row.
fn header(surface: &Surface) -> Option<Id> {
    let root = surface
        .node(0)?
        .children
        .first()
        .and_then(|id| surface.node(*id))?;
    root.children.iter().copied().find(|id| {
        surface.node(*id).is_some_and(|node| {
            node.props.get("data-pm-slot").and_then(Value::as_str) == Some("header")
        })
    })
}
/// How the host presents an element (`data-pm-variant`).
pub fn variant(node: &Node) -> Option<&str> {
    node.props.get("data-pm-variant").and_then(Value::as_str)
}
fn compile(
    surface: &Surface,
    node: &Node,
    panel: &Panel,
    table: &TokenTable,
    ax: Option<Ax>,
) -> NodeView {
    let t = &table.tokens;
    let component = Component::of(node);
    let mut style = surface.computed_style(node.id).unwrap_or_default();
    // The shell (or the panel's own window) paints the panel material. A panel
    // root that repeats `colors.panel` would turn the translucent material opaque.
    let root_fill = ["background", "backgroundColor"].iter().any(|k| {
        style
            .get(*k)
            .and_then(|v| v.get("token"))
            .and_then(Value::as_str)
            == Some("colors.panel")
    });
    if root_fill && (node.id == 0 || node.parent == Some(0)) {
        style.remove("background");
        style.remove("backgroundColor");
    }
    let hidden = style.get("display").and_then(Value::as_str) == Some("none");
    let mut sheet = StyleRefinement::default().flex().flex_col().min_w(px(0.));
    if node.id == 0 || node.parent == Some(0) {
        sheet = sheet.size_full();
    }
    if let Some(recipe) = panel.components.get(&node.id) {
        let radius = recipe.radius.unwrap_or(t.metrics.control_radius);
        sheet = match component {
            // Quiet source-list rows: selection and hover inset from the panel
            // edge by `gap`, text a further `gap` in (language §9 Lists).
            Some(Component::ListItem | Component::TreeItem) if variant(node) != Some("band") => {
                sheet
                    .flex_row()
                    .items_center()
                    .min_h(px(t.metrics.row))
                    .mx(px(t.metrics.gap))
                    .px(px(t.metrics.gap))
                    .gap(px(t.metrics.gap))
                    // A selected or hovered row is never a square slab.
                    .rounded(px(radius))
            }
            // Band rows (the timeline outline) share their band with native
            // track rows: selection spans the whole row, as in After Effects.
            Some(Component::InspectorRow | Component::ListItem | Component::TreeItem) => {
                sheet.flex_row().items_center().min_h(px(t.metrics.row))
            }
            // A section heading starts on the row-label inset; its chevron is
            // trailing and shows on hover or while collapsed (language §9).
            Some(Component::DisclosureButton) => sheet
                .flex_row_reverse()
                .justify_between()
                .items_center()
                .w_full()
                .gap(px(t.metrics.inner_radius))
                .px(px(t.metrics.inset))
                .min_h(px(recipe.hit))
                .rounded(px(radius)),
            Some(Component::ToolbarItem) => sheet
                .flex_row()
                .items_center()
                .justify_center()
                .min_w(px(recipe.hit))
                .min_h(px(recipe.hit))
                .rounded(px(radius)),
            Some(
                Component::PopUpButton
                | Component::NumberField
                | Component::TextField
                | Component::SecureField,
            ) => sheet
                .flex_row()
                .items_center()
                .px(px(t.metrics.gap))
                .h(px(recipe.visual_height.unwrap_or(t.metrics.field)))
                .bg(rgb(recipe.background))
                .rounded(px(radius)),
            Some(Component::SegmentedControl) => sheet
                .flex_row()
                .p(px(recipe.track_padding.unwrap_or(0.)))
                .bg(rgb(recipe.background))
                .rounded(px(radius)),
            // Segments fill their track's height (stretch), concentric inside it.
            Some(Component::Segment) => sheet
                .flex_row()
                .items_center()
                .justify_center()
                .px(px(t.metrics.gap)),
            Some(Component::FormGroup) => sheet
                .bg(rgb(recipe.background))
                .rounded(px(t.metrics.group_radius))
                .overflow_hidden(),
            Some(Component::FormRow) => sheet
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(t.metrics.gap))
                .px(px(t.metrics.inset)),
            Some(Component::EmptyState) => sheet
                .flex_1()
                .items_center()
                .justify_center()
                .gap(px(t.metrics.gap)),
            Some(Component::List | Component::Tree) => sheet.flex_1(),
            // Sections stand a gap apart; the heading is a row of its own.
            Some(Component::DisclosureGroup) => sheet.w_full().mt(px(t.metrics.gap)),
            // Indeterminate: a small spinner leads the label, as AppKit's
            // spinning indicator beside "Checking…"; determinate: a bar
            // under it.
            Some(Component::ProgressRow) if node.props.get("aria-valuenow").is_none() => {
                sheet.flex_row().items_center().gap(px(t.metrics.gap))
            }
            Some(Component::ProgressRow) => sheet.gap(px(t.metrics.inner_radius)).justify_center(),
            Some(Component::Banner) => sheet
                .flex_row()
                .items_center()
                .gap(px(t.metrics.gap))
                .p(px(t.metrics.gap))
                .bg(rgb(recipe.background))
                .rounded(px(t.metrics.control_radius)),
            Some(Component::ViewerOverlay) => sheet
                .absolute()
                .bottom(px(t.metrics.inset))
                .flex_row()
                .items_center()
                .gap(px(t.metrics.gap))
                .px(px(t.metrics.gap))
                .bg(rgb(t.colors.floating))
                .rounded(px(t.metrics.control_radius)),
            Some(Component::SheetAnchor | Component::PopoverAnchor | Component::ColourPopover) => {
                let mut floating = sheet
                    .bg(rgb(recipe.background))
                    .rounded(px(radius))
                    .p(px(t.metrics.inset))
                    .gap(px(t.metrics.gap));
                if let Some(width) = recipe.width {
                    floating = floating.w(px(width));
                }
                floating
            }
            _ => sheet,
        };
    }
    let mut compiled = apply(sheet, &style, table);
    // Wells in an inspector value column share it equally (Position X | Y)
    // whether or not they are leased native fields (language §9).
    let in_value_slot = node
        .parent
        .and_then(|p| surface.node(p))
        .is_some_and(|p| p.props.get("data-pm-slot").and_then(Value::as_str) == Some("value"));
    let slot_control = in_value_slot
        && matches!(
            component,
            Some(
                Component::NumberField
                    | Component::TextField
                    | Component::PopUpButton
                    | Component::SegmentedControl
            )
        );
    if slot_control {
        compiled.sheet = compiled.sheet.flex_1().min_w(px(0.)).overflow_hidden();
    }
    // A colour is one well: its chip, then its hex, which draws no well of
    // its own.
    let siblings = |of: Option<Id>| -> Vec<Option<Component>> {
        of.and_then(|p| surface.node(p))
            .map(|p| {
                p.children
                    .iter()
                    .filter_map(|c| surface.node(*c))
                    .map(Component::of)
                    .collect()
            })
            .unwrap_or_default()
    };
    let joined = |of: Option<Id>| {
        let kinds = siblings(of);
        kinds.contains(&Some(Component::ColourWell)) && kinds.contains(&Some(Component::TextField))
    };
    let slot = node.props.get("data-pm-slot").and_then(Value::as_str);
    let in_colour = in_value_slot && joined(node.parent);
    // In an inspector value column a segmented control is a compact,
    // field-high track filling the column (above); its segments share it
    // equally (Align: three cells, as wide as the wells beside it) and stay
    // concentric (language §9).
    let slot_of = |id: Option<Id>| {
        id.and_then(|p| surface.node(p))
            .and_then(|p| p.props.get("data-pm-slot"))
            .and_then(Value::as_str)
            == Some("value")
    };
    let compact = match component {
        Some(Component::SegmentedControl) => in_value_slot,
        Some(Component::Segment) => slot_of(node.parent.and_then(|p| surface.node(p)?.parent)),
        _ => false,
    };
    if compact {
        let inset = t.metrics.control_radius - t.metrics.inner_radius;
        compiled.sheet = if component == Some(Component::Segment) {
            compiled
                .sheet
                .flex_1()
                .px(px(t.metrics.gap))
                .rounded(px(t.metrics.inner_radius))
        } else {
            compiled
                .sheet
                .h(px(t.metrics.field))
                .p(px(inset))
                .gap(px(inset))
                .rounded(px(t.metrics.inner_radius))
        };
    }
    // A host-menu trigger in the value column (Font, Fill) reads as a well.
    // Icon actions beside wells (Position's Pad) keep their hit square.
    let titled = !node.children.is_empty() || !node.text.is_empty();
    if in_value_slot && titled && component == Some(Component::ToolbarItem) {
        compiled.sheet = compiled
            .sheet
            .flex_1()
            .min_w(px(0.))
            .overflow_hidden()
            .h(px(t.metrics.field))
            .min_h(px(t.metrics.field))
            .px(px(t.metrics.gap))
            .bg(well(t))
            .rounded(px(t.metrics.inner_radius));
    }
    // A colour well is a field-high square holding its chip; alone it is
    // a well itself, beside its hex it stands in the row's well.
    if component == Some(Component::ColourWell) {
        let side = px(t.metrics.field);
        compiled.sheet = compiled
            .sheet
            .flex_none()
            .flex_row()
            .items_center()
            .justify_center()
            .w(side)
            .min_w(side)
            .h(side)
            .min_h(side)
            .rounded(px(t.metrics.inner_radius));
        if !in_colour {
            compiled.sheet = compiled.sheet.bg(well(t));
        }
    }
    // The hex beside it draws no well of its own, with or without an editor
    // (a locked layer's hex is the dimmed value with no editor).
    if in_colour && component == Some(Component::TextField) {
        compiled.sheet.background = None;
    }
    let in_row = node
        .parent
        .and_then(|p| surface.node(p))
        .is_some_and(|p| Component::of(p) == Some(Component::InspectorRow));
    let has_slot = |row: &Node, name: &str| {
        row.children.iter().any(|c| {
            surface
                .node(*c)
                .is_some_and(|c| c.props.get("data-pm-slot").and_then(Value::as_str) == Some(name))
        })
    };
    // A row's keyframe mark hangs in the gutter: the mark's own leading
    // point stands on the panel's text edge, where headings start.
    let hang = (t.metrics.hit - t.metrics.symbol_inline) / 2.;
    if component == Some(Component::InspectorRow) && has_slot(node, "leading") {
        compiled.sheet = compiled.sheet.pl(px((t.metrics.inset - hang).max(0.)));
    }
    match slot {
        // The gutter a row's keyframe mark stands in; a row with no mark
        // keeps it, so every label starts on one x.
        Some("leading") if in_row => {
            compiled.sheet = compiled
                .sheet
                .flex_none()
                .flex_row()
                .items_center()
                .justify_center()
                .w(px(t.metrics.hit))
                .mr(px(t.space.hair));
        }
        // A row's label keeps its column and gives way first in a narrow
        // panel; the value takes the rest, never less than `value_column`.
        Some("label") if in_row => {
            compiled.sheet = compiled
                .sheet
                .flex_none()
                .flex_shrink(1.)
                .w(px(t.metrics.label_column))
                .min_w(px(t.metrics.hit))
                .pr(px(t.metrics.gap));
        }
        Some("label") => compiled.sheet = compiled.sheet.flex_1(),
        Some("value") => {
            compiled.sheet = compiled
                .sheet
                .flex_1()
                .flex_row()
                .items_center()
                .min_w(px(t.metrics.value_column))
                .gap(px(t.metrics.inner_radius));
            if joined(Some(node.id)) {
                compiled.sheet = compiled
                    .sheet
                    .h(px(t.metrics.field))
                    .gap(px(0.))
                    .pr(px(t.metrics.gap))
                    .bg(well(t))
                    .rounded(px(t.metrics.inner_radius));
            }
        }
        Some("trailing") => {
            compiled.sheet = compiled
                .sheet
                .flex_none()
                .w(px(t.metrics.hit))
                .items_center()
                .justify_center();
        }
        // What stands under a row (its expression, what is wrong with its
        // value): from the label's x to the panel's inset.
        Some("detail") => {
            compiled.sheet = compiled
                .sheet
                .gap(px(t.space.hair))
                .pl(px(t.metrics.inset - hang + t.metrics.hit + t.space.hair))
                .pr(px(t.metrics.inset))
                .pb(px(t.metrics.gap));
        }
        // A section's heading row: its title, and its action at the
        // trailing edge.
        Some("heading") => {
            compiled.sheet = compiled.sheet.flex_row().items_center().w_full();
        }
        Some("accessory") => {
            compiled.sheet = compiled
                .sheet
                .flex_none()
                .flex_row()
                .items_center()
                .pr(px(t.metrics.gap));
        }
        _ => {}
    }
    let selected = node.props.get("aria-selected") == Some(&Value::Bool(true))
        || node.props.get("state").and_then(|s| s.get("selected")) == Some(&Value::Bool(true));
    if selected {
        compiled.sheet = compiled.sheet.bg(rgb(t.colors.selected_fill));
    }
    let header_id = node.props.get("id").and_then(Value::as_str);
    let disabled = node.props.get("disabled") == Some(&Value::Bool(true));
    let symbol = node
        .props
        .get("metadata")
        .and_then(|m| m.get("symbol").or_else(|| m.get("requiredSymbol")))
        .or_else(|| node.props.get("attr").and_then(|m| m.get("symbol")))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| SharedString::from(s.to_owned()));
    let virtual_window = (node.r#type == "virtual-list")
        .then(|| {
            let count = node.props.get("count")?.as_u64()?;
            let range = node.props.get("range")?.as_array()?;
            let size = node
                .props
                .get("itemSize")
                .and_then(Value::as_f64)
                .or_else(|| {
                    (node
                        .props
                        .get("metadata")
                        .and_then(|m| m.get("rowHeightToken"))
                        .and_then(Value::as_str)
                        == Some("metrics.row"))
                    .then_some(t.metrics.row as f64)
                })?;
            pm_builtins::virtualization::Window::new(
                count as usize,
                (
                    range.first()?.as_u64()? as usize,
                    range.get(1)?.as_u64()? as usize,
                ),
                size,
            )
            .ok()
        })
        .flatten();
    let meta = node.props.get("metadata");
    NodeView {
        well: slot_control
            && matches!(
                component,
                Some(Component::PopUpButton | Component::SegmentedControl)
            ),
        bare: in_colour && component == Some(Component::TextField),
        code: meta.and_then(|m| m.get("code")) == Some(&Value::Bool(true)),
        sheet_body: slot == Some("body"),
        fixed_width: meta
            .and_then(|m| m.get("fixedWidthSample"))
            .and_then(Value::as_str)
            .map(|s| s.to_owned().into()),
        local_change: node.listeners.contains_key("change"),
        cancel_change: meta
            .and_then(|m| m.get("cancelChange"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        timeline_scrub: meta.and_then(|m| m.get("timelineScrub")).and_then(|t| {
            Some((
                SharedString::from(t.get("layer")?.as_str()?.to_owned()),
                SharedString::from(t.get("path")?.as_str()?.to_owned()),
            ))
        }),
        outline_column: meta.and_then(|m| m.get("nativeColumn")).is_some(),
        outline_first: meta
            .and_then(|m| m.get("nativeFirstRow"))
            .and_then(Value::as_str)
            .map(|s| s.to_owned().into()),
        outline_window: meta
            .and_then(|m| m.get("nativeWindow"))
            .and_then(Value::as_array)
            .and_then(|w| Some((w.first()?.as_u64()? as usize, w.get(1)?.as_u64()? as usize))),
        id: node.id,
        parent: node.parent,
        children: node.children.clone(),
        component,
        style: compiled.sheet,
        hidden,
        // The native timeline scrolls its outline; a nested scroller would double it.
        scroll_y: meta.and_then(|m| m.get("nativeFirstRow")).is_none()
            && (compiled.scroll_y
                || matches!(component, Some(Component::List | Component::Tree))
                || matches!(node.r#type.as_str(), "scroll" | "virtual-list")),
        scroll_x: compiled.scroll_x,
        semantic: header_id.map(|s| SharedString::from(s.to_owned())),
        text: (!node.text.is_empty()).then(|| node.text.clone().into()),
        symbol,
        symbol_size: compiled.symbol_size.unwrap_or(t.metrics.symbol_panel),
        disabled,
        events: Events::of(node),
        ax,
        tab_index: node
            .props
            .get("tabIndex")
            .and_then(Value::as_i64)
            .map(|n| n as isize),
        title: node
            .props
            .get("title")
            .and_then(Value::as_str)
            .map(|s| s.to_owned().into()),
        row_count: node
            .props
            .get("metadata")
            .and_then(|m| m.get("totalRows").or_else(|| m.get("totalCount")))
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .or(virtual_window.map(|w| w.count)),
        native: matches!(node.r#type.as_str(), "canvas" | "frame")
            .then(|| node.props.get("surface").and_then(Value::as_str))
            .flatten()
            .map(|s| s.to_owned().into()),
        kit: kit(node, component, panel, table),
        virtual_window,
        is_plain_text: node.r#type == "text" && node.props.is_empty() && node.children.is_empty(),
        binding: node
            .props
            .get("binding")
            .and_then(Value::as_str)
            .map(|s| s.to_owned().into()),
        unit: (component == Some(Component::NumberField))
            .then(|| node.props.get("unit").and_then(Value::as_str))
            .flatten()
            .filter(|u| !u.is_empty())
            // Split wells (Position X | Y) keep their values whole: the pair's
            // units give way when any value and unit would not fit (the row names it).
            .filter(|u| {
                let Some(slot) = node.parent.and_then(|p| surface.node(p)).filter(|slot| {
                    slot.props.get("data-pm-slot").and_then(Value::as_str) == Some("value")
                }) else {
                    return true;
                };
                let wells: Vec<&Node> = slot
                    .children
                    .iter()
                    .filter_map(|c| surface.node(*c))
                    .filter(|c| Component::of(c) == Some(Component::NumberField))
                    .collect();
                wells.len() < 2
                    || wells.iter().all(|w| {
                        let shown = w
                            .props
                            .get("value")
                            .and_then(Value::as_f64)
                            .map_or(0, |v| number_facts(w).text(v).chars().count());
                        shown + 1 + u.chars().count() <= SPLIT_WELL_CHARS
                    })
            })
            .map(|u| SharedString::from(u.to_owned())),
        number: (component == Some(Component::NumberField)).then(|| number_facts(node)),
    }
}
/// Characters a split value well shows at `body` size (half the value
/// column of a panel at its usual width, less a trailing control).
const SPLIT_WELL_CHARS: usize = 9;
fn number_facts(node: &Node) -> NumberFacts {
    let number = |key: &str, default| {
        node.props
            .get(key)
            .and_then(Value::as_f64)
            .unwrap_or(default)
    };
    NumberFacts {
        value: number("value", 0.),
        step: number("step", 1.),
        speed: number("speed", 0.5),
        min: number("min", -f64::MAX),
        max: number("max", f64::MAX),
        precision: node
            .props
            .get("precision")
            .and_then(Value::as_u64)
            .map(|n| n.min(9) as u32),
    }
}
/// A list of numbers (a grouped control's values); anything else is none.
fn numbers(value: Option<&Value>) -> Vec<f64> {
    value
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}
fn kit(node: &Node, component: Option<Component>, panel: &Panel, table: &TokenTable) -> Kit {
    let t = &table.tokens;
    let decoration = panel.decorations.get(&node.id);
    let value = node.props.get("value");
    match component {
        Some(Component::Checkbox | Component::Toggle) => {
            Kit::Check(value.and_then(Value::as_bool).unwrap_or(false))
        }
        Some(Component::ColourWell) => Kit::Swatch {
            color: value.and_then(Value::as_str).and_then(parse_hex),
            keyline: t.colors.separator,
        },
        Some(Component::ProgressRow) => Kit::Progress(
            node.props
                .get("aria-valuenow")
                .and_then(Value::as_f64)
                .map(|n| n.clamp(0., 1.) as f32),
        ),
        Some(Component::SheetAnchor | Component::PopoverAnchor | Component::ColourPopover) => {
            let anchor = node.props.get("anchor");
            Kit::Anchored {
                anchor: anchor.and_then(Value::as_u64),
                point: anchor.and_then(|a| {
                    Some((a.get("x")?.as_f64()? as f32, a.get("y")?.as_f64()? as f32))
                }),
                dismiss: node.props.get("data-pm-dismiss-outside") == Some(&Value::Bool(true)),
            }
        }
        Some(Component::ViewerGizmo) => match decoration {
            Some(Decoration::ViewerGizmo(g)) => Kit::Gizmo {
                position: g.position.map_or((0., 0.), |(x, y)| (x as f32, y as f32)),
                size: g.size.value.unwrap_or(t.metrics.keyframe as f64) as f32,
                fill: g.fill,
                stroke: g.stroke,
                keyline: g.keyline.value.unwrap_or(t.metrics.border as f64) as f32,
                round: matches!(
                    g.kind,
                    pm_surface::kit::GizmoKind::Rotation | pm_surface::kit::GizmoKind::Anchor
                ),
            },
            _ => Kit::None,
        },
        Some(Component::Banner) => Kit::Banner {
            symbol: match decoration {
                Some(Decoration::Banner {
                    symbol_size,
                    symbol_color,
                    ..
                }) => node
                    .props
                    .get("metadata")
                    .and_then(|m| m.get("pmKit").or(Some(m)))
                    .and_then(|m| m.get("symbol"))
                    .and_then(Value::as_str)
                    .map(|s| {
                        (
                            SharedString::from(s.to_owned()),
                            *symbol_size,
                            *symbol_color,
                        )
                    }),
                _ => None,
            },
        },
        Some(Component::SecureField) => Kit::Secure,
        Some(Component::ColorWheel) => {
            let numbers = numbers(value);
            let option = |key: &str| {
                node.props
                    .get("metadata")
                    .and_then(|m| m.get("pmKit"))
                    .and_then(|k| k.get(key))
                    .and_then(Value::as_f64)
            };
            let neutral = option("neutral").unwrap_or(0.);
            Kit::Wheel {
                rgb: std::array::from_fn(|i| numbers.get(i).copied().unwrap_or(neutral)),
                neutral,
                range: [option("min").unwrap_or(-1.), option("max").unwrap_or(1.)],
            }
        }
        Some(Component::CurveEditor) => Kit::Curve {
            values: numbers(value),
            channel: node
                .props
                .get("metadata")
                .and_then(|m| m.get("pmKit"))
                .and_then(|k| k.get("channel"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(3) as usize,
        },
        Some(Component::DisclosureButton) => {
            Kit::Disclosure(node.props.get("aria-expanded") == Some(&Value::Bool(true)))
        }
        _ => Kit::None,
    }
}
struct Compiled {
    sheet: StyleRefinement,
    scroll_y: bool,
    scroll_x: bool,
    symbol_size: Option<f32>,
}
#[derive(Clone, Copy)]
enum Len {
    Px(f32),
    Fraction(f32),
}
impl From<Len> for Length {
    fn from(value: Len) -> Self {
        match value {
            Len::Px(n) => px(n).into(),
            Len::Fraction(n) => relative(n).into(),
        }
    }
}
fn length(value: &Value, table: &TokenTable) -> Option<Len> {
    let value = table.resolve(value);
    if let Some(n) = value.as_f64() {
        return Some(Len::Px(n as f32));
    }
    let text = value.as_str()?;
    if let Some(percent) = text.strip_suffix('%') {
        return percent.parse::<f32>().ok().map(|n| Len::Fraction(n / 100.));
    }
    text.strip_suffix("px")
        .unwrap_or(text)
        .parse()
        .ok()
        .map(Len::Px)
}
fn with<T>(
    e: StyleRefinement,
    value: Option<T>,
    apply: impl FnOnce(StyleRefinement, T) -> StyleRefinement,
) -> StyleRefinement {
    match value {
        Some(value) => apply(e, value),
        None => e,
    }
}
fn tracks(text: &str) -> Option<u16> {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("repeat(") {
        return rest.split(',').next()?.trim().parse().ok();
    }
    let count = text.split_whitespace().count();
    (count > 0).then_some(count as u16)
}
fn apply(mut e: StyleRefinement, style: &pm_surface::Props, table: &TokenTable) -> Compiled {
    if style.get("display").and_then(Value::as_str) == Some("flex")
        && !style.contains_key("flexDirection")
    {
        e = e.flex_row();
    }
    let mut scroll_y = false;
    let mut scroll_x = false;
    let mut symbol_size = None;
    for (key, raw) in style {
        let value = table.resolve(raw);
        let n = table.number(raw);
        let s = value.as_str();
        let len = length(raw, table);
        match key.as_str() {
            "display" => {
                if s == Some("grid") {
                    e = e.grid();
                }
            }
            "gridTemplateColumns" => {
                if let Some(count) = s.and_then(tracks).or(n.map(|n| n as u16)) {
                    e = e.grid_cols(count);
                }
            }
            "gridTemplateRows" => {
                if let Some(count) = s.and_then(tracks).or(n.map(|n| n as u16)) {
                    e = e.grid_rows(count);
                }
            }
            "gridColumn" => {
                if let Some(span) = s
                    .and_then(|s| s.strip_prefix("span "))
                    .and_then(|s| s.trim().parse().ok())
                    .or(n.map(|n| n as u16))
                {
                    e = e.col_span(span);
                }
            }
            "position" => {
                e = match s {
                    Some("absolute") => e.absolute(),
                    Some("relative") => e.relative(),
                    _ => e,
                };
            }
            "top" => e = with(e, len, |e, v| e.top(Length::from(v))),
            "left" => e = with(e, len, |e, v| e.left(Length::from(v))),
            "right" => e = with(e, len, |e, v| e.right(Length::from(v))),
            "bottom" => e = with(e, len, |e, v| e.bottom(Length::from(v))),
            "inset" => {
                let v: Length = match (s, len) {
                    (Some("fill"), _) => px(0.).into(),
                    (_, Some(v)) => v.into(),
                    _ => continue,
                };
                e = e.top(v).left(v).right(v).bottom(v);
            }
            "flexDirection" => {
                e = if s == Some("row") {
                    e.flex_row()
                } else {
                    e.flex_col()
                };
            }
            "flexWrap" => {
                e = if s == Some("wrap") {
                    e.flex_wrap()
                } else {
                    e.flex_nowrap()
                };
            }
            "alignItems" => {
                e = match s {
                    Some("center") => e.items_center(),
                    Some("end" | "flex-end") => e.items_end(),
                    Some("start" | "flex-start") => e.items_start(),
                    Some("baseline") => e.items_baseline(),
                    _ => e,
                };
            }
            "alignSelf" => {
                e = match s {
                    Some("center") => e.self_center(),
                    Some("end" | "flex-end") => e.self_end(),
                    Some("start" | "flex-start") => e.self_start(),
                    _ => e,
                };
            }
            "justifyContent" => {
                e = match s {
                    Some("center") => e.justify_center(),
                    Some("end" | "flex-end") => e.justify_end(),
                    Some("start" | "flex-start") => e.justify_start(),
                    Some("space-between") => e.justify_between(),
                    Some("space-around") => e.justify_around(),
                    _ => e,
                };
            }
            "gap" => {
                if let Some(n) = n {
                    e = e.gap(px(n));
                }
            }
            "columnGap" => {
                if let Some(n) = n {
                    e = e.gap_x(px(n));
                }
            }
            "rowGap" => {
                if let Some(n) = n {
                    e = e.gap_y(px(n));
                }
            }
            "padding" => e = with(e, n, |e, n| e.p(px(n))),
            "paddingLeft" => e = with(e, n, |e, n| e.pl(px(n))),
            "paddingRight" => e = with(e, n, |e, n| e.pr(px(n))),
            "paddingTop" => e = with(e, n, |e, n| e.pt(px(n))),
            "paddingBottom" => e = with(e, n, |e, n| e.pb(px(n))),
            "margin" => e = with(e, n, |e, n| e.m(px(n))),
            "marginLeft" => e = with(e, n, |e, n| e.ml(px(n))),
            "marginRight" => e = with(e, n, |e, n| e.mr(px(n))),
            "marginTop" => e = with(e, n, |e, n| e.mt(px(n))),
            "marginBottom" => e = with(e, n, |e, n| e.mb(px(n))),
            "height" => e = with(e, len, |e, v| e.h(Length::from(v))),
            "width" => e = with(e, len, |e, v| e.w(Length::from(v))),
            "minHeight" => e = with(e, len, |e, v| e.min_h(Length::from(v))),
            "minWidth" => e = with(e, len, |e, v| e.min_w(Length::from(v))),
            "maxHeight" => e = with(e, len, |e, v| e.max_h(Length::from(v))),
            "maxWidth" => e = with(e, len, |e, v| e.max_w(Length::from(v))),
            "flexGrow" => {
                if n.is_some_and(|n| n > 0.) {
                    e = e.flex_1();
                }
            }
            "flexShrink" => {
                if n == Some(0.) {
                    e = e.flex_shrink_0();
                } else if n.is_some_and(|n| n > 0.) {
                    // Explicit, so a native control knows it may give way.
                    e = e.flex_shrink(1.);
                }
            }
            "flexBasis" => {
                if let Some(v) = len {
                    e = e.flex_basis(Length::from(v));
                }
            }
            "borderRadius" => e = with(e, n, |e, n| e.rounded(px(n))),
            "borderWidth" | "border" => e = with(e, n, |e, n| e.border(px(n))),
            "borderTopWidth" | "borderTop" => e = with(e, n, |e, n| e.border_t(px(n))),
            "borderBottomWidth" | "borderBottom" => e = with(e, n, |e, n| e.border_b(px(n))),
            "borderLeftWidth" | "borderLeft" => e = with(e, n, |e, n| e.border_l(px(n))),
            "borderRightWidth" | "borderRight" => e = with(e, n, |e, n| e.border_r(px(n))),
            "borderColor" => {
                if let Some(color) = table.color(raw) {
                    e = e.border_color(rgb(color));
                }
            }
            "opacity" => e = with(e, n, |e, n| e.opacity(n)),
            "fontSize" => e = with(e, n, |e, n| e.text_size(px(n))),
            "fontWeight" => e = with(e, n, |e, n| e.font_weight(FontWeight(n))),
            "lineHeight" => {
                if let Some(n) = n {
                    // Unitless leadings multiply the font size; larger values are points.
                    e = if n <= 4. {
                        e.line_height(relative(n))
                    } else {
                        e.line_height(px(n))
                    };
                }
            }
            "fontVariantNumeric" => {
                if s == Some("tabular-nums") {
                    e = e.font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])));
                }
            }
            "textOverflow" => {
                if s == Some("ellipsis") {
                    e = e.text_ellipsis().overflow_hidden();
                }
            }
            "textAlign" => {
                e = match s {
                    Some("center") => e.text_center(),
                    Some("right" | "end") => e.text_right(),
                    _ => e,
                };
            }
            "whiteSpace" => {
                if s == Some("nowrap") {
                    e = e.whitespace_nowrap();
                }
            }
            "color" => {
                if let Some(color) = table.color(raw) {
                    e = e.text_color(rgb(color));
                }
            }
            "background" | "backgroundColor" => {
                // A well is the ink's wash, whatever panel it stands on.
                if raw.get("token").and_then(Value::as_str) == Some("colors.field") {
                    e = e.bg(well(&table.tokens));
                } else if let Some(color) = table.color(raw) {
                    e = e.bg(rgb(color));
                }
            }
            "overflow" | "overflowY" | "overflowX" => {
                let scrolls = s == Some("auto") || s == Some("scroll");
                if scrolls {
                    scroll_y |= key != "overflowX";
                    scroll_x |= key == "overflowX";
                } else if s == Some("hidden") {
                    e = e.overflow_hidden();
                }
            }
            "symbolSize" => symbol_size = n,
            "cursor" => {
                e = match s {
                    Some("col-resize" | "ew-resize") => e.cursor_col_resize(),
                    Some("row-resize" | "ns-resize") => e.cursor_row_resize(),
                    Some("grab") => e.cursor_grab(),
                    Some("grabbing") => e.cursor_grabbing(),
                    Some("text") => e.cursor_text(),
                    Some("crosshair") => e.cursor_crosshair(),
                    _ => e,
                };
            }
            _ => {}
        }
    }
    Compiled {
        sheet: e,
        scroll_y,
        scroll_x,
        symbol_size,
    }
}

#[cfg(test)]
mod tests {
    #[::core::prelude::v1::test]
    fn a_small_number_keeps_its_digits_and_never_reads_zero() {
        let n = |step: f64, precision: Option<u32>| NumberFacts {
            value: 0.,
            step,
            speed: 1.,
            min: f64::NEG_INFINITY,
            max: f64::INFINITY,
            precision,
        };
        assert_eq!(n(0.01, None).text(0.002), "0.002");
        assert_eq!(n(0.01, None).text(0.0025), "0.0025");
        assert_eq!(n(0.01, None).text(0.06), "0.06");
        assert_eq!(n(0.01, None).text(0.4), "0.4");
        assert_eq!(n(1., None).text(45.), "45");
        assert_eq!(n(0.01, None).text(0.), "0");
        // A declared precision holds.
        assert_eq!(n(1., Some(1)).text(0.004), "0");
    }

    use super::*;
    use serde_json::json;
    #[::core::prelude::v1::test]
    fn number_fields_read_like_the_shipped_field() {
        let facts = |step: f64, precision: Option<u32>| NumberFacts {
            value: 0.,
            step,
            speed: 1.,
            min: f64::MIN,
            max: f64::MAX,
            precision,
        };
        assert_eq!(facts(1., None).text(1370.272623), "1370");
        assert_eq!(facts(1., None).text(4.26), "4.3");
        assert_eq!(facts(0.01, None).text(0.5), "0.5");
        assert_eq!(facts(1., Some(2)).text(960.), "960");
        assert_eq!(facts(1., Some(2)).text(12.345), "12.35");
        assert_eq!(facts(1., Some(1)).text(-0.01), "0");
    }
    #[::core::prelude::v1::test]
    fn tokens_resolve_through_one_flat_table() {
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let table = TokenTable::new(tokens.clone());
        assert_eq!(
            table.number(&json!({"token":"metrics.row"})),
            Some(tokens.metrics.row)
        );
        assert_eq!(
            table.color(&json!({"token":"colors.text"})),
            Some(tokens.colors.text)
        );
        assert_eq!(table.number(&json!({"token":"metrics.nonsense"})), None);
    }
    #[::core::prelude::v1::test]
    fn grid_lengths_and_track_counts_compile() {
        let table = TokenTable::new(Tokens::new(pm_design::Scheme::Light, Default::default()));
        assert_eq!(tracks("repeat(3, 1fr)"), Some(3));
        assert_eq!(tracks("80px 1fr"), Some(2));
        let style = json!({"display":"grid","gridTemplateColumns":"repeat(4, 1fr)","position":"absolute","left":3,"opacity":0.5,"lineHeight":1.25,"fontVariantNumeric":"tabular-nums","textOverflow":"ellipsis","borderWidth":1,"overflowY":"auto"});
        let compiled = apply(
            StyleRefinement::default(),
            style.as_object().unwrap(),
            &table,
        );
        assert_eq!(compiled.sheet.display, Some(Display::Grid));
        assert_eq!(compiled.sheet.position, Some(Position::Absolute));
        assert_eq!(compiled.sheet.opacity, Some(0.5));
        assert!(compiled.sheet.text.font_features.is_some());
        assert!(compiled.scroll_y);
    }
}
