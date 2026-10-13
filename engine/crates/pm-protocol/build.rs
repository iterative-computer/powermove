//! Computes the protocol source hash: the build id peers compare when they
//! negotiate draft contracts (contract 9). Two peers built from the same
//! `pm-protocol` sources speak the same drafts, whatever else changed.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=src");
    let root = Path::new("src");
    let mut files = Vec::new();
    collect(root, &mut files);
    let mut hasher = Sha256::new();
    for file in &files {
        let relative = file.strip_prefix(root).expect("under src");
        let name = relative.to_string_lossy().replace('\\', "/");
        let bytes = std::fs::read(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    let digest = hasher.finalize();
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    println!("cargo:rustc-env=PM_PROTOCOL_SOURCE_HASH={hex}");
}
