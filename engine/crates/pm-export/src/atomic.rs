//! Atomic output through a temporary file in the destination folder (section 17).
//!
//! The encoder writes `.<name>.<random>.partial.<ext>` next to the destination, so the
//! final move is a rename on the same volume. Nothing appears at the destination
//! until the file is complete and verified; a failed or cancelled export removes the
//! temporary file. With [`Overwrite::Fail`] the move is no-clobber (a hard link, then
//! unlinking the temporary name), so an existing file is never replaced.
//!
//! PNG sequences are folders: the frames go into a temporary folder
//! `.<name>.<random>.partial` beside the destination, which becomes the destination
//! folder on commit (a rename, which never replaces a folder that holds anything).
//! [`Overwrite::Replace`] moves each frame into an existing folder instead, replacing
//! frames of the same name and leaving every other file alone.

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
    /// The output is a folder of files (PNG sequences).
    folder: bool,
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
        if overwrite == Overwrite::InPlace {
            return Err(Error::InvalidSpec(
                "an in-place destination is opened with AtomicOutput::in_place".into(),
            ));
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
            folder: false,
        })
    }

    /// A temporary file beside `host_file`, a file host services created for the
    /// job (`Overwrite::InPlace`): the work file is `<host file>.<random>.partial.<ext>`,
    /// so a host that removes the job's temporary file can also remove anything
    /// named after it, and the commit renames the work file over the host's file.
    pub fn in_place(host_file: &Path, extension: &str, mode: Option<u32>) -> Result<Self, Error> {
        let name = host_file
            .file_name()
            .ok_or_else(|| Error::InvalidSpec("destination has no file name".into()))?
            .to_string_lossy()
            .into_owned();
        let folder = match host_file.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        if !folder.is_dir() {
            return Err(Error::Io(format!(
                "destination folder {} does not exist",
                folder.display()
            )));
        }
        let ext = if extension.is_empty() {
            String::new()
        } else {
            format!(".{extension}")
        };
        let temp = folder.join(format!("{name}.{}.partial{ext}", unique_suffix()));
        Ok(Self {
            destination: host_file.to_path_buf(),
            temp,
            overwrite: Overwrite::InPlace,
            mode,
            committed: false,
            folder: false,
        })
    }

    /// A temporary folder for a folder output (PNG sequences) at `destination`
    /// (with [`Overwrite::InPlace`], the empty folder host services created for the
    /// job). The temporary folder is created now, owner-only.
    pub fn folder(
        destination: &Path,
        overwrite: Overwrite,
        mode: Option<u32>,
    ) -> Result<Self, Error> {
        let name = destination
            .file_name()
            .ok_or_else(|| Error::InvalidSpec("destination has no folder name".into()))?
            .to_string_lossy()
            .into_owned();
        let parent = match destination.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        if !parent.is_dir() {
            return Err(Error::Io(format!(
                "destination folder {} does not exist",
                parent.display()
            )));
        }
        let existing = fs::symlink_metadata(destination).ok();
        match overwrite {
            Overwrite::Fail if existing.is_some() => {
                return Err(Error::Exists(destination.to_path_buf()));
            }
            Overwrite::InPlace if !existing.as_ref().is_some_and(|m| m.is_dir()) => {
                return Err(Error::InvalidSpec(
                    "an in-place folder output needs the host's folder".into(),
                ));
            }
            Overwrite::Replace if existing.as_ref().is_some_and(|m| !m.is_dir()) => {
                return Err(Error::Exists(destination.to_path_buf()));
            }
            _ => {}
        }
        let temp = match overwrite {
            Overwrite::InPlace => parent.join(format!("{name}.{}.partial", unique_suffix())),
            _ => parent.join(format!(".{name}.{}.partial", unique_suffix())),
        };
        create_private_dir(&temp)?;
        Ok(Self {
            destination: destination.to_path_buf(),
            temp,
            overwrite,
            mode,
            committed: false,
            folder: true,
        })
    }

    /// Where the encoder writes.
    pub fn temp_path(&self) -> &Path {
        &self.temp
    }

    pub fn destination(&self) -> &Path {
        &self.destination
    }

    /// Whether this output is a folder.
    pub fn is_folder(&self) -> bool {
        self.folder
    }

    /// Moves the finished temporary file into place and returns the destination.
    pub fn commit(mut self) -> Result<PathBuf, Error> {
        if self.folder {
            return self.commit_folder();
        }
        let meta = fs::metadata(&self.temp)
            .map_err(|e| Error::Io(format!("{}: {e}", self.temp.display())))?;
        if meta.len() == 0 {
            return Err(Error::Io("the encoder wrote an empty file".into()));
        }
        if let Some(mode) = self.mode {
            set_mode(&self.temp, mode)?;
        }
        match self.overwrite {
            Overwrite::Replace | Overwrite::InPlace => fs::rename(&self.temp, &self.destination)
                .map_err(|e| {
                    Error::Io(format!("moving into {}: {e}", self.destination.display()))
                })?,
            Overwrite::Fail => {
                match fs::hard_link(&self.temp, &self.destination) {
                    Ok(()) => {
                        let _ = fs::remove_file(&self.temp);
                    }
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        return Err(Error::Exists(self.destination.clone()));
                    }
                    // Volumes without hard links (exFAT, FAT, some network shares): check,
                    // then rename. Only a file created in between can be replaced.
                    Err(_) => {
                        if fs::symlink_metadata(&self.destination).is_ok() {
                            return Err(Error::Exists(self.destination.clone()));
                        }
                        fs::rename(&self.temp, &self.destination).map_err(|e| {
                            Error::Io(format!("moving into {}: {e}", self.destination.display()))
                        })?;
                    }
                }
            }
        }
        self.committed = true;
        remove_sidecars(&self.temp);
        Ok(self.destination.clone())
    }
}

