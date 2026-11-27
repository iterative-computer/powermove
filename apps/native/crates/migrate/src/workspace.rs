//! Historical saved-workspace transforms from main c7f46a056. These are data
//! migrations, not native layout/style defaults. Opus owns their presentation.
//! Unknown fields and customized chrome survive; retain the input checkpoint.
use serde_json::{Value, json};

fn id(panel: &Value) -> Option<&str> {
    panel.as_str().or_else(|| panel.get("id")?.as_str())
}
fn specs(dock: &Value) -> Option<&Vec<Value>> {
    dock.get("panels")?.as_array()
}
fn has(dock: &Value, target: &str) -> bool {
    specs(dock).is_some_and(|p| p.iter().any(|v| id(v) == Some(target)))
}
fn fill(dock: &mut Value) {
    let Some(panels) = dock.get_mut("panels").and_then(Value::as_array_mut) else {
        return;
    };
    if panels
        .iter()
        .any(|p| p.get("flex") == Some(&Value::Bool(true)))
    {
        return;
    }
    let index = panels
        .iter()
        .rposition(|p| id(p) != Some("agent"))
        .or_else(|| panels.len().checked_sub(1));
    if let Some(index) = index {
        panels[index]["flex"] = true.into();
    }
}
fn take(docks: &mut [Value], hidden: &mut Vec<Value>, target: &str) -> Option<Value> {
    for dock in docks {
        if let Some(panels) = dock.get_mut("panels").and_then(Value::as_array_mut)
            && let Some(index) = panels.iter().position(|p| id(p) == Some(target))
        {
            let panel = panels.remove(index);
            fill(dock);
            return Some(panel);
        }
    }
    let index = hidden
        .iter()
        .position(|p| p.get("id").and_then(Value::as_str) == Some(target))?;
    let record = hidden.remove(index);
    let mut spec = record
        .get("spec")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    spec["id"] = target.into();
    Some(spec)
}

