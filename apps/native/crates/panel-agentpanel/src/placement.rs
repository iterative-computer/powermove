//! Protocol metadata only, never a workspace mutation or an access grant.
//! Source: v1.2.0 panels/agent/ui-placement.ts:18–92. No target geometry.
use serde_json::Value;
use std::collections::{HashMap, HashSet};
const MARKER: &str = "POWERMOVE_UI_TARGET";
pub fn is_message(text: &str) -> bool {
    text.trim_start().starts_with("POWERMOVE_UI_TARGET ")
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Split {
    pub text: String,
    pub messages: Vec<String>,
}
pub fn split(source: &str, streaming: bool) -> Split {
    let mut out = Split::default();
    let mut scan = 0;
    let mut cursor = 0;
    while scan < source.len() {
        let line_end = source[scan..].find('\n').map_or(source.len(), |i| scan + i);
        let line = &source[scan..line_end];
        let head = line.trim_start_matches([' ', '\t']);
        if let Some(tail) = head.strip_prefix(MARKER)
            && tail
                .chars()
                .next()
                .is_none_or(|c| c.is_whitespace() || c == '{')
        {
            out.text.push_str(&source[cursor..scan]);
            let start = scan + line.len() - head.len() + MARKER.len();
            let json_start = start + source[start..].len() - source[start..].trim_start().len();
            let mut end = source.len();
            let mut complete = false;
            if source[json_start..].starts_with('{') {
                let (mut depth, mut quoted, mut escaped) = (0i64, false, false);
                for (i, c) in source[json_start..].char_indices() {
                    if quoted {
                        if escaped {
                            escaped = false;
                        } else if c == '\\' {
                            escaped = true;
                        } else if c == '"' {
                            quoted = false;
                        }
                    } else if c == '"' {
                        quoted = true;
                    } else if c == '{' {
                        depth += 1;
                    } else if c == '}' {
                        depth -= 1;
                        if depth == 0 {
                            end = json_start + i + 1;
                            complete = true;
                            break;
                        }
                    }
                }
                if complete {
                    out.messages
                        .push(format!("{MARKER} {}", &source[json_start..end]));
                }
            } else {
                end = source[start..]
                    .find('\n')
                    .map_or(source.len(), |i| start + i);
            }
            cursor = end;
            if end == source.len() {
                break;
            }
            scan = source[end..]
                .find('\n')
                .map_or(source.len(), |i| end + i + 1);
        } else {
            if line_end == source.len() {
                break;
            }
            scan = line_end + 1;
        }
    }
    out.text.push_str(&source[cursor..]);
    if streaming {
        let start = out.text.rfind('\n').map_or(0, |i| i + 1);
        let tail = out.text[start..].trim_start_matches([' ', '\t']);
        if !tail.is_empty()
            && tail.bytes().all(|c| c.is_ascii_uppercase() || c == b'_')
            && MARKER.starts_with(tail)
        {
            out.text.truncate(start);
        }
    }
    out
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    Panel {
        id: String,
        label: String,
        selector: Option<String>,
        insert: Option<String>,
    },
    Dock {
        id: String,
        label: String,
        before_panel: Option<String>,
    },
}
#[derive(Clone, Debug, Default)]
pub struct Targets {
    pub visible_panels: HashSet<String>,
    pub visible_docks: HashMap<String, Vec<String>>,
}
pub fn parse(message: &str, targets: &Targets) -> Option<Placement> {
    if !is_message(message) || message.encode_utf16().count() > 600 {
        return None;
    }
    let v: Value =
        serde_json::from_str(message.trim().strip_prefix("POWERMOVE_UI_TARGET ")?).ok()?;
    let id = v["id"].as_str()?;
    if id.is_empty()
        || id.len() > 80
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return None;
    }
    let raw_label = v["label"].as_str()?;
    if raw_label.encode_utf16().count() > 64 {
        return None;
    }
    let label = raw_label
        .chars()
        .map(|c| {
            if matches!(c, '\0'..='\u{1f}' | '\u{7f}') {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_owned();
    if label.is_empty() {
        return None;
    }
    match v["kind"].as_str()? {
        "panel" if targets.visible_panels.contains(id) => {
            let selector = match v.get("selector") {
                None => None,
                Some(v) => {
                    let s = v.as_str()?;
                    if s.trim().is_empty()
                        || s.encode_utf16().count() > 160
                        || s.chars().any(|c| matches!(c, '\0'..='\u{1f}' | '\u{7f}'))
                    {
                        return None;
                    }
                    Some(s.trim().into())
                }
            };
            let insert = match v.get("insert") {
                None => None,
                Some(v) => {
                    let s = v.as_str()?;
                    if selector.is_none() || !matches!(s, "before" | "after") {
                        return None;
                    }
                    Some(s.into())
                }
            };
            Some(Placement::Panel {
                id: id.into(),
                label,
                selector,
                insert,
            })
        }
        "dock" => {
            let panels = targets.visible_docks.get(id)?;
            let before = match v.get("beforePanelId") {
                None | Some(Value::Null) => None,
                Some(v) => {
                    let s = v.as_str()?;
                    if !panels.iter().any(|p| p == s) {
                        return None;
                    }
                    Some(s.into())
                }
            };
            Some(Placement::Dock {
                id: id.into(),
                label,
                before_panel: before,
            })
        }
        _ => None,
    }
}
