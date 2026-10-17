//! Output destinations and atomic commits for the CLI host (contract 14
//! invariants 4 and 5). Same rules as Electron main's
//! `apps/desktop/src/main/engine/outputs.ts`: an owner-only temporary file
//! created exclusively in the destination folder, a commit that moves a
//! verified, non-empty file into place without replacing anything (unless
//! the user chose replace), and removal of empty, unverified, and orphaned
//! outputs.

use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use pm_protocol::host::{OutputChoice, OutputDestinationRequest, OverwritePolicy};

/// Longest file name on the volumes Powermove writes to.
pub const NAME_MAX: usize = 255;

/// The prefix and suffix of temporary outputs.
pub const TEMP_PREFIX: &str = ".powermove-";
pub const TEMP_SUFFIX: &str = ".part";

/// Whether `name` is a job engine work file of the temporary output `base`
/// (`pm_export::AtomicOutput::in_place`: `<base>.<16 hex>.partial[.<ext>]`).
fn is_work_file_of(name: &str, base: &str) -> bool {
    let Some(rest) = name.strip_prefix(base).and_then(|r| r.strip_prefix('.')) else {
        return false;
    };
    let Some((hex, tail)) = rest.split_once('.') else {
        return false;
    };
    let ext_ok = match tail.strip_prefix("partial") {
        Some("") => true,
        Some(ext) => ext.strip_prefix('.').is_some_and(|e| {
            !e.is_empty() && e.len() <= 16 && e.bytes().all(|b| b.is_ascii_alphanumeric())
        }),
        None => false,
    };
    hex.len() == 16
        && hex
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && ext_ok
}

/// Removes a temporary output and the job engine's work files named after
/// it (a cancelled, failed, or crashed encode leaves one beside it), as
/// main's `outputs.ts` does.
pub fn remove_temporary(temporary: &Path) {
    remove_path(temporary);
    let (Some(dir), Some(base)) = (temporary.parent(), temporary.file_name()) else {
        return;
    };
    let base = base.to_string_lossy();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if is_work_file_of(&entry.file_name().to_string_lossy(), &base) {
            remove_path(&entry.path());
        }
    }
}

/// Removes a temporary file, or a temporary folder (PNG sequences) with its
/// frames. Only called on the host's own temporary names.
fn remove_path(path: &Path) {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {
            let _ = fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = fs::remove_file(path);
        }
        Err(_) => {}
    }
}

/// Bytes of a temporary output: a file's length, or the sum of a folder's
/// files (PNG sequences); 0 for a folder with an empty file.
pub fn output_bytes(path: &Path) -> u64 {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {
            let Ok(entries) = fs::read_dir(path) else {
                return 0;
            };
            let mut total = 0u64;
            for entry in entries.flatten() {
                match entry.metadata() {
                    Ok(m) if m.is_file() && m.len() > 0 => total += m.len(),
                    _ => return 0,
                }
            }
            total
        }
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// `Name.ext`, `Name (2).ext`, … trimmed so every candidate fits `NAME_MAX`.
pub fn candidate_name(base: &str, extension: &str, attempt: u32) -> String {
    let suffix = if attempt == 0 {
        String::new()
    } else {
        format!(" ({})", attempt + 1)
    };
    let tail = if extension.is_empty() {
        suffix
    } else {
        format!("{suffix}.{extension}")
    };
    let mut trimmed = base.to_owned();
    while trimmed.len() + tail.len() > NAME_MAX {
        trimmed.pop();
    }
    format!("{trimmed}{tail}")
}

/// Where a commit puts a file.
#[derive(Debug, Clone)]
pub struct Placement {
    pub directory: PathBuf,
    pub base: String,
    pub extension: String,
    pub overwrite: OverwritePolicy,
    /// A user-chosen destination's exact path.
    pub exact: Option<PathBuf>,
}

impl Placement {
    /// Where an agent output (`agentAutomatic`) or a chosen file goes.
    pub fn for_request(
        request: &OutputDestinationRequest,
        automatic: &Path,
        chosen: Option<&Path>,
    ) -> Option<Placement> {
        match (&request.choice, chosen) {
            (
                OutputChoice::AgentAutomatic {
                    suggested_name,
                    extension,
                },
                _,
            ) => Some(Placement {
                directory: automatic.to_owned(),
                base: suggested_name.clone(),
                extension: extension.clone(),
                overwrite: request.overwrite,
                exact: None,
            }),
            (OutputChoice::UserChosen { .. }, Some(file)) => {
                let name = file.file_name()?.to_str()?.to_owned();
                let (base, extension) = match name.rsplit_once('.') {
                    Some((base, ext)) if !base.is_empty() => (base.to_owned(), ext.to_owned()),
                    _ => (name.clone(), String::new()),
                };
                Some(Placement {
                    directory: file.parent()?.to_owned(),
                    base,
                    extension,
                    overwrite: request.overwrite,
                    exact: Some(file.to_owned()),
                })
            }
            (OutputChoice::UserChosen { .. }, None) => None,
        }
    }
}

/// Creates the temporary file (or, for PNG sequences, folder): owner-only,
/// exclusively.
pub fn create_temporary(
    directory: &Path,
    random_hex: &str,
    folder: bool,
) -> std::io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let path = directory.join(format!("{TEMP_PREFIX}{random_hex}{TEMP_SUFFIX}"));
    if folder {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path)?;
        return Ok(path);
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&path)?;
    Ok(path)
}

