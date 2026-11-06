//! Bounded source-edit review coordinator. Preserve original provider/settings
//! and already-applied edits when a capture/review fails. The actor owns all
//! edit/history operations; no model result is itself evidence of rendered pixels.
use crate::{error, session::Owner};
use serde_json::{Value, json};
use std::io;
pub struct Review {
    owner: Owner,
    settings: Value,
    applied: Vec<Value>,
    passes: u8,
    repair_pending: bool,
    frames: Option<Value>,
    review: Option<Value>,
    error: Option<String>,
}
impl Review {
    pub fn new(owner: Owner, settings: &Value, executed: Vec<Value>) -> io::Result<Self> {
        if !settings.is_object() {
            return Err(error("Invalid review settings"));
        }
        let mut settings = settings.clone();
        settings["mode"] = json!("editor");
        settings["access"] = json!("editor");
        settings["context"] = json!("project");
        Ok(Self {
            owner,
            settings,
            applied: executed,
            passes: 0,
            repair_pending: false,
            frames: None,
            review: None,
            error: None,
        })
    }
    /// Caller changes/picker state never modify the owned settings snapshot.
    pub fn settings(&self) -> &Value {
        &self.settings
    }
    pub fn applied(&self) -> &[Value] {
        &self.applied
    }
    pub fn observe(&mut self, owner: &Owner, frames: Result<Value, String>) -> io::Result<()> {
        if owner != &self.owner {
            return Err(error("The document changed during review."));
        }
        match frames {
            Ok(frames) => {
                if let Some(message) = frames["visualError"].as_str() {
                    self.error = Some(message.into());
                }
                self.frames = Some(frames);
            }
            Err(message) => {
                if self.error.is_none() {
                    self.error = Some(message);
                }
            }
        }
        Ok(())
    }
    pub fn response(&mut self, review: Value) -> io::Result<bool> {
        if !review.is_object()
            || !matches!(review["status"].as_str(), Some("pass" | "repair" | "fail"))
        {
            return Err(error("Invalid structured review"));
        }
        let repair = review["status"] == "repair"
            && review["commands"].as_array().is_some_and(|c| !c.is_empty())
            && self.passes < 2;
        self.repair_pending = repair;
        self.review = Some(review);
        Ok(repair)
    }
    /// Call only after the existing guarded actor edit succeeds. Review uses
    /// the commands actually executed; locked/skipped proposals are not added.
    pub fn repaired(&mut self, owner: &Owner, executed: Vec<Value>) -> io::Result<()> {
        if owner != &self.owner
            || !self.repair_pending
            || self.passes >= 2
            || self.review.as_ref().is_none_or(|r| r["status"] != "repair")
        {
            return Err(error("Review repair is no longer admissible"));
        }
        self.repair_pending = false;
        self.applied.extend(executed);
        self.passes += 1;
        self.frames = None;
        Ok(())
    }
    pub fn failed(&mut self, message: String) {
        if self.error.is_none() {
            self.error = Some(message);
        }
    }
    pub fn outcome(&self) -> Value {
        json!({"applied":self.applied,"review":self.review,"frames":self.frames,"reviewError":self.error.as_deref().unwrap_or_default(),"repairPasses":self.passes})
    }
}
