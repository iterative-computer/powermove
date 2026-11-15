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
    /// per file at the playhead, as `PM.pickFiles` does. Import takes files
    /// and folders: a folder of numbered frames, or one numbered frame with
    /// its run beside it, is offered as an image sequence (`plan_import`).
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
        let window = self.host.borrow().window.clone();
        let rx = if replace.is_some() {
            platform::import_paths(&window, false, false, &extensions())
        } else {
            platform::import_media_paths(&window, &extensions())
        };
        cx.spawn(async move |this, cx| {
            if let Ok(paths) = rx.recv().await
                && !paths.is_empty()
            {
                let _ = this.update(cx, |v, cx| {
                    let place_at = (place && replace.is_none()).then(|| v.time());
                    if replace.is_none() {
                        let plan = plan_import(paths, false);
                        return v.bring(plan, place_at.map(|at| (at, None)), None, cx);
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
    /// Layer ▸ Add 3D Layer ▸ Import 3D Model…: the open panel for models,
    /// then a 3D layer for each one once its asset lands (`add_3d`).
    pub(crate) fn pick_models(&mut self, cx: &mut Context<Self>) {
        if self.session.is_none() {
            self.message = "Open a project before importing a 3D model".into();
            cx.notify();
            return;
        }
        let rx = platform::import_paths(
            &self.host.borrow().window,
            false,
            true,
            &["glb", "gltf", "obj", "fbx"],
        );
        cx.spawn(async move |this, cx| {
            if let Ok(paths) = rx.recv().await
                && !paths.is_empty()
            {
                let _ = this.update(cx, |v, cx| {
                    v.import(
                        ImportRequest {
                            paths,
                            models: true,
                            add_3d: true,
                            ..Default::default()
                        },
                        cx,
                    );
                });
            }
        })
        .detach();
    }
    /// One Undo step adds a model layer per imported model, selected.
    fn add_imported_models(&mut self, assets: Vec<String>, cx: &mut Context<Self>) {
        let Some(session) = &self.session else { return };
        let Some(published) = self.kernel.published(&session.project) else {
            return;
        };
        let pairs: Vec<(String, String)> = assets
            .into_iter()
            .map(|a| (a, pm_kernel::new_id("L")))
            .collect();
        match pm_panel_studio3d::properties::add_models(&published, self.time(), &pairs) {
            Ok((label, commands, select)) => self.run_layer_plan_at(
                crate::layer_commands::Plan {
                    label,
                    commands,
                    select: Some(select),
                    count_keys: None,
                },
                Some(published.revision()),
                cx,
            ),
            Err(error) => {
                self.message = error.to_string();
                cx.notify();
            }
        }
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
                    v.link_media_folders(paths, folder, cx);
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
        let at = self.time();
        self.import_drop(paths, false, Some((at, None)), None, cx);
    }
    /// Files dropped on the viewer: placed at the playhead, on top.
    pub(crate) fn drop_on_viewer(
        &mut self,
        paths: Vec<PathBuf>,
        alt: bool,
        cx: &mut Context<Self>,
    ) {
        let at = self.time();
        self.import_drop(paths, alt, Some((at, Some(0))), None, cx);
    }
    /// Files dropped on the Project panel: assets in `folder`, no layers.
    pub(crate) fn import_to_panel(
        &mut self,
        paths: Vec<PathBuf>,
        folder: Option<String>,
        as_folder: bool,
        cx: &mut Context<Self>,
    ) {
        let (dirs, files): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| p.is_dir());
        if !dirs.is_empty() { self.link_media_folders(dirs, folder.clone(), cx); }
        if !files.is_empty() { self.import_drop(files, as_folder, None, folder, cx); }
    }
    /// Media dropped on the timeline: Finder files at the time and place in
    /// the stack under the pointer, or the Project panel's dragged items.
    pub(crate) fn drop_media(
        &mut self,
        drop: crate::timeline_surface::MediaDrop,
        cx: &mut Context<Self>,
    ) {
        use crate::timeline_surface::Dropped;
        match drop.what {
            Dropped::Files { paths, alt } => {
                self.import_drop(paths, alt, Some((drop.at, drop.index)), None, cx)
            }
            Dropped::ProjectItems => self.place_project_items(drop.at, drop.index, cx),
        }
    }
    /// The Project panel's dragged items placed at `at` from stack `index`
    /// (the timeline, the viewer): one Undo step, the new layers selected.
    /// An effect dragged from the Effects panel and dropped on a layer (or,
    /// with none under the pointer, the selected layer, as v1.2.0): added at
    /// the playhead in one Undo step, as the panel's Add adds it. Audio
    /// layers take no effects; nothing to take it says so.
    pub(crate) fn drop_effect(
        &mut self,
        drop: crate::timeline_surface::EffectDrop,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.builtins.views.get(&drop.panel).cloned() {
            view.update(cx, |v, cx| v.end_drag(cx));
        }
        let layer = drop.layer.clone().or_else(|| {
            self.published
                .as_ref()
                .and_then(|p| p.selection.primary().map(str::to_owned))
        });
        let Some(layer) = layer else {
            self.message = format!("Drop {} on a layer, or select one first", drop.label);
            return cx.notify();
        };
        let kind = self.published.as_ref().and_then(|p| {
            let root = p.snapshot.root()?;
            let at = pm_kernel::model::find_layer(root, &layer)?;
            let node = pm_kernel::model::layer_at(root, &at)?;
            Some((
                node.get("type")?.to_value().as_str()?.to_owned(),
                node.get("name")
                    .and_then(|n| n.to_value().as_str().map(str::to_owned)),
            ))
        });
        if let Some((kind, name)) = &kind
            && kind == "audio"
        {
            let name = name.as_deref().unwrap_or("This layer");
            self.message = format!("{name} is audio: audio layers don't take effects");
            return cx.notify();
        }
        let commands = self
            .builtins
            .runtime
            .as_ref()
            .ok_or_else(|| "The Effects panel closed.".to_owned())
            .and_then(|r| r.effect_commands(&drop.panel, &layer, &drop.definition, self.time()));
        match commands {
            Ok(commands) => self.run_layer_plan(
                crate::layer_commands::Plan {
                    label: format!("Add {}", drop.label),
                    commands,
                    select: Some(vec![layer]),
                    count_keys: None,
                },
                cx,
            ),
            Err(why) => {
                self.message = why;
                cx.notify();
            }
        }
    }
    pub(crate) fn place_project_items(
        &mut self,
        at: f64,
        index: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let drag = self
            .builtins
            .runtime
            .as_ref()
            .and_then(|r| r.project_drag());
        if let Some(view) = self.builtins.views.get(pm_builtins::PROJECT_PANEL).cloned() {
            view.update(cx, |v, cx| v.end_drag(cx));
        }
        let (Some(drag), Some(published)) = (drag, self.published.clone()) else {
            return;
        };
        let (assets, compositions) = match &drag.primary {
            pm_panel_media::ItemId::Composition(id) => (vec![], vec![id.clone()]),
            _ => (drag.assets.clone(), vec![]),
        };
        match pm_builtins::media_import::place_items(&published, &assets, &compositions, at, index)
        {
            Ok((layers, commands)) => {
                let label = format!("Add {}", drag.name);
                let plan = crate::layer_commands::Plan {
                    label,
                    commands,
                    select: Some(layers),
                    count_keys: None,
                };
                self.run_layer_plan(plan, cx);
            }
            Err(why) => {
                self.message = why;
                cx.notify();
            }
        }
    }
    /// What a drop imports, as Import Media does: a project opens; caption
    /// files become captions layers where media lands; a folder of numbered
    /// frames is offered as one image sequence (Option keeps its files
    /// apart), any other folder keeps its structure in the Project panel and
    /// places nothing; loose files are offered as a sequence when they are
    /// one, else imported. `place`: the time and stack place layers land at
    /// (`None`: the Project panel only, into `folder`).
    fn import_drop(
        &mut self,
        paths: Vec<PathBuf>,
        alt: bool,
        place: Option<(f64, Option<usize>)>,
        folder: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let media = self.open_dropped_project(paths, cx);
        if media.is_empty() {
            return;
        }
        if self.session.is_none() {
            self.message = "Open a project before importing media".into();
            cx.notify();
            return;
        }
        let (captions, media): (Vec<_>, Vec<_>) = media
            .into_iter()
            .partition(|p| crate::captions_panel::is_caption_file(p));
        if let Some((at, _)) = place {
            for path in captions {
                self.import_captions(path, at, cx);
            }
        }
        self.bring(plan_import(media, alt), place, folder, cx);
    }
    /// Imports what `plan` brings: kept folders into the Project panel, files
    /// placed at `place` (a time and a place in the stack), and the sequence
    /// it found offered on its sheet, its layer landing at the same place.
    fn bring(
        &mut self,
        plan: Brought,
        place: Option<(f64, Option<usize>)>,
        folder: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !plan.kept.is_empty() {
            self.import(
                ImportRequest {
                    paths: plan.kept,
                    folder: folder.clone(),
                    keep_folders: true,
                    ..Default::default()
                },
                cx,
            );
        }
        if !plan.files.is_empty() {
            self.import(
                ImportRequest {
                    paths: plan.files,
                    folder: folder.clone(),
                    place_at: place.map(|(at, _)| at),
                    place_index: place.and_then(|(_, index)| index),
                    ..Default::default()
                },
                cx,
            );
        }
        if let Some((frames, picked)) = plan.sequence {
            self.choose_sequence(frames, Some(picked), false, place, folder, cx);
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
        let add_3d = request.add_3d;
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
                        if add_3d && !outcome.assets.is_empty() {
                            v.add_imported_models(outcome.assets.clone(), cx);
                        }
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

/// What picked or dropped paths bring in ([`plan_import`]).
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Brought {
    /// Folders that keep their structure in the Project panel.
    pub kept: Vec<PathBuf>,
    /// Files imported as they are.
    pub files: Vec<PathBuf>,
    /// One image sequence to offer: its frames, and what Individual Images
    /// imports instead.
    pub sequence: Option<(Vec<PathBuf>, Vec<PathBuf>)>,
}
/// A folder of numbered frames, numbered files chosen together, or one
/// numbered frame with its run beside it is offered as one image sequence
/// (never with `alt`: Option keeps files apart); other folders keep their
/// structure; other files import as they are.
pub(crate) fn plan_import(paths: Vec<PathBuf>, alt: bool) -> Brought {
    let (folders, files): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| p.is_dir());
    let mut plan = Brought::default();
    for dir in folders {
        let frames = folder_frames(&dir);
        if !alt && plan.sequence.is_none() && crate::media_sequence::candidate(&frames) {
            plan.sequence = Some((frames.clone(), frames));
        } else {
            plan.kept.push(dir);
        }
    }
    match (!alt && plan.sequence.is_none())
        .then(|| crate::media_sequence::offered(&files))
        .flatten()
    {
        Some(frames) => plan.sequence = Some((frames, files)),
        None => plan.files = files,
    }
    plan
}

/// A dropped folder's own media files (not its subfolders'), in name order:
/// what is offered as an image sequence when they are one.
fn folder_frames(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut frames: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|p| p.is_file() && pm_builtins::media_import::classify(p).is_some())
        .collect();
    frames.sort();
    frames
}

#[cfg(test)]
mod drop_tests {
    use super::{Brought, folder_frames, plan_import};

    /// What Import and a drop bring: a folder of numbered frames and one
    /// numbered frame with its run each offer one sequence (Individual
    /// Images keeps what was picked); with Option they import apart; another
    /// folder keeps its structure.
    #[test]
    fn picked_and_dropped_frames_are_offered_as_a_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let renders = dir.path().join("renders");
        std::fs::create_dir_all(&renders).unwrap();
        for n in 1..=3 {
            std::fs::write(renders.join(format!("shot.{n:04}.png")), b"png").unwrap();
        }
        let frames = folder_frames(&renders);
        let mixed = dir.path().join("mixed");
        std::fs::create_dir_all(&mixed).unwrap();
        std::fs::write(mixed.join("intro.png"), b"png").unwrap();
        assert_eq!(
            plan_import(vec![renders.clone(), mixed.clone()], false),
            Brought {
                kept: vec![mixed.clone()],
                files: vec![],
                sequence: Some((frames.clone(), frames.clone())),
            }
        );
        let one = renders.join("shot.0002.png");
        assert_eq!(
            plan_import(vec![one.clone()], false),
            Brought {
                sequence: Some((frames.clone(), vec![one.clone()])),
                ..Default::default()
            }
        );
        assert_eq!(
            plan_import(vec![one.clone(), renders.clone()], true),
            Brought {
                kept: vec![renders],
                files: vec![one],
                sequence: None,
            }
        );
    }

    /// A dropped folder of numbered frames is offered as one image sequence
    /// (its own media files in name order; notes and subfolders are not
    /// frames); a folder of unrelated media is not.
    #[test]
    fn a_dropped_folder_of_numbered_frames_is_offered_as_a_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let renders = dir.path().join("renders");
        std::fs::create_dir_all(renders.join("old")).unwrap();
        for n in [3, 1, 2, 10] {
            std::fs::write(renders.join(format!("shot.{n:04}.png")), b"png").unwrap();
        }
        std::fs::write(renders.join("notes.txt"), b"not a frame").unwrap();
        std::fs::write(renders.join("old/shot.0001.png"), b"png").unwrap();
        let frames = folder_frames(&renders);
        let names: Vec<_> = frames
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "shot.0001.png",
                "shot.0002.png",
                "shot.0003.png",
                "shot.0010.png"
            ]
        );
        assert!(crate::media_sequence::candidate(&frames));
        let mixed = dir.path().join("mixed");
        std::fs::create_dir_all(&mixed).unwrap();
        std::fs::write(mixed.join("intro.png"), b"png").unwrap();
        std::fs::write(mixed.join("music.wav"), b"wav").unwrap();
        assert!(!crate::media_sequence::candidate(&folder_frames(&mixed)));
    }
}

impl Controller {
    fn link_media_folders(&mut self, paths: Vec<PathBuf>, folder: Option<String>, cx: &mut Context<Self>) {
        let (Some(runtime), Some(p)) = (self.builtins.runtime.clone(), self.published.clone()) else { return; };
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                runtime.link_folders(p.snapshot.doc_id, &paths, folder, &std::sync::atomic::AtomicBool::new(false))
            }).await;
            let _ = this.update(cx, |v, cx| {
                if let Err(error) = result { v.message = error; }
                v.refresh(cx);
            });
        }).detach();
    }
}
