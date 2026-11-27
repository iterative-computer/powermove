//! Parameter overrides use the web player's validation and preserve the
//! document's parameter definitions. An invalid override never writes a file.
use crate::corpus::{member, string};
use pm_model::{Map, Value};

pub fn parse(text: &str) -> Result<(String, Value), String> {
    let (name, json) = text.split_once('=').ok_or("--param needs NAME=JSON")?;
    if name.is_empty() {
        return Err("--param needs a parameter name".into());
    }
    let value =
        pm_model::json::parse(json).map_err(|e| format!("--param {name}: invalid JSON: {e}"))?;
    Ok((name.into(), value))
}

pub fn apply(project: &mut Value, overrides: &[(String, Value)]) -> Result<(), String> {
    if overrides.is_empty() {
        return Ok(());
    }
    let params = project
        .as_object_mut()
        .and_then(|p| p.get_mut("params"))
        .and_then(Value::as_object_mut)
        .ok_or("the project has no parameters")?;
    // Validate the whole request before changing even the in-memory copy.
    for (name, value) in overrides {
        let parameter = params
            .get(name)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("unknown parameter {name:?}"))?;
        validate(name, parameter, value)?;
    }
    for (name, value) in overrides {
        params
            .get_mut(name)
            .and_then(Value::as_object_mut)
            .expect("validated parameter")
            .insert("value", value.clone());
    }
    Ok(())
}

fn validate(name: &str, parameter: &Map, value: &Value) -> Result<(), String> {
    let number = |key: &str| match parameter.get(key) {
        Some(Value::Number(n)) => Some(*n),
        _ => None,
    };
    let valid = match string(parameter.get("control")).as_deref() {
        Some("num") => match value {
            Value::Number(n) => {
                n.is_finite()
                    && number("min").is_none_or(|min| *n >= min)
                    && number("max").is_none_or(|max| *n <= max)
            }
            _ => false,
        },
        Some("toggle") => matches!(value, Value::Bool(_)),
        Some("color") => string(Some(value)).is_some_and(|s| {
            (s.len() == 7 || s.len() == 9)
                && s.starts_with('#')
                && s.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        }),
        Some("select") => parameter
            .get("options")
            .and_then(Value::as_array)
            .is_some_and(|options| {
                options
                    .iter()
                    .any(|option| member(option, "v") == Some(value))
            }),
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(format!("invalid value for parameter {name:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> Value {
        pm_model::json::parse(r##"{"params":{"n":{"control":"num","value":3,"min":0,"max":10},"flag":{"control":"toggle","value":false},"color":{"control":"color","value":"#ffffff"},"choice":{"control":"select","value":"a","options":[{"v":"a"},{"v":"b"}]},"text":{"value":"before"}}}"##).unwrap()
    }
    #[test]
    fn parameters_preserve_definitions_and_validate_like_the_player() {
        let mut p = project();
        let inputs = [
            "n=5",
            "flag=true",
            "color=\"#aabbccdd\"",
            "choice=\"b\"",
            "text=\"a=b\"",
        ];
        let overrides = inputs.iter().map(|s| parse(s).unwrap()).collect::<Vec<_>>();
        apply(&mut p, &overrides).unwrap();
        let params = member(&p, "params").unwrap();
        assert_eq!(
            member(member(params, "n").unwrap(), "value"),
            Some(&Value::Number(5.0))
        );
        assert_eq!(
            member(member(params, "n").unwrap(), "max"),
            Some(&Value::Number(10.0))
        );
        for input in [
            "n=11",
            "n=\"5\"",
            "flag=0",
            "color=\"red\"",
            "choice=\"c\"",
            "missing=1",
        ] {
            let before = p.clone();
            assert!(
                apply(&mut p, &[parse("n=4").unwrap(), parse(input).unwrap()]).is_err(),
                "{input}"
            );
            assert_eq!(p, before);
        }
        for invalid in ["=1", "n", "n=undefined"] {
            assert!(parse(invalid).is_err());
        }
    }
}
