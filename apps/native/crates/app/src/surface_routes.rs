//! Menu commands that open a surface. Each route captures the document it was
//! opened for, does kernel and scratch-store work off main, and refuses a stale
//! target instead of guessing. Other lanes add their own commands in `session.rs`.
use crate::{
    dialogs::{
        CompositionDraft, DialogId, WindowId,
        kernel::{Context as Dialog, DialogError},
    },
    project_library::{Outbound, ProjectLibrary, Sink, current_format, default_format},
    session::Controller,
    sheets::{OpenSheet, from_surface},
};
use gpui::{prelude::*, *};
use pm_builtins::RESOLUTIONS;
use pm_kernel::Call;
use std::rc::Rc;

impl Controller {
    pub fn surface_command(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        match name {
            "newProject" => self.new_project(cx),
            "projects" | "openRecent" => self.show_project_library(cx),
            "newComposition" => self.composition_sheet(false, None, cx),
            "compositionSettings" => self.composition_sheet(true, None, cx),
            command if crate::keyframe_sheets::Kind::of(command).is_some() => {
                self.keyframe_sheet(crate::keyframe_sheets::Kind::of(command).unwrap(), cx)
            }
            "bringSignIns" => self.bring_sign_ins(cx),
            "migrationReport" => crate::migration_report::open(self.profile.clone(), cx),
            "errorLog" => crate::error_log::open(self.errors.clone(), cx),
            // Settings › General › Agent Notifications, when macOS says no.
            "openNotificationSettings" => {
                let id = crate::platform::bundle_id().unwrap_or_default();
                crate::platform::open_url(&format!(
                    "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={id}"
                ));
            }
            "checkForUpdates" => {
                let window = self.host.borrow().window.clone();
                crate::updates::check_now(self.profile.clone(), window, cx)
            }
            "backgroundTasks" => self.open_tasks_window(cx),
            "store" => crate::store_window::open(
                crate::store_window::Services::system(self.kernel.clone(), self.profile.clone()),
                cx,
            ),
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
                Outbound::Opened {
                    reply,
                    name,
                    file,
                    new_window: true,
                } => crate::entry::open_in_new_window(
                    crate::entry::Pending::Adopt { reply, name, file },
                    cx,
                ),
                Outbound::Opened {
                    reply, name, file, ..
                } => v.adopt_reply(*reply, name, file, cx),
                Outbound::FocusProject(id) => v.focus_project(&id, cx),
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
                let _ = v
                    .controller
                    .begin_new(owner, current.as_ref(), &default_format());
                v.pump(cx);
            });
        }
    }
    /// Settings ▸ Accounts retry. The Keychain is touched only after this click,
    /// and the library stays attached: locators go through the kernel.
    fn bring_sign_ins(&mut self, cx: &mut Context<Self>) {
        use pm_first_launch::{Config, KeychainDenied, SecretRetryOutcome, secrets};
        let Some(electron_profile) = self.electron.clone() else {
            self.message = "There’s no original app on this Mac.".into();
            cx.notify();
            return;
        };
        let config = Config {
            electron_profile,
            native_profile: self.profile.clone(),
        };
        use crate::alert::{Alert, Button};
        let rx = Alert::new(
            "Keep Your Sign-Ins",
            secrets::PROMPT,
            vec![
                Button::new("Continue", true).default(),
                Button::new("Not Now", false),
            ],
        )
        .sheet(&self.host.borrow().window);
        let kernel = self.kernel.clone();
        cx.spawn(async move |this, cx| {
            if rx.recv().await != Ok(Some(true)) {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move {
                    pm_first_launch::retry_secret_consent_in_library(
                        &config,
                        &kernel,
                        &mut secrets::MacKeychain::after_user_consent(),
                        |_| Ok(()),
                    )
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                v.message = match result {
                    Ok(SecretRetryOutcome::Complete) => "Your sign-ins came across.".into(),
                    Ok(SecretRetryOutcome::WaitingForElectron) => {
                        "Quit the original app, then try again.".into()
                    }
                    Err(error) if error.downcast_ref::<KeychainDenied>().is_some() => {
                        "Keychain access wasn’t allowed. Your sign-ins stay in the original app."
                            .into()
                    }
                    Err(_) => "Your sign-ins couldn’t be copied. Try again.".into(),
                };
                cx.notify();
            });
        })
        .detach();
    }
    fn dialog_owner(&self) -> Option<(String, WindowId)> {
        let project = self.session.as_ref()?.project.clone();
        Some((project.clone(), WindowId(project)))
    }
    /// `target` is the composition to edit; the open one when there's none.
    pub fn composition_sheet(
        &mut self,
        settings: bool,
        target: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some((project, owner)) = self.dialog_owner() else {
            self.message = "Open a project first.".into();
            cx.notify();
            return;
        };
        let Ok(mut context) = Dialog::capture(&self.kernel, &project) else {
            return;
        };
        if let Some(target) = target {
            context.composition = target;
        }
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
                    action => {
                        if action == "resolution" {
                            choose_resolution(&open, &draft);
                        }
                        let current = read_composition(&open, &draft);
                        open.select("resolution", current.resolution_index());
                        open.enable("apply", current.errors().is_empty());
                    }
                }
            }
        })
        .detach();
    }
    pub fn report_dialog_error(&mut self, error: DialogError, cx: &mut Context<Self>) {
        self.message = match error {
            DialogError::Invalid(text) => text.into(),
            DialogError::Stale => "The project was changed, so the dialog closed.".into(),
            DialogError::MissingBacking(_) | DialogError::Kernel(_) => {
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
            .changed_colour("background")
            .unwrap_or_else(|| seed.background.clone()),
    }
}
/// A resolution preset fills the size fields. `Custom` leaves them alone.
fn choose_resolution(open: &OpenSheet, seed: &CompositionDraft) {
    let mut draft = read_composition(open, seed);
    let Some((label, ..)) = open.popup("resolution").and_then(|i| RESOLUTIONS.get(i)) else {
        return;
    };
    if draft.choose_resolution(label) {
        open.set_text("size.0", &draft.width.to_string());
        open.set_text("size.1", &draft.height.to_string());
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
