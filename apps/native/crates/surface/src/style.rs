//! Deliberately bounded surface selectors, not browser CSS. No visual defaults.
use crate::{Node, Props};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Selector {
    Id(String),
    Class(String),
    Role(String),
    Attr { name: String, value: Value },
    State { name: String, value: Value },
    All(Vec<Selector>),
}
impl Selector {
    pub fn matches(&self, node: &Node) -> bool {
        match self {
            Self::Id(id) => node.props.get("id").and_then(Value::as_str) == Some(id),
            Self::Class(class) => node
                .props
                .get("class")
                .and_then(Value::as_str)
                .is_some_and(|classes| classes.split_whitespace().any(|c| c == class)),
            Self::Role(role) => node.props.get("role").and_then(Value::as_str) == Some(role),
            Self::Attr { name, value } => {
                node.props.get("attr").and_then(|a| a.get(name)) == Some(value)
            }
            Self::State { name, value } => {
                node.props.get("state").and_then(|a| a.get(name)) == Some(value)
            }
            Self::All(selectors) => selectors.iter().all(|s| s.matches(node)),
        }
    }
    fn specificity(&self) -> (u32, u32) {
        match self {
            Self::Id(_) => (1, 0),
            Self::All(all) => all.iter().fold((0, 0), |a, s| {
                let b = s.specificity();
                (a.0.saturating_add(b.0), a.1.saturating_add(b.1))
            }),
            _ => (0, 1),
        }
    }
    fn validate(&self, depth: usize) -> Result<(), String> {
        if depth > 32 {
            return Err("selector depth limit".into());
        }
        if let Self::All(all) = self {
            if all.len() > 64 {
                return Err("selector term limit".into());
            }
            for selector in all {
                selector.validate(depth + 1)?;
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub selector: Selector,
    pub style: Props,
}
#[derive(Clone, Debug, Default)]
pub struct StyleSheet {
    rules: Vec<Rule>,
}
impl StyleSheet {
    /// Invalid replacement leaves the previous stylesheet intact.
    pub fn replace(&mut self, rules: Vec<Rule>) -> Result<(), String> {
        if rules.len() > 4096 {
            return Err("rule limit".into());
        }
        for rule in &rules {
            rule.selector.validate(0)?;
            validate_style(&rule.style)?;
        }
        self.rules = rules;
        Ok(())
    }
    /// Specificity then source order; inline authored style wins. Host resolves tokens.
    pub fn resolve(&self, node: &Node) -> Props {
        let mut matched: Vec<_> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selector.matches(node))
            .collect();
        matched.sort_by_key(|(i, r)| (r.selector.specificity(), *i));
        let mut result = Props::new();
        for (_, rule) in matched {
            result.extend(rule.style.clone());
        }
        if let Some(style) = node.props.get("style").and_then(Value::as_object) {
            result.extend(style.clone());
        }
        result
    }
}
pub fn validate_style(style: &Props) -> Result<(), String> {
    for (key, value) in style {
        if key == "display" && !matches!(value.as_str(), Some("flex" | "grid" | "none")) {
            return Err("surface display must be flex, grid or none".into());
        }
        if value.is_array() || value.is_boolean() {
            return Err(format!("invalid style value {key}"));
        }
        if let Some(token) = value.as_object()
            && (token.len() != 1 || !token.get("token").is_some_and(Value::is_string))
        {
            return Err("style objects must be token references".into());
        }
    }
    Ok(())
}
