//! AppKit-backed v4 UI-kit controls, composed into GPUI's native transaction.
use gpui::{prelude::*, *};
use gpui_macos::native_control::NativeControlSlot;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::AnyObject, sel,
};
use objc2_app_kit::*;
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use serde_json::{Value, json};
use std::rc::Rc;
#[derive(Clone, Copy)]
pub enum Kind {
    Popup,
    Checkbox,
    Switch,
    Push,
    Slider,
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
            return self.value((self.position(value) + sign * span).clamp(self.min.ln(), self.max.ln()));
        }
        let step = if self.step > 0. { self.step } else { (self.max - self.min) / 20. };
        self.value((value + sign * step).clamp(self.min, self.max))
    }
}
/// The value VoiceOver reads and the field shows: up to three significant
/// digits, trailing zeros trimmed.
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
    options: std::cell::RefCell<Vec<Value>>,
    kind: Kind,
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
                Kind::Popup=>{let index:isize=unsafe{msg_send![sender,indexOfSelectedItem]};self.ivars().options.borrow().get(index as usize).map(|o|o.get("v").unwrap_or(o).clone()).unwrap_or(Value::Null)},
                Kind::Checkbox|Kind::Switch=>{let state:isize=unsafe{msg_send![sender,state]};json!(state!=0)},
                Kind::Push=>Value::Null,
                Kind::Slider=>{let position:f64=unsafe{msg_send![sender,doubleValue]};json!(Range::of(&self.ivars().options.borrow()).value(position))},
            };
            let _=self.ivars().sender.try_send(value);
        }
    }
);
enum Control {
    Popup(Retained<NSPopUpButton>),
    Button(Retained<NSButton>),
    Switch(Retained<NSSwitch>),
    Slider(Retained<NSSlider>),
}
impl Control {
    fn view(&self) -> &NSView {
        match self {
            Self::Popup(v) => v.as_ref(),
            Self::Button(v) => v.as_ref(),
            Self::Switch(v) => v.as_ref(),
            Self::Slider(v) => v.as_ref(),
        }
    }
    fn enabled(&self, enabled: bool) {
        match self {
            Self::Popup(v) => v.setEnabled(enabled),
            Self::Button(v) => v.setEnabled(enabled),
            Self::Switch(v) => v.setEnabled(enabled),
            Self::Slider(v) => v.setEnabled(enabled),
        }
    }
    fn value(&self, value: &Value, options: &[Value]) {
        match self {
            Self::Slider(v) => {
                let range = Range::of(options);
                v.setMinValue(range.position(range.min));
                v.setMaxValue(range.position(range.max));
                if let Some(n) = value.as_f64() {
                    v.setDoubleValue(range.position(n));
                    // VoiceOver reads the value, not a logarithmic position.
                    v.setAccessibilityValueDescription(Some(&NSString::from_str(&readable(n))));
                }
            }
            Self::Popup(v) => {
                if let Some(index) = options
                    .iter()
                    .position(|o| o.get("v").unwrap_or(o) == value)
                {
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
        }
    }
}
pub struct NativeControl {
    control: Control,
    slot: Rc<NativeControlSlot>,
    _target: Retained<Target>,
    options: Vec<Value>,
    width: f32,
    height: f32,
    inline: bool,
    /// Fills a slot that may be narrower than `width` (its node has
    /// `flexShrink`); AppKit then truncates the title.
    pub fill: bool,
}
impl EventEmitter<Value> for NativeControl {}
impl NativeControl {
    /// The width the control fits its title in.
    pub fn width(&self) -> f32 {
        self.width
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
}

impl NativeControl {
    pub fn new(
        spec: ControlSpec<'_>,
        t: &pm_design::Tokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let ControlSpec {
            kind,
            label,
            identifier,
            value,
            options,
            disabled,
            default,
            destructive,
            inline,
        } = spec;
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let RawWindowHandle::AppKit(handle) =
            HasWindowHandle::window_handle(window).unwrap().as_raw()
        else {
            unreachable!()
        };
        let gpui: Retained<NSView> =
            unsafe { Retained::retain(handle.ns_view.as_ptr().cast()).unwrap() };
        let mtm = MainThreadMarker::new().unwrap();
        let (tx, rx) = async_channel::bounded(32);
        let target: Retained<Target> = unsafe {
            msg_send![
                super(Target::alloc(mtm).set_ivars(TargetState {
                    sender: tx,
                    options: std::cell::RefCell::new(options.clone()),
                    kind
                })),
                init
            ]
        };
        let control = match kind {
            Kind::Popup => {
                let v = NSPopUpButton::initWithFrame_pullsDown(
                    NSPopUpButton::alloc(mtm),
                    NSRect::ZERO,
                    false,
                );
                add_items(&v, &options);
                unsafe {
                    v.setTarget(Some(&target));
                    v.setAction(Some(sel!(act:)));
                }
                v.setFont(Some(&NSFont::systemFontOfSize(t.typography.body as f64)));
                if inline {
                    v.setBordered(false);
                }
                v.sizeToFit();
                Control::Popup(v)
            }
            Kind::Checkbox | Kind::Push => {
                let v = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &NSString::from_str(if matches!(kind, Kind::Checkbox) {
                            ""
                        } else {
                            label
                        }),
                        Some(&target),
                        Some(sel!(act:)),
                        mtm,
                    )
                };
                if matches!(kind, Kind::Checkbox) {
                    v.setButtonType(NSButtonType::Switch);
                } else {
                    v.setBezelStyle(NSBezelStyle::Push);
                    v.setHasDestructiveAction(destructive);
                    if default {
                        v.setKeyEquivalent(&NSString::from_str("\r"));
                    }
                }
                v.setFont(Some(&NSFont::systemFontOfSize(t.typography.body as f64)));
                v.sizeToFit();
                Control::Button(v)
            }
            Kind::Slider => {
                let v = NSSlider::initWithFrame(NSSlider::alloc(mtm), NSRect::ZERO);
                unsafe {
                    v.setTarget(Some(&target));
                    v.setAction(Some(sel!(act:)));
                }
                // One change per pull, so a drag is one Undo.
                v.setContinuous(false);
                v.setControlSize(NSControlSize::Small);
                Control::Slider(v)
            }
            Kind::Switch => {
                let v = NSSwitch::initWithFrame(NSSwitch::alloc(mtm), NSRect::ZERO);
                unsafe {
                    v.setTarget(Some(&target));
                    v.setAction(Some(sel!(act:)));
                }
                v.sizeToFit();
                Control::Switch(v)
            }
        };
        control
            .view()
            .setAccessibilityLabel(Some(&NSString::from_str(label)));
        control
            .view()
            .setAccessibilityIdentifier(Some(&NSString::from_str(identifier)));
        control.enabled(!disabled);
        control.value(value, &options);
        let intrinsic = control.view().fittingSize();
        let width = if matches!(kind, Kind::Slider) {
            t.metrics.value_column
        } else if matches!(kind, Kind::Popup) && !inline {
            t.metrics.value_column.max(intrinsic.width as f32)
        } else {
            (intrinsic.width as f32).max(t.metrics.hit)
        };
        let height = t.metrics.control.max(intrinsic.height as f32);
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
            control,
            slot,
            _target: target,
            options,
            width,
            height,
            inline,
            fill: false,
        }
    }
    pub fn keep(&self) {
        self.slot.keep();
    }
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
            Control::Slider(v) if matches!(key, "left" | "right" | "up" | "down") => {
                let range = Range::of(&self.options);
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
    /// A push button's title follows the panel (Export's action names the
    /// chosen format); its width follows the new title.
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
    pub fn update(
        &mut self,
        value: &Value,
        options: &[Value],
        disabled: bool,
        cx: &mut Context<Self>,
    ) {
        // A pop-up's rows follow the tree (the viewer's zoom row names the
        // current zoom), so a changed list replaces the menu's items.
        if let Control::Popup(v) = &self.control
            && options != self.options.as_slice()
        {
            v.removeAllItems();
            add_items(v, options);
            options.clone_into(&mut self.options);
            options.clone_into(&mut self._target.ivars().options.borrow_mut());
        }
        if let Control::Slider(_) = &self.control
            && options != self.options.as_slice()
        {
            options.clone_into(&mut self.options);
            options.clone_into(&mut self._target.ivars().options.borrow_mut());
        }
        self.control.value(value, &self.options);
        self.control.enabled(!disabled);
        if self.inline {
            // The title names the current value, so the width follows it.
            self.width = self.control.view().fittingSize().width as f32;
        }
        cx.notify();
    }
}
impl Render for NativeControl {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let slot = self.slot.clone();
        let root = if self.fill {
            div().w_full().max_w(px(self.width))
        } else {
            div().w(px(self.width))
        };
        root.relative().h(px(self.height)).child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let to_rect = |b: Bounds<Pixels>| {
                        NSRect::new(
                            NSPoint::new(
                                f32::from(b.origin.x) as f64,
                                f32::from(b.origin.y) as f64,
                            ),
                            NSSize::new(
                                f32::from(b.size.width) as f64,
                                f32::from(b.size.height) as f64,
                            ),
                        )
                    };
                    slot.stage(to_rect(bounds), to_rect(window.content_mask().bounds));
                },
            )
            .absolute()
            .inset_0(),
        )
    }
}
fn add_items(popup: &NSPopUpButton, options: &[Value]) {
    for (i, option) in options.iter().enumerate() {
        popup.addItemWithTitle(&NSString::from_str(
            option
                .get("label")
                .and_then(Value::as_str)
                .or_else(|| option.as_str())
                .unwrap_or(""),
        ));
        // A row's second line (a quality's promise, a sky's look): the menu
        // item's own subtitle, under its title, as in System Settings.
        if let Some(detail) = option.get("detail").and_then(Value::as_str)
            && let Some(item) = popup.itemAtIndex(i as isize)
        {
            let responds: bool = unsafe { msg_send![&*item, respondsToSelector: sel!(setSubtitle:)] };
            if responds {
                let _: () = unsafe { msg_send![&*item, setSubtitle: &*NSString::from_str(detail)] };
            }
        }
    }
    checks(popup, options);
}
/// Rows marked `checked` show a checkmark: a pull-down that reports several
/// settings at once (the viewer's collapsed 3D menu).
fn checks(popup: &NSPopUpButton, options: &[Value]) {
    if !options.iter().any(|o| o.get("checked").is_some()) {
        return;
    }
    for (i, option) in options.iter().enumerate() {
        if let Some(item) = popup.itemAtIndex(i as isize) {
            item.setState(if option.get("checked") == Some(&Value::Bool(true)) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
    }
}
