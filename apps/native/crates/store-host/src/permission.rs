use crate::{Error, Result, hex, identity::ExtensionId, install::StoreKernel};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Permission {
    #[serde(rename = "network")]
    Network,
    #[serde(rename = "clipboard")]
    Clipboard,
    #[serde(rename = "assets")]
    Assets,
    #[serde(rename = "project:write")]
    ProjectWrite,
    #[serde(rename = "full-access")]
    FullAccess,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: ExtensionId,
    pub version: String,
    #[serde(rename = "apiVersion", default = "api_one")]
    pub api_version: u32,
    #[serde(default)]
    pub permissions: BTreeSet<Permission>,
    /// Authored contributions and unknown fields survive round trips.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
fn api_one() -> u32 {
    1
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        if self.version.is_empty()
            || self.version.len() > 128
            || !(1..=4).contains(&self.api_version)
        {
            return Err(Error::Invalid("manifest version"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Allow,
    Deny,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDecision {
    pub publisher: String,
    pub decision: Decision,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionPrompt {
    pub id: String,
    pub extension: ExtensionId,
    pub release: String,
    pub publisher: String,
    pub revision: u64,
    pub permission: Permission,
    /// UI must distinguish full access from isolated Store capability grants.
    pub requires_trusted_realm: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    Granted,
    Denied,
    Prompt(PermissionPrompt),
    RequiresTrustedRealm,
}

pub fn request<K: StoreKernel>(
    kernel: &K,
    id: &ExtensionId,
    permission: Permission,
) -> Result<Admission> {
    let state = kernel.load(id)?.ok_or(Error::NotInstalled)?;
    let active = state.active.as_ref().ok_or(Error::NotInstalled)?;
    if !active.manifest.permissions.contains(&permission) {
        return Ok(Admission::Denied);
    }
    if let Some(d) = state
        .permissions
        .get(&permission)
        .filter(|d| d.publisher == active.publisher)
    {
        return Ok(match (d.decision, permission) {
            (Decision::Deny, _) => Admission::Denied,
            (Decision::Allow, Permission::FullAccess) => Admission::RequiresTrustedRealm,
            (Decision::Allow, _) => Admission::Granted,
        });
    }
    let body = serde_json::to_vec(&(
        id,
        &active.bundle,
        &active.publisher,
        state.revision,
        permission,
    ))
    .map_err(|_| Error::Invalid("prompt"))?;
    Ok(Admission::Prompt(PermissionPrompt {
        id: hex(&Sha256::digest(body)),
        extension: id.clone(),
        release: active.bundle.clone(),
        publisher: active.publisher.clone(),
        revision: state.revision,
        permission,
        requires_trusted_realm: permission == Permission::FullAccess,
    }))
}
/// Called only for a native/authorized agent response, never directly from view
/// JSON. CAS durability completes before any caller receives a grant.
pub fn respond<K: StoreKernel>(
    kernel: &mut K,
    prompt: &PermissionPrompt,
    decision: Decision,
) -> Result<()> {
    if request(kernel, &prompt.extension, prompt.permission)? != Admission::Prompt(prompt.clone()) {
        return Err(Error::Stale);
    }
    let mut state = kernel.load(&prompt.extension)?.ok_or(Error::NotInstalled)?;
    if state.revision != prompt.revision {
        return Err(Error::Stale);
    }
    state.permissions.insert(
        prompt.permission,
        PermissionDecision {
            publisher: prompt.publisher.clone(),
            decision,
        },
    );
    state.revision = state
        .revision
        .checked_add(1)
        .ok_or(Error::Invalid("revision exhausted"))?;
    kernel.compare_exchange(&prompt.extension, Some(prompt.revision), &state)
}

/// Host-owned operation policy; never accept a permission name from the page as
/// authority. Resource ownership, project binding, network redirect rules and
/// clipboard focus/gesture evidence are checked again by the kernel adapter.
#[derive(Debug, Default)]
pub struct MethodPolicy(BTreeMap<String, BTreeSet<Permission>>);
impl MethodPolicy {
    pub fn register(&mut self, method: String, required: BTreeSet<Permission>) -> Result<()> {
        if method.is_empty() || method.len() > 128 || required.contains(&Permission::FullAccess) {
            return Err(Error::Invalid("method policy"));
        }
        if self.0.contains_key(&method) {
            return Err(Error::Conflict);
        }
        self.0.insert(method, required);
        Ok(())
    }
    pub fn authorize<K: StoreKernel>(
        &self,
        kernel: &K,
        id: &ExtensionId,
        method: &str,
    ) -> Result<()> {
        // Even read-only calls require a live installation.
        if kernel.load(id)?.and_then(|s| s.active).is_none() {
            return Err(Error::NotInstalled);
        }
        let permissions = self.0.get(method).ok_or(Error::Denied)?;
        for p in permissions {
            if request(kernel, id, *p)? != Admission::Granted {
                return Err(Error::Denied);
            }
        }
        Ok(())
    }
}

/// Renderer-only policy. JS logic/fetch/wasm execute in the confined V8 helper.
/// Nonce must come from the platform CSPRNG, not a manifest or page request.
pub fn renderer_csp<K: StoreKernel>(kernel: &K, id: &ExtensionId, nonce: &str) -> Result<String> {
    if !(32..=128).contains(&nonce.len()) || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Invalid("applier nonce"));
    }
    let state = kernel.load(id)?.ok_or(Error::NotInstalled)?;
    if state.active.is_none() {
        return Err(Error::NotInstalled);
    }
    let remote = if request(kernel, id, Permission::Network)? == Admission::Granted {
        " https:"
    } else {
        ""
    };
    let local = format!("pm-store://{}", state.data_store.key);
    Ok(format!(
        "default-src 'none'; script-src 'nonce-{nonce}'; style-src 'unsafe-inline' {local}/assets/{remote}; font-src {local}/assets/ data: blob:{remote}; img-src {local}/assets/ data: blob:{remote}; media-src {local}/assets/ data: blob:{remote}; connect-src {local}; worker-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
    ))
}

/// Native settings/authorized-agent action. Revocation publishes a new state;
/// adapters must cancel existing network work and rebuild a network-enabled CSP.
pub fn revoke<K: StoreKernel>(
    kernel: &mut K,
    id: &ExtensionId,
    expected: u64,
    permission: Permission,
) -> Result<()> {
    let mut state = kernel.load(id)?.ok_or(Error::NotInstalled)?;
    if state.revision != expected {
        return Err(Error::Conflict);
    }
    let publisher = state
        .active
        .as_ref()
        .ok_or(Error::NotInstalled)?
        .publisher
        .clone();
    state.permissions.insert(
        permission,
        PermissionDecision {
            publisher,
            decision: Decision::Deny,
        },
    );
    state.revision = state
        .revision
        .checked_add(1)
        .ok_or(Error::Invalid("revision exhausted"))?;
    kernel.compare_exchange(id, Some(expected), &state)
}
