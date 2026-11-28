//! Real AppKit controls composed into GPUI's native transaction: the system
//! draws them (liquid glass on macOS 26 and later), tracks them, focuses them
//! and dims them. Chrome, sheets, Settings, popovers and panel headers host
//! these; `crate::kit` keeps drawn equivalents only where a view per control
//! cannot work (long scrolling lists, the timeline, the viewer).
//!
//! Each control sits in its own clipping wrapper above the GPUI view
//! (`NativeControlSlot`), staged with the element's bounds and the content
//! mask of the scroll region it is in, so it scrolls and clips with GPUI's
//! frame and never draws outside its panel.
use gpui::{prelude::*, *};
use gpui_macos::native_control::NativeControlSlot;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained, runtime::AnyObject, sel,
};
use objc2_app_kit::*;
use objc2_foundation::{NSDate, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use serde_json::{Value, json};
use std::{cell::Cell, rc::Rc};

/// The slider is normalized to 0..1. Its target translates this position
/// into the panel's value only when the left button releases it.
struct SliderDrag {
    start: f64,
    anchor: f64,
    points_to_value: f64,
    cancelled: bool,
}

impl SliderDrag {
    fn sample(&self, x: f64) -> Option<f64> {
        (!self.cancelled).then(|| {
            (self.start + (x - self.anchor) * self.points_to_value).clamp(0., 1.)
        })
    }

    fn cancel(&mut self) -> f64 {
        self.cancelled = true;
        self.start
    }
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn slider_right_click_restores_the_start_and_suppresses_release() {
    let mut drag = SliderDrag {
        start: 0.25,
        anchor: 25.,
        points_to_value: 1. / 100.,
        cancelled: false,
    };
    assert_eq!(drag.sample(75.), Some(0.75));
    assert_eq!(drag.cancel(), 0.25);
    assert_eq!(drag.sample(200.), None);
    assert_eq!(drag.sample(50.), None, "a release sends no change");
}

define_class!(
    #[unsafe(super(NSSlider))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMV4Slider"]
    #[ivars = ()]
    struct Slider;

    impl Slider {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if !self.isEnabled() {
                return;
            }
            let Some(window) = self.window() else {
                return;
            };
            let Some(cell) = self.cell().and_then(|cell| cell.downcast::<NSSliderCell>().ok()) else {
                return;
            };
            let start = self.doubleValue();
            let track = cell.trackRect();
            let knob = cell.knobRectFlipped(self.isFlipped());
            let travel = track.size.width - knob.size.width;
            if travel <= 0. {
                unsafe { let _: () = msg_send![super(self), mouseDown: event]; }
                return;
            }
            let start_x = self.convertPoint_fromView(event.locationInWindow(), None).x;
            let center = knob.origin.x + knob.size.width / 2.;
            let grab = start_x - center;
            let anchor = if grab.abs() <= knob.size.width / 2. { start_x } else { center };
            let direction = if self.userInterfaceLayoutDirection() == NSUserInterfaceLayoutDirection::RightToLeft {
                -1.
            } else {
                1.
            };
            let mut drag = SliderDrag {
                start,
                anchor,
                points_to_value: direction / travel,
                cancelled: false,
            };
            let sample = |drag: &SliderDrag, event: &NSEvent| {
                let x = self.convertPoint_fromView(event.locationInWindow(), None).x;
                let Some(value) = drag.sample(x) else {
                    return false;
                };
                self.setDoubleValue(value);
                self.displayIfNeeded();
                true
            };
            cell.setHighlighted(true);
            sample(&drag, event);
            // AppKit's standard slider consumes left drag/up in a nested
            // tracking loop. Include the secondary button in that same loop
            // so it can stop tracking before the slider sends its action.
            let mask = NSEventMask::LeftMouseDragged | NSEventMask::LeftMouseUp
                | NSEventMask::RightMouseDown | NSEventMask::RightMouseUp;
            let deadline = NSDate::distantFuture();
            while let Some(next) = window.nextEventMatchingMask_untilDate_inMode_dequeue(
                mask,
                Some(&deadline),
                unsafe { NSEventTrackingRunLoopMode },
                true,
            ) {
                match next.r#type() {
                    NSEventType::RightMouseDown => {
                        cell.setHighlighted(false);
                        self.setDoubleValue(drag.cancel());
                        self.displayIfNeeded();
                    }
                    NSEventType::LeftMouseDragged => {
                        sample(&drag, &next);
                    }
                    NSEventType::LeftMouseUp => {
                        if sample(&drag, &next) {
                            unsafe { self.sendAction_to(self.action(), self.target().as_deref()) };
                        }
                        break;
                    }
                    _ => {}
                }
            }
            cell.setHighlighted(false);
            NSView::setNeedsDisplay(self, true);
        }
    }
);

define_class!(
    #[unsafe(super(NSSegmentedControl))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSegmentedControl"]
    #[ivars = Cell<bool>]
    pub(crate) struct SegmentedControl;

    impl SegmentedControl {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let secondary = event.modifierFlags().contains(NSEventModifierFlags::Control);
            if !self.track_toolbar_variant(event, secondary) {
                unsafe { let _: () = msg_send![super(self), mouseDown: event]; }
            }
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            if !self.track_toolbar_variant(event, true) {
                unsafe { let _: () = msg_send![super(self), rightMouseDown: event]; }
            }
        }
    }
);

impl SegmentedControl {
    /// AppKit supplies the bezel, selection and tracking in every context.
    pub(crate) fn new(mtm: MainThreadMarker, size: NSControlSize, count: usize) -> Retained<Self> {
        let control: Retained<Self> = unsafe {
            msg_send![super(Self::alloc(mtm).set_ivars(Cell::new(false))), initWithFrame: NSRect::ZERO]
        };
        // macOS 27's Tabs role gives AppKit's neutral selection and the
        // tracking lens. The automatic value-selection role uses the accent.
        // objc2's AppKit bindings predate the role property.
        const TABS_ROLE: isize = 1; // NSSegmentedControlRoleTabs
        if control.respondsToSelector(sel!(setRole:)) {
            unsafe {
                let _: () = msg_send![&*control, setRole: TABS_ROLE];
            }
        }
        control.setSegmentCount(count as isize);
        control.setTrackingMode(NSSegmentSwitchTracking::SelectOne);
        control.setControlSize(size);
        control.setFocusRingType(NSFocusRingType::None);
        if control.respondsToSelector(sel!(setBorderShape:)) {
            control.setBorderShape(NSControlBorderShape::Capsule);
        }
        control
    }

    /// Adds a native hold menu and chevron to a fixed-width toolbar segment.
    pub(crate) fn set_variants(&self, index: isize, menu: &NSMenu) {
        self.ivars().set(true);
        self.setMenu_forSegment(Some(menu), index);
        self.setShowsMenuIndicator_forSegment(true, index);
    }

    /// Use AppKit's laid-out segment frames, which include inter-segment
    /// padding and can be wider than each cell's configured width.
    fn variant_at(&self, at: NSPoint) -> Option<(isize, NSRect, Retained<NSMenu>)> {
        if !self.ivars().get() || !self.isEnabled() || !self.mouse_inRect(at, self.bounds()) {
            return None;
        }
        let window = self.window()?;
        // The control exposes its cell as a child, and the cell exposes
        // each segment. Use the full frame AppKit reports for each segment.
        let cell = self.cell()?;
        let segments = NSAccessibility::accessibilityChildren(&*cell)?;
        for (index, segment) in segments.to_vec().into_iter().enumerate() {
            let index = index as isize;
            if index >= self.segmentCount() || !self.isEnabledForSegment(index) {
                continue;
            }
            let Some(menu) = self.menuForSegment(index) else {
                continue;
            };
            let has_frame: objc2::runtime::Bool =
                unsafe { msg_send![&*segment, respondsToSelector: sel!(accessibilityFrame)] };
            if !has_frame.as_bool() {
                continue;
            }
            let rect: NSRect = unsafe { msg_send![&*segment, accessibilityFrame] };
            let rect = self.convertRect_fromView(window.convertRectFromScreen(rect), None);
            if self.mouse_inRect(at, rect) {
                return Some((index, rect, menu));
            }
        }
        None
    }

    fn track_toolbar_variant(&self, event: &NSEvent, secondary: bool) -> bool {
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        let Some((index, rect, menu)) = self.variant_at(at) else {
            return false;
        };
        if !secondary {
            let Some(window) = self.window() else {
                return false;
            };
            let show_on_click = opens_variant_menu(
                at.x - rect.origin.x,
                rect.size.width,
                self.selectedSegment() == index,
                self.showsMenuIndicatorForSegment(index),
                false,
            );
            // The native cell has a narrower press area than its laid-out
            // frame. Include padding when tracking the variant button.
            const HOLD_SECONDS: f64 = 0.45;
            let deadline = NSDate::dateWithTimeIntervalSinceNow(HOLD_SECONDS);
            let mask = NSEventMask::LeftMouseUp | NSEventMask::LeftMouseDragged;
            while let Some(next) = window.nextEventMatchingMask_untilDate_inMode_dequeue(
                mask,
                Some(&deadline),
                unsafe { NSEventTrackingRunLoopMode },
                true,
            ) {
                let location = self.convertPoint_fromView(next.locationInWindow(), None);
                let inside = self.mouse_inRect(location, rect);
                if next.r#type() == NSEventType::LeftMouseUp {
                    if !inside {
                        return true;
                    }
                    if show_on_click {
                        break;
                    }
                    self.setSelectedSegment(index);
                    unsafe { self.sendAction_to(self.action(), self.target().as_deref()) };
                    return true;
                }
                if !inside {
                    // Give a drag between tools back to AppKit's cell.
                    window.postEvent_atStart(&next, true);
                    unsafe { let _: () = msg_send![super(self), mouseDown: event]; }
                    return true;
                }
            }
        }
        let y = if self.isFlipped() {
            rect.origin.y + rect.size.height + 2.
        } else {
            rect.origin.y - 2.
        };
        menu.popUpMenuPositioningItem_atLocation_inView(
            None,
            NSPoint::new(rect.origin.x, y),
            Some(self),
        );
        true
    }

    pub(crate) fn set_segment_image(&self, index: isize, image: &NSImage, title: &str) {
        image.setTemplate(true);
        image.setAccessibilityDescription(Some(&NSString::from_str(title)));
        self.setImage_forSegment(Some(image), index);
    }
}

/// Leave room for the indicator beside a full-size toolbar glyph. The
/// segment has this same width reserved for its indicator hit target.
pub(crate) const SEGMENT_MENU_INDICATOR: f64 = 12.;

