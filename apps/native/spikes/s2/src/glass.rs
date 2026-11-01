//! Adapted from apps/mac/crates/glass: NSToolbar and AppKit control/material placement.
//! S2 only: solid work surfaces; materials are hosted in native slot bands.

use std::cell::RefCell;
use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSAppearanceCustomization, NSBox, NSBoxType};
use objc2_app_kit::{
    NSBezelStyle, NSButton, NSColor, NSControl, NSGlassEffectView, NSGlassEffectViewStyle, NSImage,
    NSSegmentSwitchTracking, NSSegmentedControl, NSStackView, NSToolbar, NSToolbarDelegate,
    NSToolbarFlexibleSpaceItemIdentifier, NSToolbarItem, NSToolbarItemIdentifier,
    NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowOrderingMode, NSWindowToolbarStyle,
};
use objc2_foundation::{NSArray, NSCopying, NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// A native control was used. `Action` codes are stable; see [`ACTIONS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Tool(Tool),
    PlayPause,
    ToggleSidebar,
    ToggleInspector,
    ToggleAgent,
    Export,
    ZoomIn,
    ZoomOut,
    ZoomFit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Pen,
    Text,
    Shape,
    Hand,
}

const TOOLS: [(Tool, &str, &str); 5] = [
    (Tool::Select, "cursorarrow", "Select (V)"),
    (Tool::Pen, "pencil.tip", "Pen (P)"),
    (Tool::Text, "textformat", "Text (T)"),
    (Tool::Shape, "square.on.circle", "Shapes (R)"),
    (Tool::Hand, "hand.raised", "Hand (H)"),
];

/// Tags for buttons whose target is [`ActionTarget`].
const ACTIONS: [Action; 8] = [
    Action::PlayPause,
    Action::ToggleSidebar,
    Action::ToggleInspector,
    Action::ToggleAgent,
    Action::Export,
    Action::ZoomIn,
    Action::ZoomOut,
    Action::ZoomFit,
];

fn tag_of(a: Action) -> isize {
    ACTIONS
        .iter()
        .position(|x| *x == a)
        .expect("action has a tag") as isize
}

/// Where a glass region sits relative to GPUI's drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Under GPUI: GPUI draws content on the glass.
    Behind,
    /// Over GPUI: native controls on glass that refracts the editor.
    Floating,
}

/// A rectangle in GPUI window coordinates (points, origin top-left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

struct Region {
    view: Retained<NSView>,
    seen: bool,
}

struct Host {
    mtm: MainThreadMarker,
    window: Retained<NSWindow>,
    content: Retained<NSView>,
    gpui: Retained<NSView>,
    regions: HashMap<&'static str, Region>,
    target: Retained<ActionTarget>,
    _toolbar: Retained<NSToolbar>,
    _delegate: Retained<ToolbarDelegate>,
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
    static SENDER: RefCell<Option<async_channel::Sender<Action>>> = const { RefCell::new(None) };
}

fn send(a: Action) {
    SENDER.with(|s| {
        if let Some(s) = s.borrow().as_ref() {
            let _ = s.try_send(a);
        }
    });
}

define_class!(
    /// Target of every native control; forwards to the action channel.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMActionTarget"]
    struct ActionTarget;

    impl ActionTarget {
        #[unsafe(method(act:))]
        fn act(&self, sender: &NSControl) {
            if let Some(a) = ACTIONS.get(sender.tag() as usize) {
                send(*a);
            }
        }

        #[unsafe(method(tool:))]
        fn tool(&self, sender: &NSSegmentedControl) {
            if let Some((t, _, _)) = TOOLS.get(sender.selectedSegment() as usize) {
                send(Action::Tool(*t));
            }
        }
    }
);

impl ActionTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

const ITEM_TOOLS: &str = "pm.tools";
const ITEM_SIDEBAR: &str = "pm.sidebar";
const ITEM_PLAY: &str = "pm.play";
const ITEM_AGENT: &str = "pm.agent";
const ITEM_EXPORT: &str = "pm.export";
const ITEM_INSPECTOR: &str = "pm.inspector";

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMToolbarDelegate"]
    #[ivars = Retained<ActionTarget>]
    struct ToolbarDelegate;

    unsafe impl NSObjectProtocol for ToolbarDelegate {}

    unsafe impl NSToolbarDelegate for ToolbarDelegate {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn item(&self, _toolbar: &NSToolbar, id: &NSToolbarItemIdentifier, _insert: bool) -> Option<Retained<NSToolbarItem>> {
            self.make_item(id)
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn defaults(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            identifiers()
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            identifiers()
        }
    }
);

