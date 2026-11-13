//! Main-thread NSAlert adapter with explicit Return/default and Cancel/Escape.
//! The app attaches sheet forms using the resolved sheet_layout tokens.
//! No runModal, JS or kernel wait here.
use super::sheet_layout::native::Sheet;
use super::*;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, sel,
};
use objc2_app_kit::{
    NSButton, NSControl, NSControlStateValueOff, NSControlStateValueOn,
    NSUserInterfaceItemIdentification, NSWindow,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use std::cell::RefCell;
pub struct PrecomposeState {
    draft: PrecomposeDraft,
    sheet: RefCell<Option<Sheet>>,
    owner: Retained<NSWindow>,
    done: async_channel::Sender<Option<PrecomposeDraft>>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMPrecomposeTarget"]
    #[ivars = PrecomposeState]
    pub struct PrecomposeTarget;
    unsafe impl NSObjectProtocol for PrecomposeTarget {}
    impl PrecomposeTarget {
        #[unsafe(method(control:))]
        fn control(&self, sender: &NSControl) {
            self.respond(&sender.identifier().map(|id| id.to_string()).unwrap_or_default());
        }
    }
);
impl PrecomposeTarget {
    fn respond(&self, id: &str) {
        let state = self.ivars();
        let sheet = state.sheet.borrow();
        let Some(sheet) = sheet.as_ref() else { return };
        let on = |id: &str| {
            sheet
                .controls
                .get(id)
                .and_then(|c| {
                    c.downcast_ref::<NSButton>()
                        .map(NSButton::state)
                        .or_else(|| {
                            c.downcast_ref::<objc2_app_kit::NSSwitch>()
                                .map(|s| s.state())
                        })
                })
                .is_some_and(|state| state == NSControlStateValueOn)
        };
        let result = match id {
            "leave" | "move" => {
                sheet.controls["adjust"].setEnabled(!on("leave"));
                return;
            }
            "confirm" => Some(PrecomposeDraft {
                name: sheet.fields["name"].stringValue().to_string(),
                leave: on("leave"),
                adjust_duration: on("adjust"),
                open_after: on("open"),
                ..state.draft.clone()
            }),
            "cancel" => None,
            _ => return,
        };
        state.owner.endSheet(&sheet.window);
        let _ = state.done.try_send(result);
    }
}
/// A live sheet; hold it until `result` answers, since controls keep no target.
pub struct PrecomposeSheet {
    pub target: Retained<PrecomposeTarget>,
    pub result: async_channel::Receiver<Option<PrecomposeDraft>>,
}
/// Pre-compose as an owner-window sheet, from the reviewed surface's copy.
pub fn precompose_sheet(
    owner: &NSWindow,
    draft: &PrecomposeDraft,
    tokens: &pm_design::Tokens,
    mtm: MainThreadMarker,
) -> PrecomposeSheet {
    let surface = draft.surface(DialogId("precompose".into()), WindowId("editor".into()));
    let field = |id: &str| surface.fields.iter().find(|f| f.id == id).expect("field");
    let mut sheet = Sheet::new(&surface.title, tokens, true, mtm);
    sheet.text("name", &field("name").label, &draft.name, false, true);
    let on = |value: bool| {
        if value {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        }
    };
    let radio = |id: &str, value: bool| {
        let button = unsafe {
            NSButton::radioButtonWithTitle_target_action(
                &NSString::from_str(&field(id).label),
                None,
                None,
                mtm,
            )
        };
        button.setState(on(value));
        button.setEnabled(field(id).enabled);
        button
    };
    let leave = draft.leave && draft.can_leave;
    for (id, button) in [
        ("leave", radio("leave", leave)),
        ("move", radio("move", !leave)),
    ] {
        sheet.option(id, button.into_super());
        if let Some(text) = &field(id).description {
            sheet.note(text, text, tokens.colors.muted);
        }
    }
    for (id, value) in [
        ("adjust", draft.adjust_duration),
        ("open", draft.open_after),
    ] {
        let field = field(id);
        sheet.toggle(id, &field.label, value, field.enabled);
    }
    sheet.button("cancel", "Cancel", true, false, true);
    sheet.button("confirm", "Pre-compose", true, true, false);
    sheet.footer(&["cancel", "confirm"], None);
    let (done, result) = async_channel::bounded(1);
    let cancelled = done.clone();
    let target: Retained<PrecomposeTarget> = unsafe {
        msg_send![
            super(PrecomposeTarget::alloc(mtm).set_ivars(PrecomposeState {
                draft: draft.clone(),
                sheet: RefCell::new(None),
                owner: Retained::from(owner),
                done,
            })),
            init
        ]
    };
    // SAFETY: `control:` is implemented and the caller holds the target until
    // the result arrives, which is after the sheet ends.
    unsafe { sheet.bind(&target, sel!(control:)) };
    sheet.present(owner, Some("name"), move || {
        let _ = cancelled.try_send(None);
    });
    *target.ivars().sheet.borrow_mut() = Some(sheet);
    PrecomposeSheet { target, result }
}
