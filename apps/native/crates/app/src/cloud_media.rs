//! Cloud media (files on demand: iCloud Drive and other File Providers), as
//! v1's `core/cloud-media.ts`. A linked original whose bytes are still in the
//! cloud is not read on open (reading would download it unasked): it stays
//! offline until the person agrees to download it, or has turned on
//! “Automatically download cloud media when opening projects”. Downloads run
//! off the main thread; the status line shows their progress, and each file
//! joins the project as it lands.
use crate::session::Controller;
use gpui::Context;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

/// v1's checkbox and Settings label.
pub const PREFERENCE: &str = "Automatically download cloud media when opening projects";

/// Where cloud files are asked about and downloaded. The platform answers in
/// the app; tests and on-screen checks use [`Fixture`] so nothing touches a
/// real cloud account.
pub trait CloudFiles: Send + Sync {
    /// The provider's name (`iCloud`, `Cloud`) when `path` is cloud-only.
    fn provider(&self, path: &Path) -> Option<String>;
    /// Brings `path` local. Blocking; `progress(done, total)` as bytes arrive.
    fn download(&self, path: &Path, progress: &dyn Fn(u64, u64)) -> Result<(), String>;
}

/// macOS: a dataless file (`SF_DATALESS`) is a cloud placeholder. Reading it
/// through to the end makes its File Provider download it.
struct Platform;
const SF_DATALESS: u32 = 0x4000_0000;
impl CloudFiles for Platform {
    fn provider(&self, path: &Path) -> Option<String> {
        use std::os::macos::fs::MetadataExt;
        let meta = std::fs::symlink_metadata(path).ok()?;
        (meta.is_file() && meta.st_flags() & SF_DATALESS != 0).then(|| {
            if path
                .to_string_lossy()
                .contains("/Library/Mobile Documents/")
            {
                "iCloud".into()
            } else {
                "Cloud".into()
            }
        })
    }
    fn download(&self, path: &Path, progress: &dyn Fn(u64, u64)) -> Result<(), String> {
        use std::io::Read;
        let total = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; 4 << 20];
        let mut done = 0u64;
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            done += n as u64;
            progress(done.min(total), total);
        }
        if done < total {
            return Err("The file ended before it finished downloading.".into());
        }
        Ok(())
    }
}

/// A stand-in cloud for tests and on-screen checks (`PM_CLOUD_FIXTURE` names
/// a JSON file: `{"cloud": [paths], "fail": [paths], "ms": 1500}`). Listed
/// files read as cloud-only until a download, which takes `ms` and reports
/// progress; `fail` ones fail.
pub struct Fixture {
    cloud: Mutex<BTreeSet<PathBuf>>,
    fail: BTreeSet<PathBuf>,
    ms: u64,
}
impl Fixture {
    pub fn new(cloud: &[PathBuf], fail: &[PathBuf], ms: u64) -> Self {
        Self {
            cloud: Mutex::new(cloud.iter().cloned().collect()),
            fail: fail.iter().cloned().collect(),
            ms,
        }
    }
    fn load(file: &Path) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
        let paths = |k: &str| -> Vec<PathBuf> {
            v[k].as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p.as_str().map(PathBuf::from))
                .collect()
        };
        Some(Self::new(
            &paths("cloud"),
            &paths("fail"),
            v["ms"].as_u64().unwrap_or(1500),
        ))
    }
}
impl CloudFiles for Fixture {
    fn provider(&self, path: &Path) -> Option<String> {
        self.cloud
            .lock()
            .unwrap()
            .contains(path)
            .then(|| "iCloud".into())
    }
    fn download(&self, path: &Path, progress: &dyn Fn(u64, u64)) -> Result<(), String> {
        let steps = 10;
        for i in 1..=steps {
            std::thread::sleep(std::time::Duration::from_millis(self.ms / steps));
            progress(i, steps);
            if self.fail.contains(path) && i == steps / 2 {
                return Err("The connection was lost.".into());
            }
        }
        self.cloud.lock().unwrap().remove(path);
        Ok(())
    }
}

