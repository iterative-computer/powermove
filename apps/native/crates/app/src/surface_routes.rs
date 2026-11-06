//! Menu commands that open a surface. Each route captures the document it was
//! opened for, does kernel and scratch-store work off main, and refuses a stale
//! target instead of guessing. Other lanes add their own commands in `session.rs`.
use crate::{
    dialogs::{
        CompositionDraft, DialogId, WindowId,
        kernel::{Context as Dialog, DialogError},
    },
    project_library::{Outbound, ProjectLibrary, Sink, current_format},
    session::Controller,
    sheets::{OpenSheet, from_surface},
};
use gpui::{prelude::*, *};
use pm_kernel::Call;
use std::rc::Rc;

impl Controller {
    pub fn surface_command(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        match name {
            "newProject" => self.new_project(cx),
            "projects" | "openRecent" => self.show_project_library(cx),
            "newComposition" => self.composition_sheet(false, cx),
            "compositionSettings" => self.composition_sheet(true, cx),
            "precompose" => self.precompose_sheet(cx),
            command if crate::keyframe_sheets::Kind::of(command).is_some() => {
                self.keyframe_sheet(crate::keyframe_sheets::Kind::of(command).unwrap(), cx)
            }
            "migrationReport" => crate::migration_report::open(self.profile.clone(), cx),
            "errorLog" => crate::error_log::open(self.errors.clone(), cx),
            _ => return false,
        }
        true
    }
    fn library(&mut self, cx: &mut Context<Self>) -> Option<Entity<ProjectLibrary>> {
        if let Some(library) = &self.library {
            return Some(library.clone());
        }
        let this = cx.entity().downgrade();
        let sink: Sink = Rc::new(move |outbound, cx| {
            let _ = this.update(cx, |v, cx| match outbound {
                Outbound::Opened(reply) => v.adopt_reply(*reply, cx),
                Outbound::Focus => v.host.borrow().window.makeKeyAndOrderFront(None),
                Outbound::Message(text) => {
                    v.message = text;
                    cx.notify();
                }
            });
        });
        let tokens = self.host.borrow().tokens.clone();
        let (port, rx) = match ProjectLibrary::port(self.kernel.clone(), self.profile.clone()) {
            Ok(port) => port,
            Err(error) => {
                self.message = format!("Project Library is unavailable: {error}");
                return None;
            }
        };
        let library = cx.new(|cx| ProjectLibrary::new(port, rx, tokens, sink, cx));
        self.library = Some(library.clone());
        Some(library)
    }
    fn show_project_library(&mut self, cx: &mut Context<Self>) {
        if let Some(library) = self.library(cx) {
            library.update(cx, |v, cx| {
                let _ = v.controller.show();
                v.pump(cx);
            });
        }
    }
    fn new_project(&mut self, cx: &mut Context<Self>) {
        let current = current_format(self.published.as_deref());
        let owner = self
            .session
            .as_ref()
            .map_or_else(String::new, |s| s.project.clone());
        if let Some(library) = self.library(cx) {
            library.update(cx, |v, cx| {
                let _ = v.controller.refresh();
                let fallback = crate::library_surface::form::Format {
                    width: 1920,
                    height: 1080,
                    fps: 30.,
                    duration: 10.,
                    background: "#000000".into(),
                };
                let _ = v.controller.begin_new(owner, current.as_ref(), &fallback);
                v.pump(cx);
            });
        }
    }
    fn dialog_owner(&self) -> Option<(String, WindowId)> {
        let project = self.session.as_ref()?.project.clone();
        Some((project.clone(), WindowId(project)))
    }
    pub fn composition_sheet(&mut self, settings: bool, cx: &mut Context<Self>) {
        let Some((project, owner)) = self.dialog_owner() else {
            self.message = "Open a project first.".into();
            return;
        };
        let context = match Dialog::capture(&self.kernel, &project) {
            Ok(context) => context,
            Err(_) => return,
        };
        let Ok(draft) = context.composition_draft(settings) else {
            return;
        };
        let extending = context.extending_layers(draft.duration).unwrap_or(0);
        let surface = draft.surface(
            DialogId(format!("composition:{project}")),
            owner,
            settings,
            extending,
        );
        let tokens = self.host.borrow().tokens.clone();
        let open = OpenSheet::present(
            from_surface(&surface, &tokens),
            &self.host.borrow().window,
            surface.first_selected.as_deref(),
        );
        let (actions, kernel) = (open.actions.clone(), self.kernel.clone());
        let open = Rc::new(open);
        cx.spawn(async move |this, cx| {
            while let Ok(action) = actions.recv().await {
                match action.as_str() {
                    "cancel" => {
                        open.close();
                        return;
                    }
                    "apply" => {
                        let draft = read_composition(&open, &draft);
                        let (context, kernel) = (context.clone(), kernel.clone());
                        let result =
                            cx.background_executor()
                                .spawn(async move {
                                    apply_composition(&context, &kernel, &draft, settings)
                                })
                                .await;
                        open.close();
                        let _ = this.update(cx, |v, cx| match result {
                            Ok(Some((comp_id, document))) => {
                                v.call(Call::OpenComposition { comp_id, document }, cx)
                            }
                            Ok(None) => v.refresh(cx),
                            Err(error) => v.report_dialog_error(error, cx),
                        });
                        return;
                    }
                    _ => open.enable("apply", read_composition(&open, &draft).errors().is_empty()),
                }
            }
        })
        .detach();
    }
    fn precompose_sheet(&mut self, cx: &mut Context<Self>) {
        let Some((project, owner)) = self.dialog_owner() else {
            self.message = "Open a project first.".into();
            return;
        };
        let Ok(context) = Dialog::capture(&self.kernel, &project) else {
            return;
        };
        let selected = self
            .published
            .as_ref()
            .map_or(vec![], |p| p.selection.layers().to_vec());
        if selected.is_empty() {
            self.message = "Select the layers to pre-compose.".into();
            cx.notify();
            return;
        }
        let Ok(draft) = context.precompose_draft(&selected) else {
            return;
        };
        let surface = draft.surface(DialogId(format!("precompose:{project}")), owner);
        let tokens = self.host.borrow().tokens.clone();
        let open = OpenSheet::present(
            from_surface(&surface, &tokens),
            &self.host.borrow().window,
            surface.first_selected.as_deref(),
        );
        let actions = open.actions.clone();
        cx.spawn(async move |this, cx| {
            while let Ok(action) = actions.recv().await {
                if action == "cancel" || action == "apply" {
                    open.close();
                    if action == "apply" {
                        let error = context.precompose(&draft).err();
                        let _ = this.update(cx, |v, cx| {
                            if let Some(error) = error {
                                v.report_dialog_error(error, cx);
                            }
                        });
                    }
                    return;
                }
            }
        })
        .detach();
    }
    pub fn report_dialog_error(&mut self, error: DialogError, cx: &mut Context<Self>) {
        self.message = match error {
            DialogError::Invalid(text) => text.into(),
            DialogError::Stale => "The project was changed, so the dialog closed.".into(),
            DialogError::MissingBacking(_)
            | DialogError::Kernel(_)
            | DialogError::InvalidPayload => {
                "That change couldn’t be made. The project wasn’t changed.".into()
            }
        };
        self.errors.borrow_mut().push("Editor", &self.message);
        cx.notify();
    }
}
/// The sheet's fields as a draft. Unparseable numbers stay invalid, never zero.
fn read_composition(open: &OpenSheet, seed: &CompositionDraft) -> CompositionDraft {
    CompositionDraft {
        name: open.text("name"),
        width: open.number("size.0"),
        height: open.number("size.1"),
        fps: open.number("fps"),
        duration: open.number("duration"),
        background: open
            .colour("background")
            .unwrap_or_else(|| seed.background.clone()),
    }
}
/// Off main: the scratch store rebuilds the document to prove the edit.
fn apply_composition(
    context: &Dialog,
    kernel: &pm_kernel::KernelHandle,
    draft: &CompositionDraft,
    settings: bool,
) -> Result<Option<(String, u64)>, DialogError> {
    let new_id = (!settings).then(|| pm_kernel::new_id("C"));
    let edit = context.composition_edit(draft, new_id.as_deref())?;
    let document = edit.document();
    edit.submit(kernel, 0)
        .wait()
        .ok_or(DialogError::Stale)?
        .map_err(|e| DialogError::Kernel(format!("{e:?}")))?;
    Ok(new_id.map(|id| (id, document)))
}
