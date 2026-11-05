//! Admission precedes Panel mutation. Absent providers must not consume a draft,
//! start a fictitious run or mark an answer sent.
use serde_json::Value;
pub fn admit(action: &Value) -> Result<(), String> {
    let allowed = match action["type"].as_str() {
        Some(
            "draft" | "focus" | "newThread" | "switchThread" | "disclose" | "removeAttachment"
            | "setModel" | "chooseAnswer" | "typeAnswer" | "keepResult" | "openSettings",
        ) => true,
        Some("setAccess") => matches!(
            action["access"].as_str(),
            Some("editor" | "supervised" | "auto")
        ),
        Some("shell") => matches!(action["action"].as_str(), Some("open" | "close")),
        Some("key") => {
            action["key"]["key"] != "Enter"
                || action["key"]["composing"] == true
                || action["key"]["shiftKey"] == true
        }
        Some("chooseSlash") => action["id"] == "settings",
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err("Agent provider or native action service is unavailable".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn absent_provider_rejects_submission_answers_consent_and_steering_before_dispatch() {
        for action in [
            json!({"type":"submit"}),
            json!({"type":"answer"}),
            json!({"type":"stop"}),
            json!({"type":"setAccess","access":"full"}),
            json!({"type":"key","key":{"key":"Enter"}}),
        ] {
            assert!(admit(&action).is_err());
        }
        for action in [
            json!({"type":"draft"}),
            json!({"type":"openSettings"}),
            json!({"type":"key","key":{"key":"Enter","composing":true}}),
        ] {
            assert!(admit(&action).is_ok());
        }
    }
}
