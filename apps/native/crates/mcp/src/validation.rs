//! The two external-session parsers run at different points in the tagged
//! control flow. Keep their Zod 4 issue order and text without loading JS.
use crate::{Error, Result};
use serde_json::{Value, json};

pub(crate) fn session(name: &str, args: &Value) -> Result<()> {
    let mut issues = Vec::new();
    let fields: &[&str] = if name == "start_session" {
        &["prompt", "access", "context"]
    } else {
        &["commit"]
    };
    if !args.is_object() {
        issues.push(invalid_type("object", args, &[]));
    } else {
        for field in fields {
            let path = [*field];
            let value = args.get(field);
            if *field == "prompt" {
                if let Some(text) = value.and_then(Value::as_str) {
                    let len = text.chars().count();
                    if !(1..=200_000).contains(&len) {
                        let small = len == 0;
                        let (code, key, bound, op) = if small {
                            ("too_small", "minimum", 1, ">=")
                        } else {
                            ("too_big", "maximum", 200_000, "<=")
                        };
                        let mut issue = json!({"origin":"string","code":code});
                        issue[key] = json!(bound);
                        issue["inclusive"] = json!(true);
                        issue["path"] = json!(path);
                        issue["message"] = json!(format!(
                            "Too {}: expected string to have {op}{bound} characters",
                            if small { "small" } else { "big" }
                        ));
                        issues.push(issue);
                    }
                } else {
                    issues.push(type_issue("string", value, &path));
                }
            } else if *field == "commit" {
                if !value.is_some_and(Value::is_boolean) {
                    issues.push(type_issue("boolean", value, &path));
                }
            } else if let Some(value) = value {
                let choices = if *field == "access" {
                    ["project", "editor"]
                } else {
                    ["project", "app"]
                };
                if !choices.iter().any(|c| value == c) {
                    issues.push(json!({"code":"invalid_value","values":choices,"path":path,
                        "message":format!("Invalid option: expected one of \"{}\"|\"{}\"",choices[0],choices[1])}));
                }
            }
        }
        let unknown: Vec<_> = args
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| !fields.contains(&k.as_str()))
            .collect();
        if !unknown.is_empty() {
            let labels = unknown
                .iter()
                .map(|key| format!("\"{key}\""))
                .collect::<Vec<_>>()
                .join(", ");
            issues.push(json!({"code":"unrecognized_keys","keys":unknown,"path":[],
                "message":format!("Unrecognized key{}: {labels}",if unknown.len()==1 {""} else {"s"})}));
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::invalid(
            serde_json::to_string_pretty(&issues).unwrap(),
        ))
    }
}

fn type_issue(expected: &str, value: Option<&Value>, path: &[&str]) -> Value {
    match value {
        Some(value) => invalid_type(expected, value, path),
        None => json!({"expected":expected,"code":"invalid_type","path":path,
            "message":format!("Invalid input: expected {expected}, received undefined")}),
    }
}
fn invalid_type(expected: &str, value: &Value, path: &[&str]) -> Value {
    let received = match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    json!({"expected":expected,"code":"invalid_type","path":path,
        "message":format!("Invalid input: expected {expected}, received {received}")})
}

pub(crate) fn extensions(args: &Value) -> Result<()> {
    let mut issues = Vec::new();
    if !args.is_object() {
        issues.push(invalid_type("object", args, &[]));
    } else {
        match args.get("extensions") {
            Some(Value::Array(items)) => {
                for (index, item) in items.iter().enumerate() {
                    if !item.is_object() {
                        let mut issue = invalid_type("object", item, &[]);
                        issue["path"] = json!(["extensions", index]);
                        issues.push(issue);
                        continue;
                    }
                    for field in ["id", "action", "summary"] {
                        let mut issue = if field == "action" {
                            if ["created", "updated", "removed"]
                                .iter()
                                .any(|a| item[field] == *a)
                            {
                                None
                            } else {
                                Some(
                                    json!({"code":"invalid_value","values":["created","updated","removed"],"path":[],
                                    "message":"Invalid option: expected one of \"created\"|\"updated\"|\"removed\""}),
                                )
                            }
                        } else if !item[field].is_string() {
                            Some(type_issue("string", item.get(field), &[]))
                        } else if field == "summary"
                            && item[field].as_str().unwrap().chars().count() > 4000
                        {
                            Some(
                                json!({"origin":"string","code":"too_big","maximum":4000,"inclusive":true,"path":[],
                                "message":"Too big: expected string to have <=4000 characters"}),
                            )
                        } else {
                            None
                        };
                        if let Some(issue) = &mut issue {
                            issue["path"] = json!(["extensions", index, field]);
                        }
                        issues.extend(issue);
                    }
                    unknown_keys(
                        item,
                        &["id", "action", "summary"],
                        json!(["extensions", index]),
                        &mut issues,
                    );
                }
                if items.len() > 100 {
                    issues.push(json!({"origin":"array","code":"too_big","maximum":100,"inclusive":true,"path":["extensions"],
                        "message":"Too big: expected array to have <=100 items"}));
                }
            }
            value => issues.push(type_issue("array", value, &["extensions"])),
        }
        unknown_keys(args, &["extensions"], json!([]), &mut issues);
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::invalid(
            serde_json::to_string_pretty(&issues).unwrap(),
        ))
    }
}

fn unknown_keys(value: &Value, fields: &[&str], path: Value, issues: &mut Vec<Value>) {
    let keys: Vec<_> = value
        .as_object()
        .unwrap()
        .keys()
        .filter(|k| !fields.contains(&k.as_str()))
        .collect();
    if !keys.is_empty() {
        let labels = keys
            .iter()
            .map(|k| format!("\"{k}\""))
            .collect::<Vec<_>>()
            .join(", ");
        issues.push(json!({"code":"unrecognized_keys","keys":keys,"path":path,
            "message":format!("Unrecognized key{}: {labels}",if keys.len()==1 {""} else {"s"})}));
    }
}
