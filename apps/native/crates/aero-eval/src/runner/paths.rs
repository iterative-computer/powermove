//! Reads the runner makes on a run's files: never outside the run folder,
//! never through a symlink, and never on a path the host merely names.
use std::path::{Component, Path, PathBuf};

/// Largest file the runner reads back (a saved project, a frame).
pub const MAX_READ: u64 = 512 << 20;

/// `path` lies under `root` (both as given, before resolution), has no `..`,
/// and no component from `root` down is a symlink. Returns the path.
pub fn inside(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|_| format!("{} is outside {}", path.display(), root.display()))?;
    let mut at = root.to_path_buf();
    for c in rel.components() {
        match c {
            Component::Normal(n) => at.push(n),
            _ => {
                return Err(format!(
                    "{} is not a plain path inside the run",
                    path.display()
                ));
            }
        }
        let m = std::fs::symlink_metadata(&at).map_err(|e| format!("{}: {e}", at.display()))?;
        if m.file_type().is_symlink() {
            return Err(format!(
                "{} is a symlink; the runner does not follow links",
                at.display()
            ));
        }
    }
    Ok(at)
}

/// Read a regular file inside one of `roots`.
pub fn read(roots: &[&Path], path: &Path) -> Result<Vec<u8>, String> {
    let p = match roots.iter().find(|r| path.starts_with(r)) {
        Some(r) => inside(r, path)?,
        None => {
            return Err(format!(
                "{} is not a file inside {}",
                path.display(),
                roots
                    .iter()
                    .map(|r| r.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" or ")
            ));
        }
    };
    let m = std::fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
    if !m.is_file() {
        return Err(format!("{} is not a regular file", p.display()));
    }
    if m.len() > MAX_READ {
        return Err(format!("{} is larger than {MAX_READ} bytes", p.display()));
    }
    std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Every regular file under `dir`, relative and sorted; a symlink anywhere
/// is an error (nothing behind it is read).
pub fn files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
        for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let e = e.map_err(|e| e.to_string())?;
            let p = e.path();
            let t = e.file_type().map_err(|e| e.to_string())?;
            if t.is_symlink() {
                return Err(format!("{} is a symlink", p.display()));
            }
            if t.is_dir() {
                walk(root, &p, out)?;
            } else if t.is_file() {
                out.push(p.strip_prefix(root).unwrap().to_path_buf());
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}
