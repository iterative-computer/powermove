//! Concrete legacy thread/attachment storage through the existing kernel actor.
//! Synchronous Store methods are worker-only; Pending::wait enforces that on Mac.
use pm_agents::persistence::Store;
use pm_kernel::{
    KernelHandle,
    library::{StoreKey, parse_key},
    profile::ProfileCall,
    state::{StateOp, StateTransaction},
};
use pm_model::json::{Value as NativeValue, parse, to_canonical_string};
use serde_json::Value;
use std::{collections::BTreeMap, io};
pub struct KernelStore {
    kernel: KernelHandle,
    observed: BTreeMap<String, Option<NativeValue>>,
}
fn error(value: impl std::fmt::Display) -> io::Error {
    io::Error::other(value.to_string())
}
fn key_allowed(key: &str) -> io::Result<()> {
    if matches!(parse_key(key),Some(StoreKey::Dynamic {prefix,..}) if matches!(prefix.as_str(),"agentThreads"|"agentAttachment"))
    {
        Ok(())
    } else {
        Err(error("Not an agent archive key"))
    }
}
impl KernelStore {
    /// Bind the application's already-attached writer, opening no profile and
    /// taking no second library lock. The read-only profile call verifies an
    /// attached library: state transactions alone can also succeed in memory.
    /// The probe creates no localStorage entry and reads no credential store.
    pub fn bind(kernel: KernelHandle) -> io::Result<Self> {
        kernel
            .profile(ProfileCall::LocalStorageGet {
                origin: "powermove:agents".into(),
                key: "durable-store-probe".into(),
            })
            .wait()
            .ok_or_else(|| error("Kernel stopped"))?
            .map_err(error)?;
        Ok(Self {
            kernel,
            observed: BTreeMap::new(),
        })
    }
}
impl Store for KernelStore {
    fn get(&mut self, key: &str) -> io::Result<Option<Value>> {
        key_allowed(key)?;
        let value = self.kernel.state().settings.get(key).cloned();
        self.observed.insert(key.into(), value.clone());
        value
            .map(|v| serde_json::from_str(&to_canonical_string(&v)).map_err(error))
            .transpose()
    }
    fn set(&mut self, key: &str, value: &Value) -> io::Result<()> {
        key_allowed(key)?;
        let next = parse(&value.to_string()).map_err(error)?;
        let expected = self
            .observed
            .get(key)
            .ok_or_else(|| error("Read an agent archive before replacing it"))?;
        let state = self.kernel.state();
        if state.settings.get(key) != expected.as_ref() {
            return Err(error("Agent archive changed; reload before replacing it"));
        }
        let result = self
            .kernel
            .state_transaction(StateTransaction {
                expected_revision: Some(state.revision),
                ops: vec![StateOp::Setting {
                    key: key.into(),
                    value: Some(next.clone()),
                }],
            })
            .wait()
            .ok_or_else(|| error("Kernel stopped"))?
            .map_err(error)?;
        // The kernel durably publishes state intent + slot data before its ack.
        // Unrelated state races are explicit errors; never replay a stale archive.
        let _ = result;
        self.observed.insert(key.into(), Some(next));
        Ok(())
    }
}
