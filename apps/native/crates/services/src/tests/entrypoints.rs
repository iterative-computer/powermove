use crate::entrypoints;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct EntryDouble {
    states: Vec<String>,
    files: Vec<PathBuf>,
    associations: Vec<String>,
    fail: bool,
}
impl entrypoints::EntryHook for EntryDouble {
    fn auth(&mut self, link: &entrypoints::AuthLink) -> Result<()> {
        if self.fail {
            bail!("auth owner unavailable")
        }
        self.states.push(link.state().into());
        assert!(!link.token().is_empty());
        Ok(())
    }
    fn open_project(&mut self, path: &Path) -> Result<()> {
        if self.fail {
            bail!("kernel unavailable")
        }
        self.files.push(path.to_path_buf());
        Ok(())
    }
    fn register_associations(&mut self, extension: &str, scheme: &str) -> Result<()> {
        self.associations.push(format!("{extension}:{scheme}"));
        Ok(())
    }
}
#[test]
fn cold_auth_queue_keeps_eight_newest_and_delivers_in_order() {
    let mut queue = entrypoints::EntryQueue::default();
    for index in 0..9 {
        let admission = queue
            .push_link(&format!(
                "powermove://auth?state=synthetic-{index}&token=synthetic-callback"
            ))
            .unwrap();
        assert_eq!(
            admission,
            if index == 8 {
                entrypoints::LinkAdmission::ReplacedOldestAuth
            } else {
                entrypoints::LinkAdmission::Queued
            }
        );
    }
    assert_eq!(queue.pending(), (8, 0));
    let mut hook = EntryDouble::default();
    queue.drain(&mut hook).unwrap();
    assert_eq!(
        hook.states,
        (1..9).map(|n| format!("synthetic-{n}")).collect::<Vec<_>>()
    );
    assert_eq!(queue.pending(), (0, 0));
}
#[test]
fn auth_parser_rejects_confused_targets_and_duplicate_fields_without_echo() {
    for raw in [
        "https://auth?state=a&token=b",
        "powermove://other?state=a&token=b",
        "powermove://auth/path?state=a&token=b",
        "powermove://user@auth?state=a&token=b",
        "powermove://auth?state=a&state=b&token=c",
        "powermove://auth?state=a",
        "powermove://auth?state=a&token=%00",
    ] {
        let error = entrypoints::AuthLink::parse(raw).err().unwrap().to_string();
        assert!(!error.contains("token="));
    }
}
#[test]
fn failed_entry_delivery_keeps_pending_files_and_links_for_retry() {
    let mut queue = entrypoints::EntryQueue::default();
    queue
        .push_link("powermove://auth?state=synthetic&token=synthetic-callback")
        .unwrap();
    queue.push_file("/synthetic/Project.PMV".into()).unwrap();
    queue.push_file("/synthetic/Project.PMV".into()).unwrap();
    let mut hook = EntryDouble {
        fail: true,
        ..Default::default()
    };
    assert!(queue.drain(&mut hook).is_err());
    assert_eq!(queue.pending(), (1, 1));
    hook.fail = false;
    queue.drain(&mut hook).unwrap();
    assert_eq!(hook.files, [PathBuf::from("/synthetic/Project.PMV")]);
    assert!(queue.push_file("relative.pmv".into()).is_err());
    assert!(
        queue
            .push_file("/synthetic/not-project.mov".into())
            .is_err()
    );
}
#[test]
fn dev_build_does_not_claim_file_associations() {
    let mut hook = EntryDouble::default();
    assert!(!entrypoints::EntryQueue::register(false, false, &mut hook).unwrap());
    assert!(hook.associations.is_empty());
    assert!(entrypoints::EntryQueue::register(true, false, &mut hook).unwrap());
    assert_eq!(hook.associations, ["pmv:powermove"]);
}

#[test]
fn auth_failure_does_not_block_independent_project_opens() {
    struct AuthUnavailable {
        files: usize,
    }
    impl entrypoints::EntryHook for AuthUnavailable {
        fn auth(&mut self, _: &entrypoints::AuthLink) -> Result<()> {
            bail!("auth unavailable")
        }
        fn open_project(&mut self, _: &Path) -> Result<()> {
            self.files += 1;
            Ok(())
        }
        fn register_associations(&mut self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
    }
    let mut queue = entrypoints::EntryQueue::default();
    queue
        .push_link("powermove://auth?state=synthetic&token=synthetic-callback")
        .unwrap();
    queue.push_file("/synthetic/Project.pmv".into()).unwrap();
    let mut hook = AuthUnavailable { files: 0 };
    assert!(queue.drain(&mut hook).is_err());
    assert_eq!(hook.files, 1);
    assert_eq!(queue.pending(), (1, 0));
}