/// Why a commit failed.
#[derive(Debug, thiserror::Error)]
pub enum PlaceError {
    #[error("a file with that name already exists")]
    Exists,
    #[error("could not choose a free export file name")]
    NoFreeName,
    #[error("the output could not be moved into place ({0})")]
    Io(#[from] std::io::Error),
}

/// Moves without replacing: a hard link fails if the name exists. Volumes
/// without hard links reserve the name exclusively, then rename over it.
fn place_no_clobber(temporary: &Path, candidate: &Path) -> Result<bool, PlaceError> {
    match fs::hard_link(temporary, candidate) {
        Ok(()) => {
            fs::remove_file(temporary)?;
            return Ok(true);
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(e)
            if matches!(
                e.kind(),
                ErrorKind::Unsupported | ErrorKind::PermissionDenied | ErrorKind::CrossesDevices
            ) => {}
        Err(e) => return Err(e.into()),
    }
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(candidate)
    {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e.into()),
    }
    fs::rename(temporary, candidate)?;
    Ok(true)
}

/// Moves a temporary folder into place without replacing anything: the name
/// is reserved by creating an empty folder, which `rename` then replaces.
fn place_folder_no_clobber(temporary: &Path, candidate: &Path) -> Result<bool, PlaceError> {
    match fs::create_dir(candidate) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e.into()),
    }
    fs::rename(temporary, candidate)?;
    Ok(true)
}

/// `<name>_<digits>.<ext>` split into (name, digit count, ext).
fn frame_parts(file: &str) -> Option<(&str, usize, &str)> {
    let (stem, ext) = file.rsplit_once('.')?;
    let (name, digits) = stem.rsplit_once('_')?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && !ext.is_empty())
        .then_some((name, digits.len(), ext))
}

