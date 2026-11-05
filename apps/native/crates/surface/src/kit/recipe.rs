//! language.md §§3,5–9,13,16. All visual measurements come from pm-design.
use crate::Node;
use pm_design::{Material, Surface, TEXT_FONT, Tokens};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Component {
    InspectorRow,
    NumberField,
    TextField,
    PopUpButton,
    SegmentedControl,
    Segment,
    Checkbox,
    Toggle,
    ColourWell,
    DisclosureGroup,
    DisclosureButton,
    List,
    Tree,
    ListItem,
    TreeItem,
    ToolbarItem,
    SheetAnchor,
    PopoverAnchor,
    ProgressRow,
    EmptyState,
    Switch,
    SecureField,
    FormGroup,
    FormRow,
    ColourPopover,
    ViewerGizmo,
    ViewerOverlay,
    Banner,
    PushButton,
}
impl Component {
    /// Original kit catalog, retained for consumers that enumerate its frozen recipes.
    /// Use `catalog()` to include the additive panel kit.
    pub const ALL: &[Self] = &[
        Self::InspectorRow,
        Self::NumberField,
        Self::TextField,
        Self::PopUpButton,
        Self::SegmentedControl,
        Self::Segment,
        Self::Checkbox,
        Self::Toggle,
        Self::ColourWell,
        Self::DisclosureGroup,
        Self::DisclosureButton,
        Self::List,
        Self::Tree,
        Self::ListItem,
        Self::TreeItem,
        Self::ToolbarItem,
        Self::SheetAnchor,
        Self::PopoverAnchor,
        Self::ProgressRow,
        Self::EmptyState,
    ];
    pub const ADDITIONS: &[Self] = &[
        Self::Switch,
        Self::SecureField,
        Self::FormGroup,
        Self::FormRow,
        Self::ColourPopover,
        Self::ViewerGizmo,
        Self::ViewerOverlay,
        Self::Banner,
        Self::PushButton,
    ];
    pub fn catalog() -> impl Iterator<Item = Self> {
        Self::ALL.iter().chain(Self::ADDITIONS).copied()
    }
    pub fn of(node: &Node) -> Option<Self> {
        serde_json::from_value(node.props.get("data-pm-component")?.clone()).ok()
    }
    pub fn interactive(self) -> bool {
        matches!(
            self,
            Self::NumberField
                | Self::TextField
                | Self::PopUpButton
                | Self::Segment
                | Self::Checkbox
                | Self::Toggle
                | Self::ColourWell
                | Self::DisclosureButton
                | Self::ListItem
                | Self::TreeItem
                | Self::ToolbarItem
                | Self::Switch
                | Self::SecureField
                | Self::ViewerGizmo
                | Self::PushButton
        )
    }
    pub fn surface(self) -> Surface {
        match self {
            Self::ToolbarItem => Surface::Toolbar,
            Self::SheetAnchor => Surface::Sheet,
            Self::PopoverAnchor | Self::ColourPopover => Surface::Popover,
            Self::ViewerOverlay | Self::Banner => Surface::ViewerOverlay,
            Self::List | Self::Tree | Self::ListItem | Self::TreeItem => Surface::List,
            _ => Surface::InspectorRow,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Widget {
    InspectorRow,
    ScrubbableTextField,
    TextField,
    PopUpButton,
    SegmentedControl,
    Segment,
    Checkbox,
    Switch,
    ColourWell,
    DisclosureGroup,
    DisclosureButton,
    List,
    Outline,
    ListItem,
    TreeItem,
    ToolbarItem,
    Sheet,
    Popover,
    Progress,
    EmptyState,
    SecureTextField,
    FormGroup,
    FormRow,
    ColourPopover,
    ViewerGizmo,
    ViewerOverlay,
    Banner,
    PushButton,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub widget: Widget,
    pub material: Material,
    pub content_material: Material,
    pub background: u32,
    pub foreground: u32,
    pub secondary: u32,
    pub muted: u32,
    pub hover: u32,
    pub selected_fill: u32,
    pub selected_segment: u32,
    pub focus: u32,
    pub border_color: u32,
    pub border: f32,
    /// Field/group outline is enabled by Increase Contrast; swatch keylines stay visible.
    pub draw_border: bool,
    pub visual_height: Option<f32>,
    pub hit: f32,
    pub width: Option<f32>,
    /// None means system-owned shape or an unspecified neutral container.
    pub radius: Option<f32>,
    /// Inset between a segmented track and its segments: concentric math
    /// (`control_radius − inner_radius`), panels/README.md §5.1.
    pub track_padding: Option<f32>,
    pub value_column: Option<f32>,
    pub inset: f32,
    pub font: &'static str,
    pub font_size: f32,
    pub weight: f32,
    pub tabular_figures: bool,
    pub units_follow_value: bool,
    pub values_left_aligned: bool,
    pub motion: Transition,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transition {
    Spring {
        response: f64,
        bounce: f64,
        start_scale: f64,
        exit: f64,
    },
    CrossFade {
        duration: f64,
    },
}
impl Recipe {
    pub fn new(kind: Component, t: &Tokens, glass_available: bool) -> Self {
        use Component::*;
        let widget = match kind {
            InspectorRow => Widget::InspectorRow,
            NumberField => Widget::ScrubbableTextField,
            TextField => Widget::TextField,
            PopUpButton => Widget::PopUpButton,
            SegmentedControl => Widget::SegmentedControl,
            Segment => Widget::Segment,
            Checkbox => Widget::Checkbox,
            // Design review 2026-11-01 (panels/README.md §5.5): panels and
            // inspector rows use checkboxes; switches belong to Settings only.
            Toggle => Widget::Checkbox,
            ColourWell => Widget::ColourWell,
            DisclosureGroup => Widget::DisclosureGroup,
            DisclosureButton => Widget::DisclosureButton,
            List => Widget::List,
            Tree => Widget::Outline,
            ListItem => Widget::ListItem,
            TreeItem => Widget::TreeItem,
            ToolbarItem => Widget::ToolbarItem,
            SheetAnchor => Widget::Sheet,
            PopoverAnchor => Widget::Popover,
            ProgressRow => Widget::Progress,
            EmptyState => Widget::EmptyState,
            Switch => Widget::Switch,
            SecureField => Widget::SecureTextField,
            FormGroup => Widget::FormGroup,
            FormRow => Widget::FormRow,
            ColourPopover => Widget::ColourPopover,
            ViewerGizmo => Widget::ViewerGizmo,
            ViewerOverlay => Widget::ViewerOverlay,
            Banner => Widget::Banner,
            PushButton => Widget::PushButton,
        };
        let floating = matches!(
            kind,
            ToolbarItem | SheetAnchor | PopoverAnchor | ColourPopover | ViewerOverlay | Banner
        );
        let field = matches!(
            kind,
            NumberField | TextField | SecureField | PopUpButton | SegmentedControl
        );
        Self {
            widget,
            material: t.material(kind.surface(), glass_available),
            content_material: Material::Solid,
            background: if matches!(kind, PopoverAnchor | ColourPopover) {
                t.colors.floating
            } else if kind == ToolbarItem {
                t.colors.window
            } else if field {
                t.colors.field
            } else {
                t.colors.panel
            },
            foreground: t.colors.text,
            secondary: t.colors.secondary,
            muted: t.colors.muted,
            hover: t.colors.hover,
            selected_fill: t.colors.selected_fill,
            selected_segment: t.colors.floating,
            focus: t.colors.focus,
            border_color: t.colors.separator,
            border: t.metrics.border,
            draw_border: kind == ColourWell
                || ((field || kind == FormGroup) && t.accessibility.increase_contrast),
            visual_height: match kind {
                NumberField | TextField | SecureField | SegmentedControl | Segment | ColourWell => {
                    Some(t.metrics.field)
                }
                // Standalone pop-ups are `control` tall; inside an InspectorRow
                // `Panel::prepare` narrows them to `field` (language §9 rows).
                PopUpButton | PushButton => Some(t.metrics.control),
                InspectorRow | ListItem | TreeItem | ProgressRow => Some(t.metrics.row),
                _ => None,
            },
            hit: t.metrics.hit,
            width: match kind {
                SheetAnchor => Some(t.metrics.sheet_width),
                PopoverAnchor | ColourPopover => Some(t.metrics.popover_width),
                ColourWell => Some(t.metrics.field),
                _ => None,
            },
            radius: match kind {
                Segment => Some(t.metrics.inner_radius),
                NumberField | TextField | SecureField | SegmentedControl | PopUpButton
                | ColourWell => Some(t.metrics.control_radius),
                // Selection shape of a row (panels/README.md §3 Selected).
                ListItem | TreeItem => Some(t.metrics.control_radius),
                _ => None,
            },
            track_padding: (kind == SegmentedControl)
                .then_some(t.metrics.control_radius - t.metrics.inner_radius),
            value_column: (kind == InspectorRow).then_some(t.metrics.value_column),
            inset: t.metrics.inset,
            font: TEXT_FONT,
            font_size: if kind == InspectorRow {
                t.typography.label
            } else if matches!(kind, DisclosureButton | DisclosureGroup) {
                t.typography.title
            } else if kind == EmptyState {
                t.typography.heading
            } else {
                t.typography.body
            },
            weight: if matches!(kind, DisclosureGroup | DisclosureButton | EmptyState) {
                t.typography.semibold_weight
            } else {
                t.typography.regular_weight
            },
            tabular_figures: kind == NumberField,
            units_follow_value: kind == NumberField,
            values_left_aligned: true,
            motion: if t.accessibility.reduce_motion || !floating {
                Transition::CrossFade {
                    duration: if t.accessibility.reduce_motion {
                        t.motion.reduced
                    } else {
                        t.motion.interaction
                    },
                }
            } else {
                Transition::Spring {
                    response: t.motion.enter,
                    bounce: t.motion.bounce,
                    start_scale: t.motion.start_scale,
                    exit: t.motion.exit,
                }
            },
        }
    }
}
// Resolved (design review 2026-11-01, docs/native/design/reviews/na-cloud-uikit.md):
// segmented-track padding is `track_padding` above; Toggle draws as a checkbox at
// the leading edge of the value column; tree indentation follows NSOutlineView's
// default until pm-design adds `metrics.tree_indent` (follow-up F2).
