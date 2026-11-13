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
        let (kind, truncated, reason) = match recovery {
            RecoveryResult::Recovered { truncated } => ("recoverable", truncated, None),
            RecoveryResult::OfferCopy { truncated, .. } => ("copy", truncated, None),
            RecoveryResult::Unrecoverable { reason } => ("unrecoverable", false, Some(reason)),
            RecoveryResult::None => ("none", false, None),
        };
        let choices = if kind == "unrecoverable" {
            json!(["open_saved_version"])
        } else {
            json!(["recover", "open_as_copy", "discard"])
        };
        let value = json!({"projectId":project,"recoveryRequired":true,"ticket":ticket,
            "recovery":{"kind":kind,"truncated":truncated,"reason":reason,"choices":choices}});
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