/// The app's cloud: the platform's, or the fixture a development run names.
pub fn files() -> Arc<dyn CloudFiles> {
    static FILES: OnceLock<Arc<dyn CloudFiles>> = OnceLock::new();
    FILES
        .get_or_init(|| {
            if cfg!(debug_assertions)
                && let Some(fixture) =
                    std::env::var_os("PM_CLOUD_FIXTURE").and_then(|f| Fixture::load(Path::new(&f)))
            {
                return Arc::new(fixture);
            }
            Arc::new(Platform)
        })
        .clone()
}

/// One cloud-only original the resolver left out: asset id, file, provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cloud {
    pub id: String,
    pub path: PathBuf,
    pub provider: String,
}

/// What this window is doing about cloud media.
#[derive(Default)]
pub struct State {
    /// Documents whose cloud media was offered (asked once per open, as v1).
    asked: BTreeSet<(String, u64)>,
    /// Files downloading now.
    downloading: BTreeSet<PathBuf>,
}

/// v1's prompt copy.
pub fn prompt(files: &[Cloud]) -> (String, String) {
    let n = files.len();
    let mut info = "These files are stored in the cloud and need to be downloaded to use them in this project.\n\n".to_string();
    let names: Vec<String> = files
        .iter()
        .take(8)
        .map(|f| {
            f.path
                .file_name()
                .map_or_else(|| f.id.clone(), |n| n.to_string_lossy().into_owned())
        })
        .collect();
    info.push_str(&names.join("\n"));
    if n > 8 {
        info.push_str(&format!("\n…and {} more", n - 8));
    }
    let message = if n == 1 {
        "Download cloud media?".into()
    } else {
        format!("Download {n} cloud media files?")
    };
    (message, info)
}

