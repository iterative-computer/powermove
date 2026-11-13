//! Import Media from the UI. Pickers and drops land here on the main thread;
//! probing, byte retention and the kernel's reply all wait on a worker.
use crate::{platform, session::Controller};
use gpui::Context;
use pm_builtins::media_import::{ImportRequest, Outcome, extensions};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

/// An extension's `assets.import`: the realm promise to answer and the held file.
pub(crate) struct ExtensionImport {
    pub answer: u64,
    pub file: u64,
}

impl Controller {
    /// File ▸ Import…, Project panel Import and Replace. `place` adds a layer
    /// per file at the playhead, as `PM.pickFiles` does.
    pub(crate) fn pick_media(
        &mut self,
        replace: Option<String>,
        place: bool,
        cx: &mut Context<Self>,
    ) {
        if self.session.is_none() {
            self.message = "Open a project before importing media".into();
            cx.notify();
            return;
        }
        let multiple = replace.is_none();
        let rx = platform::import_paths(&self.host.borrow().window, false, multiple, &extensions());
        cx.spawn(async move |this, cx| {
            if let Ok(paths) = rx.recv().await
                && !paths.is_empty()
            {
                let _ = this.update(cx, |v, cx| {
                    let place_at = (place && replace.is_none()).then(|| v.time());
                    // Numbered stills ask whether they are one clip.
                    if replace.is_none() && crate::media_sequence::candidate(&paths) {
                        return v.choose_sequence(paths, false, place_at, None, cx);
                    }
                    v.import(
                        ImportRequest {
                            paths,
                            replace,
                            place_at,
                            ..Default::default()
                        },
                        cx,
                    );
                });
            }
        })
        .detach();
    }
    /// File ▸ Import Folder…, Project panel Import Folder.
    pub(crate) fn pick_media_folder(&mut self, folder: Option<String>, cx: &mut Context<Self>) {
        if self.session.is_none() {
            self.message = "Open a project before importing media".into();
            cx.notify();
            return;
        }
        let rx = platform::import_paths(&self.host.borrow().window, true, false, &[]);
        cx.spawn(async move |this, cx| {
            if let Ok(paths) = rx.recv().await
                && !paths.is_empty()
            {
                let _ = this.update(cx, |v, cx| {
                    v.import(
                        ImportRequest {
                            paths,
                            folder,
                            keep_folders: true,
                            ..Default::default()
                        },
                        cx,
                    );
                });
            }
        })
        .detach();
    }
    /// Paste with media on the pasteboard (`pasteMedia`): files copied in
    /// Finder, or a copied picture such as a screenshot, become layers at the
    /// playhead, stored with the project as an import stores them.
    pub(crate) fn paste_media(&mut self, cx: &mut Context<Self>) -> bool {
        match platform::pasteboard_media() {
            Some(media) => self.paste_pasted(media, cx),
            None => false,
        }
    }
    pub(crate) fn paste_pasted(
        &mut self,
        media: platform::PastedMedia,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.session.is_none() {
            return false;
        }
        let place_at = Some(self.time());
        match media {
            platform::PastedMedia::Files(paths) => {
                // Files that are no media (a copied document) are not a
                // paste for the importer: the app's own clipboard has it.
                if !pasted_files_hold_media(&paths) {
                    return false;
                }
                self.import(
                    ImportRequest {
                        paths,
                        place_at,
                        ..Default::default()
                    },
                    cx,
                );
            }
            platform::PastedMedia::Image { data, png } => {
                // Converting and writing a large screenshot is worker work.
                let staged = cx
                    .background_executor()
                    .spawn(async move { stage_pasted_image(data, png) });
                cx.spawn(async move |this, cx| {
                    let staged = staged.await;
                    let _ = this.update(cx, |v, cx| match staged {
                        Some((dir, path)) => v.import(
                            ImportRequest {
                                paths: vec![path],
                                place_at,
                                staged: Some(dir),
                                ..Default::default()
                            },
                            cx,
                        ),
                        None => {
                            v.message = "The copied image could not be read".into();
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
        }
        true
    }
    /// Files dropped on the window: projects open, media lands at the playhead.
    pub(crate) fn import_dropped(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let media = self.open_dropped_project(paths, cx);
        // SubRip and WebVTT files become captions layers at the playhead.
        let (captions, media): (Vec<_>, Vec<_>) = media
            .into_iter()
            .partition(|p| crate::captions_panel::is_caption_file(p));
        if self.session.is_some() {
            let at = self.time();
            for path in captions {
                self.import_captions(path, at, cx);
            }
        }
        if !media.is_empty() && self.session.is_none() {
            self.message = "Open a project before importing media".into();
            cx.notify();
        } else if crate::media_sequence::candidate(&media) {
            let place_at = Some(self.time());
            self.choose_sequence(media, false, place_at, None, cx);
        } else if !media.is_empty() {
            let place_at = Some(self.time());
            self.import(
                ImportRequest {
                    paths: media,
                    place_at,
                    ..Default::default()
                },
                cx,
            );
        }
    }
    /// Files dropped on the Project panel: assets in `folder`, no layers.
    pub(crate) fn import_to_panel(
        &mut self,
        paths: Vec<PathBuf>,
        folder: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let paths = self.open_dropped_project(paths, cx);
        if crate::media_sequence::candidate(&paths) {
            self.choose_sequence(paths, false, None, folder, cx);
        } else if !paths.is_empty() {
            let request = ImportRequest {
                paths,
                folder,
                ..Default::default()
            };
            self.import(request, cx);
        }
    }
    /// A dropped project opens instead of importing. Returns the media to import.
    fn open_dropped_project(
        &mut self,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Vec<PathBuf> {
        let (projects, media): (Vec<_>, Vec<_>) = paths.into_iter().partition(|path| {
            matches!(
                path.extension().and_then(|x| x.to_str()),
                Some("pmv" | "json")
            )
        });
        match projects.into_iter().next() {
            Some(path) => {
                self.open_file(path, cx);
                vec![]
            }
            None => media,
        }
    }
    /// Batches run in order: user order and project identity both depend on it.
    pub(crate) fn import(&mut self, request: ImportRequest, cx: &mut Context<Self>) {
        self.imports.push_back((request, None));
        self.next_import(cx);
    }
    /// `assets.import` from an extension: the held file into the Project panel,
    /// no layer, answering the realm with the new asset's id.
    /// `assets.importUrl`: main's media download (public HTTPS, five
    /// redirects, 512 MiB, type from the bytes) on a worker, then the import.
    pub(crate) fn download_for_extension(
        &mut self,
        answer: u64,
        url: String,
        cx: &mut Context<Self>,
    ) {
        let download = cx.background_executor().spawn(async move {
            let response = pm_compat_runner::net::download(&url, 512 << 20, 600)
                .map_err(|e| format!("The download failed: {e}"))?;
            if !(200..300).contains(&response.status) {
                return Err(format!("The download returned {}", response.status));
            }
            let extension = pm_compat_runner::files::media_extension(&response.body)
                .ok_or("The download isn't an image, video, audio or 3D file")?;
            let name = pm_compat_runner::files::download_name(&url, extension);
            pm_compat_runner::files::stage(&response.body, &name).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let staged = download.await;
            let _ = this.update(cx, |v, cx| match staged {
                Ok(file) => v.import_for_extension(answer, file, None, cx),
                Err(error) => {
                    if let Some(host) = &v.extensions.host {
                        host.answer(answer, json!({ "error": error }));
                    }
                }
            });
        })
        .detach();
    }
    pub(crate) fn import_for_extension(
        &mut self,
        answer: u64,
        file: u64,
        layer_definition: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = pm_compat_runner::files::path(file) else {
            self.answer_import(
                &ExtensionImport { answer, file },
                Err("The file is no longer available"),
            );
            return;
        };
        if self.session.is_none() {
            self.answer_import(
                &ExtensionImport { answer, file },
                Err("Open a project before importing media"),
            );
            return;
        }
        // As main's assets.import: 3D models too, and the layer definition kept.
        let request = ImportRequest {
            paths: vec![path],
            models: true,
            layer_definition,
            ..Default::default()
        };
        self.imports
            .push_back((request, Some(ExtensionImport { answer, file })));
        self.next_import(cx);
    }
    fn answer_import(&self, import: &ExtensionImport, result: Result<&Outcome, &str>) {
        let value = match result {
            Ok(outcome) => match outcome.assets.first() {
                Some(id) => json!({ "id": id }),
                None => json!({ "error": outcome.summary() }),
            },
            Err(error) => json!({ "error": error }),
        };
        let file = import.file;
        // Deleting a staged copy is disk work: never on the main thread.
        std::thread::spawn(move || pm_compat_runner::files::release_staged(file));
        if let Some(host) = &self.extensions.host {
            host.answer(import.answer, value);
        }
    }
    /// Abandons queued batches; an extension waiting on one hears why.
    fn drop_imports(&mut self, reason: &str) {
        for (_, extension) in std::mem::take(&mut self.imports) {
            if let Some(extension) = extension {
                self.answer_import(&extension, Err(reason));
            }
        }
    }
    /// `assets.pick`: the open panel, the chosen files held for the realm.
    pub(crate) fn pick_for_extension(
        &mut self,
        answer: u64,
        accept: &str,
        multiple: bool,
        cx: &mut Context<Self>,
    ) {
        let extensions = pm_builtins::media_import::accept_extensions(accept);
        let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
        let rx = platform::import_paths(&self.host.borrow().window, false, multiple, &extensions);
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let paths = rx.recv().await.unwrap_or_default();
            // File facts come from disk (maybe a network volume): on a worker.
            let files: Vec<_> = executor.spawn(async move { paths
                .into_iter()
                .filter_map(|path| {
                    let meta = std::fs::metadata(&path).ok()?;
                    let name = path.file_name()?.to_string_lossy().into_owned();
                    let kind = pm_builtins::media_import::classify(&path).map(|(_, mime)| mime);
                    let modified = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_millis() as u64);
                    let token = pm_compat_runner::files::hold_picked(path);
                    Some(json!({"token":token.to_string(),"name":name,"type":kind.unwrap_or_default(),
                        "size":meta.len(),"lastModified":modified}))
                })
                .collect::<Vec<_>>() }).await;
            let _ = this.update(cx, |v, _| {
                if let Some(host) = &v.extensions.host {
                    host.answer(answer, files.into());
                }
            });
        })
        .detach();
    }
    /// `PM.MediaStore.get`: the original file of a project asset, held like a
    /// picked file (read only when the extension reads it), or null.
    pub(crate) fn media_file_for_extension(
        &mut self,
        answer: u64,
        asset: &str,
        cx: &mut Context<Self>,
    ) {
        let path = self.media.originals.get(asset).cloned();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let file = executor
                .spawn(async move {
                    let path = path?;
                    let meta = std::fs::metadata(&path).ok().filter(|m| m.is_file())?;
                    let name = path.file_name()?.to_string_lossy().into_owned();
                    let kind = pm_builtins::media_import::classify(&path).map(|(_, mime)| mime);
                    let token = pm_compat_runner::files::hold_picked(path);
                    Some(json!({"token":token.to_string(),"name":name,"type":kind.unwrap_or_default(),
                        "size":meta.len(),"lastModified":0}))
                })
                .await;
            let _ = this.update(cx, |v, _| {
                if let Some(host) = &v.extensions.host {
                    host.answer(answer, file.unwrap_or(serde_json::Value::Null));
                }
            });
        })
        .detach();
    }
    /// The encode's progress line in the status bar until the import ends.
    fn show_sequence_progress(
        &mut self,
        progress: Arc<std::sync::Mutex<String>>,
        cancel: Arc<AtomicBool>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(200))
                    .await;
                let line = progress.lock().map(|p| p.clone()).unwrap_or_default();
                let running = this
                    .update(cx, |v, cx| {
                        let current = v
                            .importing
                            .as_ref()
                            .is_some_and(|c| Arc::ptr_eq(c, &cancel));
                        if current && !line.is_empty() && v.message != line {
                            v.message = line;
                            cx.notify();
                        }
                        current
                    })
                    .unwrap_or(false);
                if !running {
                    return;
                }
            }
        })
        .detach();
    }
    fn next_import(&mut self, cx: &mut Context<Self>) {
        if self.importing.is_some() {
            return;
        }
        let (Some(published), Some(runtime)) =
            (self.published.clone(), self.builtins.runtime.clone())
        else {
            self.drop_imports("Open a project before importing media");
            return;
        };
        let Some((request, extension)) = self.imports.pop_front() else {
            return;
        };
        // Close and quit wait for this fence, so an import never lands half way.
        let Some(operation) = self.mcp.try_operation() else {
            self.imports.push_front((request, extension));
            self.drop_imports(
                "A project operation is in progress. Try importing again when it finishes.",
            );
            self.message =
                "A project operation is in progress. Try importing again when it finishes.".into();
            cx.notify();
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.importing = Some(cancel.clone());
        self.message = "Importing…".into();
        cx.notify();
        if let Some(sequence) = &request.sequence {
            self.show_sequence_progress(sequence.progress.clone(), cancel.clone(), cx);
        }
        let document = published.snapshot.doc_id;
        let profile = self.profile.clone();
        let note = request.note.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { runtime.import_media(document, &profile, request, &cancel) })
                .await;
            // The next batch takes this fence itself.
            drop(operation);
            let _ = this.update(cx, |v, cx| {
                v.importing = None;
                if let Some(extension) = &extension {
                    v.answer_import(extension, result.as_ref().map_err(String::as_str));
                }
                v.message = match result {
                    Ok(outcome) => {
                        // Settings › Media › Create Proxies, once the
                        // catalog resolves the new files.
                        v.pro_media.after_import(outcome.assets.clone());
                        match note {
                            Some(note) if outcome.failed.is_empty() && !outcome.cancelled => note,
                            _ => outcome.summary(),
                        }
                    }
                    Err(error) => error,
                };
                v.refresh(cx);
                v.next_import(cx);
            });
        })
        .detach();
    }
}

/// Whether copied files give the importer anything: a media file, or a
/// folder (which it reads for media).
pub(crate) fn pasted_files_hold_media(paths: &[PathBuf]) -> bool {
    paths
        .iter()
        .any(|p| pm_builtins::media_import::classify(p).is_some() || p.is_dir())
}
/// A copied picture as a PNG file in scratch that lasts until its import is done.
pub(crate) fn stage_pasted_image(data: Vec<u8>, png: bool) -> Option<(tempfile::TempDir, PathBuf)> {
    let png = if png {
        data
    } else {
        platform::tiff_to_png(&data)?
    };
    let dir = tempfile::Builder::new()
        .prefix("powermove-paste-")
        .tempdir()
        .ok()?;
    let path = dir.path().join("Pasted Image.png");
    std::fs::write(&path, png).ok()?;
    Some((dir, path))
}
