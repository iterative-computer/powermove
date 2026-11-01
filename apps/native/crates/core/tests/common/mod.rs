//! Shared helpers for the integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use pm_mac_core::project_io::{OpenOptions, SaveOptions, SaveTarget, open_project};
use pm_mac_core::{DocumentStore, ProjectIoError, SaveReceipt};

/// A fresh directory under the system temp dir, removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "pm-mac-core-{tag}-{}-{nanos}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Every entry name, sorted (to spot leftover temp files).
    pub fn entries(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn ts_fixture(name: &str) -> PathBuf {
    crate_dir().join("tests/fixtures/ts-writer").join(name)
}

pub fn engine_fixture(rel: &str) -> PathBuf {
    crate_dir()
        .join("../../../../engine/crates/pm-model/tests/fixtures")
        .join(rel)
}

pub fn open_store(path: &Path) -> DocumentStore {
    DocumentStore::open(open_project(path, OpenOptions::default()).expect("opens"))
}

/// prepare → run → finish, as the app does across threads.
pub fn save(
    store: &mut DocumentStore,
    target: SaveTarget,
    options: SaveOptions,
) -> Result<(), ProjectIoError> {
    let receipt = run_save(store, target, options)?;
    store.finish_save(receipt)
}

pub fn run_save(
    store: &mut DocumentStore,
    target: SaveTarget,
    options: SaveOptions,
) -> Result<SaveReceipt, ProjectIoError> {
    let plan = store.prepare_save(target, options)?;
    std::thread::spawn(move || plan.run()).join().unwrap()
}

pub fn save_as(store: &mut DocumentStore, path: &Path) -> Result<(), ProjectIoError> {
    save(
        store,
        SaveTarget::SaveAs {
            path: path.to_owned(),
            replace: false,
        },
        SaveOptions::default(),
    )
}

pub fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

pub fn opacity_of(store: &DocumentStore, layer: &str) -> Option<f64> {
    let root = store.root()?;
    let loc = pm_mac_core::model::find_layer(root, layer)?;
    let node = pm_mac_core::model::layer_at(root, &loc)?;
    match node.get("p")?.get("opacity")?.get("v")?.kind() {
        pm_model::NodeKind::Number(n) => Some(*n),
        _ => None,
    }
}