fn opens_variant_menu(
    x: f64,
    width: f64,
    selected: bool,
    indicator: bool,
    secondary: bool,
) -> bool {
    // The toolbar's chevron sits inside the trailing padding. The
    // padding outside the chevron is still part of the tool button.
    const INDICATOR_INSET: f64 = 14.;
    let center = width - INDICATOR_INSET;
    let on_indicator = indicator && (x - center).abs() <= SEGMENT_MENU_INDICATOR / 2.;
    (0. ..width).contains(&x) && (secondary || selected || on_indicator)
}

#[cfg(test)]
mod variant_click_tests {
    use super::opens_variant_menu;

    const WIDTH: f64 = 51.;
    const POINTS: [f64; 5] = [0.5, 16., 27., 37., 50.5];

    #[test]
    fn inactive_primary_only_opens_the_chevron() {
        assert_eq!(
            POINTS.map(|x| opens_variant_menu(x, WIDTH, false, true, false)),
            [false, false, false, true, false],
        );
    }

    #[test]
    fn active_primary_opens_the_menu_across_the_whole_segment() {
        assert!(
            POINTS
                .into_iter()
                .all(|x| opens_variant_menu(x, WIDTH, true, true, false))
        );
    }

    #[test]
    fn secondary_opens_the_menu_across_the_whole_segment_in_both_states() {
        for selected in [false, true] {
            assert!(
                POINTS
                    .into_iter()
                    .all(|x| opens_variant_menu(x, WIDTH, selected, true, true))
            );
        }
    }

    #[test]
    fn the_menu_hit_area_stops_at_the_adjacent_segment() {
        assert!(!opens_variant_menu(37., WIDTH, false, false, false));
        assert!(!opens_variant_menu(WIDTH, WIDTH, true, true, true));
        assert!(!opens_variant_menu(-1., WIDTH, true, true, true));
    }
}

#[derive(Clone, Copy)]
pub enum Kind {
    Popup,
    /// A kit pop-up bezel whose searchable menu is supplied by its panel.
    PopupTrigger,
    Checkbox,
    /// One radio button; its group is the caller's.
    Radio,
    Switch,
    Push,
    /// `options` are the segments (`"label"` or `"symbol"` with a `"label"`
    /// for accessibility); the value is the selected index.
    Segmented,
    /// A slider. Its one option is its range (`{"min","max","step","log"}`);
    /// without one the value is 0–1.
    Slider,
    /// The system search field: `label` is its placeholder, the value its
    /// text, sent as it is typed. Return, Escape and Down hand the keyboard
    /// back to the window's content and are sent as `{"key": …}` (`Enter`,
    /// `Escape`, `ArrowDown`) for the view to act on.
    Search,
    /// A plain one-line field: `label` is its placeholder, the value its
    /// text, sent as it is typed. `symbol` is its content type, the hint
    /// AutoFill reads: `email`, or `code` for a one-time code (digits
    /// only, six at most: a pasted "123 456" arrives as "123456"). Return,
    /// Escape and Tab are sent as `{"key": …}` (`Enter`, `Escape`, `Tab`);
    /// the keyboard stays in the field.
    Text,
    /// AppKit's colour well in its minimal style (a swatch with a chevron
    /// that opens the system's colours): the value is `#rrggbb`.
    Well,
}
/// A slider's value range. A logarithmic one moves its track in ln(value),
/// so each equal pull multiplies the value (brightness, size).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub log: bool,
}
impl Range {
    /// From the slider's single option (`{"min","max","step","log"}`).
    pub fn of(options: &[Value]) -> Self {
        let o = options.first().cloned().unwrap_or(Value::Null);
        let n = |k: &str, d: f64| o[k].as_f64().filter(|v| v.is_finite()).unwrap_or(d);
        let (min, max) = (n("min", 0.), n("max", 1.));
        let log = o["log"] == true && min > 0. && max > min;
        Self {
            min,
            max: max.max(min),
            step: n("step", 0.).max(0.),
            log,
        }
    }
    fn position(self, value: f64) -> f64 {
        let v = value.clamp(self.min, self.max);
        if self.log { v.ln() } else { v }
    }
    fn value(self, position: f64) -> f64 {
        let v = if self.log { position.exp() } else { position };
        let v = if self.step > 0. {
            (v / self.step).round() * self.step
        } else {
            // Three significant digits: a pull lands on a value one can read.
            let digits = 2 - v.abs().max(1e-9).log10().floor() as i32;
            let scale = 10f64.powi(digits.max(0));
            (v * scale).round() / scale
        };
        v.clamp(self.min, self.max)
    }
    /// One arrow-key step: the declared step, else 1/20 of the track.
    fn nudge(self, value: f64, up: bool) -> f64 {
        let sign = if up { 1. } else { -1. };
        if self.log {
            let span = (self.max.ln() - self.min.ln()) / 20.;
            return self
                .value((self.position(value) + sign * span).clamp(self.min.ln(), self.max.ln()));
        }
        let step = if self.step > 0. {
            self.step
        } else {
            (self.max - self.min) / 20.
        };
        self.value((value + sign * step).clamp(self.min, self.max))
    }
}
/// The value VoiceOver reads: up to three significant digits, trailing
/// zeros trimmed.
fn readable(v: f64) -> String {
    let digits = (2 - v.abs().max(1e-9).log10().floor() as i32).clamp(0, 6) as usize;
    let text = format!("{v:.digits$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}
struct TargetState {
    sender: async_channel::Sender<Value>,
    /// The rows the pop-up's menu shows now: what an action's index means.
    options: std::cell::RefCell<Vec<Value>>,
    /// The pop-up's menu is open.
    tracking: std::cell::Cell<bool>,
    /// Rows (and the selected value) that came while the menu was open and
    /// differ in more than their titles: shown once the menu has closed and
    /// its action has gone out.
    pending: std::cell::RefCell<Option<(Vec<Value>, Value)>>,
    popup: std::cell::RefCell<Option<Retained<NSPopUpButton>>>,
    /// Rows the host lists when the menu opens (`NativeControl::fill_rows`):
    /// the pop-up shows only its current row until then.
    fill: std::cell::RefCell<Option<RowSource>>,
    kind: Kind,
    /// The window's content view: where the keyboard goes back to.
    content: Option<Retained<NSView>>,
    /// A one-time code field: digits only, six at most.
    digits: bool,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[name="PMV4ControlTarget"]
    #[ivars=TargetState]
    struct Target;
    unsafe impl NSObjectProtocol for Target{}
    impl Target{
        #[unsafe(method(act:))]
        fn act(&self,sender:&AnyObject){
            let value=match self.ivars().kind{
                Kind::Popup=>{let index:isize=unsafe{msg_send![sender,indexOfSelectedItem]};let pulls:bool=unsafe{msg_send![sender,pullsDown]};let index=index-isize::from(pulls);self.ivars().options.borrow().get(index as usize).map(|o|row_value(o).clone()).unwrap_or(Value::Null)},
                Kind::Checkbox|Kind::Radio|Kind::Switch=>{let state:isize=unsafe{msg_send![sender,state]};json!(state!=0)},
                Kind::Segmented=>{let index:isize=unsafe{msg_send![sender,selectedSegment]};json!(index)},
                Kind::Slider=>{let position:f64=unsafe{msg_send![sender,doubleValue]};json!(Range::of(&self.ivars().options.borrow()).value(position))},
                Kind::Push|Kind::PopupTrigger=>Value::Null,
                Kind::Well=>sender.downcast_ref::<NSColorWell>().map_or(Value::Null,|w|json!(colour_hex(&w.color()))),
                Kind::Text=>Value::Null,
                Kind::Search=>{let text:Retained<NSString>=unsafe{msg_send![sender,stringValue]};let text=text.to_string();if let Some(field)=sender.downcast_ref::<NSView>(){search_hint(field,text.is_empty());}json!(text)},
            };
            if matches!(self.ivars().kind,Kind::Text){return;}
            let _=self.ivars().sender.try_send(value);
        }
        /// A plain field's text, as it is typed (NSControlTextEditingDelegate).
        #[unsafe(method(controlTextDidChange:))]
        fn text_changed(&self,note:&objc2_foundation::NSNotification){
            if !matches!(self.ivars().kind,Kind::Text){return;}
            let Some(field)=note.object().and_then(|o|o.downcast::<NSTextField>().ok()) else{return};
            let typed=field.stringValue().to_string();
            let text=if self.ivars().digits{typed.chars().filter(char::is_ascii_digit).take(6).collect()}else{typed.clone()};
            if text!=typed{field.setStringValue(&NSString::from_str(&text));}
            let _=self.ivars().sender.try_send(json!(text));
        }
        #[unsafe(method(menuBegan:))]
        fn menu_began(&self,_:&AnyObject){self.ivars().tracking.set(true);}
        /// The action of the row that was chosen goes out after this, so
        /// the rows it indexes stay until the next turn of the run loop.
        #[unsafe(method(menuEnded:))]
        fn menu_ended(&self,_:&AnyObject){
            self.ivars().tracking.set(false);
            if self.ivars().pending.borrow().is_some(){
                unsafe{let _:()=msg_send![self,performSelector:sel!(showPending),withObject:std::ptr::null::<AnyObject>(),afterDelay:0.0f64];}
            }
        }
        /// The menu is about to show (NSMenuDelegate): a pop-up whose rows
        /// the host lists takes them now, with its current row selected.
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self,_:&AnyObject){
            let Some(fill)=self.ivars().fill.borrow().clone() else{return};
            let Some(popup)=self.ivars().popup.borrow().clone() else{return};
            fill_now(self,&popup,&fill);
        }
        #[unsafe(method(showPending))]
        fn show_pending(&self){
            let popup=self.ivars().popup.borrow().clone();
            if self.ivars().tracking.get(){return;}
            if let (Some(popup),Some((options,value)))=(popup,self.ivars().pending.take()){
                replace_rows(&popup,&options);
                if let Some(index)=options.iter().position(|o|row_value(o)==&value){popup.selectItemAtIndex(index as isize);}
                *self.ivars().options.borrow_mut()=options;
            }
        }
        /// A search field's editor: Return, Escape and Down give the
        /// keyboard back to the window's content and tell the view.
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn command(&self,control:&NSControl,_editor:&NSTextView,command:objc2::runtime::Sel)->bool{
            let text=matches!(self.ivars().kind,Kind::Text);
            let key=if command==sel!(cancelOperation:){Some("Escape")}
                else if command==sel!(insertNewline:){Some("Enter")}
                else if command==sel!(moveDown:)&&!text{Some("ArrowDown")}
                else if command==sel!(insertTab:)&&text{Some("Tab")}
                else{None};
            // A plain field keeps the keyboard, but for Tab; a search gives it back.
            if let Some(key)=key{
                if (!text||key=="Tab")&&let Some(window)=control.window(){
                    window.makeFirstResponder(self.ivars().content.as_deref().map(AsRef::as_ref));
                }
                let _=self.ivars().sender.try_send(json!({"key":key}));
            }
            key.is_some()
        }
    }
);
define_class!(
    #[unsafe(super(NSPopUpButton, NSButton, NSControl, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSearchablePopupTrigger"]
    #[ivars = ()]
    struct SearchablePopupTrigger;
    unsafe impl NSObjectProtocol for SearchablePopupTrigger {}
    impl SearchablePopupTrigger {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _: &NSEvent) { self.fire(); }
        #[unsafe(method(performClick:))]
        fn perform_click(&self, _: Option<&AnyObject>) { self.fire(); }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if matches!(event.keyCode(), 0x31 | 0x24 | 0x4c | 0x7d | 0x7e) {
                self.fire();
            } else {
                unsafe { let _: () = msg_send![super(self), keyDown: event]; }
            }
        }
    }
);
impl SearchablePopupTrigger {
    fn fire(&self) {
        if !self.isEnabled() {
            return;
        }
        let target: *mut AnyObject = unsafe { msg_send![self, target] };
        if !target.is_null() {
            unsafe {
                let _: () = msg_send![target, act: self];
            }
        }
    }
}

