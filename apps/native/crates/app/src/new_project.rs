//! File ▸ New Project: the native S018 sheet, validated by `NewProjectForm`,
//! opening through the kernel's session route on the existing open path.
use crate::library_surface::{
    Format,
    appkit::new_project_sheet,
    form::{NewProject, NewProjectForm, PRESETS},
    kernel::project_value,
    sheet_layout::native::Sheet,
};
use crate::session::Controller;
use gpui::Context;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{NSObject, NSObjectProtocol},
    sel,
};
use objc2_app_kit::{
    NSColorSpace, NSColorWell, NSControl, NSPopUpButton, NSUserInterfaceItemIdentification,
};
use objc2_foundation::NSString;
use pm_kernel::session::SessionSource;

pub struct TargetState {
    controls: async_channel::Sender<String>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMNewProjectTarget"]
    #[ivars = TargetState]
    pub struct Target;
    unsafe impl NSObjectProtocol for Target {}
    impl Target {
        #[unsafe(method(control:))]
        fn control(&self, sender: &NSControl) {
            if let Some(id) = sender.identifier() {
                let _ = self.ivars().controls.try_send(id.to_string());
            }
        }
    }
);

/// The defaults of `createNewProjectForm`, for a window with no project.
pub const DEFAULT_FORMAT: (u32, u32, f64, f64, &str) = (1920, 1080, 30., 10., "#09090A");

pub fn default_format() -> Format {
    let (width, height, fps, duration, background) = DEFAULT_FORMAT;
    Format {
        width,
        height,
        fps,
        duration,
        background: background.into(),
    }
}

/// `PM.newProject`: start from the open composition's format.
pub fn form(owner: String, current: Option<Format>, names: &[String]) -> NewProjectForm {
    NewProjectForm::new(owner, current.as_ref(), &default_format(), names)
}

/// What Create opens: a fresh id and the project written from the form's values.
pub fn project(values: &NewProject) -> Option<(String, pm_model::Value)> {
    let id = pm_kernel::new_id("P");
    let project = project_value(&id, values).ok()?;
    Some((id, project))
}

struct Presented {
    sheet: Sheet,
    form: NewProjectForm,
    _target: Retained<Target>,
}
impl Presented {
    fn present(
        form: NewProjectForm,
        tokens: &pm_design::Tokens,
        window: &objc2_app_kit::NSWindow,
        controls: async_channel::Sender<String>,
    ) -> Self {
        let mtm = MainThreadMarker::new().expect("AppKit main executor");
        let sheet = new_project_sheet(&form, tokens, mtm);
        let target = Target::alloc(mtm).set_ivars(TargetState { controls });
        let target: Retained<Target> = unsafe { msg_send![super(target), init] };
        // SAFETY: `control:` exists, and this value keeps the target alive.
        unsafe { sheet.bind(&target, sel!(control:)) };
        // Return in a plain field must reach the default button, and the size
        // fields report on end of editing, which applies the aspect link.
        for (id, field) in &sheet.fields {
            let size = matches!(id.as_str(), "width" | "height");
            if size {
                field.setIdentifier(Some(&NSString::from_str(id)));
            }
            field.setContinuous(false);
            unsafe {
                if size {
                    let _: () = msg_send![&**field, setSendsActionOnEndEditing: true];
                } else {
                    field.setTarget(None);
                    field.setAction(None);
                }
            }
        }
        sheet.present(window, Some("name"));
        Self {
            sheet,
            form,
            _target: target,
        }
    }
    fn close(&self, window: &objc2_app_kit::NSWindow) {
        window.endSheet(&self.sheet.window);
    }
    fn text(&self, id: &str) -> String {
        self.sheet.fields[id].stringValue().to_string()
    }
    /// Editing fields report no change of their own; read them when it matters.
    fn read_fields(&mut self) {
        self.form.name = self.text("name");
        self.form.fps = self.text("fps");
        self.form.duration = self.text("duration");
        let (width, height) = (self.text("width"), self.text("height"));
        if width != self.form.width || height != self.form.height {
            let _ = self.form.edit_size(width, height);
        }
    }
    fn show_size(&self) {
        for (id, value) in [("width", &self.form.width), ("height", &self.form.height)] {
            self.sheet.fields[id].setStringValue(&NSString::from_str(value));
        }
    }
    fn choose_preset(&mut self) {
        let Some(popup) = self
            .sheet
            .controls
            .get("resolution")
            .and_then(|c| c.downcast_ref::<NSPopUpButton>())
        else {
            return;
        };
        let index = usize::try_from(popup.indexOfSelectedItem())
            .ok()
            .filter(|index| *index < PRESETS.len());
        let _ = self.form.choose_preset(index);
        self.show_size();
    }
    fn edit_size(&mut self, width: bool) {
        let draft = self.text(if width { "width" } else { "height" });
        let unchanged = if width {
            draft == self.form.width
        } else {
            draft == self.form.height
        };
        if !unchanged && self.form.edit_dimension(width, draft).is_ok() {
            self.show_size();
        }
    }
    fn toggle_link(&mut self) {
        self.form.linked = self.form.linked.map(|linked| !linked);
    }
    /// The well only speaks when the user chose a colour; the authored string
    /// stays exact until then.
    fn choose_background(&mut self) {
        let Some(well) = self
            .sheet
            .controls
            .get("background")
            .and_then(|c| c.downcast_ref::<NSColorWell>())
        else {
            return;
        };
        let Some(colour) = well
            .color()
            .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
        else {
            return;
        };
        let channel = |value: f64| (value.clamp(0., 1.) * 255.).round() as u8;
        self.form.background = format!(
            "#{:02X}{:02X}{:02X}",
            channel(colour.redComponent()),
            channel(colour.greenComponent()),
            channel(colour.blueComponent())
        );
    }
}

