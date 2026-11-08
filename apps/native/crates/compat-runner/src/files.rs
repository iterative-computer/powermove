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
/// Disk work: call off the main thread. The table lock is not held while deleting.
pub fn release(token: u64) {
    let held = FILES.lock().unwrap().remove(&token);
    if let Some(held) = held
        && held.owned
        && let Some(folder) = held.path.parent()
    {
        let _ = std::fs::remove_dir_all(folder);
    }
}

/// Forgets a token only if this module wrote its file; a picked file stays
/// available to import again.
/// Disk work: call off the main thread.
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

/// The file type of downloaded media from its bytes, as an extension Import
/// Media knows; `None` for anything that isn't media.
pub fn media_extension(bytes: &[u8]) -> Option<&'static str> {
    let at = |offset: usize, magic: &[u8]| bytes.get(offset..offset + magic.len()) == Some(magic);
    let brand = bytes.get(8..12).unwrap_or_default();
    Some(if at(0, b"\x89PNG") {
        "png"
    } else if at(0, b"\xff\xd8\xff") {
        "jpg"
    } else if at(0, b"GIF8") {
        "gif"
    } else if at(0, b"RIFF") && at(8, b"WEBP") {
        "webp"
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        "wav"
    } else if at(0, b"II*\0") || at(0, b"MM\0*") {
        "tiff"
    } else if at(4, b"ftyp") {
        match brand {
            b"avif" | b"avis" => "avif",
            b"heic" | b"heix" | b"mif1" => "heic",
            b"qt  " => "mov",
            b"M4A " => "m4a",
            _ => "mp4",
        }
    } else if at(0, b"\x1a\x45\xdf\xa3") {
        if bytes[..bytes.len().min(64)]
            .windows(4)
            .any(|w| w == b"webm")
        {
            "webm"
        } else {
            "mkv"
        }
    } else if at(0, b"ID3") || (bytes.len() > 1 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0) {
        "mp3"
    } else if at(0, b"OggS") {
        "ogg"
    } else if at(0, b"fLaC") {
        "flac"
    } else if at(0, b"FORM") && (at(8, b"AIFF") || at(8, b"AIFC")) {
        "aiff"
    } else if at(0, b"glTF") {
        "glb"
    } else {
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]);
        let text = text.trim_start_matches('\u{feff}').trim_start();
        if text.starts_with('<') && text.contains("<svg") {
            "svg"
        } else {
            return None;
        }
    })
}

/// A downloaded file's name: the URL's last segment with the type its bytes
/// are, so a `?id=1` URL still imports as what it is.
pub fn download_name(url: &str, extension: &str) -> String {
    let segment = url::Url::parse(url)
        .ok()
        .and_then(|u| u.path_segments()?.next_back().map(str::to_owned))
        .map(|s| percent_decode(&s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Download".into());
    let stem = std::path::Path::new(&segment)
        .file_stem()
        .map_or(segment.clone(), |s| s.to_string_lossy().into_owned());
    format!("{stem}.{extension}")
}
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (
                bytes.get(i + 1).and_then(|b| hex(*b)),
                bytes.get(i + 2).and_then(|b| hex(*b)),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Writes an extension's bytes under their name, for the app's import.
pub fn stage(bytes: &[u8], name: &str) -> Result<u64> {
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
    #[test]
    fn downloads_are_named_for_what_their_bytes_are() {
        let png = b"\x89PNG\r\n\x1a\n";
        assert_eq!(super::media_extension(png), Some("png"));
        assert_eq!(super::media_extension(b"<html>nope</html>"), None);
        assert_eq!(
            super::download_name("https://cdn.example/download?id=1", "png"),
            "download.png"
        );
        assert_eq!(
            super::download_name("https://cdn.example/a/My%20Logo.svg", "png"),
            "My Logo.png"
        );
    }
}