/// Removes files in `folder` of the frame sequences in `written` (same name,
/// digit count and extension) that this export did not write.
fn remove_stale_frames(folder: &Path, written: &[String]) -> std::io::Result<()> {
    let sequences: Vec<_> = written.iter().filter_map(|n| frame_parts(n)).collect();
    if sequences.is_empty() {
        return Ok(());
    }
    for entry in fs::read_dir(folder)?.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if frame_parts(&name).is_some_and(|parts| sequences.contains(&parts))
            && !written.contains(&name)
            && entry.file_type().is_ok_and(|t| t.is_file())
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Moves a verified temporary output into place by the placement's policy.
/// A folder (PNG sequence) chosen with replace keeps the destination folder's
/// other files, replaces frames of the same name, and then removes the frames
/// of an earlier, longer export of the same sequence, so old and new frames
/// never mix (pm_export's `AtomicOutput` and main's `outputs.ts` do the same).
pub fn place(temporary: &Path, placement: &Placement) -> Result<PathBuf, PlaceError> {
    let folder = fs::symlink_metadata(temporary).is_ok_and(|m| m.is_dir());
    if folder {
        if let (Some(exact), OverwritePolicy::Replace) = (&placement.exact, placement.overwrite) {
            if exact.is_dir() {
                let mut written = Vec::new();
                for entry in fs::read_dir(temporary)?.flatten() {
                    fs::rename(entry.path(), exact.join(entry.file_name()))?;
                    written.extend(entry.file_name().into_string());
                }
                let _ = fs::remove_dir(temporary);
                remove_stale_frames(exact, &written)?;
            } else {
                fs::rename(temporary, exact)?;
            }
            return Ok(exact.clone());
        }
        let attempts = if placement.exact.is_some() && placement.overwrite == OverwritePolicy::Fail
        {
            1
        } else {
            10_000
        };
        for attempt in 0..attempts {
            let candidate = match (&placement.exact, attempt) {
                (Some(exact), 0) => exact.clone(),
                _ => placement
                    .directory
                    .join(candidate_name(&placement.base, "", attempt)),
            };
            if place_folder_no_clobber(temporary, &candidate)? {
                return Ok(candidate);
            }
        }
        return Err(if attempts == 1 {
            PlaceError::Exists
        } else {
            PlaceError::NoFreeName
        });
    }
    if let (Some(exact), OverwritePolicy::Replace) = (&placement.exact, placement.overwrite) {
        fs::rename(temporary, exact)?;
        return Ok(exact.clone());
    }
    let attempts = if placement.exact.is_some() && placement.overwrite == OverwritePolicy::Fail {
        1
    } else {
        10_000
    };
    for attempt in 0..attempts {
        let candidate = match (&placement.exact, attempt) {
            (Some(exact), 0) => exact.clone(),
            _ => placement.directory.join(candidate_name(
                &placement.base,
                &placement.extension,
                attempt,
            )),
        };
        if place_no_clobber(temporary, &candidate)? {
            return Ok(candidate);
        }
    }
    Err(if attempts == 1 {
        PlaceError::Exists
    } else {
        PlaceError::NoFreeName
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_a_temporary_output_removes_its_work_files_only() {
        let dir = std::env::temp_dir().join(format!("pm-host-work-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let temp = dir.join(".powermove-0123456789abcdef.part");
        for name in [
            ".powermove-0123456789abcdef.part",
            ".powermove-0123456789abcdef.part.fedcba9876543210.partial.mov",
            ".powermove-0123456789abcdef.part.fedcba9876543210.partial",
            ".powermove-0123456789abcdef.part.notours",
            ".powermove-0123456789abcdef.part.fedcba987654321.partial.mov",
            ".powermove-1111111111111111.part.fedcba9876543210.partial.mov",
        ] {
            fs::write(dir.join(name), b"x").unwrap();
        }
        remove_temporary(&temp);
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                ".powermove-0123456789abcdef.part.fedcba987654321.partial.mov",
                ".powermove-0123456789abcdef.part.notours",
                ".powermove-1111111111111111.part.fedcba9876543210.partial.mov",
            ]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn candidates_number_from_two_and_fit_name_max() {
        assert_eq!(candidate_name("Cut", "mov", 0), "Cut.mov");
        assert_eq!(candidate_name("Cut", "mov", 1), "Cut (2).mov");
        let long = candidate_name(&"x".repeat(300), "mov", 3);
        assert_eq!(long.len(), NAME_MAX);
        assert!(long.ends_with(" (4).mov"));
    }

    #[test]
    fn folder_outputs_are_created_measured_placed_and_removed() {
        let dir = std::env::temp_dir().join(format!("pm-host-folder-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let temp = create_temporary(&dir, "0123456789abcdef", true).unwrap();
        assert!(temp.is_dir());
        assert_eq!(output_bytes(&temp), 0, "an empty folder is empty output");
        fs::write(temp.join("Seq_00000.png"), b"png!").unwrap();
        fs::write(temp.join("Seq_00001.png"), b"png!!").unwrap();
        assert_eq!(output_bytes(&temp), 9);
        let placement = Placement {
            directory: dir.clone(),
            base: "Seq".into(),
            extension: String::new(),
            overwrite: OverwritePolicy::UniqueName,
            exact: None,
        };
        fs::create_dir(dir.join("Seq")).unwrap();
        let placed = place(&temp, &placement).unwrap();
        assert_eq!(
            placed,
            dir.join("Seq (2)"),
            "an existing folder is never replaced"
        );
        assert!(placed.join("Seq_00001.png").is_file() && !temp.exists());
        // Replace into the chosen folder: same-named frames are replaced, the
        // earlier longer export's extra frames go, other files stay.
        fs::write(placed.join("Seq_00002.png"), b"old").unwrap();
        fs::write(placed.join("notes.txt"), b"mine").unwrap();
        fs::write(placed.join("Other_00003.png"), b"other").unwrap();
        let temp = create_temporary(&dir, "2222222222222222", true).unwrap();
        fs::write(temp.join("Seq_00000.png"), b"new").unwrap();
        let replace = Placement {
            overwrite: OverwritePolicy::Replace,
            exact: Some(placed.clone()),
            ..placement
        };
        assert_eq!(place(&temp, &replace).unwrap(), placed);
        let mut names: Vec<_> = fs::read_dir(&placed)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["Other_00003.png", "Seq_00000.png", "notes.txt"]);
        assert_eq!(fs::read(placed.join("Seq_00000.png")).unwrap(), b"new");
        // Removing a temporary folder takes its frames with it.
        let temp = create_temporary(&dir, "1111111111111111", true).unwrap();
        fs::write(temp.join("f.png"), b"x").unwrap();
        remove_temporary(&temp);
        assert!(!temp.exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
