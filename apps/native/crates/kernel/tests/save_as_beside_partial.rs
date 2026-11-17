//! Data-safety pass, moments 7 and 8: a Save As killed while it wrote leaves
//! its hidden partial temporary on the destination volume (unrecorded, so it
//! is never deleted: PM-451). The next Save As to the same name succeeds with
//! it there, the saved file opens, and the partial is left as it was.
//! Synthetic fixtures only.
mod common;
use common::TempDir;
use pm_kernel::{journal, persistence::DiskFormat, session::*, *};
use pm_model::Value;
use std::fs;

fn value() -> Value {
    pm_model::json::parse(r##"{"id":"p","name":"Base","compId":"c","w":320,"h":180,"fps":30,"dur":1,"bg":"#000000","layers":[],"assets":{},"comps":{}}"##).unwrap()
}
fn kernel(root: &std::path::Path) -> Kernel {
    let mut k = Kernel::new(KernelConfig {
        journal_root: Some(root.join("journals")),
        backup_root: None,
        journal: journal::JournalConfig::default(),
        ..Default::default()
    });
    k.shell(ShellCall::AttachLibrary {
        profile: root.into(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .unwrap();
    k
}
fn session(reply: ShellReply) -> SessionHandle {
    match reply {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_save_as_succeeds_beside_a_partial_an_interrupted_one_left() {
    let root = TempDir::new("save-as-partial");
    let target = root.path("Paper-p7.pmv");
    // What a Save As killed mid-write leaves: `.<name>.<uuid>.tmp`, partial.
    let partial = root.path(".Paper-p7.pmv.4fcd1377-c02e-4592-a07a-81541068a759.tmp");
    fs::write(&partial, vec![0x50u8; 1 << 20]).unwrap();
    let mut k = kernel(&root.0);
    let s = session(
        k.shell(ShellCall::Open {
            id: "p".into(),
            source: SessionSource::New(value()),
            electron_profile: None,
        })
        .unwrap(),
    );
    k.shell(ShellCall::SaveAs {
        session: s.clone(),
        path: target.clone(),
        replace: false,
    })
    .expect("a Save As beside an earlier partial");
    // Again, replacing what it wrote.
    k.shell(ShellCall::SaveAs {
        session: s.clone(),
        path: target.clone(),
        replace: true,
    })
    .expect("a second Save As to the same name");
    k.shell(ShellCall::Close(s)).unwrap();
    // The saved file opens.
    let mut again = kernel(&root.0);
    session(
        again
            .shell(ShellCall::Open {
                id: "q".into(),
                source: SessionSource::File(target.clone()),
                electron_profile: None,
            })
            .expect("the saved file opens"),
    );
    // The partial is not ours to delete: it is as it was.
    assert_eq!(fs::read(&partial).unwrap().len(), 1 << 20);
}
