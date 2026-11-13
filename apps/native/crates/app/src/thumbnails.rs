//! Home's project cards show a poster frame: a small Draft-quality still of
//! the project at its playhead, rendered off the main thread after a save or
//! when the project closes. Files live in `<profile>/thumbnails/` as
//! `<project>-<stamp>.png`; a new stamp per render keeps GPUI's image cache
//! from showing an older one, and older files are removed.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

static DIR: OnceLock<PathBuf> = OnceLock::new();
/// The newest poster per project id (`None` until the directory was read).
static INDEX: Mutex<Option<HashMap<String, PathBuf>>> = Mutex::new(None);
/// Projects with a render in flight: a second request waits for the next save.
static BUSY: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Where posters live. Call once at launch with the profile directory.
pub fn install(profile: &Path) {
    let _ = DIR.set(profile.join("thumbnails"));
}

/// The project id a file name belongs to (`<id>-<stamp>.png`).
fn id_of(name: &str) -> Option<(&str, u128)> {
    let stem = name.strip_suffix(".png")?;
    let (id, stamp) = stem.rsplit_once('-')?;
    Some((id, stamp.parse().ok()?))
}

fn scan(dir: &Path) -> HashMap<String, PathBuf> {
    let mut newest: HashMap<String, (u128, PathBuf)> = HashMap::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some((id, stamp)) = id_of(&name)
            && newest.get(id).is_none_or(|(s, _)| stamp > *s)
        {
            newest.insert(id.to_owned(), (stamp, entry.path()));
        }
    }
    newest.into_iter().map(|(id, (_, p))| (id, p)).collect()
}

/// The newest poster of `project`, if one was rendered.
pub fn poster(project: &str) -> Option<PathBuf> {
    let dir = DIR.get()?;
    let mut index = INDEX.lock().unwrap();
    index.get_or_insert_with(|| scan(dir)).get(project).cloned()
}

/// Keeps `png` as `project`'s poster and removes the older ones.
fn store(dir: &Path, project: &str, png: &[u8]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let path = dir.join(format!("{project}-{stamp}.png"));
    let partial = dir.join(format!(".{project}-{stamp}.partial"));
    std::fs::write(&partial, png)?;
    std::fs::rename(&partial, &path)?;
    for entry in std::fs::read_dir(dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some((id, _)) = id_of(&name)
            && id == project
            && entry.path() != path
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let mut index = INDEX.lock().unwrap();
    index
        .get_or_insert_with(|| scan(dir))
        .insert(project.to_owned(), path.clone());
    Ok(path)
}

/// Removes every poster of `project` (it was deleted or moved to the Trash).
pub fn forget(project: &str) {
    let Some(dir) = DIR.get() else {
        return;
    };
    remove_posters(dir, project);
}
fn remove_posters(dir: &Path, project: &str) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if id_of(&name).is_some_and(|(id, _)| id == project) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    if let Some(index) = INDEX.lock().unwrap().as_mut() {
        index.remove(project);
    }
}
/// Renders `published` at `time` as a poster on a worker thread, then calls
/// `done` (Home redraws). Failures are quiet: the card keeps its size label.
pub fn refresh(
    kernel: pm_kernel::KernelHandle,
    published: Arc<pm_kernel::Published>,
    profile: PathBuf,
    time: f64,
    done: impl FnOnce() + Send + 'static,
) {
    let Some(dir) = DIR.get().cloned() else {
        return;
    };
    let project = published.project.clone();
    {
        let mut busy = BUSY.lock().unwrap();
        if busy.contains(&project) {
            return;
        }
        busy.push(project.clone());
    }
    std::thread::Builder::new()
        .name("project-poster".into())
        .spawn(move || {
            let result = crate::mcp_services::poster_png(kernel, &published, profile, time)
                .and_then(|png| store(&dir, &project, &png).map_err(Into::into));
            BUSY.lock().unwrap().retain(|p| *p != project);
            match result {
                Ok(_) => done(),
                Err(error) => eprintln!("Project poster for {project}: {error}"),
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_newest_poster_wins_and_older_ones_go() {
        let dir = std::env::temp_dir().join(format!("pm-posters-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("P1-100.png"), b"old").unwrap();
        std::fs::write(dir.join("P1-200.png"), b"new").unwrap();
        std::fs::write(dir.join("P2-50.png"), b"other").unwrap();
        let found = scan(&dir);
        assert_eq!(found["P1"], dir.join("P1-200.png"));
        assert_eq!(found["P2"], dir.join("P2-50.png"));
        let kept = store(&dir, "P1", b"newest").unwrap();
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("P1-"))
            .collect();
        assert_eq!(left.len(), 1);
        assert_eq!(std::fs::read(kept).unwrap(), b"newest");
        // Deleting the project removes its poster and keeps the others.
        remove_posters(&dir, "P1");
        let found = scan(&dir);
        assert!(!found.contains_key("P1"));
        assert!(found.contains_key("P2"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
