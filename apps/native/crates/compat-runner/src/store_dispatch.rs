//! Store contributions go to the kernel's real provider stacks. Callback
//! continuations return to the confined owner after releasing the kernel lock.
use crate::{StoreAuthority, StoreDispatch};
use pm_kernel::{
    state::{Registration, StateOp, StateTransaction},
    tx::Writer,
};
use pm_runtime_v8::{Error, Generation, HostBudget, Result};
use pm_store_host::adapter::StoreSession;
use serde_json::{Value, json};
use std::sync::Mutex;

pub struct KernelStoreDispatch {
    owner: Writer,
    session: StoreSession,
    failures: Mutex<Vec<String>>,
}
impl KernelStoreDispatch {
    /// The kernel/session coordinator supplies a unique non-native writer.
    /// A replacement helper needs a new writer, even for the same release.
    pub fn new(owner: Writer, session: StoreSession) -> Result<Self> {
        if owner.realm == 0 || owner.generation == 0 {
            return Err(Error::Invalid("Store writer"));
        }
        Ok(Self {
            owner,
            session,
            failures: Mutex::new(Vec::new()),
        })
    }
    pub fn failures(&self) -> Vec<String> {
        self.failures.lock().unwrap().clone()
    }
    fn token(&self, token: &str) -> Result<String> {
        if token.is_empty() || token.encode_utf16().count() > 128 {
            return Err(Error::Invalid("Store registration token"));
        }
        Ok(format!(
            "store:{}:{}:{token}",
            self.owner.realm, self.owner.generation
        ))
    }
    fn qualify(&self, id: &str) -> String {
        let owner = self.session.data_store.extension.as_str();
        if id == owner || id.starts_with(&format!("{owner}.")) {
            id.into()
        } else {
            format!("{owner}.{id}")
        }
    }
    fn apply(&self, kernel: &dyn StoreAuthority, ops: Vec<StateOp>) -> Result<()> {
        kernel
            .transact(StateTransaction {
                expected_revision: Some(kernel.state().revision),
                ops,
            })
            .map_err(Error::Host)
    }
    fn check(&self, session: &StoreSession) -> Result<()> {
        if session.data_store != self.session.data_store
            || session.release != self.session.release
            || session.revision != self.session.revision
        {
            return Err(Error::Invalid("Store dispatcher identity"));
        }
        Ok(())
    }
}
fn text<'a>(value: &'a Value, key: &str, max: usize) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty() && text.encode_utf16().count() <= max)
        .ok_or(Error::Invalid("Store registration string"))
}
/// Native registry of a Store contribution kind. Each is declarative data; the
/// callbacks a definition names stay handles into the confined helper.
fn registry(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "commands" => "commands",
        "effects" => "effects",
        "transitions" => "transitions",
        "layers" => "layerTypes",
        "panels" => "panels",
        "theme" => "themes",
        "status" => "status",
        _ => return None,
    })
}
fn handle(value: &Value) -> bool {
    value.as_u64().is_some_and(|id| id < (1 << 53))
}
fn command(value: &Value) -> Result<()> {
    let object = value.as_object().ok_or(Error::Invalid("Store command"))?;
    if object
        .keys()
        .any(|key| !["id", "label", "category", "kb", "run", "when"].contains(&key.as_str()))
        || !handle(&value["run"])
        || value.get("when").is_some_and(|v| !handle(v))
        || value
            .get("category")
            .is_some_and(|v| v.as_str().is_none_or(|v| v.encode_utf16().count() > 4096))
        || value.get("kb").is_some_and(|v| {
            !v.is_null() && v.as_str().is_none_or(|v| v.encode_utf16().count() > 4096)
        })
    {
        return Err(Error::Invalid("Store command schema"));
    }
    text(value, "id", 128)?;
    text(value, "label", 512)?;
    Ok(())
}
impl StoreDispatch for KernelStoreDispatch {
    fn call(
        &self,
        kernel: &dyn StoreAuthority,
        session: &StoreSession,
        method: &str,
        args: &Value,
        budget: &HostBudget,
    ) -> Result<Value> {
        budget.check()?;
        self.check(session)?;
        let args = args
            .as_array()
            .ok_or(Error::Invalid("Store dispatch arguments"))?;
        match method {
            "register" => {
                let registry = args
                    .first()
                    .and_then(Value::as_str)
                    .filter(|_| args.len() == 3)
                    .and_then(registry)
                    .ok_or_else(|| {
                        Error::Host("Store registration backend not attached for this kind".into())
                    })?;
                let token = self.token(args[1].as_str().ok_or(Error::Invalid("Store token"))?)?;
                let mut definition = args[2].clone();
                if registry == "commands" {
                    command(&definition)?;
                } else {
                    text(&definition, "id", 128)?;
                }
                let id = self.qualify(definition["id"].as_str().unwrap());
                definition["id"] = json!(id);
                let state = kernel.state();
                if state
                    .registries
                    .values()
                    .flatten()
                    .filter(|entry| entry.owner == self.owner)
                    .count()
                    >= 200
                {
                    return Err(Error::Quota("Store registrations"));
                }
                if state
                    .provider(registry, &id)
                    .is_some_and(|entry| entry.owner != self.owner)
                {
                    return Err(Error::Invalid("Store registration collision"));
                }
                self.apply(
                    kernel,
                    vec![StateOp::Register {
                        registry: registry.into(),
                        entry: Registration {
                            token,
                            owner: self.owner,
                            id,
                            definition: pm_model::json::parse(&definition.to_string())
                                .map_err(|_| Error::Invalid("Store definition"))?,
                        },
                    }],
                )?;
                Ok(json!({}))
            }
            "dispose-registration" => {
                if args.len() != 1 {
                    return Err(Error::Invalid("Store dispose arity"));
                }
                let token = self.token(args[0].as_str().ok_or(Error::Invalid("Store token"))?)?;
                self.apply(kernel, vec![StateOp::Dispose { token }])?;
                Ok(json!({}))
            }
            "invoke" if args.len() == 3 && args[0] == "commands" && args[1] == "run" => {
                let params = args[2]
                    .as_array()
                    .filter(|args| !args.is_empty())
                    .ok_or(Error::Invalid("Store command arguments"))?;
                let id = self.qualify(
                    params[0]
                        .as_str()
                        .ok_or(Error::Invalid("Store command id"))?,
                );
                let state = kernel.state();
                let entry = state
                    .provider("commands", &id)
                    .filter(|entry| entry.owner == self.owner)
                    .ok_or(Error::Invalid("Store command owner"))?;
                let definition: Value =
                    serde_json::from_str(&pm_model::json::to_canonical_string(&entry.definition))
                        .map_err(|_| Error::Invalid("Store command"))?;
                Ok(
                    json!({"callback":{"run":definition["run"],"when":definition.get("when"),"args":&params[1..]}}),
                )
            }
            _ => Err(Error::Host("Store operation backend not attached".into())),
        }
    }
    fn revoke(&self, kernel: &dyn StoreAuthority, session: &StoreSession, _: &Generation) {
        let result = self
            .check(session)
            .and_then(|()| self.apply(kernel, vec![StateOp::DisposeOwner { owner: self.owner }]));
        if let Err(error) = result {
            let mut failures = self.failures.lock().unwrap();
            if failures.len() < 256 {
                failures.push(error.to_string());
            }
        }
    }
    fn seal(&self, _: &dyn StoreAuthority, _: &StoreSession, _: &Generation) {}
}
