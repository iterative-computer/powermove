//! Explicit consent boundary for Chromium/macOS safeStorage. Never called by
//! default first launch. Plaintext exists only in zeroizing buffers and Keychain.
use crate::disk;
use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::path::Path;
use zeroize::Zeroizing;

pub const PROMPT: &str = "Powermove can keep your extension secrets and sign-ins. Continue to allow one macOS Keychain request for Powermove Safe Storage. Your original app stays unchanged.";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SecretStep {
    NotStarted,
    PromptRequired,
    AccessStarted,
    /// Explicit Not Now / Continue Without Them; ordinary launches never prompt.
    Skipped,
    Complete,
}
/// Implementations must be idempotent, preserve existing differing items, and
/// never log secrets. The concrete macOS backend is opt-in, never used by tests.
pub trait SecretAccess {
    fn legacy_password(&mut self) -> Result<Zeroizing<Vec<u8>>>;
    fn put_native(&mut self, id: &str, plaintext: &[u8]) -> Result<()>;
}
pub(crate) struct Payload {
    pub id: String,
    pub ciphertext: Vec<u8>,
}
pub(crate) fn inventory(profile: &Path) -> Result<Vec<Payload>> {
    fn push(out: &mut Vec<Payload>, id: String, bytes: Vec<u8>) {
        out.push(Payload {
            id,
            ciphertext: bytes,
        });
    }
    let mut out = Vec::new();
    let env = profile.join("env");
    if env.is_dir() {
        for p in disk::walk(&env)?.keys() {
            if p.extension().is_some_and(|x| x == "env") {
                let text = std::fs::read_to_string(env.join(p))?;
                for line in text.lines() {
                    let Some((key, value)) = line.split_once('=') else {
                        continue;
                    };
                    let value = value.trim();
                    // Quoted "enc:..." is a literal according to shipped decodeStored.
                    if let Some(encoded) = value.strip_prefix("enc:") {
                        let key = key.trim();
                        ensure!(!key.is_empty(), "invalid sealed environment key");
                        push(
                            &mut out,
                            format!("env/{}/{}", p.display(), key),
                            STANDARD
                                .decode(encoded)
                                .map_err(|_| anyhow::anyhow!("invalid sealed environment value"))?,
                        );
                    }
                }
            }
        }
    }
    let session = profile.join("cloud/session.bin");
    if session.is_file() {
        push(&mut out, "cloud/session".into(), std::fs::read(session)?);
    }
    let human = profile.join("cloud/human");
    if human.is_dir() {
        for p in disk::walk(&human)?.keys() {
            if p.extension().is_some_and(|x| x == "bin") {
                push(
                    &mut out,
                    format!("cloud/human/{}", p.display()),
                    std::fs::read(human.join(p))?,
                );
            }
        }
    }
    let provider = profile.join("agent-provider.json");
    if provider.is_file() {
        let value: serde_json::Value = disk::json(&provider)?;
        if let Some(encoded) = value
            .get("secret")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
        {
            push(
                &mut out,
                "agent-provider".into(),
                STANDARD
                    .decode(encoded)
                    .map_err(|_| anyhow::anyhow!("invalid sealed provider value"))?,
            );
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}
/// Chromium OSCrypt macOS v10: PBKDF2-SHA1/1003, AES-128-CBC, space IV.
/// No Linux fallback, no lossy text conversion, no plaintext in error strings.
pub fn decrypt_mac(password: &[u8], ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        ciphertext.starts_with(b"v10") && ciphertext.len() > 3,
        "unsupported safeStorage ciphertext"
    );
    let mut key = Zeroizing::new([0u8; 16]);
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", 1003, &mut *key);
    let mut plain = Zeroizing::new(ciphertext[3..].to_vec());
    let len = cbc::Decryptor::<aes::Aes128>::new((&*key).into(), &[b' '; 16].into())
        .decrypt_padded_mut::<Pkcs7>(&mut plain)
        .map_err(|_| anyhow::anyhow!("safeStorage decryption failed"))?
        .len();
    plain.truncate(len);
    ensure!(
        std::str::from_utf8(&plain).is_ok(),
        "safeStorage plaintext is not UTF-8"
    );
    Ok(plain)
}

/// Explicitly construct only after the host has displayed PROMPT and received
/// consent. This backend reads the existing item; it never creates/rekeys it.
#[cfg(target_os = "macos")]
pub struct MacKeychain {
    service: String,
}
#[cfg(target_os = "macos")]
impl MacKeychain {
    pub fn after_user_consent() -> Self {
        Self {
            service: format!("{} migration secrets", crate::BETA_BUNDLE_ID),
        }
    }
}
#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use std::{ffi::c_void, ptr};
    type Item = *mut c_void;
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecKeychainFindGenericPassword(
            keychain: Item,
            service_len: u32,
            service: *const u8,
            account_len: u32,
            account: *const u8,
            len: *mut u32,
            data: *mut *mut c_void,
            item: *mut Item,
        ) -> i32;
        fn SecKeychainItemFreeContent(attrs: *mut c_void, data: *mut c_void) -> i32;
        fn SecKeychainAddGenericPassword(
            keychain: Item,
            service_len: u32,
            service: *const u8,
            account_len: u32,
            account: *const u8,
            len: u32,
            data: *const c_void,
            item: *mut Item,
        ) -> i32;
    }
    fn find(service: &str, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let mut len = 0u32;
        let mut data = ptr::null_mut();
        // SAFETY: fixed byte slices with explicit lengths, output owned by
        // Security.framework until FreeContent; no password in process args.
        let status = unsafe {
            SecKeychainFindGenericPassword(
                ptr::null_mut(),
                service.len() as u32,
                service.as_ptr(),
                account.len() as u32,
                account.as_ptr(),
                &mut len,
                &mut data,
                ptr::null_mut(),
            )
        };
        if status == -25300 {
            return Ok(None);
        }
        ensure!(status == 0, "Keychain access failed (OSStatus {status})");
        let bytes = if len == 0 {
            Zeroizing::new(Vec::new())
        } else {
            ensure!(!data.is_null(), "invalid Keychain reply");
            Zeroizing::new(
                unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len as usize) }.to_vec(),
            )
        };
        unsafe {
            SecKeychainItemFreeContent(ptr::null_mut(), data);
        }
        Ok(Some(bytes))
    }
    impl SecretAccess for MacKeychain {
        fn legacy_password(&mut self) -> Result<Zeroizing<Vec<u8>>> {
            find("Powermove Safe Storage", "Powermove")?
                .ok_or_else(|| anyhow::anyhow!("Powermove Safe Storage item not found"))
        }
        fn put_native(&mut self, id: &str, plaintext: &[u8]) -> Result<()> {
            let account = blake3::hash(id.as_bytes()).to_hex().to_string();
            if let Some(existing) = find(&self.service, &account)? {
                ensure!(
                    &existing[..] == plaintext,
                    "native Keychain conflict; both identities retained"
                );
                return Ok(());
            }
            ensure!(plaintext.len() <= u32::MAX as usize, "secret too large");
            let status = unsafe {
                SecKeychainAddGenericPassword(
                    ptr::null_mut(),
                    self.service.len() as u32,
                    self.service.as_ptr(),
                    account.len() as u32,
                    account.as_ptr(),
                    plaintext.len() as u32,
                    plaintext.as_ptr().cast(),
                    ptr::null_mut(),
                )
            };
            ensure!(
                status == 0,
                "native Keychain publication failed (OSStatus {status})"
            );
            Ok(())
        }
    }
}
#[cfg(not(target_os = "macos"))]
pub struct MacKeychain;
#[cfg(not(target_os = "macos"))]
impl SecretAccess for MacKeychain {
    fn legacy_password(&mut self) -> Result<Zeroizing<Vec<u8>>> {
        anyhow::bail!("macOS Keychain required")
    }
    fn put_native(&mut self, _: &str, _: &[u8]) -> Result<()> {
        anyhow::bail!("macOS Keychain required")
    }
}

