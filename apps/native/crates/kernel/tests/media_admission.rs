mod common;
use common::TempDir;
use pm_kernel::{
    Ack, Call, Caller, HistoryCall, Kernel, KernelConfig, MediaAdmission, MediaBinding, OpenSource,
    Reply, Request,
    media_archive::{Archive, Original},
    project_io::{MediaEntry, MediaSource},
};
use pm_model::{Value, json::parse};
use std::sync::{Arc, atomic::AtomicBool};
const SOURCE: &str = r#"{"id":"P","name":"Synthetic","w":320,"h":180,"fps":30,"dur":2,"layers":[],"comps":{},"assets":{"asset":{"id":"asset","name":"Old","kind":"image","mimeType":"application/octet-stream","folder":"kept","future":"\ud800"}}}"#;
struct Fixture {
    kernel: Kernel,
    archive: Archive,
    dir: TempDir,
    next: usize,
}
fn kernel(dir: &TempDir) -> Kernel {
    let mut kernel = Kernel::new(KernelConfig {
        journal_root: Some(dir.path("journals")),
        ..Default::default()
    });
    kernel
        .shell(pm_kernel::session::ShellCall::AttachLibrary {
            profile: dir.path("profile"),
            format: pm_kernel::persistence::DiskFormat::V1_2,
            electron_profile: None,
        })
        .unwrap();
    kernel
}
impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new("media-admission");
        let mut kernel = kernel(&dir);
        kernel
            .open("P", OpenSource::Untitled(parse(SOURCE).unwrap()))
            .unwrap();
        kernel
            .project_mut("P")
            .unwrap()
            .store_mut()
            .media_mut()
            .insert(MediaEntry {
                id: "asset".into(),
                media_type: "application/octet-stream".into(),
                revision: "legacy".into(),
                source: MediaSource::Memory(Arc::from(b"old original".as_slice())),
            });
        let archive = Archive::open(&dir.path("profile")).unwrap();
        Self {
            kernel,
            archive,
            dir,
            next: 0,
        }
    }
    fn document(&self) -> Value {
        self.kernel
            .project("P")
            .unwrap()
            .store()
            .root()
            .unwrap()
            .to_value()
    }
    fn call(&mut self, call: impl Into<Call>) -> Ack {
        let doc = self.kernel.project("P").unwrap().store().doc_id();
        self.kernel
            .handle("P", Request::new(Caller::native(), call).with_document(doc))
            .unwrap()
            .reply
            .unwrap()
    }
    fn original(&mut self, bytes: &[u8]) -> Original {
        self.next += 1;
        let path = self.dir.path(&format!("source-{}", self.next));
        std::fs::write(&path, bytes).unwrap();
        let receipt = self
            .archive
            .retain_path(&path, "application/octet-stream", &AtomicBool::new(false))
            .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        receipt
    }
    fn request(&mut self, original: Original) -> Request {
        let store = self.kernel.project("P").unwrap().store();
        let previous = store
            .media()
            .get("asset")
            .map(|entry| self.archive.retain_entry(entry).unwrap());
        let expected = self.document().get("assets").unwrap().get("asset").cloned();
        let mut project = self.document();
        let meta = project
            .as_object_mut()
            .unwrap()
            .get_mut("assets")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .get_mut("asset")
            .unwrap()
            .as_object_mut()
            .unwrap();
        meta.insert("originalVersion", Value::from(original.version()));
        meta.insert("name", Value::from("Replacement"));
        Request::new(
            Caller::native(),
            Call::AdmitMedia(MediaAdmission {
                expected_revision: store.revision(),
                project,
                label: "Replace Media".into(),
                bindings: vec![MediaBinding {
                    asset: "asset".into(),
                    expected,
                    original,
                    previous,
                }],
            }),
        )
        .with_document(store.doc_id())
    }
    fn replace(&mut self, bytes: &[u8]) -> Ack {
        let original = self.original(bytes);
        let request = self.request(original);
        self.kernel.handle("P", request).unwrap().reply.unwrap()
    }
    fn bytes(&mut self) -> Vec<u8> {
        self.next += 1;
        let path = self.dir.path(&format!("read-{}", self.next));
        self.kernel
            .project("P")
            .unwrap()
            .store()
            .media()
            .get("asset")
            .expect("selected original capability")
            .copy_to(&path)
            .unwrap();
        std::fs::read(path).unwrap()
    }
}
#[test]
fn actual_byte_versions_follow_one_undo_redo_and_keep_opaque_metadata() {
    let mut f = Fixture::new();
    let ack = f.replace(b"new original");
    assert!(
        matches!(ack.reply, Reply::Applied { changed: true, .. }),
        "{ack:?}"
    );
    assert!(ack.notice.is_none(), "{ack:?}");
    assert_eq!(f.bytes(), b"new original");
    let after = f.document();
    assert_eq!(
        after
            .get("assets")
            .unwrap()
            .get("asset")
            .unwrap()
            .get("future"),
        parse(r#""\ud800""#).ok().as_ref()
    );
    assert_eq!(f.kernel.project("P").unwrap().store().history().len(), 1);
    f.call(HistoryCall::Undo);
    assert_eq!(f.bytes(), b"old original");
    assert!(!f.kernel.project("P").unwrap().store().is_dirty());
    assert!(
        f.kernel
            .project("P")
            .unwrap()
            .store()
            .has_pending_metadata()
    );
    assert!(f.document().get("nativeMediaVersions").is_some());
    f.call(HistoryCall::Redo);
    assert_eq!(f.bytes(), b"new original");
    assert_eq!(f.document(), after);
}
#[test]
fn stale_and_open_gesture_completions_refuse_without_losing_retained_bytes() {
    let mut f = Fixture::new();
    let original = f.original(b"late original");
    let path = original.path();
    let request = f.request(original);
    f.replace(b"winner");
    let before = f.document();
    let ack = f.kernel.handle("P", request).unwrap().reply.unwrap();
    assert!(matches!(ack.reply, Reply::Throw(_)), "{ack:?}");
    assert_eq!(f.document(), before);
    assert_eq!(std::fs::read(path).unwrap(), b"late original");
    let original = f.original(b"gesture original");
    let path = original.path();
    let request = f.request(original);
    let Reply::Gesture(id) = f
        .call(pm_kernel::GestureCall::Begin {
            label: "Drag".into(),
        })
        .reply
    else {
        panic!()
    };
    let ack = f.kernel.handle("P", request).unwrap().reply.unwrap();
    assert!(matches!(ack.reply, Reply::Throw(_)), "{ack:?}");
    assert_eq!(f.document(), before);
    assert_eq!(std::fs::read(path).unwrap(), b"gesture original");
    f.call(pm_kernel::GestureCall::Cancel { gesture: id });
    assert_eq!(f.bytes(), b"winner");
}
#[test]
fn crash_recovery_before_save_restores_both_original_versions() {
    let mut f = Fixture::new();
    let base = f.kernel.base_hash("P").unwrap().to_owned();
    f.replace(b"recovered original");
    f.kernel.durability_barrier().unwrap();
    let recovered = match f
        .kernel
        .recover("P", Some(&base), Some(&parse(SOURCE).unwrap()))
    {
        pm_kernel::journal::Recovery::Recovered(r) => r,
        other => panic!("{other:?}"),
    };
    drop(f.kernel);
    let mut reopened = kernel(&f.dir);
    reopened
        .open(
            "P",
            OpenSource::Recovered {
                recovered,
                supersedes: vec![],
            },
        )
        .unwrap();
    f.kernel = reopened;
    assert_eq!(f.bytes(), b"recovered original");
    f.call(HistoryCall::Undo);
    assert_eq!(f.bytes(), b"old original");
    f.call(HistoryCall::Redo);
    assert_eq!(f.bytes(), b"recovered original");
}
#[test]
fn portable_export_reopens_without_profile_archive_and_keeps_exact_history_bytes() {
    let mut f = Fixture::new();
    f.replace(b"portable original");
    let path = f.dir.path("portable.pmv");
    f.kernel
        .project("P")
        .unwrap()
        .store()
        .prepare_portable_export(path.clone())
        .unwrap()
        .run()
        .unwrap();
    let opened = pm_kernel::open_project(&path, Default::default()).unwrap();
    assert_eq!(
        opened.file.media.len(),
        2,
        "active and old originals each written once"
    );
    let mut standalone = pm_kernel::DocumentStore::open(opened);
    std::fs::rename(
        f.dir.path("profile/media/retained"),
        f.dir.path("originals-away"),
    )
    .unwrap();
    let copy = |store: &pm_kernel::DocumentStore, name: &str| {
        let destination = f.dir.path(name);
        store
            .media()
            .get("asset")
            .unwrap()
            .copy_to(&destination)
            .unwrap();
        std::fs::read(destination).unwrap()
    };
    assert_eq!(copy(&standalone, "new-read"), b"portable original");
    standalone.undo().unwrap();
    assert_eq!(copy(&standalone, "old-read"), b"old original");
    standalone.redo().unwrap();
    assert_eq!(copy(&standalone, "redo-read"), b"portable original");
}
#[test]
fn foreign_profile_receipts_refuse_and_keep_both_archive_copies() {
    let mut f = Fixture::new();
    let foreign = f.dir.path("foreign");
    std::fs::create_dir(&foreign).unwrap();
    let source = f.dir.path("foreign-source");
    std::fs::write(&source, b"foreign bytes").unwrap();
    let original = Archive::open(&foreign)
        .unwrap()
        .retain_path(&source, "application/octet-stream", &AtomicBool::new(false))
        .unwrap();
    let path = original.path();
    let before = f.document();
    let request = f.request(original);
    assert!(f.kernel.handle("P", request).is_err());
    assert_eq!(f.document(), before);
    assert_eq!(std::fs::read(path).unwrap(), b"foreign bytes");
    assert_eq!(f.bytes(), b"old original");
}

#[test]
fn note_metadata_written_during_retention_survives_admission_and_composition_restore() {
    let mut f = Fixture::new();
    let original = f.original(b"next original");
    let request = f.request(original);
    let ack = f.call(pm_kernel::MemoryCall::Mutate {
        expected: None,
        operation: pm_model::memory::Operation::Add {
            kind: "decision".into(),
            text: "Keep this note".into(),
            why: None,
            resolved: false,
        },
        now: 1000.,
        id: "kept-note".into(),
    });
    assert!(matches!(ack.reply, Reply::Memory { .. }));
    let memory = f.document().get("memory").cloned();
    let ack = f.kernel.handle("P", request).unwrap().reply.unwrap();
    assert!(
        matches!(ack.reply, Reply::Applied { changed: true, .. }),
        "{ack:?}"
    );
    assert_eq!(f.document().get("memory"), memory.as_ref());
    assert_eq!(f.bytes(), b"next original");
    let store = f.kernel.project("P").unwrap().store();
    let doc = store.doc_id();
    let revision = store.revision();
    f.call(Call::Restore {
        project: parse(SOURCE).unwrap(),
        label: "Restore".into(),
        expected_revision: revision,
        expected_document: doc,
    });
    assert_eq!(f.bytes(), b"old original");
    assert_eq!(f.document().get("memory"), memory.as_ref());
    assert!(f.document().get("nativeMediaVersions").is_some());
    f.call(HistoryCall::Undo);
    assert_eq!(f.bytes(), b"next original");
    assert_eq!(f.document().get("memory"), memory.as_ref());
}

#[test]
fn invalid_asset_selection_and_proxy_fallbacks_never_admit_original_receipts() {
    let mut f = Fixture::new();
    let before = f.document();
    let history = f.kernel.project("P").unwrap().store().history().len();
    for proxy in [false, true] {
        let original = f.original(b"unadmitted original");
        let path = original.path();
        let mut request = f.request(original);
        let Call::AdmitMedia(admission) = &mut request.call else {
            panic!()
        };
        let asset = admission
            .project
            .as_object_mut()
            .unwrap()
            .get_mut("assets")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .get_mut("asset")
            .unwrap()
            .as_object_mut()
            .unwrap();
        if proxy {
            asset.insert("playbackProxy", Value::Bool(true));
        } else {
            asset.insert("originalVersion", Value::from("unsubstantiated"));
        }
        let ack = f.kernel.handle("P", request).unwrap().reply.unwrap();
        assert!(matches!(ack.reply, Reply::Throw(_)), "{ack:?}");
        assert_eq!(f.document(), before);
        assert_eq!(
            f.kernel.project("P").unwrap().store().history().len(),
            history
        );
        assert_eq!(std::fs::read(path).unwrap(), b"unadmitted original");
        assert_eq!(f.bytes(), b"old original");
    }
}

#[test]
fn ordinary_save_during_retention_does_not_stale_identical_bytes_and_legacy_undo_exports_once() {
    let mut f = Fixture::new();
    let original = f.original(b"replacement after save");
    let request = f.request(original);
    let working = f.dir.path("working.pmv");
    let plan = f
        .kernel
        .project_mut("P")
        .unwrap()
        .store_mut()
        .prepare_save(
            pm_kernel::SaveTarget::SaveAs {
                path: working,
                replace: false,
            },
            Default::default(),
        )
        .unwrap();
    let receipt = plan.run().unwrap();
    f.kernel
        .project_mut("P")
        .unwrap()
        .store_mut()
        .finish_save(receipt)
        .unwrap();
    let ack = f.kernel.handle("P", request).unwrap().reply.unwrap();
    assert!(
        matches!(ack.reply, Reply::Applied { changed: true, .. }),
        "{ack:?}"
    );
    assert_eq!(f.bytes(), b"replacement after save");
    f.call(HistoryCall::Undo);
    assert_eq!(f.bytes(), b"old original");
    let path = f.dir.path("legacy-state-portable.pmv");
    f.kernel
        .project("P")
        .unwrap()
        .store()
        .prepare_portable_export(path.clone())
        .unwrap()
        .run()
        .unwrap();
    let opened = pm_kernel::open_project(&path, Default::default()).unwrap();
    assert_eq!(
        opened.file.media.len(),
        2,
        "each actual original is included once, even when the current legacy descriptor predates originalVersion"
    );
    let container = pm_model::container::read_container(
        &std::fs::File::open(&path).unwrap(),
        Default::default(),
    )
    .unwrap();
    assert_eq!(
        container
            .document
            .get("requiresEngine")
            .and_then(Value::as_str),
        Some("native")
    );
    let mut reopened = pm_kernel::DocumentStore::open(opened);
    std::fs::rename(
        f.dir.path("profile/media/retained"),
        f.dir.path("archive-away-after-save"),
    )
    .unwrap();
    let read = |store: &pm_kernel::DocumentStore, name: &str| {
        let path = f.dir.path(name);
        store.media().get("asset").unwrap().copy_to(&path).unwrap();
        std::fs::read(path).unwrap()
    };
    assert_eq!(read(&reopened, "old-portable"), b"old original");
    reopened.redo().unwrap();
    assert_eq!(read(&reopened, "new-portable"), b"replacement after save");
}