impl AtomicOutput {
    fn commit_folder(mut self) -> Result<PathBuf, Error> {
        let io = |e: io::Error, p: &Path| Error::Io(format!("{}: {e}", p.display()));
        let mut files: Vec<PathBuf> = fs::read_dir(&self.temp)
            .map_err(|e| io(e, &self.temp))?
            .map(|e| e.map(|e| e.path()).map_err(|e| io(e, &self.temp)))
            .collect::<Result<_, _>>()?;
        files.sort();
        if files.is_empty() {
            return Err(Error::Io("the encoder wrote no files".into()));
        }
        for f in &files {
            let len = fs::metadata(f).map_err(|e| io(e, f))?.len();
            if len == 0 {
                return Err(Error::Io(format!("{} is empty", f.display())));
            }
            if let Some(mode) = self.mode {
                set_mode(f, mode)?;
            }
        }
        match self.overwrite {
            Overwrite::Replace if self.destination.is_dir() => {
                for f in &files {
                    let to = self.destination.join(f.file_name().expect("a file"));
                    fs::rename(f, &to).map_err(|e| io(e, &to))?;
                }
                let _ = fs::remove_dir(&self.temp);
            }
            _ => {
                if let Some(mode) = self.mode {
                    // Folders keep the owner's search bit with an owner-only mode.
                    set_mode(&self.temp, mode | ((mode & 0o444) >> 2))?;
                }
                // rename(2) replaces a destination folder only when it is empty:
                // the host's own folder (in place), or nothing at all.
                fs::rename(&self.temp, &self.destination).map_err(|e| {
                    if e.kind() == io::ErrorKind::DirectoryNotEmpty
                        || e.kind() == io::ErrorKind::AlreadyExists
                        || e.raw_os_error() == Some(libc::ENOTEMPTY)
                    {
                        Error::Exists(self.destination.clone())
                    } else {
                        io(e, &self.destination)
                    }
                })?;
            }
        }
        self.committed = true;
        Ok(self.destination.clone())
    }
}

fn create_private_dir(path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))
    }
}