/// An explicit later Settings retry reads CURRENT encrypted input, never stale
/// sign-ins from the first migration checkpoint. Caller holds first-launch lease.
/// Failed attempts remain private and retained. No Keychain access happens here.
pub(crate) fn capture_retry(source: &Path, native: &Path) -> Result<std::path::PathBuf> {
    use std::{collections::BTreeMap, fs};
    fn files(source: &Path) -> Result<BTreeMap<std::path::PathBuf, disk::Stamp>> {
        let mut out = BTreeMap::new();
        for family in ["env", "cloud", "agent-provider.json"] {
            let root = source.join(family);
            if root.is_file() {
                out.extend(disk::walk(&root)?.into_values().map(|s| (family.into(), s)));
            } else {
                out.extend(
                    disk::walk(&root)?
                        .into_iter()
                        .map(|(p, s)| (std::path::Path::new(family).join(p), s)),
                );
            }
        }
        Ok(out)
    }
    let retries = native.join("secret-consent-retries");
    disk::private_dir(&retries)?;
    let attempt = (0u64..)
        .map(|n| retries.join(format!("retry-{n:016x}")))
        .find(|p| fs::symlink_metadata(p).is_err())
        .unwrap();
    disk::private_dir(&attempt)?;
    let before = files(source)?;
    let mut copied = BTreeMap::new();
    for relative in before.keys() {
        let target = attempt.join(relative);
        disk::private_dir(target.parent().unwrap())?;
        copied.insert(
            relative.clone(),
            disk::copy(&source.join(relative), &target)?,
        );
    }
    ensure!(
        before == files(source)?,
        "secret source changed; retry after original app quits"
    );
    for (relative, entry) in &copied {
        ensure!(
            disk::hash(&source.join(relative))? == (entry.hash.clone(), entry.bytes),
            "secret source changed during capture"
        );
    }
    ensure!(
        !pm_kernel::locks::older_electron_running(source),
        "original app started during secret capture"
    );
    disk::atomic(&attempt.join("encrypted-inputs.json"), &copied)?;
    Ok(attempt)
}
