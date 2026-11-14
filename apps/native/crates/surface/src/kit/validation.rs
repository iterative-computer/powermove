use super::{Component, DesignTokenTable, Layout};
use crate::{Diagnostic, Id, Node, Surface};
use pm_design::Tokens;
use serde_json::Value;

pub fn accessible_label(surface: &Surface, node: &Node) -> String {
    if let Some(label) = node.props.get("label").and_then(Value::as_str)
        && !label.trim().is_empty()
    {
        return label.into();
    }
    if let Some(labelled) = node.props.get("labelledBy").and_then(Value::as_str) {
        let mut label = String::new();
        for name in labelled.split_whitespace() {
            if let Some(target) = surface
                .nodes()
                .values()
                .find(|n| n.props.get("id").and_then(Value::as_str) == Some(name))
            {
                if !label.is_empty() {
                    label.push(' ');
                }
                let mut pending = vec![target.id];
                while let Some(id) = pending.pop() {
                    let n = surface.node(id).unwrap();
                    label.push_str(&n.text);
                    pending.extend(n.children.iter().rev());
                }
            }
        }
        if !label.trim().is_empty() {
            return label;
        }
    }
    // Native button text is also an accessible name; a container is not named
    // by unrelated field values or by all of its descendants.
    if matches!(node.r#type.as_str(), "text" | "button") {
        let mut text = node.text.clone();
        for id in &node.children {
            if let Some(n) = surface.node(*id)
                && n.r#type == "text"
            {
                text.push_str(&n.text);
                for child in &n.children {
                    text.push_str(&surface.node(*child).unwrap().text);
                }
            }
        }
        return text;
    }
    String::new()
}
pub fn interactive(node: &Node) -> bool {
    Component::of(node).is_some_and(Component::interactive)
        || matches!(node.r#type.as_str(), "input" | "textarea" | "button")
        || node
            .props
            .get("tabIndex")
            .and_then(Value::as_i64)
            .is_some_and(|v| v >= 0)
}
/// Diagnostic policy for v4 panels; does not change the v1–3 wire contract or
/// silently reject/normalize authored data. Run after accepted atomic commits.
pub fn validate_panel(surface: &Surface, tokens: &Tokens, layout: &Layout) -> Vec<Diagnostic> {
    validate_panel_with_table(
        surface,
        tokens,
        &DesignTokenTable::from_design(tokens),
        layout,
    )
}
pub fn validate_panel_with_table(
    surface: &Surface,
    tokens: &Tokens,
    table: &DesignTokenTable,
    layout: &Layout,
) -> Vec<Diagnostic> {
    let table = table.value();
    let mut out = vec![];
    for node in surface.nodes().values().filter(|n| n.id != 0) {
        let mut flag = |prop: &str, code: &str| {
            out.push(Diagnostic {
                id: node.id,
                prop: prop.into(),
                code: code.into(),
            })
        };
        let component = Component::of(node);
        if node.props.contains_key("data-pm-component") && component.is_none() {
            flag("data-pm-component", "unknownComponent");
        }
        if component == Some(Component::Switch)
            && !super::additions::settings_host(surface, node)
            && !super::additions::in_sheet(surface, node)
        {
            flag("metadata.hostWindow", "switchOutsideSettings");
        }
        if component == Some(Component::SecureField) {
            for prop in ["value", "control", "onInput", "onChange"] {
                if node.props.contains_key(prop) {
                    flag(prop, "secureValueOnSurface");
                }
            }
            if node
                .props
                .get("binding")
                .and_then(Value::as_str)
                .is_none_or(|s| s.is_empty())
            {
                flag("binding", "missingSecureBinding");
            }
        }
        if component == Some(Component::ViewerGizmo)
            && !matches!(
                super::additions::str_option(node, "kind"),
                Some("transform" | "rotation" | "anchor" | "path-vertex" | "tangent")
            )
        {
            flag("metadata.pmKit.kind", "invalidGizmo");
        }
        let needs_name = interactive(node) || component.is_some();
        if needs_name && accessible_label(surface, node).trim().is_empty() {
            flag("label", "missingLabel");
        }
        let style = surface.computed_style(node.id).unwrap_or_default();
        for (key, value) in &style {
            if !valid_style(key, value, table) {
                flag(&format!("style.{key}"), "nonTokenValue");
            }
        }
        if interactive(node) {
            let bounds = layout.get(&node.id);
            let width = bounds
                .map(|r| r.x1 - r.x0)
                .or_else(|| style.get("minWidth").and_then(|v| token_number(v, table)));
            let height = bounds
                .map(|r| r.y1 - r.y0)
                .or_else(|| style.get("minHeight").and_then(|v| token_number(v, table)));
            match (width, height) {
                (Some(w), Some(h))
                    if w.is_finite()
                        && h.is_finite()
                        && w >= f64::from(tokens.metrics.hit)
                        && h >= f64::from(tokens.metrics.hit) => {}
                (Some(_), Some(_)) => flag("hitTarget", "smallHitTarget"),
                _ => flag("hitTarget", "missingHitTarget"),
            }
        }
        if component == Some(Component::ProgressRow)
            && let Some(value) = node.props.get("value")
            && !value.is_null()
            && !value.as_f64().is_some_and(|v| (0. ..=1.).contains(&v))
        {
            flag("value", "invalidProgress");
        }
        if matches!(
            component,
            Some(Component::PopoverAnchor | Component::SheetAnchor | Component::ColourPopover)
        ) && !node
            .props
            .get("anchor")
            .and_then(Value::as_u64)
            .is_some_and(|id| id != node.id && surface.node(id).is_some())
        {
            flag("anchor", "missingAnchor");
        }
    }
    out
}
fn token_number(value: &Value, table: &Value) -> Option<f64> {
    let obj = value.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    let token = obj.get("token")?.as_str()?;
    token
        .split('.')
        .try_fold(table, |v, key| v.get(key))?
        .as_f64()
}
fn valid_style(key: &str, value: &Value, table: &Value) -> bool {
    if value.is_null() || (matches!(key, "flexGrow" | "flexShrink") && value.is_number()) {
        return true;
    }
    // Only structural keywords are literal. Strings like "22px", hex colours,
    // font names and numeric strings are never accepted as visual values.
    let categorical = match key {
        "display" => &["flex", "grid", "none"][..],
        "position" => &["relative", "absolute"][..],
        "inset" => &["fill"][..],
        "flexDirection" => &["row", "column"][..],
        "flexWrap" => &["wrap", "nowrap"][..],
        "alignItems" | "justifyContent" => {
            &["start", "end", "center", "stretch", "space-between"][..]
        }
        "textAlign" => &["left", "right", "center"][..],
        "fontVariantNumeric" => &["tabular-nums"][..],
        "overflow" => &["hidden", "visible", "scroll"][..],
        "textOverflow" => &["ellipsis", "middle"][..],
        "whiteSpace" => &["nowrap", "normal"][..],
        _ => &[][..],
    };
    if value.as_str().is_some_and(|v| categorical.contains(&v)) {
        return true;
    }
    if token_number(value, table).is_none() {
        return false;
    }
    let token = value["token"].as_str().unwrap();
    let group = if key.to_ascii_lowercase().contains("color") || key == "background" {
        "colors."
    } else if matches!(key, "fontSize" | "fontWeight" | "lineHeight") {
        "typography."
    } else if matches!(key, "duration" | "response" | "bounce" | "startScale") {
        "motion."
    } else {
        "metrics."
    };
    token.starts_with(group)
}
/// IDs in visual depth-first order, compatible with Interaction::tab. Positive
/// tabindex never reorders a panel away from its visual order.
pub fn focus_order(surface: &Surface) -> Vec<Id> {
    crate::interaction::focus_order(surface, 0).expect("accepted surface root")
}
