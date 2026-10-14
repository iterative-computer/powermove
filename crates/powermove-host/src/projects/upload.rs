//! Staged uploads: `file:save-*` (port of `main/file-upload.ts`) and the
//! bridge's `web:upload-*` files (from `server/index.ts`).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, bail};

use super::durable::{read_at, write_all_at};
use crate::ids::FILE_CHUNK_BYTES;

/// One acknowledged disk write at a time; project size never becomes a RAM budget.
pub struct FileUpload {
    pub size: u64,
    received: u64,
    directory: PathBuf,
    file: Option<File>,
    claimed: bool,
    failed: bool,
}

impl FileUpload {
    pub fn create(size: u64, root: &Path) -> Result<Self> {
        if size < 1 {
            bail!("Invalid save size");
        }
        fs::create_dir_all(root)?;
        let directory = root.join(format!("powermove-save-upload-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir(&directory)?;
        match OpenOptions::new().read(true).write(true).create_new(true).open(directory.join("data")) {
            Ok(file) => Ok(Self { size, received: 0, directory, file: Some(file), claimed: false, failed: false }),
            Err(error) => {
                let _ = fs::remove_dir_all(&directory);
                Err(error.into())
            }
        }
    }

    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        let Some(file) = self.file.as_ref().filter(|_| !self.claimed && !self.failed) else { bail!("Invalid save chunk") };
        if data.is_empty() || data.len() > FILE_CHUNK_BYTES || data.len() as u64 > self.size - self.received {
            bail!("Invalid save chunk");
        }
        if let Err(error) = write_all_at(file, data, self.received) {
            self.failed = true;
            return Err(error);
        }
        self.received += data.len() as u64;
        Ok(())
    }

    /// Claim only complete uploads. An incomplete finish cannot consume the upload.
    pub fn claim(&mut self) -> Result<()> {
        if self.file.is_none() || self.claimed || self.failed || self.received != self.size {
            bail!("The save upload is incomplete.");
        }
        self.claimed = true;
        Ok(())
    }

    /// The claimed bytes, start to end.
    pub fn reader(&self) -> Result<UploadReader<'_>> {
        match self.file.as_ref().filter(|_| self.claimed) {
            Some(file) => Ok(UploadReader { file, offset: 0, size: self.size }),
            None => bail!("The save upload is incomplete."),
        }
    }
}

impl Drop for FileUpload {
    fn drop(&mut self) {
        self.file = None;
        let _ = fs::remove_dir_all(&self.directory);
    }
}

pub struct UploadReader<'a> {
    file: &'a File,
    offset: u64,
    size: u64,
}

impl Read for UploadReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let want = (self.size - self.offset).min(buffer.len() as u64) as usize;
        if want == 0 {
            return Ok(0);
        }
        let read = read_at(self.file, &mut buffer[..want], self.offset)?;
        if read == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "The save upload is truncated."));
        }
        self.offset += read as u64;
        Ok(read)
    }
}

/// A browser file uploaded to `<root>/<uuid>/<name>`; chunks arrive out of order.
pub struct WebUpload {
    pub owner: u64,
    pub path: PathBuf,
    pub size: u64,
    file: File,
    received: AtomicU64,
}

pub const MAX_WEB_UPLOAD_BYTES: u64 = 4 * 1024 * 1024 * 1024;

impl WebUpload {
    pub fn create(owner: u64, root: &Path, id: &str, name: &str, size: u64) -> Result<Self> {
        let directory = root.join(id);
        fs::create_dir_all(&directory)?;
        let path = directory.join(name);
        let file = OpenOptions::new().write(true).create_new(true).open(&path)?;
        Ok(Self { owner, path, size, file, received: AtomicU64::new(0) })
    }

    pub fn write(&self, offset: u64, data: &[u8]) -> Result<()> {
        if data.is_empty() || data.len() > FILE_CHUNK_BYTES || !offset.is_multiple_of(FILE_CHUNK_BYTES as u64) || offset + data.len() as u64 > self.size {
            bail!("upload: invalid chunk");
        }
        write_all_at(&self.file, data, offset)?;
        self.received.fetch_add(data.len() as u64, Ordering::AcqRel);
        Ok(())
    }

    pub fn complete(&self) -> bool {
        self.received.load(Ordering::Acquire) == self.size
    }

    /// Remove the upload's folder (the file handle may still be open; Windows
    /// deletes it once the last handle closes).
    pub fn remove(&self) {
        if let Some(directory) = self.path.parent() {
            let _ = fs::remove_dir_all(directory);
        }
    }
}

/// `safeFileName` from server/index.ts, plus the characters Windows forbids.
pub fn safe_file_name(value: &str) -> Option<String> {
    if value.is_empty() || value.encode_utf16().count() > 255 || value.contains('\0') {
        return None;
    }
    let replaced: String = value.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c }).collect();
    let mut name = replaced.trim_end_matches(['.', ' ']).to_owned();
    if name.is_empty() {
        return None;
    }
    let stem = name.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        name.insert(0, '_');
    }
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_windows_safe_upload_names() {
        assert_eq!(safe_file_name("a/b\\c.pmv").as_deref(), Some("a_b_c.pmv"));
        assert_eq!(safe_file_name("..").as_deref(), None);
        assert_eq!(safe_file_name("con.txt").as_deref(), Some("_con.txt"));
        assert_eq!(safe_file_name("what?.mp4").as_deref(), Some("what_.mp4"));
    }
}