enum Control {
    Popup(Retained<NSPopUpButton>),
    Button(Retained<NSButton>),
    Switch(Retained<NSSwitch>),
    Segmented(Retained<SegmentedControl>),
    Slider(Retained<NSSlider>),
    Search(Retained<NSSearchField>),
    Text(Retained<NSTextField>),
    Well(Retained<NSColorWell>),
}
impl Control {
    fn view(&self) -> &NSView {
        match self {
            Self::Popup(v) => v.as_ref(),
            Self::Button(v) => v.as_ref(),
            Self::Switch(v) => v.as_ref(),
            Self::Segmented(v) => v,
            Self::Slider(v) => v.as_ref(),
            Self::Search(v) => v.as_ref(),
            Self::Text(v) => v.as_ref(),
            Self::Well(v) => v.as_ref(),
        }
    }
    fn enabled(&self, enabled: bool) {
        match self {
            Self::Popup(v) => v.setEnabled(enabled),
            Self::Button(v) => v.setEnabled(enabled),
            Self::Switch(v) => v.setEnabled(enabled),
            Self::Segmented(v) => v.setEnabled(enabled),
            Self::Slider(v) => v.setEnabled(enabled),
            Self::Search(v) => v.setEnabled(enabled),
            Self::Text(v) => v.setEnabled(enabled),
            Self::Well(v) => v.setEnabled(enabled),
        }
    }
    fn value(&self, value: &Value, options: &[Value]) {
        match self {
            Self::Popup(v) if v.pullsDown() => {
                // A value no row has titles the button itself (Publish).
                let title = options
                    .iter()
                    .find(|o| row_value(o) == value)
                    .map(row_title)
                    .unwrap_or_else(|| NSString::from_str(value.as_str().unwrap_or("")));
                if let Some(item) = v.itemAtIndex(0) {
                    item.setTitle(&title);
                }
                // The value's row is ticked (a pull-down selects none).
                for (i, option) in options.iter().enumerate() {
                    if let Some(item) = v.itemAtIndex(i as isize + 1) {
                        item.setState(if row_value(option) == value {
                            NSControlStateValueOn
                        } else {
                            NSControlStateValueOff
                        });
                    }
                }
            }
            Self::Popup(v) => {
                if let Some(index) = options.iter().position(|o| row_value(o) == value) {
                    v.selectItemAtIndex(index as isize);
                } else if options.iter().any(|o| o.get("checked").is_some()) {
                    // A settings pull-down has no single value: no row is
                    // "selected", only the checked ones are marked.
                    v.selectItemAtIndex(-1);
                }
                checks(v, options);
            }
            Self::Button(v) => v.setState(if value == &Value::Bool(true) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            }),
            Self::Switch(v) => v.setState(if value == &Value::Bool(true) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            }),
            Self::Segmented(v) => {
                let selected = value.as_i64().map_or(-1, |i| i as isize);
                if v.selectedSegment() != selected {
                    v.setSelectedSegment(selected);
                }
            }
            Self::Well(v) => {
                if let Some(colour) = value.as_str().and_then(hex_colour) {
                    v.setColor(&colour);
                }
            }
            Self::Slider(v) => {
                let range = Range::of(options);
                v.setMinValue(range.position(range.min));
                v.setMaxValue(range.position(range.max));
                let n = value.as_f64().unwrap_or(range.min);
                v.setDoubleValue(range.position(n));
                // VoiceOver reads the value, not a logarithmic position.
                v.setAccessibilityValueDescription(Some(&NSString::from_str(&readable(n))));
            }
            // The field owns its text while the keyboard is in it (an echo
            // of an earlier keystroke must not replace a later one); at rest
            // it shows the view's text.
            Self::Search(v) => {
                let text = value.as_str().unwrap_or("");
                if v.currentEditor().is_none() && v.stringValue().to_string() != text {
                    v.setStringValue(&NSString::from_str(text));
                }
                search_hint(v, v.stringValue().is_empty());
            }
            // As a search field: the typed text stands while it is typed in.
            Self::Text(v) => {
                let text = value.as_str().unwrap_or("");
                if v.currentEditor().is_none() && v.stringValue().to_string() != text {
                    v.setStringValue(&NSString::from_str(text));
                }
            }
        }
    }
}
/// A prominent search field's shortcut hint stands at its trailing end while
/// the field is empty (the clear button takes that place once it is not).
const SEARCH_HINT: isize = 0x4846;
fn add_search_hint(
    v: &NSSearchField,
    shortcut: &str,
    font: f32,
    inset: f64,
    mtm: MainThreadMarker,
) {
    let hint = NSTextField::labelWithString(&NSString::from_str(shortcut), mtm);
    hint.setFont(Some(&NSFont::systemFontOfSize(f64::from(font))));
    hint.setTextColor(Some(&NSColor::tertiaryLabelColor()));
    hint.setTag(SEARCH_HINT);
    hint.setAccessibilityElement(false);
    hint.sizeToFit();
    let (field, own) = (v.frame().size, hint.frame().size);
    hint.setFrameOrigin(NSPoint::new(
        field.width - own.width - inset,
        ((field.height - own.height) / 2.).round(),
    ));
    hint.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewMinXMargin
            | NSAutoresizingMaskOptions::ViewMinYMargin
            | NSAutoresizingMaskOptions::ViewMaxYMargin,
    );
    v.addSubview(&hint);
}

fn search_hint(field: &NSView, empty: bool) {
    if let Some(hint) = field.viewWithTag(SEARCH_HINT) {
        hint.setHidden(!empty);
    }
}
#[derive(Clone, Copy, PartialEq)]
pub struct BuildSnapshot {
    pub revision: Option<u64>,
    pub ready: Option<f32>,
    /// A row filters its label and snapshot together; standalone controls filter here.
    pub filter: Option<Filter>,
}
/// Shared boundary for live GPUI content and build snapshots.
/// Row-owned filters remain on the ancestor when `filter` is None.
pub(crate) fn reveal_paint(build: Option<BuildSnapshot>) -> (Vec<Filter>, f32) {
    let filters = build
        .and_then(|b| b.filter)
        .filter(|f| !f.is_identity())
        .into_iter()
        .collect();
    let alpha = if build.is_some_and(|b| b.revision.is_none() && b.ready.is_none()) {
        0.
    } else {
        1.
    };
    (filters, alpha)
}
fn capture_key(bounds: Bounds<Pixels>, revision: u64) -> (u64, [u32; 4]) {
    (
        revision,
        [
            0,
            0,
            f32::from(bounds.size.width).to_bits(),
            f32::from(bounds.size.height).to_bits(),
        ],
    )
}

#[derive(Default)]
pub(crate) struct BuildPixels {
    key: Option<(u64, [u32; 4])>,
    image: Option<std::sync::Arc<RenderImage>>,
}
impl BuildPixels {
    pub(crate) fn texture(&self, revision: Option<u64>) -> Option<std::sync::Arc<RenderImage>> {
        revision
            .and_then(|r| self.key.filter(|(key, _)| *key == r))
            .and(self.image.clone())
    }
    pub(crate) fn captured(&self) -> bool {
        self.image.is_some()
    }
    pub(crate) fn attempted(&self, revision: u64) -> bool {
        self.key.is_some_and(|(key, _)| key == revision)
    }
    pub(crate) fn capture(
        &mut self,
        native: &NSView,
        bounds: Bounds<Pixels>,
        revision: u64,
    ) -> bool {
        let key = capture_key(bounds, revision);
        if self.key == Some(key) {
            return false;
        }
        native.setFrameSize(NSSize::new(
            f32::from(bounds.size.width) as f64,
            f32::from(bounds.size.height) as f64,
        ));
        // cacheDisplay draws into its own bitmap; presentation alpha remains
        // owned exclusively by the staged native frame transaction.
        let rect = native.bounds();
        self.image = native
            .bitmapImageRepForCachingDisplayInRect(rect)
            .and_then(|rep| {
                native.cacheDisplayInRect_toBitmapImageRep(rect, &rep);
                snapshot_bitmap(&rep)
            });
        self.key = Some(key);
        true
    }
}
/// PNG decoding normalizes AppKit bitmap layouts; GPUI sprites use straight BGRA.
pub fn snapshot_bitmap(rep: &NSBitmapImageRep) -> Option<std::sync::Arc<RenderImage>> {
    snapshot_bitmap_on_ground(rep, None)
}
fn snapshot_bitmap_on_ground(
    rep: &NSBitmapImageRep,
    ground: Option<u32>,
) -> Option<std::sync::Arc<RenderImage>> {
    let properties = objc2_foundation::NSDictionary::new();
    let data =
        unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties) }?;
    let mut pixels = image::load_from_memory(unsafe { data.as_bytes_unchecked() })
        .ok()?
        .to_rgba8();
    for pixel in pixels.pixels_mut() {
        if let Some(ground) = ground {
            let matches = [(ground >> 16) as u8, (ground >> 8) as u8, ground as u8];
            if pixel.0[..3]
                .iter()
                .zip(matches)
                .all(|(a, b)| a.abs_diff(b) <= 2)
            {
                pixel.0[3] = 0;
            }
        }
        pixel.0.swap(0, 2);
    }
    if pixels.pixels().all(|p| p.0[3] == 0) {
        return None;
    }
    Some(std::sync::Arc::new(RenderImage::new([image::Frame::new(
        pixels,
    )])))
}
pub fn snapshot_image(image: &NSImage) -> Option<std::sync::Arc<RenderImage>> {
    let data = image.TIFFRepresentation()?;
    let rep = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &data)?;
    snapshot_bitmap(&rep)
}
/// The normal panel ground is painted separately, so a WK body contributes
/// its actual controls rather than turning an opaque background into hot dots.
pub fn snapshot_image_on_ground(
    image: &NSImage,
    ground: u32,
) -> Option<std::sync::Arc<RenderImage>> {
    let data = image.TIFFRepresentation()?;
    let rep = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &data)?;
    snapshot_bitmap_on_ground(&rep, Some(ground))
}
/// `rgb` (0xRRGGBB) as an sRGB colour.
fn srgb(rgb: u32) -> Retained<NSColor> {
    let channel = |shift: u32| ((rgb >> shift) & 255) as f64 / 255.;
    NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), 1.)
}

