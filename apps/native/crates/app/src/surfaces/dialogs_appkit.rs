//! Main-thread NSAlert adapter with explicit Return/default and Cancel/Escape.
//! The app attaches sheet forms using the resolved sheet_layout tokens.
//! No runModal, JS or kernel wait here.
use super::sheet_layout::native::Sheet;
use super::*;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, sel,
};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSButton, NSButtonCell,
    NSControl, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags,
    NSModalResponse, NSTextField, NSUserInterfaceItemIdentification, NSWindow,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::sync::mpsc::Sender;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertInput {
    pub dialog: DialogId,
    /// Explicit button identity; Return never implies the destructive action.
    pub action: &'static str,
    pub accessory: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeError {
    NotAnAlert,
    MissingParent,
    UnsupportedAccessory,
}

/// The shell supplies real windows registered by stable owner and sheet IDs.
/// Keeping this trait separate permits headless owner-routing tests.
pub trait WindowLookup {
    fn editor(&self, id: &WindowId) -> Option<Retained<NSWindow>>;
    fn sheet(&self, id: &DialogId) -> Option<Retained<NSWindow>>;
}

/// Native alert + accessory kept alive through the asynchronous completion.
pub struct PreparedAlert {
    pub alert: Retained<NSAlert>,
    field: Option<Retained<NSTextField>>,
    surface: Surface,
}
impl PreparedAlert {
    pub fn new(
        surface: &Surface,
        tokens: &pm_design::Tokens,
        mtm: MainThreadMarker,
    ) -> Result<Self, NativeError> {
        if surface.piece != NativePiece::Alert {
            return Err(NativeError::NotAnAlert);
        }
        let alert = NSAlert::new(mtm);
        // None of these reversible editing confirmations is specified critical.
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str(&surface.title));
        if let Some(line) = &surface.line {
            alert.setInformativeText(&NSString::from_str(line));
        }
        alert.window().setDefaultButtonCell(None);
        for action in &surface.buttons {
            let button = alert.addButtonWithTitle(&NSString::from_str(action.label));
            button.setEnabled(action.enabled);
            button.setHasDestructiveAction(action.destructive);
            button.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
            if action.default {
                let cell = button
                    .cell()
                    .and_then(|c| c.downcast::<NSButtonCell>().ok());
                alert.window().setDefaultButtonCell(cell.as_deref());
            }
            // N3: the default cell owns Return; an explicit equivalent owns Esc.
            button.setKeyEquivalent(&NSString::from_str(if action.label == "Cancel" {
                "\u{1b}"
            } else if action.default {
                "\r"
            } else {
                ""
            }));
        }
        let field = if let Some(spec) = surface.fields.first() {
            if surface.fields.len() != 1 || spec.piece != NativePiece::TextField {
                return Err(NativeError::UnsupportedAccessory);
            }
            let field = NSTextField::textFieldWithString(
                &NSString::from_str(spec.values.first().map_or("", String::as_str)),
                mtm,
            );
            field.setFrame(NSRect::new(
                NSPoint::new(0., 0.),
                NSSize::new(PRESET_RENAME_ACCESSORY_WIDTH, tokens.metrics.field as f64),
            ));
            field.setAccessibilityLabel(Some(&NSString::from_str(spec.voiceover_label())));
            field.setEnabled(spec.enabled);
            alert.setAccessoryView(Some(&field));
            Some(field)
        } else {
            None
        };
        Ok(Self {
            alert,
            field,
            surface: surface.clone(),
        })
    }
    /// Resolve the Export sheet as parent when requested; never silently fall
    /// back to the editor if its sheet was dismissed/replaced.
    pub fn present(
        self,
        windows: &impl WindowLookup,
        input: Sender<AlertInput>,
    ) -> Result<(), NativeError> {
        let parent = match &self.surface.parent_sheet {
            Some(id) => windows.sheet(id),
            None => windows.editor(&self.surface.owner),
        }
        .ok_or(NativeError::MissingParent)?;
        let alert = self.alert.clone();
        let presentation = self.alert.clone();
        let selected = self.field.clone();
        let handler = block2::RcBlock::new(move |response: NSModalResponse| {
            let index = response
                .checked_sub(NSAlertFirstButtonReturn)
                .and_then(|i| usize::try_from(i).ok());
            let action = index
                .and_then(|i| self.surface.buttons.get(i))
                .map_or("cancel", |b| b.id);
            let accessory = self.field.as_ref().map(|f| f.stringValue().to_string());
            let _ = input.send(AlertInput {
                dialog: self.surface.id.clone(),
                action,
                accessory,
            });
            // Capture retains the alert until the asynchronous completion ends.
            let _ = &alert;
        });
        presentation.beginSheetModalForWindow_completionHandler(&parent, Some(&handler));
        if let Some(field) = selected {
            presentation.window().makeFirstResponder(Some(&field));
            // SAFETY: a live AppKit field, on the thread certified by MainThreadMarker.
            unsafe {
                field.selectText(None);
            }
        }
        Ok(())
    }
}

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
                .and_then(|c| c.downcast_ref::<NSButton>())
                .is_some_and(|b| b.state() == NSControlStateValueOn)
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
    let surface = draft.surface(
        DialogId("precompose".into()),
        WindowId("editor".into()),
    );
    let field = |id: &str| surface.fields.iter().find(|f| f.id == id).expect("field");
    let mut sheet = Sheet::new(&surface.title, 9, tokens, true, mtm);
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
    let checkbox = |id: &str, value: bool| {
        let button = unsafe {
            NSButton::checkboxWithTitle_target_action(
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
    for (id, button) in [("leave", radio("leave", leave)), ("move", radio("move", !leave))] {
        sheet.option(id, button.into_super());
        if let Some(text) = &field(id).description {
            sheet.line(text, tokens.typography.minimum.into(), tokens.colors.muted);
        }
    }
    sheet.option(
        "adjust",
        checkbox("adjust", draft.adjust_duration).into_super(),
    );
    sheet.option("open", checkbox("open", draft.open_after).into_super());
    sheet.button("cancel", "Cancel", true, false, true);
    sheet.button("confirm", "Pre-compose", true, true, false);
    sheet.footer(&["cancel", "confirm"], None);
    let (done, result) = async_channel::bounded(1);
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
    sheet.present(owner, Some("name"));
    *target.ivars().sheet.borrow_mut() = Some(sheet);
    PrecomposeSheet { target, result }
}

/// Actual system-alert presenter. The integration owner supplies its window
/// registry and consumes completion IDs from the native input queue.
pub struct AppKitAlertHost<'a, W> {
    pub windows: &'a W,
    pub tokens: &'a pm_design::Tokens,
    pub mtm: MainThreadMarker,
    pub input: Sender<AlertInput>,
}
impl<W: WindowLookup> AlertHost for AppKitAlertHost<'_, W> {
    type Error = NativeError;
    fn show_alert(&mut self, surface: &Surface) -> Result<(), Self::Error> {
        PreparedAlert::new(surface, self.tokens, self.mtm)?
            .present(self.windows, self.input.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_alert_controls_match_spec() {
        // Rust test threads are not the AppKit main thread. Host integration
        // schedules this probe on main; it is never a misleading Linux pass.
        let Some(mtm) = MainThreadMarker::new() else {
            eprintln!("SKIP native alert inspection: requires AppKit main thread");
            return;
        };
        let tokens = pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default());
        let surface = delete_composition(
            DialogId("delete".into()),
            WindowId("editor".into()),
            "Title",
            2,
        );
        let prepared = PreparedAlert::new(&surface, &tokens, mtm).unwrap();
        assert_eq!(prepared.alert.messageText().to_string(), surface.title);
        assert_eq!(
            prepared.alert.informativeText().to_string(),
            surface.line.unwrap()
        );
        assert_eq!(prepared.alert.alertStyle(), NSAlertStyle::Informational);
        let buttons = prepared.alert.buttons();
        assert_eq!(buttons.objectAtIndex(1).title().to_string(), "Delete");
        assert!(buttons.objectAtIndex(1).hasDestructiveAction());
        assert_eq!(
            buttons.objectAtIndex(0).keyEquivalent().to_string(),
            "\u{1b}"
        );
        assert!(prepared.alert.window().defaultButtonCell().is_some());
    }
}
