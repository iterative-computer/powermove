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
                            ..Default::default()
                        },
                        cx,
                    );
                });
            }
        })
        .detach();
    }
    /// Files dropped on the window: projects open, media lands at the playhead.
    pub(crate) fn import_dropped(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let media = self.open_dropped_project(paths, cx);
        if !media.is_empty() && self.session.is_none() {
            self.message = "Open a project before importing media".into();
            cx.notify();
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
        if !paths.is_empty() {
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
    pub(crate) fn import_for_extension(&mut self, answer: u64, file: u64, cx: &mut Context<Self>) {
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
        let request = ImportRequest {
            paths: vec![path],
            ..Default::default()
        };
        self.imports
            .push_back((request, Some(ExtensionImport { answer, file })));
        self.next_import(cx);
    }
    fn answer_import(&self, import: &ExtensionImport, result: Result<&Outcome, &str>) {
        let value = match result {
            Ok(outcome) => match outcome.ids.first() {
                Some(id) => json!({ "id": id }),
                None => json!({ "error": outcome.summary() }),
            },
            Err(error) => json!({ "error": error }),
        };
        pm_compat_runner::files::release_staged(import.file);
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
        cx.spawn(async move |this, cx| {
            let paths = rx.recv().await.unwrap_or_default();
            let files: Vec<_> = paths
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
                .collect();
            let _ = this.update(cx, |v, _| {
                if let Some(host) = &v.extensions.host {
                    host.answer(answer, files.into());
                }
            });
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
        let document = published.snapshot.doc_id;
        let profile = self.profile.clone();
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
                    Ok(outcome) => outcome.summary(),
                    Err(error) => error,
                };
                v.refresh(cx);
                v.next_import(cx);
            });
        })
        .detach();
    }
}
