//! Folder policy for user-facing document panels. Agent saves bypass this module.
use std::path::{Path, PathBuf};

pub fn needs_panel(has_file: bool, save_as: bool) -> bool {
    save_as || !has_file
}
pub fn project_directory(home: &Path, last: Option<&Path>) -> PathBuf {
    last.map(Path::to_path_buf)
        .unwrap_or_else(|| home.join("Movies/Powermove"))
}
pub fn export_directory(home: &Path, project: Option<&Path>, last: Option<&Path>) -> PathBuf {
    last.or_else(|| project.and_then(Path::parent))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join("Movies"))
}
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}
/// Saved documents use their file identity across sessions; untitled ones use
/// the library identity until their first file save.
pub fn export_key(id: &str, file: Option<&Path>) -> String {
    use std::hash::{Hash, Hasher};
    if let Some(file) = file {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        file.canonicalize()
            .unwrap_or_else(|_| file.to_path_buf())
            .hash(&mut hash);
        format!("file-{:016x}", hash.finish())
    } else {
        id.to_owned()
    }
}
pub fn carry_export_folder(profile: &Path, id: &str, old: Option<&Path>, new: &Path) {
    let old = export_key(id, old);
    let new = export_key(id, Some(new));
    if old != new
        && let Some(folder) = remembered(profile, Some(&old))
    {
        remember_folder(profile, Some(&new), &folder);
    }
}
fn preference(profile: &Path, project: Option<&str>) -> PathBuf {
    let name = project
        .map(|id| {
            format!(
                "export-{}",
                id.as_bytes()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            )
        })
        .unwrap_or_else(|| "project-save".into());
    profile.join("document-folders").join(name)
}
fn remembered(profile: &Path, project: Option<&str>) -> Option<PathBuf> {
    let folder = PathBuf::from(std::fs::read_to_string(preference(profile, project)).ok()?);
    folder.is_dir().then_some(folder)
}
/// Called on a background executor, never during main-thread rendering.
pub fn save_folder(profile: &Path) -> PathBuf {
    prepare_save_folder(profile, &home())
}
fn prepare_save_folder(profile: &Path, home: &Path) -> PathBuf {
    let folder = project_directory(home, remembered(profile, None).as_deref());
    if std::fs::create_dir_all(&folder).is_ok() {
        return folder;
    }
    // A full or read-only default volume must not prevent the person from
    // opening the panel and choosing another location.
    let movies = home.join("Movies");
    if movies.is_dir() {
        movies
    } else {
        home.to_path_buf()
    }
}
pub fn export_folder(profile: &Path, id: &str, project: Option<&Path>) -> PathBuf {
    export_directory(&home(), project, remembered(profile, Some(id)).as_deref())
}
/// Preference failure never changes the durable save/export result.
pub fn remember(profile: &Path, project: Option<&str>, destination: &Path) {
    let Some(folder) = destination.parent() else {
        return;
    };
    remember_folder(profile, project, folder);
}
fn remember_folder(profile: &Path, project: Option<&str>, folder: &Path) {
    let file = preference(profile, project);
    if std::fs::create_dir_all(file.parent().unwrap()).is_err() {
        return;
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temp = file.with_extension(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    if std::fs::write(&temp, folder.to_string_lossy().as_bytes()).is_ok() {
        let _ = std::fs::rename(&temp, &file);
    }
    let _ = std::fs::remove_file(temp);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_save_uses_movies_powermove() {
        assert_eq!(
            project_directory(Path::new("/editor"), None),
            PathBuf::from("/editor/Movies/Powermove")
        );
    }
    #[test]
    fn save_reuses_last_project_folder() {
        assert_eq!(
            project_directory(Path::new("/editor"), Some(Path::new("/projects"))),
            PathBuf::from("/projects")
        );
    }
    #[test]
    fn export_prefers_per_project_history_then_project_then_movies() {
        let home = Path::new("/editor");
        assert_eq!(export_directory(home, None, None), home.join("Movies"));
        assert_eq!(
            export_directory(home, Some(Path::new("/projects/cut.pmv")), None),
            PathBuf::from("/projects")
        );
        assert_eq!(
            export_directory(
                home,
                Some(Path::new("/projects/cut.pmv")),
                Some(Path::new("/deliveries"))
            ),
            PathBuf::from("/deliveries")
        );
    }
    #[test]
    fn first_save_and_save_as_ask_saved_saves_do_not() {
        assert!(needs_panel(false, false));
        assert!(!needs_panel(true, false));
        assert!(needs_panel(true, true));
        assert!(needs_panel(false, true));
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn remembers_save_and_export_folders_separately_per_project() {
        let dir = std::env::temp_dir().join(format!("pm-folder-test-{}", std::process::id()));
        let profile = dir.join("profile");
        let save = dir.join("projects");
        let exports = dir.join("deliveries");
        std::fs::create_dir_all(&save).unwrap();
        std::fs::create_dir_all(&exports).unwrap();
        remember(&profile, None, &save.join("cut.pmv"));
        remember(&profile, Some("../project/a"), &exports.join("cut.mp4"));
        assert_eq!(remembered(&profile, None), Some(save));
        assert_eq!(remembered(&profile, Some("../project/a")), Some(exports));
        assert_eq!(remembered(&profile, Some("project/b")), None);
        assert_ne!(
            preference(&profile, Some("a/b")),
            preference(&profile, Some("ab"))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn deleted_remembered_folder_falls_back() {
        let dir = std::env::temp_dir().join(format!("pm-stale-folder-test-{}", std::process::id()));
        let profile = dir.join("profile");
        let folder = dir.join("deleted");
        std::fs::create_dir_all(&folder).unwrap();
        remember(&profile, None, &folder.join("cut.pmv"));
        std::fs::remove_dir(folder).unwrap();
        assert_eq!(remembered(&profile, None), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn file_export_history_survives_reopening_and_first_save() {
        let dir = std::env::temp_dir().join(format!("pm-export-key-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cut.pmv");
        std::fs::write(&file, b"synthetic").unwrap();
        assert_eq!(
            export_key("new-session", Some(&file)),
            export_key("reopened-session", Some(&file))
        );
        remember(&dir, Some("untitled"), &dir.join("cut.mp4"));
        carry_export_folder(&dir, "untitled", None, &file);
        let key = export_key("reopened", Some(&file));
        assert_eq!(remembered(&dir, Some(&key)), Some(dir.clone()));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod unavailable_folder_tests {
    use super::*;
    #[test]
    fn an_uncreatable_default_still_allows_choosing_another_save_location() {
        let root =
            std::env::temp_dir().join(format!("pm-save-default-test-{}", std::process::id()));
        let home = root.join("home");
        let movies = home.join("Movies");
        let profile = root.join("profile");
        std::fs::create_dir_all(&movies).unwrap();
        std::fs::write(movies.join("Powermove"), b"synthetic obstruction").unwrap();
        assert_eq!(prepare_save_folder(&profile, &home), movies);
        std::fs::remove_file(movies.join("Powermove")).unwrap();
        assert_eq!(
            prepare_save_folder(&profile, &home),
            movies.join("Powermove")
        );
        assert!(movies.join("Powermove").is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }
}
