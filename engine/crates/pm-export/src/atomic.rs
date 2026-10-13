//! Atomic output through a temporary file in the destination folder (section 17).
//!
//! The encoder writes `.<name>.<random>.partial.<ext>` next to the destination, so the
//! final move is a rename on the same volume. Nothing appears at the destination
//! until the file is complete and verified; a failed or cancelled export removes the
//! temporary file. With [`Overwrite::Fail`] the move is no-clobber (a hard link, then
//! unlinking the temporary name), so an existing file is never replaced.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::Error;
use crate::settings::Overwrite;

/// A temporary output file that becomes the destination on [`AtomicOutput::commit`].
#[derive(Debug)]
pub struct AtomicOutput {
    destination: PathBuf,
    temp: PathBuf,
    overwrite: Overwrite,
    mode: Option<u32>,
    committed: bool,
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mixed = crate::dither::hash_u32(std::process::id() ^ nanos as u32) as u64
        | (crate::dither::hash_u32((nanos >> 32) as u32 ^ 0x9e37_79b9) as u64) << 32;
    format!("{mixed:016x}")
}

impl AtomicOutput {
    /// Prepares a temporary path next to `destination`. Fails when the folder does not
    /// exist or, with [`Overwrite::Fail`], when the destination already exists.
    pub fn new(destination: &Path, overwrite: Overwrite, mode: Option<u32>) -> Result<Self, Error> {
        let name = destination
            .file_name()
            .ok_or_else(|| Error::InvalidSpec("destination has no file name".into()))?
            .to_string_lossy()
            .into_owned();
        let folder = match destination.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        if !folder.is_dir() {
            return Err(Error::Io(format!(
                "destination folder {} does not exist",
                folder.display()
            )));
        }
        if overwrite == Overwrite::Fail && fs::symlink_metadata(destination).is_ok() {
            return Err(Error::Exists(destination.to_path_buf()));
        }
        let (stem, ext) = match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
            _ => (name.clone(), String::new()),
        };
        let temp = folder.join(format!(".{stem}.{}.partial{ext}", unique_suffix()));
        Ok(Self {
            destination: destination.to_path_buf(),
            temp,
            overwrite,
            mode,
            committed: false,
        })
    }

    /// Where the encoder writes.
    pub fn temp_path(&self) -> &Path {
        &self.temp
    }

    pub fn destination(&self) -> &Path {
        &self.destination
    }

    /// Moves the finished temporary file into place and returns the destination.
    pub fn commit(mut self) -> Result<PathBuf, Error> {
        let meta = fs::metadata(&self.temp)
            .map_err(|e| Error::Io(format!("{}: {e}", self.temp.display())))?;
        if meta.len() == 0 {
            return Err(Error::Io("the encoder wrote an empty file".into()));
        }
        if let Some(mode) = self.mode {
            set_mode(&self.temp, mode)?;
        }
        match self.overwrite {
            Overwrite::Replace => fs::rename(&self.temp, &self.destination).map_err(|e| {
                Error::Io(format!("moving into {}: {e}", self.destination.display()))
            })?,
            Overwrite::Fail => {
                match fs::hard_link(&self.temp, &self.destination) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        return Err(Error::Exists(self.destination.clone()));
                    }
                    Err(e) => {
                        return Err(Error::Io(format!(
                            "moving into {}: {e}",
                            self.destination.display()
                        )));
                    }
                }
                let _ = fs::remove_file(&self.temp);
            }
        }
        self.committed = true;
        remove_sidecars(&self.temp);
        Ok(self.destination.clone())
    }
}

impl Drop for AtomicOutput {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.temp);
            remove_sidecars(&self.temp);
        }
    }
}

/// Removes files named `<temp file name>.<anything>`: AVAssetWriter keeps a
/// `.sb-<uuid>` work file next to an MP4 it optimizes for network use, and leaves it
/// behind when writing is cancelled.
fn remove_sidecars(temp: &Path) {
    let (Some(folder), Some(name)) = (temp.parent(), temp.file_name()) else {
        return;
    };
    let prefix = format!("{}.", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| Error::Io(format!("{}: {e}", path.display())))
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> Result<(), Error> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pm-export-atomic-{name}-{}", unique_suffix()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn temp_file_lives_next_to_the_destination_and_vanishes_on_drop() {
        let dir = scratch("drop");
        let dest = dir.join("Clip.mov");
        let out = AtomicOutput::new(&dest, Overwrite::Fail, None).unwrap();
        let temp = out.temp_path().to_path_buf();
        assert_eq!(temp.parent(), Some(dir.as_path()));
        let name = temp.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with(".Clip.") && name.ends_with(".partial.mov"),
            "{name}"
        );
        fs::write(&temp, b"partial").unwrap();
        let sidecar = dir.join(format!("{name}.sb-1234"));
        fs::write(&sidecar, b"work").unwrap();
        fs::write(dir.join("Unrelated.mov"), b"keep").unwrap();
        drop(out);
        assert!(!temp.exists() && !dest.exists() && !sidecar.exists());
        assert!(dir.join("Unrelated.mov").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn commit_moves_into_place_with_the_requested_mode() {
        let dir = scratch("commit");
        let dest = dir.join("Agent.mp4");
        let out = AtomicOutput::new(&dest, Overwrite::Fail, Some(0o600)).unwrap();
        fs::write(out.temp_path(), b"movie").unwrap();
        let temp = out.temp_path().to_path_buf();
        assert_eq!(out.commit().unwrap(), dest);
        assert_eq!(fs::read(&dest).unwrap(), b"movie");
        assert!(!temp.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fail_policy_never_clobbers_and_replace_does() {
        let dir = scratch("clobber");
        let dest = dir.join("Out.mov");
        fs::write(&dest, b"old").unwrap();
        assert!(matches!(
            AtomicOutput::new(&dest, Overwrite::Fail, None),
            Err(Error::Exists(_))
        ));
        // A file that appears while the export runs is not replaced either.
        fs::remove_file(&dest).unwrap();
        let out = AtomicOutput::new(&dest, Overwrite::Fail, None).unwrap();
        fs::write(out.temp_path(), b"new").unwrap();
        fs::write(&dest, b"raced").unwrap();
        let temp = out.temp_path().to_path_buf();
        assert!(matches!(out.commit(), Err(Error::Exists(_))));
        assert_eq!(fs::read(&dest).unwrap(), b"raced");
        assert!(!temp.exists(), "the failed commit drops the temporary file");
        let out = AtomicOutput::new(&dest, Overwrite::Replace, None).unwrap();
        fs::write(out.temp_path(), b"new").unwrap();
        out.commit().unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_output_and_missing_folders_fail() {
        let dir = scratch("empty");
        let out = AtomicOutput::new(&dir.join("E.mov"), Overwrite::Fail, None).unwrap();
        fs::write(out.temp_path(), b"").unwrap();
        assert!(out.commit().is_err());
        assert!(!dir.join("E.mov").exists());
        assert!(AtomicOutput::new(&dir.join("missing/E.mov"), Overwrite::Fail, None).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