pub struct NativeControl {
    /// A push button's accent, worn only while it is enabled.
    tint: Option<u32>,
    build: Option<BuildSnapshot>,
    build_pixels: Rc<std::cell::RefCell<BuildPixels>>,
    control: Control,
    slot: Rc<NativeControlSlot>,
    _target: Retained<Target>,
    width: f32,
    height: f32,
    inline: bool,
    /// The least width of a pop-up in a row (`value_column`): it is never
    /// narrower, and wider when its widest row needs it.
    least: f32,
    /// Fills a slot that may be narrower than `width` (its node has
    /// `flexShrink`); AppKit then truncates the title.
    pub fill: bool,
    /// A segmented control as wide as its row, in equal segments.
    stretch: bool,
    /// The keyboard was asked for before the control was in its window.
    focus: Rc<std::cell::Cell<bool>>,
    /// Tab is ordered by GPUI, then AppKit takes the segmented control's keys.
    tab_focus: Option<FocusHandle>,
    tab_subscription: Option<Subscription>,
    /// The last text the view's host set (`host_text`).
    host_revision: Option<u64>,
    menu_open: bool,
    /// A search field's last value from its view. The view's value can lag
    /// what is typed (an echo of the typing commits nothing), so only a new
    /// value replaces the field's text; an unchanged one is left alone
    /// (PM-342: a panel laid out again showed the search empty while its
    /// list stayed filtered).
    search_value: Option<Value>,
    /// How opaque AppKit draws the control (a code field drawn by its view
    /// is 0: it keeps the keyboard, pastes and VoiceOver).
    pub alpha: f32,
    /// A push button at work: its title, kept while a spinner stands in it.
    busy: Option<(Retained<NSString>, Retained<NSProgressIndicator>)>,
}
impl EventEmitter<Value> for NativeControl {}

impl NativeControl {
    pub fn set_build_snapshot(&mut self, build: Option<BuildSnapshot>) -> bool {
        let build = build.filter(|b| b.ready.unwrap_or(0.) < 1.);
        if self.build == build {
            return false;
        }
        if build.is_none() {
            *self.build_pixels.borrow_mut() = Default::default();
        }
        self.build = build;
        true
    }
    pub fn snapshot_pending(&self) -> bool {
        !self.slot.wrapper().isHidden()
            && self.build.and_then(|b| b.revision).is_some_and(|r| {
                self.build_pixels
                    .borrow()
                    .key
                    .is_none_or(|(key, _)| key != r)
            })
    }
    /// The width the control fits its title in.
    pub fn width(&self) -> f32 {
        self.width
    }
    pub fn height(&self) -> f32 {
        self.height
    }
}

pub struct ControlSpec<'a> {
    pub kind: Kind,
    pub label: &'a str,
    pub identifier: &'a str,
    pub value: &'a Value,
    pub options: Vec<Value>,
    pub disabled: bool,
    pub default: bool,
    pub destructive: bool,
    /// A borderless pop-up sized to its title, for panel bars (the viewer's
    /// zoom and resolution); rows and sheets keep the bezel at `value_column`.
    pub inline: bool,
    /// An inline pop-up titled in the secondary ink: an option under the
    /// content it qualifies (a composer's option bar, a list's sort). A bar
    /// of controls keeps the primary ink, so its titles never read disabled.
    pub quiet: bool,
    /// A pull-down (the menu drops below a title that names the value, as
    /// the Project header's picker): its first item is the title.
    pub pull: bool,
    /// The view's one accent action: a push button tinted with this colour.
    pub tint: Option<u32>,
    /// A push button's leading SF Symbol. On a search field, its shortcut
    /// (`⌘F`): the field is then the view's one prominent search, drawn large
    /// with the shortcut at its trailing end while it is empty.
    pub symbol: Option<&'a str>,
    /// A push button with the system's liquid glass bezel at the large size
    /// (macOS 26 and later; the plain bezel before): a surface's one raised
    /// action. A segmented control in the system's capsule at the large
    /// size: a header's one mode switch.
    pub glass: bool,
    /// In a row's value column: as high as a value well. A pop-up has no
    /// bezel there and fills the well its row draws under it, so it is the
    /// same well as the fields beside it; a segmented control is the small
    /// size.
    pub well: bool,
}

/// What a button's or a switch's size follows: one made with the same
/// look is the same size, so it is measured once (`measured`).
#[derive(Clone, PartialEq, Eq, Hash)]
struct Look {
    kind: u8,
    title: String,
    symbol: Option<String>,
    glass: bool,
    font: u32,
}
thread_local! {
    static MEASURED: std::cell::RefCell<std::collections::HashMap<Look, (NSSize, NSSize)>> =
        std::cell::RefCell::default();
}
/// A button's frame as `sizeToFit` sets it and its fitting size, measured
/// the first time its look is made: each measure is a layout pass on
/// macOS 27 (about two milliseconds), and Settings › Shortcuts alone makes
/// a few hundred buttons of two looks.
fn measured(look: Look, control: &NSControl) -> (NSSize, NSSize) {
    if let Some(sizes) = MEASURED.with_borrow(|m| m.get(&look).copied()) {
        return sizes;
    }
    control.sizeToFit();
    let sizes = (control.frame().size, control.fittingSize());
    MEASURED.with_borrow_mut(|m| m.insert(look, sizes));
    sizes
}
/// The same controls that GPUI hosts, placed directly in an AppKit form.
pub struct FormControl {
    control: Control,
    target: Retained<Target>,
    seen: std::cell::RefCell<Option<(Value, Vec<Value>, bool)>>,
}

impl FormControl {
    pub fn new(
        spec: &ControlSpec<'_>,
        tokens: &pm_design::Tokens,
        sender: async_channel::Sender<Value>,
    ) -> Self {
        let (control, target) = make_control(spec, tokens, None, true, sender);
        Self {
            control,
            target,
            seen: Default::default(),
        }
    }

    pub fn view(&self) -> &NSView {
        self.control.view()
    }

    pub fn update(&self, value: &Value, options: &[Value], disabled: bool) {
        if self
            .seen
            .borrow()
            .as_ref()
            .is_some_and(|(old_value, old_options, old_disabled)| {
                old_value == value && old_options == options && *old_disabled == disabled
            })
        {
            return;
        }
        *self.seen.borrow_mut() = Some((value.clone(), options.to_vec(), disabled));
        self.control.enabled(!disabled);
        match &self.control {
            Control::Popup(popup) if !popup.pullsDown() => {
                set_rows(&self.target, popup, options, value);
            }
            Control::Search(field) if field.currentEditor().is_some() => {}
            Control::Text(field) if field.currentEditor().is_some() => {}
            _ => self.control.value(value, options),
        }
    }

    pub fn set_title(&self, title: &str) {
        if let Control::Button(button) = &self.control {
            let title = NSString::from_str(title);
            if button.title() != title {
                button.setTitle(&title);
            }
        }
    }

    pub fn set_placeholder(&self, text: &str) {
        if let Control::Search(field) = &self.control {
            field.setPlaceholderString(Some(&NSString::from_str(text)));
        }
    }
}

