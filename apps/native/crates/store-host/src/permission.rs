use crate::{
    Error, Result, hex,
    identity::ExtensionId,
    install::{ExtensionState, StoreKernel},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

mod presentation;
pub use presentation::*;

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
/// Shipped disclosure order; independent of enum/JSON ordering.
pub const PERMISSION_ORDER: [Permission; 5] = [
    Permission::FullAccess,
    Permission::ProjectWrite,
    Permission::Assets,
    Permission::Network,
    Permission::Clipboard,
];
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: ExtensionId,
    pub version: String,
    /// Display name from the signed manifest. Empty only for older records
    /// admitted before display metadata was read; never fall back to the ID.
    #[serde(default)]
    pub name: String,
    /// Exact bundle asset ID; release admission checks the signed inventory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Author-supplied plain text; adapters must never interpret markup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
        if self.reason.as_ref().is_some_and(|reason| {
            reason.chars().count() > 140 || reason.chars().any(char::is_control)
        }) {
            return Err(Error::Invalid("permission reason"));
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
pub struct PermissionRequest {
    pub id: String,
    pub extension: ExtensionId,
    pub release: String,
    pub publisher: String,
    pub revision: u64,
    pub extension_name: String,
    pub publisher_name: String,
    /// Host-pinned Store identity evidence, never a manifest assertion.
    pub publisher_verified: bool,
    pub icon: Option<String>,
    pub reason: Option<String>,
    /// Every undecided declared permission, in shipped disclosure order.
    pub permissions: Vec<Permission>,
    /// UI must distinguish full access from isolated Store capability grants.
    pub requires_trusted_realm: bool,
}
/// Kept for callers using the original prompt name; responses are now grouped.
pub type PermissionPrompt = PermissionRequest;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    Granted,
    Denied,
    Prompt(Box<PermissionPrompt>),
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
    Ok(Admission::Prompt(Box::new(pending_request(&state)?)))
}

fn pending_request(state: &ExtensionState) -> Result<PermissionRequest> {
    let active = state.active.as_ref().ok_or(Error::NotInstalled)?;
    let permissions: Vec<_> = PERMISSION_ORDER
        .into_iter()
        .filter(|p| {
            active.manifest.permissions.contains(p)
                && state
                    .permissions
                    .get(p)
                    .is_none_or(|d| d.publisher != active.publisher)
        })
        .collect();
    let id = &active.manifest.id;
    let mut prompt = PermissionRequest {
        id: String::new(),
        extension: id.clone(),
        release: active.bundle.clone(),
        publisher: active.publisher.clone(),
        revision: state.revision,
        extension_name: active.manifest.name.clone(),
        publisher_name: active.publisher_identity.name.clone(),
        publisher_verified: active.publisher_identity.verified,
        icon: active.manifest.icon.clone(),
        reason: active.manifest.reason.clone(),
        requires_trusted_realm: permissions.contains(&Permission::FullAccess),
        permissions,
    };
    // Include display metadata in the binding, with an empty ID for hashing.
    let body = serde_json::to_vec(&prompt).map_err(|_| Error::Invalid("prompt"))?;
    prompt.id = hex(&Sha256::digest(body));
    Ok(prompt)
}
/// Called only for a native/authorized agent response, never directly from view
/// JSON. CAS durability completes before any caller receives a grant.
pub fn respond<K: StoreKernel>(
    kernel: &mut K,
    prompt: &PermissionPrompt,
    decision: Decision,
) -> Result<()> {
    let decisions = prompt.permissions.iter().map(|p| (*p, decision)).collect();
    respond_all(kernel, prompt, &decisions)
}

/// Record exactly one decision for every disclosed permission in one durable
/// CAS. A partial/extra response cannot publish a subset of an install's consent.
/// `Stale` means dismiss and re-request; it is not a user-facing error.
pub fn respond_all<K: StoreKernel>(
    kernel: &mut K,
    prompt: &PermissionRequest,
    decisions: &BTreeMap<Permission, Decision>,
) -> Result<()> {
    let mut state = kernel.load(&prompt.extension)?.ok_or(Error::Stale)?;
    if state.active.is_none()
        || pending_request(&state)? != *prompt
        || prompt.permissions.is_empty()
    {
        return Err(Error::Stale);
    }
    if decisions.len() != prompt.permissions.len()
        || !prompt.permissions.iter().all(|p| decisions.contains_key(p))
    {
        return Err(Error::Invalid("permission response coverage"));
    }
    for (permission, decision) in decisions {
        state.permissions.insert(
            *permission,
            PermissionDecision {
                publisher: prompt.publisher.clone(),
                decision: *decision,
            },
        );
    }
    state.revision = state
        .revision
        .checked_add(1)
        .ok_or(Error::Invalid("revision exhausted"))?;
    kernel
        .compare_exchange(&prompt.extension, Some(prompt.revision), &state)
        .map_err(|error| {
            if error == Error::Conflict {
                Error::Stale
            } else {
                error
            }
        })
}

/// Settings/authorized-agent inspection. Includes inactive/old-publisher
/// decisions for preservation, but only `active` rows can be current switches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredPermission {
    pub permission: Permission,
    pub publisher: String,
    pub decision: Decision,
    pub active: bool,
}

pub fn stored_decisions<K: StoreKernel>(
    kernel: &K,
    id: &ExtensionId,
) -> Result<Vec<StoredPermission>> {
    let state = kernel.load(id)?.ok_or(Error::NotInstalled)?;
    Ok(PERMISSION_ORDER
        .into_iter()
        .filter_map(|permission| {
            state
                .permissions
                .get(&permission)
                .map(|stored| StoredPermission {
                    permission,
                    publisher: stored.publisher.clone(),
                    decision: stored.decision,
                    active: state.active.as_ref().is_some_and(|release| {
                        release.publisher == stored.publisher
                            && release.manifest.permissions.contains(&permission)
                    }),
                })
        })
        .collect())
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
