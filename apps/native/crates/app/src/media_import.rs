//! Import Media from the UI. Pickers and drops land here on the main thread;
//! probing, byte retention and the kernel's reply all wait on a worker.
use crate::{platform, session::Controller};
use gpui::Context;
use pm_builtins::media_import::{ImportRequest, extensions};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

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
        let (projects, media): (Vec<_>, Vec<_>) = paths.into_iter().partition(|path| {
            matches!(
                path.extension().and_then(|x| x.to_str()),
                Some("pmv" | "json")
            )
        });
        if let Some(path) = projects.into_iter().next() {
            self.open_file(path, cx);
        } else if !media.is_empty() && self.session.is_none() {
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
    /// Batches run in order: user order and project identity both depend on it.
    pub(crate) fn import(&mut self, request: ImportRequest, cx: &mut Context<Self>) {
        self.imports.push_back(request);
        self.next_import(cx);
    }
    fn next_import(&mut self, cx: &mut Context<Self>) {
        if self.importing.is_some() {
            return;
        }
        let (Some(published), Some(runtime)) =
            (self.published.clone(), self.builtins.runtime.clone())
        else {
            self.imports.clear();
            return;
        };
        let Some(request) = self.imports.pop_front() else {
            return;
        };
        // Close and quit wait for this fence, so an import never lands half way.
        let Some(operation) = self.mcp.try_operation() else {
            self.imports.clear();
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
            let _operation = operation;
            let result = cx
                .background_executor()
                .spawn(async move { runtime.import_media(document, &profile, request, &cancel) })
                .await;
            let _ = this.update(cx, |v, cx| {
                v.importing = None;
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
