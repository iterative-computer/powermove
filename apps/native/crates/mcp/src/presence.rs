//! Project-bound, view-only agent presence. No timer and no document writes.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind { Focus, Create, Set, Key, Remove, Held, Landed, Dropped, RunEnd }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Presence {
    pub project: String,
    pub run: String,
    pub kind: Kind,
    pub layer: Option<String>,
}
