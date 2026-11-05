//! Kit2 platform conversion and native controls. N8: targets enqueue only;
//! Keychain access belongs to the values worker, never a native field callback.
use super::authority::{PrivateValue, ServiceCode};
use std::sync::mpsc::{SyncSender, TrySendError};

/// Binding-only secure publication: no Serialize, Display or inspectable value.
#[derive(Debug)]
pub struct SecureEdit {
    pub binding: String,
    pub value: PrivateValue,
}
pub fn enqueue_secure(sender: &SyncSender<SecureEdit>, edit: SecureEdit) -> Result<(), SecureEdit> {
    sender.try_send(edit).map_err(|error| match error {
        TrySendError::Full(edit) | TrySendError::Disconnected(edit) => edit,
    })
}
/// The existing env/Keychain worker implements this on its worker thread. A
/// caller supplies the validated env identity; no account/service name is guessed.
pub trait SecretWorker {
    fn save(&mut self, binding: &str, value: PrivateValue) -> Result<(), ServiceCode>;
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelInput {
    pub notches: f64,
    pub trackpad: bool,
    pub momentum: bool,
    pub shift: bool,
    pub option: bool,
}
impl WheelInput {
    /// AppKit precise deltas are points; discrete deltaY is in line/notch units.
    /// The scroll view's actual line increment supplies the point conversion.
    /// Preserve fractional/accelerated deltas instead of counting event packets.
    pub fn from_units(
        delta: f64,
        trackpad: bool,
        points_per_line: f64,
        momentum: bool,
        shift: bool,
        option: bool,
    ) -> Option<Self> {
        if !delta.is_finite()
            || (trackpad && (!points_per_line.is_finite() || points_per_line <= 0.))
        {
            return None;
        }
        Some(Self {
            notches: if trackpad {
                delta / points_per_line
            } else {
                delta
            },
            trackpad,
            momentum,
            shift,
            option,
        })
    }
}
#[cfg(target_os = "macos")]
pub mod appkit {
    use super::*;
    use objc2::{MainThreadMarker, MainThreadOnly, Message, msg_send, rc::Retained};
    use objc2_app_kit::*;
    use objc2_foundation::{NSRect, NSString};
    pub fn secure_field(
        binding: &str,
        label: &str,
        mtm: MainThreadMarker,
    ) -> Option<Retained<NSSecureTextField>> {
        if binding.is_empty() {
            return None;
        }
        let field = NSSecureTextField::initWithFrame(NSSecureTextField::alloc(mtm), NSRect::ZERO);
        field.setAccessibilityLabel(Some(&NSString::from_str(label)));
        field.setIdentifier(Some(&NSString::from_str(binding)));
        Some(field)
    }
    /// Read only on an originating native edit; enqueue to the worker. On a full
    /// mailbox the caller retains the exact private bytes and retries admission.
    pub fn secure_edit(field: &NSSecureTextField) -> Option<SecureEdit> {
        Some(SecureEdit {
            binding: field.identifier()?.to_string(),
            value: PrivateValue::new(field.stringValue().to_string()),
        })
    }
    /// Use the real system pop-up control to measure sample text and its chevron.
    /// No estimated character widths or substituted metrics.
    pub fn menu_width(sample: &str, tokens: &pm_design::Tokens, mtm: MainThreadMarker) -> f64 {
        let popup =
            NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
        popup.setFont(Some(&NSFont::systemFontOfSize(
            tokens.typography.body.into(),
        )));
        popup.addItemWithTitle(&NSString::from_str(sample));
        popup.sizeToFit();
        popup.frame().size.width
    }
    pub fn wheel(event: &NSEvent, scroll: &NSScrollView) -> Option<WheelInput> {
        if event.r#type() != NSEventType::ScrollWheel {
            return None;
        }
        let precise = event.hasPreciseScrollingDeltas();
        let flags = event.modifierFlags();
        WheelInput::from_units(
            if precise {
                event.scrollingDeltaY()
            } else {
                event.deltaY()
            },
            precise,
            scroll.verticalLineScroll(),
            event.momentumPhase() != NSEventPhase::None,
            flags.contains(NSEventModifierFlags::Shift),
            flags.contains(NSEventModifierFlags::Option),
        )
    }
    /// Called from the kit's OverlayVisibility result; the host schedules its
    /// monotonic idle deadline even when no JS realm is running.
    pub fn overlay_visibility(view: &NSView, visible: bool, tokens: &pm_design::Tokens) {
        let duration = if tokens.accessibility.reduce_motion {
            tokens.motion.reduced
        } else if visible {
            tokens.motion.enter
        } else {
            tokens.motion.exit
        };
        let view = view.retain();
        let changes =
            block2::RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
                let context = unsafe { context.as_ref() };
                context.setDuration(duration);
                // SAFETY: NSView's public animator proxy implements setAlphaValue:.
                unsafe {
                    let animator: Retained<objc2::runtime::AnyObject> = msg_send![&*view, animator];
                    let _: () = msg_send![&*animator,setAlphaValue:if visible {1_f64}else{0_f64}];
                }
            });
        NSAnimationContext::runAnimationGroup_completionHandler(&changes, None);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secure_queue_is_redacted_and_backpressure_retains_exact_bytes() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let edit = || SecureEdit {
            binding: "env/declared-key".into(),
            value: PrivateValue::new(" synthetic\nvalue ".into()),
        };
        assert!(!format!("{:?}", edit()).contains("synthetic"));
        enqueue_secure(&sender, edit()).unwrap();
        let retry = enqueue_secure(&sender, edit()).unwrap_err();
        assert_eq!(retry.value.expose(), " synthetic\nvalue ");
        assert_eq!(receiver.recv().unwrap().binding, "env/declared-key");
        enqueue_secure(&sender, retry).unwrap();
        drop(receiver);
        assert!(enqueue_secure(&sender, edit()).is_err());
    }
    #[test]
    fn wheel_preserves_fractional_units_and_native_modifier_momentum_facts() {
        let sample = WheelInput::from_units(6., true, 12., true, true, true).unwrap();
        assert_eq!(sample.notches, 0.5);
        assert!(sample.trackpad && sample.momentum && sample.shift && sample.option);
        assert_eq!(
            WheelInput::from_units(-1.25, false, 0., false, false, false)
                .unwrap()
                .notches,
            -1.25
        );
        for units in [0., -1., f64::NAN, f64::INFINITY] {
            assert!(WheelInput::from_units(1., true, units, false, false, false).is_none());
        }
        assert!(WheelInput::from_units(f64::NAN, false, 12., false, false, false).is_none());
    }
}