fn make_control(
    spec: &ControlSpec<'_>,
    t: &pm_design::Tokens,
    content: Option<Retained<NSView>>,
    form: bool,
    sender: async_channel::Sender<Value>,
) -> (Control, Retained<Target>) {
    let ControlSpec {
        kind,
        label,
        identifier,
        value,
        ref options,
        disabled,
        default,
        destructive,
        inline,
        quiet,
        pull,
        tint,
        symbol,
        glass,
        well,
    } = *spec;
    let body_size = if form { 13. } else { t.typography.body as f64 };
    let glass = !form && glass && objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some();
    let mtm = MainThreadMarker::new().unwrap();

    let target: Retained<Target> = unsafe {
        msg_send![
            super(Target::alloc(mtm).set_ivars(TargetState {
                sender,
                options: std::cell::RefCell::new(options.clone()),
                tracking: std::cell::Cell::new(false),
                pending: std::cell::RefCell::new(None),
                popup: std::cell::RefCell::new(None),
                fill: std::cell::RefCell::new(None),
                kind,
                content,
                digits: matches!(kind, Kind::Text) && symbol == Some("code"),
            })),
            init
        ]
    };
    let control = match kind {
        Kind::Popup | Kind::PopupTrigger => {
            let v = if matches!(kind, Kind::PopupTrigger) {
                let trigger: Retained<SearchablePopupTrigger> = unsafe {
                    msg_send![super(SearchablePopupTrigger::alloc(mtm).set_ivars(())), initWithFrame: NSRect::ZERO, pullsDown: pull]
                };
                Retained::into_super(trigger)
            } else {
                NSPopUpButton::initWithFrame_pullsDown(
                    NSPopUpButton::alloc(mtm),
                    NSRect::ZERO,
                    pull,
                )
            };
            add_items(&v, options);
            unsafe {
                v.setTarget(Some(&target));
                v.setAction(Some(sel!(act:)));
            }
            watch_menu(&target, &v);
            let font = if pull && inline {
                // A header's title (borderless): the name it heads, in
                // its type. A bezelled pull-down (Publish) is a button.
                unsafe {
                    NSFont::systemFontOfSize_weight(
                        t.typography.heading as f64,
                        objc2_app_kit::NSFontWeightSemibold,
                    )
                }
            } else {
                NSFont::systemFontOfSize(body_size)
            };
            v.setFont(Some(&font));
            if pull && inline {
                // The title is a heading; its menu lists in menu type.
                if let Some(menu) = v.menu() {
                    unsafe { menu.setFont(Some(&NSFont::menuFontOfSize(0.))) };
                }
            }
            if form {
                v.setBezelStyle(NSBezelStyle::Push);
            }
            if inline || well {
                v.setBordered(false);
            }
            if inline && quiet {
                // An option under the content it qualifies: a caption,
                // never the content's own ink.
                v.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
            }
            // A pop-up in a row's well fills the well: its own size is
            // never used, and measuring it is a layout pass of its own.
            if !well {
                v.sizeToFit();
            }
            Control::Popup(v)
        }
        Kind::Segmented => {
            let size = if well {
                NSControlSize::Small
            } else if glass {
                NSControlSize::Large
            } else {
                NSControlSize::Regular
            };
            let v = SegmentedControl::new(mtm, size, options.len());
            for (i, option) in options.iter().enumerate() {
                let title = option
                    .get("label")
                    .and_then(Value::as_str)
                    .or_else(|| option.as_str())
                    .unwrap_or("");
                // A glyph segment: a template symbol the system tints,
                // its name as the tip; a vendored glyph where one stands
                // for the symbol (`crate::icons`).
                let glyph = option.get("symbol").and_then(Value::as_str).and_then(|name| {
                    if let Some(image) = crate::icons::glyph(name).and_then(|g| {
                        crate::native::glyph_image(g, t.metrics.symbol_panel as f64, title)
                    }) {
                        return Some(image);
                    }
                    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                        &NSString::from_str(name),
                        Some(&NSString::from_str(title)),
                    )?;
                    let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(
                        t.metrics.symbol_panel as f64,
                        unsafe { NSFontWeightRegular },
                    );
                    Some(image.imageWithSymbolConfiguration(&config).unwrap_or(image))
                });
                let index = i as isize;
                if let Some(image) = glyph {
                    v.set_segment_image(index, &image, title);
                } else {
                    v.setLabel_forSegment(&NSString::from_str(title), index);
                }
                let tip = option.get("tip").and_then(Value::as_str).unwrap_or(title);
                v.setToolTip_forSegment(Some(&NSString::from_str(tip)), index);
                v.setEnabled_forSegment(
                    option.get("disabled") != Some(&Value::Bool(true)),
                    index,
                );
            }
            unsafe {
                v.setTarget(Some(&target));
                v.setAction(Some(sel!(act:)));
            }
            v.sizeToFit();
            Control::Segmented(v)
        }
        Kind::Slider => {
            let v: Retained<Slider> = unsafe {
                msg_send![super(Slider::alloc(mtm).set_ivars(())), initWithFrame: NSRect::ZERO]
            };
            v.setMinValue(0.);
            v.setMaxValue(1.);
            v.setContinuous(false);
            unsafe {
                v.setTarget(Some(&target));
                v.setAction(Some(sel!(act:)));
            }
            Control::Slider(Retained::into_super(v))
        }
        Kind::Checkbox | Kind::Radio | Kind::Push => {
            // Not `buttonWithTitle:target:action:`, which sizes the
            // button as it makes it: a measure is a layout pass of its
            // own on macOS 27, and a list of rows makes the same button
            // hundreds of times (`measured`).
            let v = NSButton::initWithFrame(NSButton::alloc(mtm), NSRect::ZERO);
            unsafe {
                v.setTarget(Some(&target));
                v.setAction(Some(sel!(act:)));
            }
            v.setTitle(&NSString::from_str(
                if matches!(kind, Kind::Checkbox | Kind::Radio) {
                    ""
                } else {
                    label
                },
            ));
            if matches!(kind, Kind::Checkbox) {
                v.setButtonType(NSButtonType::Switch);
            } else if matches!(kind, Kind::Radio) {
                v.setButtonType(NSButtonType::Radio);
            } else {
                v.setButtonType(NSButtonType::MomentaryPushIn);
                v.setBezelStyle(if glass {
                    NSBezelStyle::Glass
                } else {
                    NSBezelStyle::Push
                });
                if glass {
                    v.setControlSize(NSControlSize::Large);
                }
                v.setHasDestructiveAction(destructive);
                if let Some(image) = symbol.and_then(|name| {
                    crate::icons::glyph(name)
                        .and_then(|g| {
                            crate::native::glyph_image(g, t.metrics.symbol_panel as f64, "")
                        })
                        .or_else(|| {
                            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                                &NSString::from_str(name),
                                None,
                            )
                        })
                }) {
                    v.setImage(Some(&image));
                    v.setImagePosition(NSCellImagePosition::ImageLeading);
                }
                // A disabled tinted button is the system's plain grey one
                // until it can be pressed (a dimmed accent reads as muddy).
                if let Some(rgb) = tint.filter(|_| !disabled) {
                    v.setBezelColor(Some(&srgb(rgb)));
                }
                if default {
                    v.setKeyEquivalent(&NSString::from_str("\r"));
                }
            }
            v.setFont(Some(&NSFont::systemFontOfSize(body_size)));
            Control::Button(v)
        }
        Kind::Search => {
            let v = NSSearchField::initWithFrame(NSSearchField::alloc(mtm), NSRect::ZERO);
            v.setPlaceholderString(Some(&NSString::from_str(label)));
            v.setFont(Some(&NSFont::systemFontOfSize(body_size)));
            v.setSendsSearchStringImmediately(true);
            v.setSendsWholeSearchString(false);
            unsafe {
                v.setTarget(Some(&target));
                v.setAction(Some(sel!(act:)));
                if !form {
                    let _: () = msg_send![&*v, setDelegate: &*target];
                }
            }
            v.sizeToFit();
            // A view's one prominent search (Home): the large field, with
            // its shortcut at the trailing end while it is empty.
            if let Some(shortcut) = symbol {
                v.setControlSize(NSControlSize::Large);
                v.setFont(Some(&NSFont::systemFontOfSize(t.typography.title as f64)));
                v.sizeToFit();
                add_search_hint(&v, shortcut, body_size as f32, 12., mtm);
            }
            Control::Search(v)
        }
        Kind::Text => {
            let v = NSTextField::initWithFrame(NSTextField::alloc(mtm), NSRect::ZERO);
            v.setPlaceholderString(Some(&NSString::from_str(label)));
            v.setEditable(true);
            v.setSelectable(true);
            v.setBezeled(true);
            v.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
            v.setControlSize(NSControlSize::Large);
            v.setFont(Some(&NSFont::systemFontOfSize(15.)));
            v.cell().inspect(|c| {
                c.setScrollable(true);
                c.setWraps(false);
            });
            unsafe {
                let _: () = msg_send![&*v, setDelegate: &*target];
                // The code, and an address that AutoFill may offer; a field
                // that says "plain" (the sign-in sheet's email, the handle)
                // starts empty, nothing filled in or selected for the person.
                match symbol {
                    Some("code") => v.setContentType(Some(NSTextContentTypeOneTimeCode)),
                    Some("plain") => {}
                    _ => v.setContentType(Some(NSTextContentTypeEmailAddress)),
                }
            }
            // An address is never corrected or completed as words.
            v.setAutomaticTextCompletionEnabled(false);
            v.sizeToFit();
            Control::Text(v)
        }
        Kind::Well => {
            let v = NSColorWell::initWithFrame(NSColorWell::alloc(mtm), NSRect::ZERO);
            v.setColorWellStyle(NSColorWellStyle::Minimal);
            unsafe {
                v.setTarget(Some(&target));
                v.setAction(Some(sel!(act:)));
            }
            Control::Well(v)
        }
        Kind::Switch => {
            let v = NSSwitch::initWithFrame(NSSwitch::alloc(mtm), NSRect::ZERO);
            unsafe {
                v.setTarget(Some(&target));
                v.setAction(Some(sel!(act:)));
            }
            Control::Switch(v)
        }
    };
    if form && let Some(view) = control.view().downcast_ref::<NSControl>() {
        view.setControlSize(NSControlSize::Regular);
    }
    control.view().setFocusRingType(NSFocusRingType::None);
    control
        .view()
        .setAccessibilityLabel(Some(&NSString::from_str(label)));
    control
        .view()
        .setAccessibilityIdentifier(Some(&NSString::from_str(identifier)));
    control.enabled(!disabled);
    control.value(value, options);
    (control, target)
}

