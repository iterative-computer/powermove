//! Media the first launch moved into the library's `Media Store`: provenance
//! survives into exports and files, and Undo-only references travel.
//! All projects and bytes here are synthetic.
mod common;
use pm_kernel::{persistence::DiskFormat, session::*, *};
use pm_model::json::parse;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

const PROJECT: &str = r##"{"id":"P","name":"Synthetic","compId":"c","w":64,"h":36,"fps":30,"dur":1,"bg":"#000000","assets":{"a":{"id":"a","name":"a.png","kind":"image","storageKey":"store:a"},"b":{"id":"b","name":"b.png","kind":"image","storageKey":"store:b"}},"layers":[],"comps":{}}"##;

/// One stored object for `key`; `proxy_only` records the first launch's verdict.
fn store_object(store: &Path, key: &str, bytes: &[u8]) {
    let hash = format!("{:x}", Sha256::digest(bytes));
    fs::create_dir_all(store.join("objects").join(&hash[..2])).unwrap();
    fs::create_dir_all(store.join("keys")).unwrap();
    fs::write(store.join("objects").join(&hash[..2]).join(&hash), bytes).unwrap();
    let record = format!(
        r#"{{"key":"{key}","hash":"{hash}","size":{},"type":"image/png","revision":"r-{key}","at":1,"source":"indexeddb"}}"#,
        bytes.len()
    );
    let name = format!("{:x}.json", Sha256::digest(key.as_bytes()));
    fs::write(store.join("keys").join(name), record).unwrap();
}

fn profile(dir: &common::TempDir) -> std::path::PathBuf {
    let profile = dir.path("native");
    let store = profile.join("Media Store");
    store_object(&store, "store:a", b"synthetic proxy bytes");
    store_object(&store, "store:b", b"synthetic original bytes");
    // The pass's verdict for store:a: a transformed payload without proxy flags.
    fs::write(
        store.join("migration.json"),
        r#"{"version":1,"passes":{"p":{"phase":"complete","closure":[{"reference":{"key":"store:a"},"status":"proxy-only","original":null,"stored":{"key":"store:a"},"exportWarning":true},{"reference":{"key":"store:b"},"status":"verified-original","original":null,"stored":{"key":"store:b"},"exportWarning":false}]}}}"#,
    )
    .unwrap();
    profile
}

fn kernel(profile: &Path) -> Kernel {
    let mut k = Kernel::new(KernelConfig {
        journal_root: Some(profile.join("journals")),
        ..Default::default()
    });
    k.shell(ShellCall::AttachLibrary {
        profile: profile.into(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .unwrap();
    k
}

fn open_library(k: &mut Kernel, profile: &Path) -> SessionHandle {
    match k
        .shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::Library {
                profile: profile.into(),
                project: "P".into(),
            },
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_proxy_only_payload_stays_a_proxy_and_never_enters_a_file() {
    let dir = common::TempDir::new("proxy-only-store");
    let profile = profile(&dir);
    let mut k = kernel(&profile);
    match k
        .shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::New(parse(PROJECT).unwrap()),
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::Opened { session, .. } => {
            let media = k.project("P").unwrap().store().media();
            assert!(media.is_proxy_only("a"), "the first launch's verdict holds");
            assert!(media.get("b").is_some() && !media.is_proxy_only("b"));
            let file = dir.path("copy.pmv");
            k.shell(ShellCall::SaveAs {
                session,
                path: file.clone(),
                replace: false,
            })
            .unwrap();
            // Embedded, the proxy would reopen as the asset's own bytes.
            let saved = common::open_store(&file);
            assert!(saved.media().get("a").is_none());
            assert!(saved.media().get("b").is_some());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn media_only_undo_can_restore_travels_with_save_as() {
    let dir = common::TempDir::new("undo-only-store");
    let profile = profile(&dir);
    {
        let mut k = kernel(&profile);
        k.shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::New(parse(PROJECT).unwrap()),
            electron_profile: None,
        })
        .unwrap();
        let ack = k
            .handle(
                "P",
                Request::new(
                    Caller::native(),
                    Call::Transact {
                        label: Some("Remove".into()),
                        body: WriteBody::Commands(vec![Command::Media(
                            MediaCommand::RemoveAsset {
                                asset_id: "b".into(),
                            },
                        )]),
                        expected_revision: None,
                    },
                ),
            )
            .unwrap()
            .reply
            .unwrap();
        assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
        let session = k.session_metadata("P").unwrap().handle;
        k.shell(ShellCall::Close(session)).unwrap();
    }
    let mut k = kernel(&profile);
    let session = open_library(&mut k, &profile);
    let store = k.project("P").unwrap().store();
    assert!(
        store
            .root()
            .unwrap()
            .get("assets")
            .unwrap()
            .get("b")
            .is_none()
    );
    assert!(
        store.media().get("b").is_some(),
        "Undo can still bring b back"
    );
    let file = dir.path("portable.pmv");
    k.shell(ShellCall::SaveAs {
        session,
        path: file.clone(),
        replace: false,
    })
    .unwrap();
    assert!(common::open_store(&file).media().get("b").is_some());
}
