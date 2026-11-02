//! Main-thread NSAlert adapter. Sheet forms deliberately remain delegated until
//! the missing Opus metric tokens land. No runModal, JS or kernel wait here.
use super::*;
use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSEventModifierFlags,
    NSModalResponse, NSTextField, NSWindow,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
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
        for action in &surface.buttons {
            let button = alert.addButtonWithTitle(&NSString::from_str(action.label));
            button.setEnabled(action.enabled);
            button.setHasDestructiveAction(action.destructive);
            button.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
            button.setKeyEquivalent(&NSString::from_str(if action.default {
                "\r"
            } else if action.id == "cancel" {
                "\u{1b}"
            } else {
                ""
            }));
        }
        // Cancel must also respond to Esc when it owns Return. Cocoa's cancel
        // button semantics use its title in addition to the key equivalent.
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
        assert_eq!(buttons.objectAtIndex(0).title().to_string(), "Delete");
        assert!(buttons.objectAtIndex(0).hasDestructiveAction());
        assert_eq!(buttons.objectAtIndex(1).keyEquivalent().to_string(), "\r");
    }
}