impl NativeControl {
    pub fn new(
        spec: ControlSpec<'_>,
        t: &pm_design::Tokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let RawWindowHandle::AppKit(handle) =
            HasWindowHandle::window_handle(window).unwrap().as_raw()
        else {
            unreachable!()
        };
        let gpui: Retained<NSView> =
            unsafe { Retained::retain(handle.ns_view.as_ptr().cast()).unwrap() };
        let (tx, rx) = async_channel::bounded(32);
        let (control, target) = make_control(&spec, t, Some(gpui.clone()), false, tx);
        let ControlSpec {
            kind,
            label,
            symbol,
            glass,
            inline,
            well,
            tint,
            ..
        } = spec;
        let glass = glass && objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some();
        let sized: Option<&NSControl> = match &control {
            Control::Button(v) => Some(v.as_ref()),
            Control::Switch(v) => Some(v.as_ref()),
            _ => None,
        };
        let intrinsic = match sized {
            Some(sized) => {
                let look = Look {
                    kind: kind as u8,
                    title: if matches!(kind, Kind::Push) {
                        label.to_owned()
                    } else {
                        String::new()
                    },
                    symbol: symbol.map(str::to_owned),
                    glass,
                    font: t.typography.body.to_bits(),
                };
                let (frame, fitting) = measured(look, sized);
                sized.setFrameSize(frame);
                fitting
            }
            None if well && matches!(kind, Kind::Popup | Kind::PopupTrigger) => {
                NSSize::new(t.metrics.value_column as f64, t.metrics.field as f64)
            }
            None => control.view().fittingSize(),
        };
        let width = if matches!(kind, Kind::Popup | Kind::PopupTrigger) && !inline {
            t.metrics.value_column.max(intrinsic.width as f32)
        } else if matches!(kind, Kind::Slider | Kind::Search | Kind::Text) {
            t.metrics.value_column
        } else {
            (intrinsic.width as f32).max(t.metrics.hit)
        };
        let height = if well {
            t.metrics.field
        } else {
            t.metrics.control.max(intrinsic.height as f32)
        };
        let native =
            unsafe { Retained::retain(control.view() as *const NSView as *mut NSView).unwrap() };
        let slot = Rc::new(NativeControlSlot::new(&gpui, native));
        cx.spawn(async move |this, cx| {
            while let Ok(value) = rx.recv().await {
                if this.update(cx, |_, cx| cx.emit(value)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            tint,
            build: None,
            build_pixels: Default::default(),
            control,
            slot,
            _target: target,
            width,
            height,
            inline,
            least: if matches!(kind, Kind::Popup | Kind::PopupTrigger) && !inline {
                t.metrics.value_column
            } else {
                0.
            },
            // A search field is as wide as the place it is given.
            fill: matches!(kind, Kind::Search | Kind::Text),
            stretch: well && matches!(kind, Kind::Popup | Kind::PopupTrigger),
            focus: Rc::default(),
            tab_focus: matches!(kind, Kind::Segmented).then(|| cx.focus_handle().tab_stop(true)),
            tab_subscription: None,
            host_revision: None,
            menu_open: false,
            search_value: None,
            alpha: 1.,
            busy: None,
        }
    }
    /// Puts the keyboard in a search field (⌘F), with its text selected;
    /// one not yet in its window takes it when it is first placed.
    pub fn focus(&self) {
        let v: &NSView = match &self.control {
            Control::Search(v) => v,
            Control::Text(v) => v,
            _ => return,
        };
        match v.window() {
            Some(window) => {
                window.makeFirstResponder(Some(v));
            }
            None => self.focus.set(true),
        }
    }
    /// Joins the AppKit key view when Tab lands on the GPUI slot.
    fn focus_segmented(&self) {
        if let Control::Segmented(control) = &self.control
            && !control.isHiddenOrHasHiddenAncestor()
            && let Some(window) = control.window()
        {
            window.makeFirstResponder(Some(control));
        }
    }

    /// The surrounding surface can tune its traversal order.
    pub fn set_tab_index(&mut self, index: isize) {
        if let Some(focus) = &self.tab_focus
            && (focus.tab_index, focus.tab_stop) != (index.max(0), index >= 0)
        {
            self.tab_focus = Some(focus.clone().tab_stop(index >= 0).tab_index(index.max(0)));
        }
    }

    /// A native search with the same height as the kit's header buttons.
    pub fn header_search(
        label: &str,
        id: &str,
        value: &str,
        t: &pm_design::Tokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut control = Self::new(
            ControlSpec {
                kind: Kind::Search,
                label,
                identifier: id,
                value: &Value::String(value.to_owned()),
                options: vec![],
                disabled: false,
                default: false,
                destructive: false,
                inline: false,
                quiet: false,
                pull: false,
                tint: None,
                symbol: None,
                glass: false,
                well: false,
            },
            t,
            window,
            cx,
        );
        if let Control::Search(field) = &control.control {
            let mtm = MainThreadMarker::new().expect("main thread");
            field.setControlSize(NSControlSize::Large);
            field.sizeToFit();
            control.height = field.fittingSize().height as f32;
            add_search_hint(field, "⌘F", t.typography.body, f64::from(t.space.md), mtm);
            search_hint(field, value.is_empty());
        }
        control
    }
    /// Lets the kit draw the header well around the native pull-down.
    pub fn use_header_well(&mut self) {
        if let Control::Popup(popup) = &self.control {
            self.height = popup.fittingSize().height as f32;
            popup.setBordered(false);
        }
    }
    /// A text the view's host set (a filter applied from outside) replaces
    /// a search field's text even while it is typed in, once per revision.
    pub fn host_text(&mut self, text: &str, revision: u64) {
        if self.host_revision.replace(revision) != Some(revision) {
            self.set_search_text(text);
        }
    }
    /// Clears a search owned by its native editor, even on repeated Escapes.
    pub fn clear_search(&self) {
        self.set_search_text("");
    }
    fn set_search_text(&self, text: &str) {
        match &self.control {
            Control::Search(v) => {
                v.setStringValue(&NSString::from_str(text));
                search_hint(v, text.is_empty());
            }
            Control::Text(v) => {
                // The field editor holds the text while typed in.
                if let Some(editor) = v.currentEditor() {
                    editor.setString(&NSString::from_str(text));
                }
                v.setStringValue(&NSString::from_str(text));
            }
            _ => {}
        }
    }
    /// Takes the keyboard when it is next placed (a field that comes back
    /// with its step: the view that holds it is not on screen yet).
    pub fn focus_on_paint(&self) {
        self.focus.set(true);
    }
    /// The height its place gives it (a field under the cells its view draws).
    pub fn set_height(&mut self, height: f32) {
        self.height = height;
    }
    /// A text field drawn by its view (a well around it): AppKit draws the
    /// text and the caret only.
    pub fn borderless(&mut self) {
        if let Control::Text(v) = &self.control {
            v.setBezeled(false);
            v.setBordered(false);
            v.setDrawsBackground(false);
            self.height = v.fittingSize().height as f32;
        }
    }
    /// What VoiceOver calls it, when that is not its placeholder.
    pub fn describe(&self, label: &str) {
        self.control
            .view()
            .setAccessibilityLabel(Some(&NSString::from_str(label)));
    }
    /// A push button at work: a spinner in place of its title, the button
    /// where it was and as wide (never a jump); `false` gives the title back.
    pub fn set_busy(&mut self, busy: bool) {
        let Control::Button(v) = &self.control else {
            return;
        };
        match (busy, self.busy.take()) {
            (true, None) => {
                let mtm = MainThreadMarker::new().expect("main thread");
                let spinner = NSProgressIndicator::initWithFrame(
                    NSProgressIndicator::alloc(mtm),
                    NSRect::ZERO,
                );
                spinner.setStyle(NSProgressIndicatorStyle::Spinning);
                spinner.setControlSize(NSControlSize::Small);
                spinner.setDisplayedWhenStopped(false);
                // Light on the accent fill, as the title it stands for.
                spinner.setAppearance(
                    NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }).as_deref(),
                );
                spinner.sizeToFit();
                let (own, button) = (spinner.frame().size, v.bounds().size);
                spinner.setFrameOrigin(NSPoint::new(
                    ((button.width - own.width) / 2.).round(),
                    ((button.height - own.height) / 2.).round(),
                ));
                spinner.setAutoresizingMask(
                    NSAutoresizingMaskOptions::ViewMinXMargin
                        | NSAutoresizingMaskOptions::ViewMaxXMargin
                        | NSAutoresizingMaskOptions::ViewMinYMargin
                        | NSAutoresizingMaskOptions::ViewMaxYMargin,
                );
                v.addSubview(&spinner);
                unsafe { spinner.startAnimation(None) };
                let title = v.title();
                v.setTitle(&NSString::from_str(""));
                // VoiceOver still names the button.
                v.setAccessibilityLabel(Some(&title));
                self.busy = Some((title, spinner));
            }
            (false, Some((title, spinner))) => {
                unsafe { spinner.stopAnimation(None) };
                spinner.removeFromSuperview();
                v.setTitle(&title);
            }
            (_, kept) => self.busy = kept,
        }
    }
    /// A search field's placeholder, when it is not its accessibility label.
    pub fn placeholder(&self, text: &str) {
        if let Control::Search(v) = &self.control {
            v.setPlaceholderString(Some(&NSString::from_str(text)));
        }
    }
    pub fn is_popup(&self) -> bool {
        matches!(self.control, Control::Popup(_))
    }
    pub fn is_search(&self) -> bool {
        matches!(self.control, Control::Search(_))
    }
    /// A window's one primary action, as macOS welcome and setup windows
    /// set it: a push button at the extra-large size and a fixed `width`.
    pub fn prominent(&mut self, width: f32) {
        let Control::Button(v) = &self.control else {
            return;
        };
        v.setControlSize(NSControlSize::ExtraLarge);
        self.height = v.fittingSize().height as f32;
        self.width = width;
    }
    /// A push button at the small size, a capsule: a row's own button.
    pub fn small(&mut self) {
        let Control::Button(v) = &self.control else {
            return;
        };
        v.setControlSize(NSControlSize::Small);
        // A capsule (macOS 26 and later), the shape of a row's own button.
        if v.respondsToSelector(objc2::sel!(setBorderShape:)) {
            v.setBorderShape(NSControlBorderShape::Capsule);
        }
        let fit = v.fittingSize();
        self.height = fit.height as f32;
        self.width = fit.width as f32;
    }
    /// A full-width push button of a sheet's column (the sign-in sheet's
    /// Continue and Continue with Google): extra-large, `width` wide, a
    /// rounded rectangle so its corners follow the sheet's.
    pub fn wide(&mut self, width: f32) {
        let Control::Button(v) = &self.control else {
            return;
        };
        v.setControlSize(NSControlSize::ExtraLarge);
        if v.respondsToSelector(objc2::sel!(setBorderShape:)) {
            v.setBorderShape(NSControlBorderShape::RoundedRectangle);
        }
        self.height = v.fittingSize().height as f32;
        self.width = width;
    }
    /// An image before the title (a sign-in provider's mark).
    pub fn set_leading_image(&mut self, image: &objc2_app_kit::NSImage) {
        let Control::Button(v) = &self.control else {
            return;
        };
        v.setImage(Some(image));
        v.setImagePosition(NSCellImagePosition::ImageLeading);
        v.setImageScaling(objc2_app_kit::NSImageScaling::ScaleNone);
    }
    /// A push button beside a `prominent` one: the same extra-large size,
    /// as wide as its title needs.
    pub fn extra_large(&mut self) {
        let Control::Button(v) = &self.control else {
            return;
        };
        v.setControlSize(NSControlSize::ExtraLarge);
        let fit = v.fittingSize();
        self.height = fit.height as f32;
        self.width = fit.width as f32;
    }
    /// Whether the control takes the whole width of its row.
    pub fn stretches(&self) -> bool {
        self.stretch
    }
    /// A segmented control fills its row with equal segments (a sheet's
    /// destinations), instead of standing at the width of its titles.
    pub fn stretch(&mut self) {
        let Control::Segmented(v) = &self.control else {
            return;
        };
        for i in 0..v.segmentCount() {
            v.setWidth_forSegment(0., i);
        }
        v.setSegmentDistribution(NSSegmentDistribution::FillEqually);
        self.stretch = true;
    }
    /// Whether this control or its field editor has the keyboard.
    pub fn focused(&self) -> bool {
        match &self.control {
            Control::Search(v) => v.currentEditor().is_some(),
            Control::Text(v) => v.currentEditor().is_some(),
            Control::Segmented(v) => {
                v.window()
                    .and_then(|w| w.firstResponder())
                    .is_some_and(|responder| {
                        let control: &NSResponder = v;
                        std::ptr::eq(&*responder, control)
                    })
            }
            _ => false,
        }
    }
    /// Stages the slot again on a frame whose cached paint did not run, as
    /// last painted.
    pub fn keep(&self, _window: &Window) {
        // In a scrolling region it keeps its place in the region's content;
        // the region's container follows the scroll (P-SLOT-REGION).
        self.slot.keep();
    }
    /// A push button's title follows the panel (Export's action names the
    /// chosen format); its width follows the new title.
    /// Full Keyboard Access: the panel's focus ring sits on this control and
    /// forwards its keys. Space, Return and the vertical arrows open a
    /// pop-up's menu (AppKit then runs the menu's own keys); Space toggles a
    /// checkbox or switch and presses a button; the arrows step a slider.
    /// Returns whether the key was the control's.
    pub fn key(&self, key: &str) -> bool {
        let press = |view: &NSView| unsafe {
            let _: () = msg_send![view, performClick: std::ptr::null::<AnyObject>()];
        };
        match &self.control {
            Control::Popup(v) if matches!(key, "space" | "enter" | "down" | "up") => {
                press(v.as_ref());
                true
            }
            Control::Button(v) if matches!(key, "space" | "enter") => {
                press(v.as_ref());
                true
            }
            Control::Switch(v) if key == "space" => {
                press(v.as_ref());
                true
            }
            // GPUI owns Tab traversal in the hosted surface; route its
            // horizontal arrows through the AppKit control's action.
            Control::Segmented(v) if matches!(key, "left" | "right") => {
                let count = v.segmentCount();
                let current = v.selectedSegment();
                let next = if key == "right" {
                    (current + 1..count).find(|&i| v.isEnabledForSegment(i))
                } else {
                    (0..if current < 0 { count } else { current })
                        .rev()
                        .find(|&i| v.isEnabledForSegment(i))
                };
                if v.isEnabled()
                    && let Some(next) = next
                {
                    v.setSelectedSegment(next);
                    unsafe {
                        let _: bool =
                            msg_send![&**v, sendAction: v.action(), to: v.target().as_deref()];
                    }
                }
                true
            }
            Control::Slider(v) if matches!(key, "left" | "right" | "up" | "down") => {
                let range = Range::of(&self._target.ivars().options.borrow());
                let now = range.value(v.doubleValue());
                let next = range.nudge(now, matches!(key, "right" | "up"));
                if next != now {
                    v.setDoubleValue(range.position(next));
                    let _ = self._target.ivars().sender.try_send(json!(next));
                }
                true
            }
            _ => false,
        }
    }
    pub fn retitle(&mut self, title: &str, cx: &mut Context<Self>) {
        let Control::Button(button) = &self.control else {
            return;
        };
        let title = NSString::from_str(title);
        if button.title() == title {
            return;
        }
        button.setTitle(&title);
        button.setAccessibilityLabel(Some(&title));
        self.width = (button.fittingSize().width as f32).max(self.height);
        cx.notify();
    }
    /// A pop-up's own title, shown instead of its selected row; the row
    /// keeps its checkmark. `None` shows the selected row.
    /// The panel keeps a custom searchable menu's trigger pressed until it closes.
    pub fn popup_width(&mut self, width: Option<f32>, minimum: Option<f32>) {
        if let Control::Popup(v) = &self.control {
            self.width = width
                .unwrap_or(v.fittingSize().width as f32)
                .max(minimum.unwrap_or(0.))
                .max(self.height);
        }
    }
    pub fn menu_open(&mut self, open: bool) {
        if self.menu_open != open {
            self.menu_open = open;
            if let Control::Popup(v) = &self.control {
                v.highlight(open);
            }
        }
    }
    pub fn display(&mut self, title: Option<&str>) {
        let Control::Popup(v) = &self.control else {
            return;
        };
        let Some(cell) = v.cell() else { return };
        unsafe {
            let _: () = objc2::msg_send![&*cell, setUsesItemFromMenu: title.is_none()];
            if let Some(title) = title {
                let mtm = MainThreadMarker::new().expect("main thread");
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    None,
                    &NSString::from_str(""),
                );
                let _: () = objc2::msg_send![&*cell, setMenuItem: &*item];
            }
        }
    }
    /// Rows this pop-up's menu lists when it opens, from `source` (a long
    /// list the panel never builds: every layer a layer can follow). The
    /// published `options` stay what the closed button shows.
    pub fn fill_rows(&mut self, source: Option<RowSource>) {
        let Control::Popup(v) = &self.control else {
            return;
        };
        fill_rows_on(&self._target, v, source);
    }
    pub fn update(
        &mut self,
        value: &Value,
        options: &[Value],
        disabled: bool,
        cx: &mut Context<Self>,
    ) {
        // A pop-up's rows follow the tree (the viewer's zoom row names the
        // current zoom).
        if let Control::Popup(v) = &self.control {
            set_rows(&self._target, v, options, value);
        }
        // A slider's one option is its range.
        if let Control::Slider(_) = &self.control {
            let mut range = self._target.ivars().options.borrow_mut();
            if range.as_slice() != options {
                options.clone_into(&mut range);
            }
        }
        let shown = self._target.ivars().options.borrow().clone();
        let unchanged_search = matches!(self.control, Control::Search(_))
            && self.search_value.replace(value.clone()).as_ref() == Some(value);
        if !unchanged_search {
            self.control.value(value, &shown);
        }
        self.control.enabled(!disabled);
        if let (Control::Button(v), Some(rgb)) = (&self.control, self.tint) {
            v.setBezelColor((!disabled).then(|| srgb(rgb)).as_deref());
        }
        if self.inline {
            // The title names the current value, so the width follows it.
            self.width = self.control.view().fittingSize().width as f32;
        } else if matches!(self.control, Control::Popup(_)) && !self.stretch {
            // New rows (the Export sheet's Format, built for one kind and
            // then given another's): as wide as its widest row again, so
            // no value is cut off (PM-452).
            self.width = self
                .least
                .max(self.control.view().fittingSize().width as f32);
        }
        cx.notify();
    }
}
impl Render for NativeControl {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.tab_subscription.is_none()
            && let Some(focus) = &self.tab_focus
        {
            self.tab_subscription = Some(cx.on_focus(focus, window, |control, _, _| {
                control.focus_segmented();
            }));
        }
        let build = self.build;
        let pixels = self.build_pixels.clone();
        let owner = cx.entity().downgrade();
        let slot = self.slot.clone();
        let focus = self.focus.clone();
        let alpha = self.alpha;
        let native: Retained<NSView> = unsafe {
            Retained::retain(self.control.view() as *const NSView as *mut NSView).unwrap()
        };
        let root = if self.stretch || matches!(self.control, Control::Search(_) | Control::Text(_))
        {
            div().w_full()
        } else if self.fill {
            div().w_full().max_w(px(self.width))
        } else {
            div().w(px(self.width))
        };
        let segmented = matches!(&self.control, Control::Segmented(v) if v.isEnabled());
        root.id("native-control")
            .when_some(
                self.tab_focus.as_ref().filter(|_| segmented),
                |root, focus| {
                    root.track_focus(focus).on_key_down(cx.listener(
                        |control, event: &KeyDownEvent, _, cx| {
                            let modifiers = &event.keystroke.modifiers;
                            if !modifiers.modified() && control.key(&event.keystroke.key) {
                                cx.stop_propagation();
                            }
                        },
                    ))
                },
            )
            .relative()
            .h(px(self.height))
            .filter(reveal_paint(build).0)
            .opacity(reveal_paint(build).1)
            .child(
                canvas(
                    |_, _, _| (),
                    move |painted, _, window, cx| {
                        let bounds = painted;
                        if build.is_none() {
                            slot.stage_alpha(1.);
                        }
                        match crate::slot_stage::place(painted, window.content_mask(), window) {
                            crate::slot_stage::Place::Window { bounds, clip, fade } => {
                                slot.stage_with_scroll_fade(bounds, clip, fade)
                            }
                            crate::slot_stage::Place::Region { id, bounds, clip } => {
                                slot.stage_in_region(id, bounds, clip)
                            }
                        }
                        if let Some(build) = build {
                            if let Some(revision) = build.revision {
                                if pixels.borrow_mut().capture(&native, bounds, revision) {
                                    let _ = owner.update(cx, |_, cx| cx.notify());
                                }
                            }
                            let captured = pixels.borrow().image.is_some();
                            let (_, live) =
                                pm_design::dots::snapshot_visibility(captured, build.ready);
                            slot.stage_alpha(live as f64);
                        } else {
                            slot.stage_alpha(alpha as f64);
                        }
                        // Capture may have completed in this paint.
                        let snapshot = pixels.borrow().texture(build.and_then(|b| b.revision));
                        if let Some(image) = snapshot {
                            let _ = window.paint_image(
                                painted,
                                painted,
                                Corners::default(),
                                0.,
                                image,
                                0,
                                false,
                            );
                        }
                        if build.is_none()
                            && focus.get()
                            && let Some(window) = native.window()
                        {
                            focus.set(false);
                            window.makeFirstResponder(Some(&native));
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}

/// Record an opaque GPUI overlay rect so AppKit controls/text slots staged
/// beneath it in the same paint are hidden instead of bleeding through.
pub fn occlude_native_overlay(bounds: Bounds<Pixels>, window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    let Some(view) = (unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }) else {
        return;
    };
    let rect = ns_rect(bounds);
    gpui_macos::native_control::occlude_below(&view, rect);
    gpui_macos::native_text::occlude_below(&view, rect);
}

/// First child for GPUI popovers/menus so AppKit subviews staged earlier in
/// the frame (like Timeline's segmented control) never bleed through them,
/// while native text fields painted *inside* the overlay remain visible.
pub fn native_occlusion_backdrop() -> AnyElement {
    canvas(
        |_, _, _| (),
        |bounds, _, window, _| occlude_native_overlay(bounds, window),
    )
    .absolute()
    .inset_0()
    .into_any_element()
}
/// The value a row stands for (a plain string row is its own value).
fn row_value(option: &Value) -> &Value {
    option.get("v").unwrap_or(option)
}
fn row_title(option: &Value) -> Retained<NSString> {
    NSString::from_str(
        option
            .get("label")
            .and_then(Value::as_str)
            .or_else(|| option.as_str())
            .unwrap_or(""),
    )
}
/// One menu item a row, in order. (`addItemWithTitle:` would drop an
/// earlier row of the same title, and an action's index would then name
/// another row.)
fn add_items(popup: &NSPopUpButton, options: &[Value]) {
    let Some(menu) = popup.menu() else { return };
    let mtm = MainThreadMarker::new().expect("main thread");
    // A pull-down's first item is its title (`Control::value` names it).
    if popup.pullsDown() {
        let title = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(""),
                None,
                &NSString::from_str(""),
            )
        };
        menu.addItem(&title);
    }
    for option in options {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &row_title(option),
                None,
                &NSString::from_str(""),
            )
        };
        subtitle(&item, option);
        menu.addItem(&item);
    }
    checks(popup, options);
}
/// A row's second line (a quality's promise, a sky's look): the menu item's
/// own subtitle, under its title, as in System Settings.
fn subtitle(item: &NSMenuItem, option: &Value) {
    let Some(detail) = option.get("detail").and_then(Value::as_str) else {
        return;
    };
    let responds: bool = unsafe { msg_send![item, respondsToSelector: sel!(setSubtitle:)] };
    if responds {
        let _: () = unsafe { msg_send![item, setSubtitle: &*NSString::from_str(detail)] };
    }
}
/// Rows marked `checked` show a checkmark: a pull-down that reports several
/// settings at once (the viewer's collapsed 3D menu).
fn checks(popup: &NSPopUpButton, options: &[Value]) {
    if !options.iter().any(|o| o.get("checked").is_some()) {
        return;
    }
    let first = isize::from(popup.pullsDown());
    for (i, option) in options.iter().enumerate() {
        if let Some(item) = popup.itemAtIndex(i as isize + first) {
            item.setState(if option.get("checked") == Some(&Value::Bool(true)) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
    }
}
fn replace_rows(popup: &NSPopUpButton, options: &[Value]) {
    popup.removeAllItems();
    add_items(popup, options);
}
/// Brings a pop-up's rows to `next`. Rows that stand for the same values in
/// the same order keep their menu items and take the new titles, open menu
/// or not (the item under the pointer stays the item under the pointer).
/// Any other change replaces the items, and waits for an open menu to close
/// and send its action first: the action names a row by its index.
fn set_rows(target: &Target, popup: &NSPopUpButton, next: &[Value], value: &Value) {
    let state = target.ivars();
    let shown = state.options.borrow().clone();
    let same_rows = shown.len() == next.len()
        && shown
            .iter()
            .zip(next)
            .all(|(a, b)| row_value(a) == row_value(b));
    if same_rows {
        state.pending.take();
        if shown.as_slice() != next {
            let first = isize::from(popup.pullsDown());
            for (index, option) in next.iter().enumerate() {
                let title = row_title(option);
                if let Some(item) = popup.itemAtIndex(index as isize + first) {
                    if item.title() != title {
                        item.setTitle(&title);
                    }
                    subtitle(&item, option);
                }
            }
            *state.options.borrow_mut() = next.to_vec();
        }
    } else if state.tracking.get() {
        *state.pending.borrow_mut() = Some((next.to_vec(), value.clone()));
    } else {
        state.pending.take();
        replace_rows(popup, next);
        *state.options.borrow_mut() = next.to_vec();
    }
}
/// Lists a pop-up's rows when its menu opens.
pub type RowSource = Rc<dyn Fn() -> Vec<Value>>;
fn fill_rows_on(target: &Target, popup: &NSPopUpButton, source: Option<RowSource>) {
    let on = source.is_some();
    *target.ivars().fill.borrow_mut() = source;
    if let Some(menu) = popup.menu() {
        unsafe {
            if on {
                let _: () = msg_send![&*menu, setDelegate: target];
            } else {
                let none: *const AnyObject = std::ptr::null();
                let _: () = msg_send![&*menu, setDelegate: none];
            }
        }
    }
}
/// The rows the source lists now, the selected value kept selected (and
/// listed, should the source no longer offer it).
fn fill_now(target: &Target, popup: &NSPopUpButton, fill: &RowSource) {
    let state = target.ivars();
    let index = popup.indexOfSelectedItem();
    let current = usize::try_from(index)
        .ok()
        .and_then(|i| state.options.borrow().get(i).cloned());
    let mut rows = fill();
    if rows.is_empty() {
        // Nothing to list from (no project): the button's own rows stay.
        return;
    }
    if let Some(current) = &current
        && !rows.iter().any(|r| row_value(r) == row_value(current))
    {
        rows.insert(rows.len().min(1), current.clone());
    }
    if *state.options.borrow() != rows {
        replace_rows(popup, &rows);
        if let Some(current) = &current
            && let Some(at) = rows.iter().position(|r| row_value(r) == row_value(current))
        {
            popup.selectItemAtIndex(at as isize);
        }
        *state.options.borrow_mut() = rows;
    }
}
/// Tells `target` when the pop-up's menu opens and closes.
fn watch_menu(target: &Target, popup: &NSPopUpButton) {
    *target.ivars().popup.borrow_mut() = Some(unsafe {
        Retained::retain(popup as *const NSPopUpButton as *mut NSPopUpButton).unwrap()
    });
    let Some(menu) = popup.menu() else { return };
    let center = objc2_foundation::NSNotificationCenter::defaultCenter();
    unsafe {
        for (selector, name) in [
            (sel!(menuBegan:), NSMenuDidBeginTrackingNotification),
            (sel!(menuEnded:), NSMenuDidEndTrackingNotification),
        ] {
            center.addObserver_selector_name_object(target, selector, Some(name), Some(&menu));
        }
    }
}
/// A pop-up and its rows without a window, for tests of how rows follow a
/// publication while its menu is open.
#[doc(hidden)]
pub struct PopupRows {
    pub popup: Retained<NSPopUpButton>,
    target: Retained<Target>,
    pub chosen: async_channel::Receiver<Value>,
}
impl PopupRows {
    pub fn new(options: &[Value]) -> Self {
        let mtm = MainThreadMarker::new().expect("main thread");
        let (sender, chosen) = async_channel::unbounded();
        let content = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        let target: Retained<Target> = unsafe {
            msg_send![
                super(Target::alloc(mtm).set_ivars(TargetState {
                    sender,
                    options: std::cell::RefCell::new(options.to_vec()),
                    tracking: std::cell::Cell::new(false),
                    pending: std::cell::RefCell::new(None),
                    popup: std::cell::RefCell::new(None),
                    fill: std::cell::RefCell::new(None),
                    kind: Kind::Popup,
                    content: Some(content),
                    digits: false,
                })),
                init
            ]
        };
        let popup =
            NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
        add_items(&popup, options);
        unsafe {
            popup.setTarget(Some(&target));
            popup.setAction(Some(sel!(act:)));
        }
        watch_menu(&target, &popup);
        Self {
            popup,
            target,
            chosen,
        }
    }
    pub fn update(&self, options: &[Value], value: &Value) {
        set_rows(&self.target, &self.popup, options, value);
    }
    pub fn fill_rows(&self, source: Option<RowSource>) {
        fill_rows_on(&self.target, &self.popup, source);
    }
    /// What AppKit asks the menu's delegate just before the menu shows.
    pub fn menu_will_show(&self) {
        if let Some(menu) = self.popup.menu() {
            let delegate: *mut AnyObject = unsafe { msg_send![&*menu, delegate] };
            if !delegate.is_null() {
                let _: () = unsafe { msg_send![delegate, menuNeedsUpdate: &*menu] };
            }
        }
    }
    /// The value the pop-up's action sends for the selected row.
    pub fn act(&self) {
        unsafe {
            let _: () = msg_send![&*self.target, act: &*self.popup];
        }
    }
}
/// GPUI top-left points as an AppKit rectangle (the slot flips them).
pub(crate) fn ns_rect(b: Bounds<Pixels>) -> NSRect {
    NSRect::new(
        NSPoint::new(f32::from(b.origin.x) as f64, f32::from(b.origin.y) as f64),
        NSSize::new(
            f32::from(b.size.width) as f64,
            f32::from(b.size.height) as f64,
        ),
    )
}

/// `#rrggbb` as an sRGB colour.
fn hex_colour(hex: &str) -> Option<Retained<NSColor>> {
    let v = u32::from_str_radix(hex.trim_start_matches('#').get(..6)?, 16).ok()?;
    let c = |shift: u32| f64::from((v >> shift) & 0xff) / 255.;
    Some(NSColor::colorWithSRGBRed_green_blue_alpha(
        c(16),
        c(8),
        c(0),
        1.,
    ))
}
/// A colour as `#rrggbb` in sRGB.
fn colour_hex(colour: &NSColor) -> String {
    let srgb = colour
        .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
        .unwrap_or_else(|| objc2::Message::retain(colour));
    let c = |v: f64| (v.clamp(0., 1.) * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        c(srgb.redComponent()),
        c(srgb.greenComponent()),
        c(srgb.blueComponent())
    )
}

#[cfg(test)]
mod snapshot_bridge_tests {
    use super::{NSBitmapImageRep, NSImage, snapshot_bitmap, snapshot_image};
    use objc2::AnyThread;
    #[::core::prelude::v1::test]
    fn cocoa_snapshots_upload_straight_bgra_with_alpha() {
        objc2::rc::autoreleasepool(|_| {
            let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([40, 80, 120, 160]));
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            let data = objc2_foundation::NSData::with_bytes(bytes.get_ref());
            let rep = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &data).unwrap();
            let texture = snapshot_bitmap(&rep).unwrap();
            let p = &texture.as_bytes(0).unwrap()[..4];
            assert_eq!(p, &[120, 80, 40, 160]);
            let image = NSImage::initWithData(NSImage::alloc(), &data).unwrap();
            let texture = snapshot_image(&image).unwrap();
            let p = &texture.as_bytes(0).unwrap()[..4];
            for (actual, expected) in p.iter().zip([120_u8, 80, 40, 160]) {
                assert!(actual.abs_diff(expected) <= 2);
            }
        });
    }
}

