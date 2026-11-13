//! Recovery choices belong to the explicit open that produced them. MCP never
//! accepts or discards interrupted work just because another client connected.
use crate::{Error, Result};
use pm_kernel::session::{RecoveryDecision, RecoveryResult};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct PendingOpens(BTreeMap<String, Value>);
impl PendingOpens {
    pub fn required(&mut self, project: &str, ticket: String, recovery: RecoveryResult) -> Value {
        self.0.retain(|_, v| v["projectId"] != project);
        let (kind, truncated, reason, source_name) = match recovery {
            RecoveryResult::Recovered { truncated } => ("recoverable", truncated, None, None),
            RecoveryResult::OfferCopy { truncated, .. } => ("copy", truncated, None, None),
            RecoveryResult::Unrecoverable { reason } => {
                ("unrecoverable", false, Some(reason), None)
            }
            RecoveryResult::FromOtherFile { name, truncated } => (
                "from_other_file",
                truncated,
                Some("Declining leaves the unsaved changes with their original file.".into()),
                Some(name),
            ),
            RecoveryResult::None => ("none", false, None, None),
        };
        let choices = match kind {
            "unrecoverable" => json!(["open_saved_version"]),
            // Match the native Recover / Leave Them prompt. This work may
            // belong to a different file, so neither a copy nor destruction
            // is offered; the kernel leaves its journal untouched on decline.
            "from_other_file" => json!(["recover", "discard"]),
            _ => json!(["recover", "open_as_copy", "discard"]),
        };
        let mut value = json!({"projectId":project,"recoveryRequired":true,"ticket":ticket,
            "recovery":{"kind":kind,"truncated":truncated,"reason":reason,"choices":choices}});
        if let Some(name) = source_name {
            value["recovery"]["sourceName"] = json!(name);
        }
        self.0.insert(ticket, value.clone());
        value
    }
    pub fn list(&self) -> Value {
        json!(self.0.values().collect::<Vec<_>>())
    }
    pub fn take(&mut self, project: &str, choice: &Value) -> Result<(String, RecoveryDecision)> {
        let ticket = choice["ticket"]
            .as_str()
            .ok_or_else(|| Error::invalid("Recovery ticket is required"))?;
        let pending = self
            .0
            .get(ticket)
            .filter(|v| v["projectId"] == project)
            .ok_or_else(|| {
                Error::invalid(
                    "Recovery choice expired or belongs to another project; open the project again",
                )
            })?;
        let name = choice["decision"].as_str().unwrap_or("");
        if !pending["recovery"]["choices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == name)
        {
            return Err(Error::invalid(
                "This recovery decision is not available; use one of the returned choices",
            ));
        }
        let decision = match name {
            "recover" => RecoveryDecision::Recover,
            "open_as_copy" => RecoveryDecision::OpenAsCopy,
            "discard" => RecoveryDecision::Discard,
            "open_saved_version" => RecoveryDecision::OpenSavedVersion,
            _ => unreachable!(),
        };
        self.0.remove(ticket);
        Ok((ticket.into(), decision))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_files_recovery_keeps_its_name_and_offers_only_recover_or_leave() {
        let mut pending = PendingOpens::default();
        let offer = pending.required(
            "restored",
            "ticket".into(),
            RecoveryResult::FromOtherFile {
                name: "original.pmv".into(),
                truncated: true,
            },
        );
        assert_eq!(offer["recovery"]["kind"], "from_other_file");
        assert_eq!(offer["recovery"]["sourceName"], "original.pmv");
        assert_eq!(offer["recovery"]["truncated"], true);
        assert_eq!(offer["recovery"]["choices"], json!(["recover", "discard"]));
        for decision in ["open_as_copy", "open_saved_version"] {
            assert!(
                pending
                    .take("restored", &json!({"ticket":"ticket","decision":decision}))
                    .is_err()
            );
        }
        assert_eq!(
            pending
                .take("restored", &json!({"ticket":"ticket","decision":"discard"}))
                .unwrap(),
            ("ticket".into(), RecoveryDecision::Discard),
        );
        assert_eq!(pending.list(), json!([]));
    }
}
