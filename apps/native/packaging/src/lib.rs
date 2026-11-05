//! Worker-side distribution plumbing. No UI, JS, installation helper or publishing.
pub use pm_services::updater;
pub use pm_update_policy::{update_distribution, update_handoff};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use std::{io::Read, time::Duration};
use update_distribution::{SignatureVerifier, Transport};
use url::Url;

/// A key pinned in the signed app configuration, never supplied by the feed.
pub struct Ed25519Verifier(VerifyingKey);
impl Ed25519Verifier {
    pub fn from_base64(key: &str) -> Result<Self> {
        let bytes: [u8; 32] = STANDARD
            .decode(key)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid key length"))?;
        let key = VerifyingKey::from_bytes(&bytes)?;
        ensure!(!key.is_weak(), "weak update key");
        Ok(Self(key))
    }
}
impl SignatureVerifier for Ed25519Verifier {
    fn verify(&self, message: &[u8], signature: &str) -> Result<()> {
        let signature = Signature::from_slice(&STANDARD.decode(signature)?)?;
        self.0
            .verify_strict(message, &signature)
            .context("update signature rejected")
    }
}

/// HTTPS only, bounded time, TLS validation on, and redirects disabled. Construct
/// on a worker. Tests use an independent loopback transport, not a TLS bypass.
pub struct HttpsTransport(reqwest::blocking::Client);
impl HttpsTransport {
    pub fn new() -> Result<Self> {
        Ok(Self(
            reqwest::blocking::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(30 * 60))
                .build()?,
        ))
    }
}
impl Transport for HttpsTransport {
    fn open(&mut self, url: &Url) -> Result<Box<dyn Read>> {
        update_distribution::validate_https(url)?;
        let response = self.0.get(url.clone()).send()?;
        ensure!(
            response.status() == reqwest::StatusCode::OK,
            "update HTTP status rejected"
        );
        Ok(Box::new(response))
    }
}

/// Production entry point binds the verifier to the *bundled* public key. The
/// generic service functions remain injectable for tests and other native hosts.
pub struct UpdateClient {
    config: update_distribution::UpdateConfig,
    verifier: Option<Ed25519Verifier>,
    transport: HttpsTransport,
}
impl UpdateClient {
    pub fn new(config: update_distribution::UpdateConfig) -> Result<Self> {
        config.validate()?;
        let verifier = if config.enabled {
            Some(Ed25519Verifier::from_base64(
                config
                    .public_key
                    .as_deref()
                    .context("missing bundled update key")?,
            )?)
        } else {
            None
        };
        Ok(Self {
            config,
            verifier,
            transport: HttpsTransport::new()?,
        })
    }
    pub fn check(
        &mut self,
        eligibility: update_distribution::Eligibility,
        current: &str,
        architecture: update_distribution::Architecture,
        now: u64,
    ) -> Result<update_distribution::Check> {
        let Some(verifier) = self.verifier.as_ref() else {
            return Ok(update_distribution::Check::Gated);
        };
        update_distribution::check(
            &self.config,
            eligibility,
            current,
            architecture,
            now,
            &mut self.transport,
            verifier,
        )
    }
    pub fn verifier(&self) -> Result<&Ed25519Verifier> {
        self.verifier
            .as_ref()
            .context("updates disabled in bundled configuration")
    }
    pub fn download_and_stage(
        &mut self,
        store: &update_distribution::StagingStore,
        offer: &update_distribution::Offer,
        readiness: &mut impl update_distribution::Readiness,
        now: u64,
        installer: &mut impl update_distribution::Installer,
    ) -> Result<()> {
        let verifier = self
            .verifier
            .as_ref()
            .context("updates disabled in bundled configuration")?;
        ensure!(
            offer.release().channel == self.config.channel,
            "update channel mismatch"
        );
        store.download_and_stage(
            offer,
            readiness,
            now,
            &mut self.transport,
            verifier,
            installer,
        )
    }
}
