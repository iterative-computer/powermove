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
            };
            let _=self.ivars().sender.try_send(value);
        }
    }
);
enum Control {
    Popup(Retained<NSPopUpButton>),
    Button(Retained<NSButton>),
    Switch(Retained<NSSwitch>),
}
impl Control {
    fn view(&self) -> &NSView {
        match self {
            Self::Popup(v) => v.as_ref(),
            Self::Button(v) => v.as_ref(),
            Self::Switch(v) => v.as_ref(),
        }
    }
    fn enabled(&self, enabled: bool) {
        match self {
            Self::Popup(v) => v.setEnabled(enabled),
            Self::Button(v) => v.setEnabled(enabled),
            Self::Switch(v) => v.setEnabled(enabled),
        }
    }
    fn value(&self, value: &Value, options: &[Value]) {
        match self {
            Self::Popup(v) => {
                if let Some(index) = options
                    .iter()
                    .position(|o| o.get("v").unwrap_or(o) == value)
                {
                    v.selectItemAtIndex(index as isize);
                }
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
}
impl EventEmitter<Value> for NativeControl {}

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
        let width = if matches!(kind, Kind::Popup) && !inline {
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
        }
    }
    pub fn keep(&self) {
        self.slot.keep();
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
        div().relative().w(px(self.width)).h(px(self.height)).child(
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
    for option in options {
        popup.addItemWithTitle(&NSString::from_str(
            option
                .get("label")
                .and_then(Value::as_str)
                .or_else(|| option.as_str())
                .unwrap_or(""),
        ));
    }
}
