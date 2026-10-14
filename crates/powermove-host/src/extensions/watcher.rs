//! Watch user extension sources and batch rebuilds by top-level id (port of
//! `watcher.ts`). AI agents write mods straight to disk; this is how they
//! reach open windows.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::mpsc;

use super::manifest::is_extension_id;

pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// Stops watching when dropped.
pub struct Watcher {
    _inner: RecommendedWatcher,
}

/// Watch `user_dir`; after `debounce` without further changes, call `flush`
/// with the sorted ids that changed. Must be called inside a tokio runtime.
pub fn start<F, Fut>(user_dir: &Path, build_dir: &Path, debounce: Duration, flush: F) -> notify::Result<Watcher>
where
    F: Fn(Vec<String>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let source_root = std::path::absolute(user_dir).unwrap_or_else(|_| user_dir.to_path_buf());
    let build_root = std::path::absolute(build_dir).unwrap_or_else(|_| build_dir.to_path_buf());
    let (changes, mut queue) = mpsc::unbounded_channel::<String>();
    let root = source_root.clone();
    let mut inner = notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
        Ok(event) => {
            for id in event.paths.iter().filter_map(|path| changed_id(&root, &build_root, path)) {
                let _ = changes.send(id);
            }
        }
        Err(error) => tracing::warn!("extension watcher: {error}"),
    })?;
    inner.watch(&source_root, RecursiveMode::Recursive)?;

    tokio::spawn(async move {
        while let Some(first) = queue.recv().await {
            let mut pending = BTreeSet::from([first]);
            loop {
                match tokio::time::timeout(debounce, queue.recv()).await {
                    Ok(Some(id)) => {
                        pending.insert(id);
                    }
                    Ok(None) => return,
                    Err(_) => break,
                }
            }
            flush(pending.into_iter().collect()).await;
        }
    });
    Ok(Watcher { _inner: inner })
}

/// The extension id a changed path belongs to: its first component under the
/// source root, skipping the build root and hidden paths inside an extension.
fn changed_id(source_root: &Path, build_root: &Path, path: &Path) -> Option<String> {
    let path: PathBuf = path.components().filter(|component| *component != Component::CurDir).collect();
    if path.starts_with(build_root) {
        return None;
    }
    let mut components = path.strip_prefix(source_root).ok()?.components().map(|component| component.as_os_str().to_string_lossy().into_owned());
    let id = components.next().filter(|id| is_extension_id(id))?;
    if components.any(|component| component.starts_with('.')) {
        return None;
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn maps_changed_paths_to_extension_ids() {
        let root = Path::new(r"C:\data\extensions");
        let build = Path::new(r"C:\data\extensions-build");
        let id = |path: &str| changed_id(root, build, Path::new(path));
        assert_eq!(id(r"C:\data\extensions\my-mod\index.ts").as_deref(), Some("my-mod"));
        assert_eq!(id(r"C:\data\extensions\my-mod").as_deref(), Some("my-mod"));
        assert_eq!(id(r"C:\data\extensions\my-mod\.forked-from\index.ts"), None);
        assert_eq!(id(r"C:\data\extensions\.my-mod.tmp-1\index.ts"), None);
        assert_eq!(id(r"C:\data\extensions\Bad\index.ts"), None);
        assert_eq!(id(r"C:\data\extensions-build\my-mod\bundle.js"), None);
        assert_eq!(id(r"C:\data\extensions"), None);
    }
}
