//! Realm-independent activation staging and immutable export generations.
//!
//! Hosts run/await activate outside the actor. Stage only appends operations;
//! Finish validates against its starting revision and commits in one turn.
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::document::Snapshot;
use crate::tx::Writer;
use pm_model::{Op, Value};

/// Runtime-owned resources (callback/shader handles etc.) are identified by
/// writer and immutable definitions. Hosts retain the associated realm while
/// any job holds this generation, and dispose it only after those pins drop.
#[derive(Clone, Debug)]
pub struct ExtensionGeneration {
    pub extension: String,
    pub writer: Writer,
    pub definitions: Value,
}

/// One export's document and extension bindings, frozen together. Clone this
/// job for every frame/worker; never look up the current generation mid-job.
#[derive(Clone, Debug)]
pub struct ExportJob {
    pub snapshot: Snapshot,
    pub generations: Arc<BTreeMap<String, Arc<ExtensionGeneration>>>,
}

#[derive(Clone, Debug)]
pub enum ExtensionCall {
    BeginActivation {
        extension: String,
        definitions: Value,
    },
    Stage {
        ticket: u64,
        ops: Vec<Op>,
    },
    Finish {
        ticket: u64,
        success: bool,
    },
    PinExport,
}
impl From<ExtensionCall> for crate::tx::Call {
    fn from(value: ExtensionCall) -> Self {
        Self::Extension(value)
    }
}

pub(crate) struct Activation {
    pub generation: Arc<ExtensionGeneration>,
    pub revision: u64,
    pub epoch: u64,
    pub ops: Vec<Op>,
}
#[derive(Default)]
pub(crate) struct Extensions {
    pub epoch: u64,
    pub current: Arc<BTreeMap<String, Arc<ExtensionGeneration>>>,
    pub staged: BTreeMap<u64, Activation>,
}