#[cfg(test)]
mod scroll_reveal_tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn build_snapshot_uses_reveal_filter_and_alpha_until_ready() {
        let filter = Filter::DotMatrix {
            detail: 2.2,
            resolve: 0.,
            accent: 0xff8800,
        };
        let mut build = BuildSnapshot {
            revision: None,
            ready: None,
            filter: Some(filter),
        };
        assert_eq!(reveal_paint(Some(build)), (vec![filter], 0.));
        build.revision = Some(7);
        // The same boundary wraps the live control and its snapshot.
        assert_eq!(reveal_paint(Some(build)), (vec![filter], 1.));
        assert_eq!(
            pm_design::dots::snapshot_visibility(true, build.ready).1,
            0.
        );
    }
    #[::core::prelude::v1::test]
    fn scroll_preserves_capture_and_moves_the_content_grid() {
        let before = Bounds::new(point(px(20.), px(100.)), size(px(160.), px(24.)));
        let after = Bounds::new(point(px(20.), px(63.)), before.size);
        assert_eq!(capture_key(before, 7), capture_key(after, 7));
        // Shader sampling is relative to the translated filter boundary, so
        // identical content coordinates keep their dot generation/stagger.
        let dot = point(px(16.), px(8.));
        let first = before.origin + dot;
        let scrolled = after.origin + dot;
        assert_eq!(scrolled - first, after.origin - before.origin);
        assert_eq!(first - before.origin, scrolled - after.origin);
        let wider = Bounds::new(after.origin, size(px(180.), px(24.)));
        assert_ne!(capture_key(after, 7), capture_key(wider, 7));
    }
    #[::core::prelude::v1::test]
    fn resolved_snapshot_has_integration_paint_policy() {
        assert_eq!(reveal_paint(None), (vec![], 1.));
        assert_eq!(
            reveal_paint(Some(BuildSnapshot {
                revision: Some(7),
                ready: Some(1.),
                filter: Some(Filter::DotMatrix {
                    detail: 4.,
                    resolve: 1.,
                    accent: 0xff8800
                })
            })),
            (vec![], 1.)
        );
    }
}