impl ToolbarDelegate {
    fn make_item(&self, id: &NSToolbarItemIdentifier) -> Option<Retained<NSToolbarItem>> {
        let mtm = self.mtm();
        let target = self.ivars();
        let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), id);
        let key = id.to_string();
        match key.as_str() {
            ITEM_TOOLS => {
                let images: Vec<Retained<NSImage>> =
                    TOOLS.iter().map(|(_, s, l)| symbol(s, l)).collect();
                let refs: Vec<&NSImage> = images.iter().map(|i| &**i).collect();
                let control = unsafe {
                    NSSegmentedControl::segmentedControlWithImages_trackingMode_target_action(
                        &NSArray::from_slice(&refs),
                        NSSegmentSwitchTracking::SelectOne,
                        Some(target),
                        Some(sel!(tool:)),
                        mtm,
                    )
                };
                for (i, (_, _, label)) in TOOLS.iter().enumerate() {
                    control.setToolTip_forSegment(Some(&NSString::from_str(label)), i as isize);
                }
                control.setSelectedSegment(0);
                item.setView(Some(&control));
                item.setLabel(&NSString::from_str("Tools"));
            }
            ITEM_SIDEBAR => button_item(
                &item,
                target,
                "sidebar.left",
                "Layers",
                Action::ToggleSidebar,
            ),
            ITEM_PLAY => button_item(&item, target, "play.fill", "Play", Action::PlayPause),
            ITEM_AGENT => button_item(&item, target, "sparkles", "Agent", Action::ToggleAgent),
            ITEM_EXPORT => button_item(
                &item,
                target,
                "square.and.arrow.up",
                "Export",
                Action::Export,
            ),
            ITEM_INSPECTOR => button_item(
                &item,
                target,
                "sidebar.right",
                "Inspector",
                Action::ToggleInspector,
            ),
            _ => return None,
        }
        Some(item)
    }

    fn new(mtm: MainThreadMarker, target: Retained<ActionTarget>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(target);
        unsafe { msg_send![super(this), init] }
    }
}

fn identifiers() -> Retained<NSArray<NSToolbarItemIdentifier>> {
    let flex: &NSToolbarItemIdentifier = unsafe { NSToolbarFlexibleSpaceItemIdentifier };
    let ids: Vec<Retained<NSString>> = vec![
        NSString::from_str(ITEM_SIDEBAR),
        NSString::from_str(ITEM_TOOLS),
        flex.copy(),
        NSString::from_str(ITEM_PLAY),
        flex.copy(),
        NSString::from_str(ITEM_AGENT),
        NSString::from_str(ITEM_EXPORT),
        NSString::from_str(ITEM_INSPECTOR),
    ];
    let refs: Vec<&NSString> = ids.iter().map(|s| &**s).collect();
    NSArray::from_slice(&refs)
}

fn symbol(name: &str, label: &str) -> Retained<NSImage> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        Some(&NSString::from_str(label)),
    )
    .unwrap_or_else(|| panic!("SF Symbol {name}"))
}

fn button_item(
    item: &NSToolbarItem,
    target: &ActionTarget,
    symbol_name: &str,
    label: &str,
    action: Action,
) {
    let label_ns = NSString::from_str(label);
    item.setImage(Some(&symbol(symbol_name, label)));
    item.setLabel(&label_ns);
    item.setToolTip(Some(&label_ns));
    unsafe {
        item.setTarget(Some(target));
        item.setAction(Some(sel!(act:)));
    }
    item.setTag(tag_of(action));
}

/// Attach the chrome to GPUI's window. Returns the action channel.
pub fn install(window: &gpui::Window) -> async_channel::Receiver<Action> {
    let mtm = MainThreadMarker::new().expect("main thread");
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window)
        .expect("window handle")
        .as_raw()
    else {
        panic!("not an AppKit window");
    };
    // SAFETY: GPUI hands out its live NSView; we only retain it.
    let gpui: Retained<NSView> =
        unsafe { Retained::retain(handle.ns_view.as_ptr().cast()) }.expect("GPUI view");
    let ns_window = gpui.window().expect("GPUI view is in a window");
    let content = ns_window.contentView().expect("content view");

    // The window background shows where GPUI draws nothing and under the
    // behind-glass regions.
    let backdrop = NSBox::initWithFrame(NSBox::alloc(mtm), content.bounds());
    backdrop.setBoxType(NSBoxType::Custom);
    backdrop.setBorderWidth(0.0);
    backdrop.setFillColor(&NSColor::windowBackgroundColor());
    backdrop.setAutoresizingMask(
        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    content.addSubview_positioned_relativeTo(&backdrop, NSWindowOrderingMode::Below, None);

    let target = ActionTarget::new(mtm);
    let delegate = ToolbarDelegate::new(mtm, target.clone());
    let toolbar =
        NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &NSString::from_str("pm.main"));
    toolbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    toolbar.setDisplayMode(objc2_app_kit::NSToolbarDisplayMode::IconOnly);
    ns_window.setToolbar(Some(&toolbar));
    ns_window.setToolbarStyle(NSWindowToolbarStyle::Unified);
    ns_window.setTitlebarAppearsTransparent(true);
    // The editor is dark-only until the light theme lane lands.
    let dark = unsafe { objc2_app_kit::NSAppearanceNameDarkAqua };
    ns_window.setAppearance(objc2_app_kit::NSAppearance::appearanceNamed(dark).as_deref());

    let (tx, rx) = async_channel::unbounded();
    SENDER.with(|s| *s.borrow_mut() = Some(tx));
    HOST.with(|h| {
        *h.borrow_mut() = Some(Host {
            mtm,
            window: ns_window,
            content,
            gpui,
            regions: HashMap::new(),
            target,
            _toolbar: toolbar,
            _delegate: delegate,
        });
    });
    rx
}

