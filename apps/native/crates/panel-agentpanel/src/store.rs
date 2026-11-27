//! Durable thread/attachment storage through the existing profile owner.
//! Synchronous Store methods are worker-only; Pending::wait enforces that on Mac.
use pm_agents::persistence::Store;
use pm_kernel::{
    KernelHandle,
    library::{StoreKey, parse_key},
    profile::{ProfileCall, ProfileReply},
};
use serde_json::Value;
use std::{collections::BTreeMap, io};
pub struct KernelStore {
    kernel: KernelHandle,
    observed: BTreeMap<String, Option<Value>>,
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
        let result = self
            .kernel
            .profile(ProfileCall::AgentGet {
                application: false,
                key: key.into(),
            })
            .wait()
            .ok_or_else(|| error("Kernel stopped"))?
            .map_err(error)?;
        let ProfileReply::AgentValue(value) = result else {
            return Err(error("Unexpected Agent storage reply"));
        };
        self.observed.insert(key.into(), value.clone());
        Ok(value)
    }
    fn set(&mut self, key: &str, value: &Value) -> io::Result<()> {
        key_allowed(key)?;
        let expected = self
            .observed
            .get(key)
            .ok_or_else(|| error("Read an agent archive before replacing it"))?;
        self.kernel
            .profile(ProfileCall::AgentSet {
                application: false,
                key: key.into(),
                expected: Some(expected.clone()),
                value: value.clone(),
            })
            .wait()
            .ok_or_else(|| error("Kernel stopped"))?
            .map_err(error)?;
        self.observed.insert(key.into(), Some(value.clone()));
        Ok(())
    }
}
