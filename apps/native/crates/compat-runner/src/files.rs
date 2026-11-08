//! Files an extension holds that live on disk: ones the user picked
//! (`assets.pick`), and bytes written out for `assets.import`. The realm only
//! ever sees an opaque token, never a path it could choose.
use crate::{Error, Result};
use std::{
    collections::BTreeMap,
    hash::{BuildHasher, RandomState},
    path::{Path, PathBuf},
    sync::Mutex,
};

struct Held {
    path: PathBuf,
    /// Written here, so removed with its token; a picked file is the user's.
    owned: bool,
}
static FILES: Mutex<BTreeMap<u64, Held>> = Mutex::new(BTreeMap::new());

fn token() -> u64 {
    RandomState::new().hash_one(std::time::Instant::now()) | 1
}

/// A token for a file the user chose in an open panel.
pub fn hold_picked(path: PathBuf) -> u64 {
    let token = token();
    FILES
        .lock()
        .unwrap()
        .insert(token, Held { path, owned: false });
    token
}

/// The file behind a token, if it is still held.
pub fn path(token: u64) -> Option<PathBuf> {
    FILES
        .lock()
        .unwrap()
        .get(&token)
        .map(|held| held.path.clone())
}

/// Forgets a token; a file written for it goes with it.
pub fn release(token: u64) {
    if let Some(held) = FILES.lock().unwrap().remove(&token)
        && held.owned
        && let Some(folder) = held.path.parent()
    {
        let _ = std::fs::remove_dir_all(folder);
    }
}

/// Forgets a token only if this module wrote its file; a picked file stays
/// available to import again.
pub fn release_staged(token: u64) {
    let owned = FILES
        .lock()
        .unwrap()
        .get(&token)
        .is_some_and(|held| held.owned);
    if owned {
        release(token);
    }
}

/// A file name that stays one path component and keeps its extension.
fn file_name(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '\0') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim_start_matches('.').trim();
    if name.is_empty() {
        "Untitled".into()
    } else {
        name.chars().take(200).collect()
    }
}

/// Writes an extension's bytes under their name, for the app's import.
pub(crate) fn stage(bytes: &[u8], name: &str) -> Result<u64> {
    let token = token();
    let folder = std::env::temp_dir()
        .join("powermove-extension-files")
        .join(format!("{token:016x}"));
    let write = |folder: &Path| -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(folder)?;
        let path = folder.join(file_name(name));
        std::fs::write(&path, bytes)?;
        Ok(path)
    };
    let path = write(&folder).map_err(|e| Error::Host(format!("Could not write {name}: {e}")))?;
    FILES
        .lock()
        .unwrap()
        .insert(token, Held { path, owned: true });
    Ok(token)
}

/// A held file's bytes, refusing more than `max`.
pub(crate) fn read(token: u64, max: usize) -> Result<Vec<u8>> {
    let path = path(token).ok_or(Error::Host("The file is no longer available".into()))?;
    let size = std::fs::metadata(&path)
        .map_err(|e| Error::Host(e.to_string()))?
        .len();
    if size > max as u64 {
        return Err(Error::Quota("file read"));
    }
    std::fs::read(&path).map_err(|e| Error::Host(e.to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn staged_names_stay_one_component_and_release_removes_them() {
        assert_eq!(super::file_name("../a/b.png"), "_a_b.png");
        assert_eq!(super::file_name(".."), "Untitled");
        let token = super::stage(b"x", "logo.svg").unwrap();
        let path = super::path(token).unwrap();
        assert_eq!(path.file_name().unwrap(), "logo.svg");
        assert_eq!(super::read(token, 1).unwrap(), b"x");
        assert!(super::read(token, 0).is_err());
        super::release(token);
        assert!(!path.exists() && super::path(token).is_none());
    }
}
