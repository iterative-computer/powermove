use super::{Component, Layout, accessible_label, interactive};
use crate::{Id, Node, Surface};
use accesskit::{
    Action, ActionData, ActionRequest, HasPopup, NodeId, Role, Toggled, Tree, TreeId, TreeUpdate,
};
use serde_json::Value;

fn role(node: &Node) -> Role {
    use Component::*;
    match Component::of(node) {
        Some(NumberField) => Role::NumberInput,
        Some(TextField) => Role::TextInput,
        Some(PopUpButton) => Role::ComboBox,
        Some(SegmentedControl) => Role::RadioGroup,
        Some(Segment) => Role::RadioButton,
        Some(Checkbox) => Role::CheckBox,
        Some(Toggle) => Role::Switch,
        Some(ColourWell) => Role::ColorWell,
        Some(DisclosureButton) => Role::DisclosureTriangle,
        Some(List) => Role::List,
        Some(Tree) => Role::Tree,
        Some(ListItem) => Role::ListItem,
        Some(TreeItem) => Role::TreeItem,
        Some(ToolbarItem) => Role::Button,
        Some(SheetAnchor | PopoverAnchor) => Role::Dialog,
        Some(ProgressRow) => Role::ProgressIndicator,
        Some(InspectorRow | DisclosureGroup | EmptyState) => Role::Group,
        None if node.id == 0 => Role::Pane,
        None => match node.props.get("role").and_then(Value::as_str) {
            Some("heading") => Role::Heading,
            Some("group") => Role::Group,
            _ => match node.r#type.as_str() {
                "text" => Role::Label,
                "input" => Role::TextInput,
                "textarea" => Role::MultilineTextInput,
                "button" => Role::Button,
                _ => Role::GenericContainer,
            },
        },
    }
}
fn project(surface: &Surface, source: &Node, layout: &Layout) -> accesskit::Node {
    let mut node = accesskit::Node::new(role(source));
    node.set_children(
        source
            .children
            .iter()
            .copied()
            .map(NodeId)
            .collect::<Vec<_>>(),
    );
    let label = accessible_label(surface, source);
    if !label.is_empty() {
        node.set_label(label);
    }
    if source.props.get("aria-modal").and_then(Value::as_bool) == Some(true) {
        node.set_modal();
    }
    match Component::of(source) {
        Some(Component::PopUpButton) => node.set_has_popup(HasPopup::Menu),
        Some(Component::ColourWell) => node.set_has_popup(HasPopup::Dialog),
        _ => {}
    }
    if let Some(active) = source.props.get("aria-pressed").and_then(Value::as_bool) {
        node.set_toggled(Toggled::from(active));
    }
    if let Some(bounds) = layout.get(&source.id) {
        node.set_bounds(*bounds);
    }
    if let Some(value) = source.props.get("value") {
        let text = source
            .input
            .as_ref()
            .map(|input| input.displayed.clone())
            .unwrap_or_else(|| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string())
            });
        let text = if Component::of(source) == Some(Component::PopUpButton) {
            source
                .props
                .get("options")
                .and_then(Value::as_array)
                .and_then(|options| {
                    options.iter().find_map(|o| {
                        if o.is_string() && o == value {
                            o.as_str()
                        } else if o.get("v") == Some(value) {
                            o.get("label").and_then(Value::as_str)
                        } else {
                            None
                        }
                    })
                })
                .map(str::to_owned)
                .unwrap_or(text)
        } else {
            text
        };
        let unit = source
            .props
            .get("unit")
            .and_then(Value::as_str)
            .unwrap_or("");
        node.set_value(if unit.is_empty() {
            text
        } else {
            format!("{text} {unit}")
        });
        if let Some(number) = value.as_f64() {
            node.set_numeric_value(number);
        }
    } else if let Some(input) = &source.input {
        node.set_value(input.displayed.clone());
    }
    if Component::of(source) == Some(Component::ProgressRow) && source.props.contains_key("value") {
        node.set_min_numeric_value(0.);
        node.set_max_numeric_value(1.);
    }
    for (prop, setter) in [
        (
            "min",
            accesskit::Node::set_min_numeric_value as fn(&mut accesskit::Node, f64),
        ),
        ("max", accesskit::Node::set_max_numeric_value),
        ("step", accesskit::Node::set_numeric_value_step),
    ] {
        if let Some(v) = source.props.get(prop).and_then(Value::as_f64) {
            setter(&mut node, v);
        }
    }
    if let Some(expanded) = source.props.get("aria-expanded").and_then(Value::as_bool) {
        node.set_expanded(expanded);
    }
    if let Some(selected) = source.props.get("aria-selected").and_then(Value::as_bool) {
        node.set_selected(selected);
    }
    if matches!(
        Component::of(source),
        Some(Component::Checkbox | Component::Toggle | Component::Segment)
    ) {
        node.set_toggled(
            if source.props.get("mixed").and_then(Value::as_bool) == Some(true) {
                Toggled::Mixed
            } else {
                Toggled::from(
                    source
                        .props
                        .get("aria-checked")
                        .or_else(|| source.props.get("value"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                )
            },
        );
    }
    if source.props.get("disabled").and_then(Value::as_bool) == Some(true) {
        node.set_disabled();
    } else if interactive(source) {
        node.add_action(Action::Focus);
        match Component::of(source) {
            Some(Component::NumberField) => {
                node.add_action(Action::Increment);
                node.add_action(Action::Decrement);
                node.add_action(Action::SetValue);
            }
            Some(Component::TextField) => node.add_action(Action::SetValue),
            Some(Component::DisclosureButton) => {
                node.add_action(Action::Click);
                node.add_action(
                    if source.props.get("aria-expanded").and_then(Value::as_bool) == Some(true) {
                        Action::Collapse
                    } else {
                        Action::Expand
                    },
                );
            }
            Some(Component::TreeItem) => {
                node.add_action(Action::Click);
                if let Some(expanded) = source.props.get("aria-expanded").and_then(Value::as_bool) {
                    node.add_action(if expanded {
                        Action::Collapse
                    } else {
                        Action::Expand
                    });
                }
            }
            _ => node.add_action(Action::Click),
        }
    }
    node
}
/// Complete tree, including every descendant (P-A11Y), scoped by the host to a
/// panel generation. Host grafts this subtree into its window AccessKit tree.
pub fn accessibility_tree(surface: &Surface, layout: &Layout, focus: Option<Id>) -> TreeUpdate {
    TreeUpdate {
        nodes: surface
            .nodes()
            .values()
            .map(|n| (NodeId(n.id), project(surface, n, layout)))
            .collect(),
        tree: Some(Tree::new(NodeId(0))),
        tree_id: TreeId::ROOT,
        focus: NodeId(focus.filter(|id| surface.node(*id).is_some()).unwrap_or(0)),
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum NativeAction {
    Focus(Id),
    Click(Id),
    Expand(Id, bool),
    Step(Id, bool),
    SetText(Id, String),
    SetNumber(Id, f64),
}
/// Validate on the native side, then enqueue through the same input/binding
/// driver as pointer/keyboard input. The caller must also match realm generation
/// and revision; an AccessKit request alone carries neither.
pub fn accessibility_action(
    surface: &Surface,
    expected_revision: u64,
    request: ActionRequest,
) -> Result<NativeAction, String> {
    if expected_revision != surface.revision() || request.target_tree != TreeId::ROOT {
        return Err("stale accessibility tree".into());
    }
    let id = request.target_node.0;
    let source = surface.node(id).ok_or("missing accessibility target")?;
    if !project(surface, source, &Layout::new()).supports_action(request.action) {
        return Err("unsupported accessibility action".into());
    }
    Ok(match request.action {
        Action::Focus => NativeAction::Focus(id),
        Action::Click => NativeAction::Click(id),
        Action::Expand => NativeAction::Expand(id, true),
        Action::Collapse => NativeAction::Expand(id, false),
        Action::Increment => NativeAction::Step(id, true),
        Action::Decrement => NativeAction::Step(id, false),
        Action::SetValue => match request.data {
            Some(ActionData::Value(value)) => NativeAction::SetText(id, value.into()),
            Some(ActionData::NumericValue(value))
                if value.is_finite() && Component::of(source) == Some(Component::NumberField) =>
            {
                NativeAction::SetNumber(id, value)
            }
            _ => return Err("invalid accessibility value".into()),
        },
        _ => return Err("unsupported accessibility action".into()),
    })
}
