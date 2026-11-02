//! AppKit-only attachment. The existing native/v4 renderer supplies the content
//! view; this adapter owns system popover placement, text policy and sampler.
//! AppKit input/AX/IME behavior still requires verification on macOS.
use super::{
    EditingHost,
    colour::PopoverPlan,
    text::{Setting, TextPolicy},
};
use objc2::{MainThreadMarker, msg_send, rc::Retained, sel};
use objc2_app_kit::{
    NSColor, NSColorSampler, NSPasteboard, NSPasteboardTypeString, NSPopover, NSPopoverBehavior,
    NSTextView, NSView, NSViewController,
};
use objc2_foundation::{NSRect, NSRectEdge, NSString};
use std::{collections::BTreeMap, rc::Rc};

#[derive(Clone, Copy)]
struct Defaults {
    spelling: bool,
    grammar: bool,
    correction: bool,
    quotes: bool,
    dashes: bool,
    replacement: bool,
    links: bool,
    data: bool,
    writing_tools: Option<isize>,
}
impl Defaults {
    fn capture(view: &NSTextView) -> Self {
        // SAFETY: public NSTextView selectors, on the main thread. The 15+
        // Writing Tools selector is guarded for the supported macOS 14 floor.
        unsafe {
            let has: bool = msg_send![view, respondsToSelector: sel!(writingToolsBehavior)];
            Self {
                spelling: msg_send![view, isContinuousSpellCheckingEnabled],
                grammar: msg_send![view, isGrammarCheckingEnabled],
                correction: msg_send![view, isAutomaticSpellingCorrectionEnabled],
                quotes: msg_send![view, isAutomaticQuoteSubstitutionEnabled],
                dashes: msg_send![view, isAutomaticDashSubstitutionEnabled],
                replacement: msg_send![view, isAutomaticTextReplacementEnabled],
                links: msg_send![view, isAutomaticLinkDetectionEnabled],
                data: msg_send![view, isAutomaticDataDetectionEnabled],
                writing_tools: has.then(|| msg_send![view, writingToolsBehavior]),
            }
        }
    }
    fn apply(self, view: &NSTextView, policy: TextPolicy) {
        let keep = |setting, original| setting == Setting::SystemDefault && original;
        // Restoring the captured defaults matters when an AppKit editor slot is
        // reused for a number and then a layer name. Never force spelling on.
        unsafe {
            let _: () = msg_send![view, setContinuousSpellCheckingEnabled: keep(policy.spelling, self.spelling)];
            let _: () =
                msg_send![view, setGrammarCheckingEnabled: keep(policy.grammar, self.grammar)];
            let _: () = msg_send![view, setAutomaticSpellingCorrectionEnabled: keep(policy.spelling, self.correction)];
            let _: () = msg_send![view, setAutomaticQuoteSubstitutionEnabled: keep(policy.smart_quotes, self.quotes)];
            let _: () = msg_send![view, setAutomaticDashSubstitutionEnabled: keep(policy.substitutions, self.dashes)];
            let _: () = msg_send![view, setAutomaticTextReplacementEnabled: keep(policy.substitutions, self.replacement)];
            let _: () = msg_send![view, setAutomaticLinkDetectionEnabled: keep(policy.substitutions, self.links)];
            let _: () = msg_send![view, setAutomaticDataDetectionEnabled: keep(policy.substitutions, self.data)];
            if let Some(original) = self.writing_tools {
                // NSWritingToolsBehaviorNone = -1; Default = 0. Do not touch
                // text context-menu delegates or install a replacement menu.
                let behavior = if policy.writing_tools == Setting::Off {
                    -1_isize
                } else {
                    original
                };
                let _: () = msg_send![view, setWritingToolsBehavior: behavior];
            }
        }
    }
}

struct Anchor {
    view: Retained<NSView>,
    content: Retained<NSViewController>,
    rect: NSRect,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeError {
    UnknownTextField,
    UnknownAnchor,
    ClipboardRejected,
}
type SampleReply = Rc<dyn Fn(u64, Option<Retained<NSColor>>)>;
pub struct AppKitHost {
    mtm: MainThreadMarker,
    text: BTreeMap<String, (Retained<NSTextView>, Defaults)>,
    anchors: BTreeMap<String, Anchor>,
    popovers: BTreeMap<String, Retained<NSPopover>>,
    sample_reply: SampleReply,
}
impl AppKitHost {
    pub fn new(mtm: MainThreadMarker, sample_reply: SampleReply) -> Self {
        Self {
            mtm,
            text: BTreeMap::new(),
            anchors: BTreeMap::new(),
            popovers: BTreeMap::new(),
            sample_reply,
        }
    }
    /// For NSTextField, register its actual NSTextView field editor when it
    /// becomes first responder. Canvas input uses its NSTextInputClient host.
    pub fn register_text(&mut self, id: &str, view: Retained<NSTextView>) {
        let defaults = Defaults::capture(&view);
        self.text.insert(id.into(), (view, defaults));
    }
    pub fn register_anchor(
        &mut self,
        id: &str,
        view: Retained<NSView>,
        rect: NSRect,
        content: Retained<NSViewController>,
    ) {
        self.anchors.insert(
            id.into(),
            Anchor {
                view,
                rect,
                content,
            },
        );
    }
    pub fn close(&mut self, id: &str) {
        if let Some(popover) = self.popovers.remove(id) {
            popover.close();
        }
    }
    pub fn unregister(&mut self, id: &str) {
        self.close(id);
        self.anchors.remove(id);
        if let Some((view, defaults)) = self.text.remove(id) {
            defaults.apply(&view, TextPolicy::default());
        }
    }
}
impl EditingHost for AppKitHost {
    type Error = NativeError;
    fn configure_text(&mut self, id: &str, policy: TextPolicy) -> Result<(), NativeError> {
        let (view, defaults) = self.text.get(id).ok_or(NativeError::UnknownTextField)?;
        defaults.apply(view, policy);
        Ok(())
    }
    fn colour_popover(&mut self, id: &str, plan: PopoverPlan) -> Result<(), NativeError> {
        let anchor = self.anchors.get(id).ok_or(NativeError::UnknownAnchor)?;
        let popover = self
            .popovers
            .entry(id.into())
            .or_insert_with(|| NSPopover::new(self.mtm));
        popover.setBehavior(NSPopoverBehavior::Transient);
        popover.setContentViewController(Some(&anchor.content));
        let mut size = popover.contentSize();
        size.width = plan.width as f64;
        popover.setContentSize(size);
        // The system chooses the actual side/placement. Frame/material and
        // solid token content are resolved by the native UI-kit renderer.
        popover.showRelativeToRect_ofView_preferredEdge(
            anchor.rect,
            &anchor.view,
            NSRectEdge::MaxY,
        );
        Ok(())
    }
    fn sample_colour(&mut self, request: u64) -> Result<(), NativeError> {
        let sampler = NSColorSampler::new();
        let reply = self.sample_reply.clone();
        let done = block2::RcBlock::new(move |colour: *mut NSColor| {
            // SAFETY: sampler supplies a nullable borrowed NSColor for callback.
            let colour = unsafe { Retained::retain(colour) };
            reply(request, colour);
        });
        unsafe { sampler.showSamplerWithSelectionHandler(&done) };
        Ok(())
    }
    fn copy_colour(&mut self, notation: &str) -> Result<(), NativeError> {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        if unsafe {
            pasteboard.setString_forType(&NSString::from_str(notation), NSPasteboardTypeString)
        } {
            Ok(())
        } else {
            Err(NativeError::ClipboardRejected)
        }
    }
}
