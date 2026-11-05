use anyhow::{Result, ensure};
use pm_update_policy::update_distribution::{
    sparkle::{Enclosure, Event, Session},
    *,
};
use std::io::{Cursor, Read};
use url::Url;

// Signature mechanics have real Ed25519 coverage in pm-distribution. This double
// isolates admission/callback/retention, with no Apple API or installer success.
struct Verifier;
impl SignatureVerifier for Verifier {
    fn verify(&self, message: &[u8], signature: &str) -> Result<()> {
        ensure!(
            message.starts_with(SIGNING_DOMAIN) && signature == "synthetic-policy",
            "rejected"
        );
        Ok(())
    }
}
struct Feed(Vec<u8>);
impl Transport for Feed {
    fn open(&mut self, _: &Url) -> Result<Box<dyn Read>> {
        Ok(Box::new(Cursor::new(self.0.clone())))
    }
}
struct Proof {
    eligible: Eligibility,
    now: u64,
}
impl Readiness for Proof {
    fn current(&mut self) -> Result<Eligibility> {
        Ok(self.eligible)
    }
    fn now_unix(&mut self) -> Result<u64> {
        Ok(self.now)
    }
}
struct Barrier(bool);
impl QuitBarrier for Barrier {
    fn ready(&mut self) -> Result<bool> {
        Ok(self.0)
    }
}
#[derive(Default)]
struct Native {
    calls: Vec<&'static str>,
    fail: bool,
}
impl Session for Native {
    fn check(&mut self, _: &str, _: &Release) -> Result<()> {
        self.calls.push("check");
        ensure!(!self.fail, "interrupted native dispatch");
        Ok(())
    }
    fn download(&mut self, _: &str) -> Result<()> {
        self.calls.push("download");
        Ok(())
    }
    fn install(&mut self, _: &str) -> Result<()> {
        self.calls.push("install");
        ensure!(!self.fail, "interrupted install");
        Ok(())
    }
    fn cancel(&mut self, _: &str) -> Result<()> {
        self.calls.push("cancel");
        Ok(())
    }
}
fn offer() -> (Offer, Proof) {
    let proof = Proof {
        eligible: Eligibility {
            packaged_macos: true,
            macos_major: 14,
            installed_identity_verified: true,
            transition_or_native_receipt_verified: true,
            owner_approved: true,
            source_generation: 2,
            checkpoint_generation: 2,
            final_delta_complete: true,
            all_six_owner_domains: true,
            state_and_secrets_complete: true,
            restore_verified: true,
        },
        now: 10,
    };
    let release = Release {
        schema: 1,
        channel: Channel::Beta,
        version: "2.0.0".into(),
        architecture: Architecture::Arm64,
        bundle_id: BETA_ID.into(),
        minimum_macos: 14,
        archive_url: "https://example.invalid/native.zip".into(),
        size: 100,
        sha256: "a".repeat(64),
        sparkle_signature: format!("{}==", "A".repeat(86)),
        expires_unix: 100,
    };
    let mut feed = Feed(
        serde_json::to_vec(&SignedPolicy {
            payload: serde_json::to_string(&release).unwrap(),
            signature: "synthetic-policy".into(),
        })
        .unwrap(),
    );
    let config = UpdateConfig {
        version: 1,
        enabled: true,
        feed_url: Some("https://example.invalid/policy".into()),
        public_key: Some("synthetic-pin".into()),
        channel: Channel::Beta,
    };
    let Check::Available(offer) = check(
        &config,
        proof.eligible,
        "1.0.0",
        Architecture::Arm64,
        10,
        &mut feed,
        &Verifier,
    )
    .unwrap() else {
        panic!("expected offer")
    };
    (offer, proof)
}
fn enclosure(offer: &Offer) -> Enclosure {
    let r = offer.release();
    Enclosure {
        version: r.version.clone(),
        url: r.archive_url.clone(),
        bytes: r.size,
        ed_signature: r.sparkle_signature.clone(),
    }
}
fn ready(store: &StagingStore, offer: &Offer, proof: &mut Proof, native: &mut Native) {
    store
        .begin_sparkle(offer, proof, &Verifier, native)
        .unwrap();
    store
        .sparkle_event(
            offer.id(),
            Event::Found(enclosure(offer)),
            proof,
            &Verifier,
            native,
        )
        .unwrap();
    store
        .sparkle_event(offer.id(), Event::Extracting, proof, &Verifier, native)
        .unwrap();
    store
        .sparkle_event(offer.id(), Event::Ready, proof, &Verifier, native)
        .unwrap();
}
#[test]
fn public_api_callbacks_require_exact_enclosure_and_normal_quit() {
    let t = tempfile::tempdir().unwrap();
    let store = StagingStore::open(&t.path().canonicalize().unwrap()).unwrap();
    let (offer, mut proof) = offer();
    let mut native = Native::default();
    ready(&store, &offer, &mut proof, &mut native);
    assert!(
        !store
            .install_sparkle(&mut proof, &Verifier, &mut Barrier(false), &mut native)
            .unwrap()
    );
    assert_eq!(store.status().unwrap().unwrap().1, Stage::SparkleReady);
    assert!(
        store
            .install_sparkle(&mut proof, &Verifier, &mut Barrier(true), &mut native)
            .unwrap()
    );
    assert_eq!(native.calls, ["check", "download", "install"]);
    assert!(
        store
            .sparkle_event(
                offer.id(),
                Event::Installed,
                &mut proof,
                &Verifier,
                &mut native
            )
            .is_err()
    );
    assert_eq!(
        store.status().unwrap().unwrap().1,
        Stage::SparkleInstallRequested
    );
}
#[test]
fn all_enclosure_mismatches_cancel_before_download() {
    for field in 0..4 {
        let t = tempfile::tempdir().unwrap();
        let store = StagingStore::open(&t.path().canonicalize().unwrap()).unwrap();
        let (offer, mut proof) = offer();
        let mut native = Native::default();
        store
            .begin_sparkle(&offer, &mut proof, &Verifier, &mut native)
            .unwrap();
        let mut e = enclosure(&offer);
        match field {
            0 => e.version = "3.0.0".into(),
            1 => e.url.push_str(".bad"),
            2 => e.bytes += 1,
            _ => e.ed_signature = "bad".into(),
        }
        assert!(
            store
                .sparkle_event(
                    offer.id(),
                    Event::Found(e),
                    &mut proof,
                    &Verifier,
                    &mut native
                )
                .is_err()
        );
        assert_eq!(native.calls, ["check", "cancel"]);
        assert_eq!(store.status().unwrap().unwrap().1, Stage::SparkleChecking);
    }
}
#[test]
fn changed_generation_and_expiry_cancel_found_without_side_effects() {
    for expired in [false, true] {
        let t = tempfile::tempdir().unwrap();
        let store = StagingStore::open(&t.path().canonicalize().unwrap()).unwrap();
        let (offer, mut proof) = offer();
        let mut native = Native::default();
        store
            .begin_sparkle(&offer, &mut proof, &Verifier, &mut native)
            .unwrap();
        if expired {
            proof.now = 100;
        } else {
            proof.eligible.source_generation += 1;
        }
        assert!(
            store
                .sparkle_event(
                    offer.id(),
                    Event::Found(enclosure(&offer)),
                    &mut proof,
                    &Verifier,
                    &mut native
                )
                .is_err()
        );
        assert_eq!(native.calls, ["check", "cancel"]);
    }
}
#[test]
fn stale_readiness_cannot_begin_or_install_and_unknown_callbacks_cannot_advance() {
    let t = tempfile::tempdir().unwrap();
    let store = StagingStore::open(&t.path().canonicalize().unwrap()).unwrap();
    let (offer, mut proof) = offer();
    let mut native = Native::default();
    proof.eligible.owner_approved = false;
    assert!(
        store
            .begin_sparkle(&offer, &mut proof, &Verifier, &mut native)
            .is_err()
    );
    assert!(native.calls.is_empty());
    assert!(store.status().unwrap().is_none());
    proof.eligible.owner_approved = true;
    store
        .begin_sparkle(&offer, &mut proof, &Verifier, &mut native)
        .unwrap();
    assert!(
        store
            .sparkle_event(offer.id(), Event::Ready, &mut proof, &Verifier, &mut native)
            .is_err()
    );
    assert!(
        store
            .sparkle_event(
                offer.id(),
                Event::Aborted,
                &mut proof,
                &Verifier,
                &mut native
            )
            .is_err()
    );
    store
        .sparkle_event(
            offer.id(),
            Event::Found(enclosure(&offer)),
            &mut proof,
            &Verifier,
            &mut native,
        )
        .unwrap();
    store
        .sparkle_event(
            offer.id(),
            Event::Extracting,
            &mut proof,
            &Verifier,
            &mut native,
        )
        .unwrap();
    store
        .sparkle_event(offer.id(), Event::Ready, &mut proof, &Verifier, &mut native)
        .unwrap();
    proof.eligible.source_generation += 1;
    assert!(
        store
            .install_sparkle(&mut proof, &Verifier, &mut Barrier(true), &mut native)
            .is_err()
    );
    assert_eq!(store.status().unwrap().unwrap().1, Stage::SparkleReady);
    assert_eq!(native.calls, ["check", "download"]);
}
#[test]
fn interrupted_native_dispatch_retains_receipt_and_requires_recovery_after_reopen() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let (offer, mut proof) = offer();
    let mut native = Native {
        fail: true,
        ..Default::default()
    };
    {
        let store = StagingStore::open(&root).unwrap();
        assert!(
            store
                .begin_sparkle(&offer, &mut proof, &Verifier, &mut native)
                .is_err()
        );
    }
    native.fail = false;
    let store = StagingStore::open(&root).unwrap();
    assert_eq!(store.status().unwrap().unwrap().1, Stage::SparkleChecking);
    assert!(
        store
            .begin_sparkle(&offer, &mut proof, &Verifier, &mut native)
            .is_err()
    );
    assert_eq!(native.calls, ["check"]);
}

#[test]
fn stale_callback_and_persisted_native_state_cannot_become_ready() {
    let t = tempfile::tempdir().unwrap();
    let store = StagingStore::open(&t.path().canonicalize().unwrap()).unwrap();
    let (offer, mut proof) = offer();
    let mut native = Native::default();
    store
        .begin_sparkle(&offer, &mut proof, &Verifier, &mut native)
        .unwrap();
    assert!(
        store
            .sparkle_event(
                "different-policy",
                Event::Found(enclosure(&offer)),
                &mut proof,
                &Verifier,
                &mut native
            )
            .is_err()
    );
    assert!(
        store
            .sparkle_event(
                offer.id(),
                Event::RecoveryRequired,
                &mut proof,
                &Verifier,
                &mut native
            )
            .is_err()
    );
    assert_eq!(native.calls, ["check"]);
    assert_eq!(store.status().unwrap().unwrap().1, Stage::SparkleChecking);
}