impl Controller {
    /// The resolver found cloud-only originals for the open document: download
    /// them when the person said to always, else ask once for this open.
    pub(crate) fn offer_cloud_media(&mut self, found: Vec<Cloud>, cx: &mut Context<Self>) {
        let found: Vec<Cloud> = found
            .into_iter()
            .filter(|c| !self.cloud.downloading.contains(&c.path))
            .collect();
        let (Some(session), Some(p)) = (&self.session, &self.published) else {
            return;
        };
        let key = (session.project.clone(), p.snapshot.doc_id);
        if found.is_empty() || !self.cloud.asked.insert(key) {
            return;
        }
        let automatic = self
            .kernel
            .state()
            .settings
            .get("autoDownloadCloudMedia")
            .and_then(pm_model::Value::as_bool)
            == Some(true);
        if automatic {
            return self.download_cloud_media(found, cx);
        }
        use crate::alert::{Alert, Button};
        let (message, info) = prompt(&found);
        let alert = Alert::new(
            message,
            info,
            vec![
                Button::new("Download", true).default(),
                Button::new("Not Now", false).cancel(),
            ],
        );
        let checkbox: std::rc::Rc<
            std::cell::RefCell<Option<objc2::rc::Retained<objc2_app_kit::NSButton>>>,
        > = Default::default();
        let keep = checkbox.clone();
        let answer = alert.sheet_with(&self.host.borrow().window.clone(), move |a| {
            a.setShowsSuppressionButton(true);
            if let Some(b) = a.suppressionButton() {
                b.setTitle(&objc2_foundation::NSString::from_str(PREFERENCE));
                *keep.borrow_mut() = Some(b);
            }
        });
        cx.spawn(async move |this, cx| {
            let download = answer.recv().await == Ok(Some(true));
            let always = checkbox
                .borrow()
                .as_ref()
                .is_some_and(|b| b.state() == objc2_app_kit::NSControlStateValueOn);
            let _ = this.update(cx, |v, cx| {
                // Checking the box opts into later openings even when this
                // batch waits (v1).
                if always {
                    drop(
                        v.kernel
                            .state_transaction(pm_kernel::state::StateTransaction {
                                expected_revision: None,
                                ops: vec![pm_kernel::state::StateOp::Setting {
                                    key: "autoDownloadCloudMedia".into(),
                                    value: Some(pm_model::Value::Bool(true)),
                                }],
                            }),
                    );
                }
                if download {
                    v.download_cloud_media(found, cx);
                } else {
                    v.note("Some media is still in the cloud.".into(), cx);
                }
            });
        })
        .detach();
    }
    /// Downloads `files` one after another off the main thread; the status line
    /// follows, and each file is resolved into the project as it lands.
    fn download_cloud_media(&mut self, files: Vec<Cloud>, cx: &mut Context<Self>) {
        let cloud = self::files();
        for f in &files {
            self.cloud.downloading.insert(f.path.clone());
        }
        let total = files.len();
        let provider = files
            .first()
            .map_or("the cloud".into(), |f| f.provider.clone());
        let noun = if total == 1 {
            "1 file".into()
        } else {
            format!("{total} files")
        };
        self.note(format!("Downloading {noun} from {provider}…"), cx);
        cx.spawn(async move |this, cx| {
            let mut done = 0;
            for f in files {
                let (tx, rx) = async_channel::unbounded::<(u64, u64)>();
                let (path, files) = (f.path.clone(), cloud.clone());
                let job = cx.background_executor().spawn(async move {
                    files.download(&path, &|a, b| {
                        let _ = tx.try_send((a, b));
                    })
                });
                let mut shown = std::time::Instant::now();
                while let Ok((a, b)) = rx.recv().await {
                    if shown.elapsed().as_millis() >= 250 && b > 0 {
                        shown = std::time::Instant::now();
                        let pct = (a as f64 / b as f64 * 100.).round();
                        let line = format!(
                            "Downloading {noun} from {provider}… {}{pct:.0}%",
                            if total > 1 {
                                format!("{} of {total}, ", done + 1)
                            } else {
                                String::new()
                            }
                        );
                        let _ = this.update(cx, |v, cx| v.note(line, cx));
                    }
                }
                let result = job.await;
                let name = f
                    .path
                    .file_name()
                    .map_or_else(|| f.id.clone(), |n| n.to_string_lossy().into_owned());
                let _ = this.update(cx, |v, cx| {
                    v.cloud.downloading.remove(&f.path);
                    match result {
                        Ok(()) => {
                            done += 1;
                            v.resolve_media_again(cx);
                        }
                        Err(e) => v.note(format!("Could not restore “{name}” · {e}"), cx),
                    }
                });
            }
            let _ = this.update(cx, |v, cx| {
                if done == total {
                    v.note(format!("{done} of {total} downloaded"), cx);
                }
            });
        })
        .detach();
    }
    /// Resolves the open document's media again without retiring what shows
    /// (a file that came down from the cloud joins the catalog).
    fn resolve_media_again(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let Some(published) = self.kernel.published(&session.project) else {
            return;
        };
        let cache = crate::media::cache_root(&self.profile);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::media::for_session(
                        &published.snapshot,
                        &published.media,
                        published.path.as_deref(),
                        &cache,
                    )
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                if let Ok(resolution) = result {
                    v.adopt_media(resolution, cx);
                    v.update_builtins(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
#[cfg(test)]
mod tests {
    use super::{Cloud, CloudFiles, Fixture, prompt};
    use std::path::PathBuf;

    #[test]
    fn the_fixture_cloud_downloads_with_progress_and_fails_what_it_names() {
        let (a, b) = (PathBuf::from("/x/a.mov"), PathBuf::from("/x/b.mov"));
        let cloud = Fixture::new(&[a.clone(), b.clone()], std::slice::from_ref(&b), 20);
        assert_eq!(cloud.provider(&a).as_deref(), Some("iCloud"));
        let seen = std::sync::Mutex::new(vec![]);
        cloud
            .download(&a, &|x, y| seen.lock().unwrap().push((x, y)))
            .unwrap();
        assert_eq!(seen.lock().unwrap().last(), Some(&(10, 10)));
        assert_eq!(cloud.provider(&a), None);
        assert!(cloud.download(&b, &|_, _| {}).is_err());
        assert!(cloud.provider(&b).is_some());
    }

    #[test]
    fn the_prompt_names_up_to_eight_files() {
        let files: Vec<Cloud> = (0..10)
            .map(|i| Cloud {
                id: format!("a{i}"),
                path: PathBuf::from(format!("/x/clip{i}.mov")),
                provider: "iCloud".into(),
            })
            .collect();
        let (message, info) = prompt(&files);
        assert_eq!(message, "Download 10 cloud media files?");
        assert!(info.contains("clip7.mov") && !info.contains("clip8.mov"));
        assert!(info.ends_with("…and 2 more"));
        assert_eq!(prompt(&files[..1]).0, "Download cloud media?");
    }
}