/// Height of the titlebar + toolbar band GPUI should keep clear.
pub fn toolbar_inset() -> f64 {
    HOST.with(|h| {
        h.borrow().as_ref().map_or(52.0, |h| {
            let frame = h.window.frame();
            let layout = h.window.contentLayoutRect();
            frame.size.height - layout.size.height
        })
    })
}

/// Floating HUD contents, built once per region.
#[derive(Clone, Copy)]
pub enum Contents {
    /// GPUI draws on top.
    None,
    /// Native buttons: (SF Symbol, label, action).
    Buttons(&'static [(&'static str, &'static str, Action)]),
}

/// Put a glass region at `rect` for this frame.
pub fn place(id: &'static str, rect: Rect, radius: f64, layer: Layer, contents: Contents) {
    HOST.with(|h| {
        let mut h = h.borrow_mut();
        let Some(h) = h.as_mut() else { return };
        let height = h.content.bounds().size.height;
        // AppKit's content view is bottom-left origin.
        let frame = NSRect::new(
            NSPoint::new(rect.x, height - rect.y - rect.h),
            NSSize::new(rect.w, rect.h),
        );
        let (mtm, target, content, gpui) =
            (h.mtm, h.target.clone(), h.content.clone(), h.gpui.clone());
        let region = h.regions.entry(id).or_insert_with(|| {
            let view = crate::native::material(mtm, frame);
            if let Contents::Buttons(buttons) = contents {
                let row = button_row(mtm, &target, buttons);
                row.setFrame(view.bounds());
                view.addSubview(&row);
            }
            let order = if layer == Layer::Behind {
                NSWindowOrderingMode::Below
            } else {
                NSWindowOrderingMode::Above
            };
            content.addSubview_positioned_relativeTo(&view, order, Some(&gpui));
            Region { view, seen: false }
        });
        if region.view.frame() != frame {
            region.view.setFrame(frame);
        }
        region.view.setWantsLayer(true);
        if let Some(layer) = region.view.layer() {
            layer.setCornerRadius(radius);
        }
        region.view.setHidden(false);
        region.seen = true;
    });
}

/// Hide regions not placed since the last call.
pub fn finish_frame() {
    HOST.with(|h| {
        let mut h = h.borrow_mut();
        let Some(h) = h.as_mut() else { return };
        for r in h.regions.values_mut() {
            if !r.seen {
                r.view.setHidden(true);
            }
            r.seen = false;
        }
    });
}

fn button_row(
    mtm: MainThreadMarker,
    target: &ActionTarget,
    buttons: &[(&str, &str, Action)],
) -> Retained<NSView> {
    let views: Vec<Retained<NSView>> = buttons
        .iter()
        .map(|(sym, label, action)| {
            let b = unsafe {
                NSButton::buttonWithImage_target_action(
                    &symbol(sym, label),
                    Some(target),
                    Some(sel!(act:)),
                    mtm,
                )
            };
            b.setBordered(false);
            b.setBezelStyle(
                if objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some() {
                    NSBezelStyle::Glass
                } else {
                    NSBezelStyle::Rounded
                },
            );
            b.setTag(tag_of(*action));
            b.setToolTip(Some(&NSString::from_str(label)));
            Retained::into_super(Retained::into_super(b))
        })
        .collect();
    let refs: Vec<&NSView> = views.iter().map(|v| &**v).collect();
    let stack = NSStackView::stackViewWithViews(&NSArray::from_slice(&refs), mtm);
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    stack.setSpacing(2.0);
    stack.setDistribution(objc2_app_kit::NSStackViewDistribution::FillEqually);
    stack.setEdgeInsets(objc2_foundation::NSEdgeInsets {
        top: 4.0,
        left: 8.0,
        bottom: 4.0,
        right: 8.0,
    });
    Retained::into_super(stack)
}