impl Controller {
    fn current_format(&self) -> Option<Format> {
        self.session.as_ref()?;
        let background = self
            .published
            .as_ref()?
            .snapshot
            .root()
            .and_then(|root| {
                let open = self.published.as_ref()?.snapshot.open_comp();
                pm_kernel::model::comp_node(root, open.as_deref())?.get("bg")
            })
            .and_then(crate::node_string)
            .map_or_else(|| default_format().background, str::to_owned);
        Some(Format {
            width: self.dimensions[0] as u32,
            height: self.dimensions[1] as u32,
            fps: self.rate.to_f64(),
            duration: self.duration,
            background,
        })
    }
    pub(crate) fn show_new_project(&mut self, cx: &mut Context<Self>) {
        if self.new_project_open {
            return;
        }
        self.new_project_open = true;
        let names: Vec<_> = self.recent.iter().map(|(_, name)| name.clone()).collect();
        let owner = self
            .session
            .as_ref()
            .map_or_else(String::new, |s| s.project.clone());
        let form = form(owner, self.current_format(), &names);
        let (tx, rx) = async_channel::unbounded();
        let host = self.host.clone();
        let (window, tokens) = {
            let host = host.borrow();
            (host.window.clone(), host.tokens.clone())
        };
        let mut presented = Presented::present(form, &tokens, &window, tx.clone());
        cx.spawn(async move |this, cx| {
            while let Ok(id) = rx.recv().await {
                match id.as_str() {
                    "cancel" => break,
                    "resolution" => presented.choose_preset(),
                    "width" => presented.edit_size(true),
                    "height" => presented.edit_size(false),
                    "link" => presented.toggle_link(),
                    "background" => presented.choose_background(),
                    "create" => {
                        presented.read_fields();
                        let Some(values) = presented.form.values() else {
                            // Show what is wrong in the sheet's own error lines.
                            presented.close(&window);
                            let form = presented.form.clone();
                            presented = Presented::present(form, &tokens, &window, tx.clone());
                            continue;
                        };
                        if let Some((id, project)) = project(&values) {
                            let _ = this
                                .update(cx, |v, cx| v.open_as(id, SessionSource::New(project), cx));
                        }
                        break;
                    }
                    _ => {}
                }
            }
            presented.close(&window);
            let _ = this.update(cx, |v, cx| {
                v.new_project_open = false;
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::{KernelActor, KernelConfig, persistence::DiskFormat, session::*};

    #[test]
    fn create_opens_a_library_project_with_the_forms_format_and_saves_it() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let actor = KernelActor::spawn(KernelConfig {
            journal_root: Some(root.join("journals")),
            ..Default::default()
        });
        let kernel = actor.handle();
        kernel
            .shell(ShellCall::AttachLibrary {
                profile: root.to_owned(),
                format: DiskFormat::V1_2,
                electron_profile: None,
            })
            .wait()
            .unwrap()
            .unwrap();
        let current = Format {
            width: 1080,
            height: 1920,
            fps: 24.,
            duration: 5.,
            background: "#112233".into(),
        };
        let mut form = form(String::new(), Some(current), &["Untitled".into()]);
        assert_eq!(form.name, "Untitled 2", "names stay unique");
        form.choose_preset(Some(1)).unwrap();
        let values = form.values().unwrap();
        let (id, project) = super::project(&values).unwrap();
        let ShellReply::Opened { session, .. } = kernel
            .shell(ShellCall::PrepareOpen {
                id: id.clone(),
                source: SessionSource::New(project),
                electron_profile: None,
            })
            .wait()
            .unwrap()
            .unwrap()
        else {
            panic!("a new project opens without recovery");
        };
        let published = kernel.published(&id).unwrap();
        let open = published.snapshot.open_comp();
        let root_node = published.snapshot.root().unwrap();
        let composition = pm_kernel::model::comp_node(root_node, open.as_deref()).unwrap();
        let number = |key: &str| composition.get(key).unwrap().to_value().as_f64().unwrap();
        assert_eq!(
            [number("w"), number("h"), number("fps"), number("dur")],
            [1920., 1080., 24., 5.]
        );
        assert_eq!(
            root_node.get("name").unwrap().to_value().as_str(),
            Some("Untitled 2")
        );
        kernel
            .shell(ShellCall::Save(session))
            .wait()
            .unwrap()
            .unwrap();
        actor.shutdown();
        assert!(
            pm_kernel::LibraryStore::open(root)
                .load(&id)
                .unwrap()
                .is_some(),
            "the new project is in the library"
        );
    }

    #[test]
    fn a_window_without_a_project_uses_the_shipped_defaults() {
        let form = form(String::new(), None, &[]);
        assert_eq!(
            (form.width.as_str(), form.height.as_str(), form.fps.as_str()),
            ("1920", "1080", "30")
        );
        assert_eq!(form.background, "#09090A");
        assert_eq!(form.preset, Some(1));
    }
}