impl Drop for AtomicOutput {
    fn drop(&mut self) {
        if !self.committed {
            if self.folder {
                // Only the output's own temporary folder, which it created.
                let _ = fs::remove_dir_all(&self.temp);
            } else {
                let _ = fs::remove_file(&self.temp);
                remove_sidecars(&self.temp);
            }
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
    fn in_place_writes_beside_the_host_file_and_renames_over_it() {
        let dir = scratch("inplace");
        let host = dir.join(".powermove-0123456789abcdef.part");
        fs::write(&host, b"").unwrap();
        let out = AtomicOutput::in_place(&host, "mov", Some(0o600)).unwrap();
        let temp = out.temp_path().to_path_buf();
        let name = temp.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with(".powermove-0123456789abcdef.part.") && name.ends_with(".partial.mov"),
            "{name}"
        );
        fs::write(&temp, b"movie").unwrap();
        assert_eq!(out.commit().unwrap(), host);
        assert_eq!(fs::read(&host).unwrap(), b"movie");
        assert!(!temp.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&host).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // A dropped in-place output leaves the host's file to the host.
        let out = AtomicOutput::in_place(&host, "mp4", None).unwrap();
        fs::write(out.temp_path(), b"partial").unwrap();
        let temp = out.temp_path().to_path_buf();
        drop(out);
        assert!(!temp.exists() && host.exists());
        assert!(AtomicOutput::new(&host, Overwrite::InPlace, None).is_err());
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

    #[test]
    fn folder_outputs_appear_whole_and_never_replace_a_folder_with_files() {
        let dir = scratch("folder");
        let dest = dir.join("Frames");
        let out = AtomicOutput::folder(&dest, Overwrite::Fail, Some(0o600)).unwrap();
        assert!(out.is_folder() && out.temp_path().is_dir());
        fs::write(out.temp_path().join("Frames_00000.png"), b"png").unwrap();
        fs::write(out.temp_path().join("Frames_00001.png"), b"png").unwrap();
        assert!(
            !dest.exists(),
            "nothing at the destination before the commit"
        );
        assert_eq!(out.commit().unwrap(), dest);
        assert!(dest.join("Frames_00001.png").is_file());
        // Fail: an existing folder is not replaced.
        assert!(matches!(
            AtomicOutput::folder(&dest, Overwrite::Fail, None),
            Err(Error::Exists(_))
        ));
        // Replace: frames of the same name are replaced, other files stay.
        fs::write(dest.join("notes.txt"), b"mine").unwrap();
        let out = AtomicOutput::folder(&dest, Overwrite::Replace, None).unwrap();
        fs::write(out.temp_path().join("Frames_00000.png"), b"new").unwrap();
        let temp = out.temp_path().to_path_buf();
        out.commit().unwrap();
        assert_eq!(fs::read(dest.join("Frames_00000.png")).unwrap(), b"new");
        assert_eq!(fs::read(dest.join("notes.txt")).unwrap(), b"mine");
        assert!(!temp.exists());
        // A dropped folder output removes only its own temporary folder.
        let out = AtomicOutput::folder(&dir.join("Other"), Overwrite::Fail, None).unwrap();
        fs::write(out.temp_path().join("a.png"), b"x").unwrap();
        let temp = out.temp_path().to_path_buf();
        drop(out);
        assert!(!temp.exists() && dest.join("notes.txt").exists());
        // In place: the host's empty folder is replaced by the finished one.
        let host = dir.join(".powermove-0123456789abcdef.part");
        fs::create_dir(&host).unwrap();
        let out = AtomicOutput::folder(&host, Overwrite::InPlace, None).unwrap();
        fs::write(out.temp_path().join("f_00000.png"), b"x").unwrap();
        assert_eq!(out.commit().unwrap(), host);
        assert!(host.join("f_00000.png").exists());
        // An empty output is refused.
        let out = AtomicOutput::folder(&dir.join("Empty"), Overwrite::Fail, None).unwrap();
        assert!(out.commit().is_err());
        assert!(!dir.join("Empty").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