/// Apply only recognized historical versions. A newer schema remains intact.
pub fn migrate(input: &Value) -> Value {
    let Some(mut out) = input.as_object().cloned() else {
        return input.clone();
    };
    let version = out
        .get("agentDockMigration")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let Some(mut docks) = out
        .get("layout")
        .and_then(|l| l.get("docks"))
        .and_then(Value::as_array)
        .cloned()
    else {
        return input.clone();
    };
    // Malformed extension workspaces are preserved for recovery, not reset.
    if docks.iter().any(|d| {
        !d.is_object()
            || specs(d).is_none()
            || specs(d)
                .unwrap()
                .iter()
                .any(|p| !p.is_object() && !p.is_string())
    }) {
        return input.clone();
    }
    let mut hidden = out
        .get("hiddenPanels")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if version < 5 {
        for dock in &mut docks {
            for panel in dock["panels"].as_array_mut().unwrap() {
                if let Some(name) = panel.as_str() {
                    *panel = json!({"id": name});
                }
                if version < 1 && id(panel) == Some("agent") {
                    panel["id"] = "layer-effects".into();
                }
            }
        }
        if version < 2
            && !docks.iter().any(|d| has(d, "layer-effects"))
            && !hidden
                .iter()
                .any(|p| p.get("id").and_then(Value::as_str) == Some("layer-effects"))
            && let Some(media) = docks.iter_mut().find(|d| has(d, "assets"))
        {
            let panels = media["panels"].as_array_mut().unwrap();
            let at = panels.iter().position(|p| id(p) == Some("assets")).unwrap();
            panels.insert(at + 1, json!({"id":"layer-effects","size":350}));
        }
        if version < 4
            && !docks
                .iter()
                .any(|d| d.get("id").and_then(Value::as_str) == Some("bottom"))
            && docks
                .iter()
                .any(|d| d.get("id").and_then(Value::as_str) == Some("left"))
            && let Some(center) = docks
                .iter_mut()
                .find(|d| d.get("id").and_then(Value::as_str) == Some("center"))
        {
            let panels = center["panels"].as_array_mut().unwrap();
            if panels.len() > 1
                && panels.last().is_some_and(|p| {
                    id(p) == Some("timeline") && p.get("flex") != Some(&Value::Bool(true))
                })
            {
                let mut panel = panels.pop().unwrap();
                let size = panel
                    .get("size")
                    .cloned()
                    .filter(|n| n.as_f64().is_some_and(|n| n != 0.0))
                    .unwrap_or(json!(300));
                panel["flex"] = true.into();
                fill(center);
                let at = docks
                    .iter()
                    .position(|d| d.get("id").and_then(Value::as_str) == Some("right"))
                    .unwrap_or(docks.len());
                docks.insert(at, json!({"id":"bottom","size":size,"panels":[panel]}));
            }
        }
        let default_family = out.get("id").and_then(Value::as_str) == Some("design")
            || out.get("name").and_then(Value::as_str).is_some_and(|s| {
                s == "Design"
                    || s.strip_prefix("Design").is_some_and(|tail| {
                        tail.starts_with(|c: char| !c.is_alphanumeric() && c != '_')
                    })
            });
        if let Some(media_index) = docks.iter().position(|d| has(d, "assets")) {
            let panels = specs(&docks[media_index]).unwrap();
            let project_index = panels.iter().position(|p| id(p) == Some("assets")).unwrap();
            let adjacent = panels
                .iter()
                .position(|p| id(p) == Some("layer-effects"))
                .is_some_and(|i| i.abs_diff(project_index) == 1);
            if default_family || adjacent {
                let mut effects = take(&mut docks, &mut hidden, "layer-effects")
                    .unwrap_or_else(|| json!({"id":"layer-effects"}));
                let panels = docks[media_index]["panels"].as_array_mut().unwrap();
                let at = panels.iter().position(|p| id(p) == Some("assets")).unwrap();
                let project = &mut panels[at];
                project["group"] = "project".into();
                project.as_object_mut().unwrap().remove("active");
                effects["group"] = "project".into();
                effects["active"] = true.into();
                effects.as_object_mut().unwrap().remove("collapsed");
                for key in ["flex", "size"] {
                    if let Some(value) = project.get(key) {
                        effects[key] = value.clone();
                    } else {
                        effects.as_object_mut().unwrap().remove(key);
                    }
                }
                panels.insert(at + 1, effects);
            }
        }
        if default_family && !docks.iter().any(|d| has(d, "fxbrowser")) {
            let target = docks.iter().position(|d| has(d, "inspector")).or_else(|| {
                docks
                    .iter()
                    .position(|d| d.get("id").and_then(Value::as_str) == Some("right"))
            });
            // Without a specified target keep the hidden record; do not invent a dock.
            if let Some(target) = target {
                let mut browser = take(&mut docks, &mut hidden, "fxbrowser")
                    .unwrap_or_else(|| json!({"id":"fxbrowser"}));
                browser.as_object_mut().unwrap().remove("collapsed");
                if browser.get("size").is_none() && browser.get("flex").is_none() {
                    browser["size"] = 300.into();
                }
                let panels = docks[target]["panels"].as_array_mut().unwrap();
                let at = panels
                    .iter()
                    .position(|p| id(p) == Some("inspector"))
                    .map(|i| i + 1)
                    .unwrap_or(panels.len());
                panels.insert(at, browser);
                fill(&mut docks[target]);
            }
        }
        if out.get("builtin") == Some(&Value::Bool(true)) {
            let rename = match (
                out.get("id").and_then(Value::as_str),
                out.get("name").and_then(Value::as_str),
            ) {
                (Some("design"), Some("Design")) => Some("Default"),
                (Some("animate"), Some("Animate")) => Some("Animation"),
                _ => None,
            };
            if let Some(name) = rename {
                out.insert("name".into(), name.into());
            }
        }
        // Preserve empty docks and their unknown metadata; native consumers can
        // ignore them without deleting recoverable data.
        out.get_mut("layout").unwrap()["docks"] = docks.into();
        out.insert("hiddenPanels".into(), hidden.into());
        out.insert("agentDockMigration".into(), 5.into());
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v3_project_workspace_reruns_bottom_migration_and_v5_preserves_customizations() {
        let input: Value =
            serde_json::from_str(include_str!("../tests/fixtures/workspace-v3.json")).unwrap();
        let migrated = migrate(&input);
        assert_eq!(migrated["name"], "Default");
        assert_eq!(migrated["layout"]["futureLayout"], 7);
        assert_eq!(migrated["chrome"], input["chrome"]);
        let docks = migrated["layout"]["docks"].as_array().unwrap();
        let bottom = docks.iter().find(|d| d["id"] == "bottom").unwrap();
        assert_eq!(bottom["size"], 321);
        assert_eq!(bottom["panels"][0]["futurePanel"], true);
        let left = docks.iter().find(|d| d["id"] == "left").unwrap();
        assert_eq!(left["panels"][1]["active"], true);
        assert_eq!(left["panels"][0]["group"], "project");
        assert_eq!(migrate(&migrated), migrated);
    }
    #[test]
    fn authored_v5_groups_and_newer_or_malformed_layouts_remain_exact() {
        for input in [
            json!({"agentDockMigration":5,"layout":{"docks":[{"id":"left","panels":[{"id":"assets","group":"chosen","active":true}]}]}}),
            json!({"agentDockMigration":9,"layout":{"docks":[]},"future":true}),
            json!({"layout":{"docks":[{"id":"broken","panels":[null]}]}}),
        ] {
            assert_eq!(migrate(&input), input);
        }
    }
}
