//! AppKit windows and toolbar. No project/document state is authored here.
use crate::{
    keyboard::{Class, Effect, Key, Resolver},
    menu,
    transient::{Choice, Picker},
};
use foreign_types::ForeignType;
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};
use pm_design::{Accessibility, Material, Scheme, Surface, Tokens};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};
#[derive(Clone, Debug)]
pub struct Command {
    pub name: String,
    pub args: Vec<serde_json::Value>,
    pub token: Option<u64>,
}
thread_local! {
    static ROUTES:RefCell<HashMap<isize,async_channel::Sender<Command>>>=RefCell::new(HashMap::new());
    static MENU_TARGET:RefCell<Option<Retained<Target>>>=const{RefCell::new(None)};
    static REPLAYING:Cell<bool>=const{Cell::new(false)};
}
fn active_send(command: Command) {
    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    if let Some(w) = app.keyWindow().or_else(|| app.mainWindow()) {
        ROUTES.with(|r| {
            if let Some(tx) = r.borrow().get(&w.windowNumber()) {
                let _ = tx.try_send(command);
            }
        });
    }
}
pub fn settings() -> Accessibility {
    let w = NSWorkspace::sharedWorkspace();
    Accessibility {
        reduce_transparency: w.accessibilityDisplayShouldReduceTransparency(),
        increase_contrast: w.accessibilityDisplayShouldIncreaseContrast(),
        reduce_motion: w.accessibilityDisplayShouldReduceMotion(),
    }
}
pub fn native_color(rgb: u32) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        ((rgb >> 16) & 255) as f64 / 255.,
        ((rgb >> 8) & 255) as f64 / 255.,
        (rgb & 255) as f64 / 255.,
        1.,
    )
}
pub fn appearance(w: &NSWindow, t: &Tokens) {
    if std::env::var_os("PM_NATIVE_APPEARANCE").is_none() {
        w.setAppearance(None);
        return;
    }
    let name = unsafe {
        if t.scheme == Scheme::Light {
            NSAppearanceNameAqua
        } else {
            NSAppearanceNameDarkAqua
        }
    };
    w.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
}
pub fn rect(r: crate::layout::Rect, h: f64) -> NSRect {
    NSRect::new(
        NSPoint::new(r.x as f64, h - r.y as f64 - r.h as f64),
        NSSize::new(r.w as f64, r.h as f64),
    )
}
pub fn material(
    mtm: MainThreadMarker,
    frame: NSRect,
    t: &Tokens,
    surface: Surface,
) -> Retained<NSView> {
    match t.material(
        surface,
        objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some(),
    ) {
        Material::Solid => {
            let v = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
            v.setBoxType(NSBoxType::Custom);
            v.setFillColor(&native_color(t.colors.floating));
            v.setBorderColor(&native_color(t.colors.separator));
            v.setBorderWidth(t.metrics.border as f64);
            v.setCornerRadius(t.metrics.panel_radius as f64);
            Retained::into_super(v)
        }
        Material::Glass => {
            let v = NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), frame);
            v.setCornerRadius(t.metrics.panel_radius as f64);
            let body =
                NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, frame.size));
            v.setContentView(Some(&body));
            Retained::into_super(v)
        }
        Material::VisualEffect => {
            let v = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
            v.setMaterial(NSVisualEffectMaterial::Popover);
            v.setState(NSVisualEffectState::Active);
            Retained::into_super(v)
        }
    }
}
pub fn contents(v: &NSView) -> Retained<NSView> {
    unsafe {
        if objc2::runtime::AnyClass::get(c"NSGlassEffectView")
            .is_some_and(|c| msg_send![v,isKindOfClass:c])
        {
            msg_send![v, contentView]
        } else {
            Retained::retain(v as *const _ as *mut NSView).unwrap()
        }
    }
}
struct TargetState {
    replays: RefCell<HashMap<usize, u64>>,
    commands: Vec<String>,
    sender: Option<async_channel::Sender<Command>>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMShellTarget"]
    #[ivars = TargetState]
    struct Target;

    impl Target {
        #[unsafe(method(act:))]
        fn act(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            if let Some(name) = self.ivars().commands.get(tag as usize) {
                if name == "settings" {
                    let app = NSApplication::sharedApplication(self.mtm());
                    if let Some(w) = app.keyWindow() && w.title().to_string() == "Settings" {
                        w.performClose(None);
                        return;
                    }
                }
                let cmd = Command { name: name.clone(), args: vec![], token: None };
                if let Some(tx) = &self.ivars().sender {
                    let _ = tx.try_send(cmd);
                } else {
                    active_send(cmd);
                }
            }
        }

        #[unsafe(method(replayEvent:))]
        fn replay_event(&self,event:&NSEvent){
            let token=self.ivars().replays.borrow_mut().remove(&(event as *const NSEvent as usize));
            REPLAYING.with(|guard|{guard.set(true);NSApplication::sharedApplication(self.mtm()).sendEvent(event);guard.set(false);});
            if let (Some(token),Some(sender))=(token,&self.ivars().sender){let _=sender.try_send(Command{name:"shellReplayComplete".into(),args:vec![],token:Some(token)});}
        }
        #[unsafe(method(tool:))]
        fn tool(&self, sender: &NSSegmentedControl) {
            if let Some((command, _, _)) = TOOLS.get(sender.selectedSegment() as usize)
                && let Some(tx) = &self.ivars().sender {
                let _ = tx.try_send(Command { name: command.to_string(), args: vec![], token: None });
            }
        }
    }
);
impl Target {
    fn new(
        mtm: MainThreadMarker,
        commands: Vec<String>,
        sender: Option<async_channel::Sender<Command>>,
    ) -> Retained<Self> {
        unsafe {
            msg_send![
                super(Self::alloc(mtm).set_ivars(TargetState {
                    replays: RefCell::new(HashMap::new()),
                    commands,
                    sender
                })),
                init
            ]
        }
    }
}
const TOOLS: [(&str, &str, &str); 8] = [
    ("toolSelect", "cursorarrow", "Select (V)"),
    ("toolHand", "hand.raised", "Hand (H)"),
    ("toolZoom", "magnifyingglass", "Zoom (Z)"),
    ("toolRotate", "crop.rotate", "Rotate (W)"),
    ("toolAnchor", "scope", "Anchor (Y)"),
    ("toolShape", "square.on.circle", "Shape (Q)"),
    ("toolPen", "pencil.tip", "Pen (G)"),
    ("toolText", "textformat", "Text (⌘T)"),
];
const TOOLBAR: [&str; 6] = [
    "toggleSidebar",
    "title",
    "tools",
    "agent",
    "export",
    "toggleInspector",
];
struct ToolbarState {
    target: Retained<Target>,
    tokens: Tokens,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[name = "PMShellToolbarDelegate"]
    #[ivars=ToolbarState]
    struct ToolbarDelegate;
    unsafe impl NSObjectProtocol for ToolbarDelegate {}
    unsafe impl NSToolbarDelegate for ToolbarDelegate {
        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn defaults(&self, _: &NSToolbar) -> Retained<NSArray<NSString>> {
            toolbar_ids()
        }
        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed(&self, _: &NSToolbar) -> Retained<NSArray<NSString>> {
            toolbar_ids()
        }
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn item(&self, _: &NSToolbar, id: &NSString, _: bool) -> Option<Retained<NSToolbarItem>> {
            self.make(id)
        }
    }
);
fn toolbar_ids() -> Retained<NSArray<NSString>> {
    let mut v: Vec<_> = TOOLBAR.iter().map(|s| NSString::from_str(s)).collect();
    let f = unsafe { NSToolbarFlexibleSpaceItemIdentifier };
    v.insert(2, NSString::from_str(&f.to_string()));
    v.insert(4, NSString::from_str(&f.to_string()));
    NSArray::from_retained_slice(&v)
}
fn symbol(s: &str, label: &str) -> Retained<NSImage> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(s),
        Some(&NSString::from_str(label)),
    )
    .expect("SF Symbol")
}
impl ToolbarDelegate {
    fn new(mtm: MainThreadMarker, target: Retained<Target>, tokens: Tokens) -> Retained<Self> {
        unsafe {
            msg_send![
                super(Self::alloc(mtm).set_ivars(ToolbarState { target, tokens })),
                init
            ]
        }
    }
    fn make(&self, id: &NSString) -> Option<Retained<NSToolbarItem>> {
        let mtm = self.mtm();
        let t = &self.ivars().tokens;
        let target = &self.ivars().target;
        let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), id);
        let name = id.to_string();
        if name == "tools" {
            let images: Vec<_> = TOOLS.iter().map(|(_, s, l)| symbol(s, l)).collect();
            let control = unsafe {
                NSSegmentedControl::segmentedControlWithImages_trackingMode_target_action(
                    &NSArray::from_retained_slice(&images),
                    NSSegmentSwitchTracking::SelectOne,
                    Some(target),
                    Some(sel!(tool:)),
                    mtm,
                )
            };
            control.setSelectedSegment(0);
            for (i, (_, _, l)) in TOOLS.iter().enumerate() {
                control.setToolTip_forSegment(Some(&NSString::from_str(l)), i as isize);
            }
            item.setView(Some(&control));
            item.setLabel(&NSString::from_str("Tools"));
            return Some(item);
        }
        if name == "title" {
            let title = NSTextField::labelWithString(&NSString::from_str("Untitled"), mtm);
            title.setFont(Some(&NSFont::systemFontOfSize(t.typography.title as f64)));
            let edited = NSTextField::labelWithString(&NSString::from_str("Edited"), mtm);
            edited.setFont(Some(&NSFont::systemFontOfSize(t.typography.minimum as f64)));
            edited.setTextColor(Some(&native_color(t.colors.muted)));
            let view = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(
                    NSPoint::ZERO,
                    NSSize::new(
                        t.metrics.toolbar_title_width as f64,
                        t.metrics.control as f64,
                    ),
                ),
            );
            title.setFrame(NSRect::new(
                NSPoint::new(0., t.metrics.toolbar_title_line as f64),
                NSSize::new(
                    t.metrics.toolbar_title_width as f64,
                    t.metrics.toolbar_title_line as f64,
                ),
            ));
            edited.setFrame(NSRect::new(
                NSPoint::ZERO,
                NSSize::new(
                    t.metrics.toolbar_title_width as f64,
                    t.metrics.toolbar_title_line as f64,
                ),
            ));
            view.addSubview(&title);
            view.addSubview(&edited);
            view.widthAnchor()
                .constraintEqualToConstant(t.metrics.toolbar_title_width as f64)
                .setActive(true);
            view.heightAnchor()
                .constraintEqualToConstant(t.metrics.control as f64)
                .setActive(true);
            item.setView(Some(&view));
            item.setBordered(false);
            item.setLabel(&NSString::from_str("Document"));
            return Some(item);
        }
        let (sym, label) = match name.as_str() {
            "toggleSidebar" => ("sidebar.left", "Sidebar"),
            "agent" => ("sparkles", "Agent"),
            "export" => ("square.and.arrow.up", "Export"),
            "toggleInspector" => ("sidebar.right", "Inspector"),
            _ => return None,
        };
        let b = unsafe {
            if name == "export" {
                NSButton::buttonWithTitle_image_target_action(
                    &NSString::from_str(label),
                    &symbol(sym, label),
                    Some(target),
                    Some(sel!(act:)),
                    mtm,
                )
            } else {
                NSButton::buttonWithImage_target_action(
                    &symbol(sym, label),
                    Some(target),
                    Some(sel!(act:)),
                    mtm,
                )
            }
        };
        b.setTag(TOOLBAR.iter().position(|n| *n == name).unwrap() as isize);
        b.setToolTip(Some(&NSString::from_str(label)));
        b.setAccessibilityLabel(Some(&NSString::from_str(label)));
        b.setBezelStyle(
            if t.material(
                Surface::Toolbar,
                objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some(),
            ) == Material::Glass
            {
                NSBezelStyle::Glass
            } else {
                NSBezelStyle::Push
            },
        );
        if name == "export" {
            if objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some() {
                b.setTintProminence(NSTintProminence::Primary);
            }
            b.setBezelColor(Some(&native_color(t.colors.accent)));
            b.setContentTintColor(Some(&native_color(t.colors.selected_text)));
        }
        item.setView(Some(&b));
        item.setLabel(&NSString::from_str(label));
        Some(item)
    }
}

pub struct Host {
    pub window: Retained<NSWindow>,
    pub gpui: Retained<NSView>,
    content: Retained<NSView>,
    viewer: Retained<NSView>,
    pub viewer_rect: Rc<Cell<Option<crate::layout::Rect>>>,
    pub splitter_focus: bool,
    layer: metal::MetalLayer,
    viewer_transform: Option<([f64; 2], Option<f64>)>,
    _toolbar: Retained<NSToolbar>,
    _delegate: Retained<ToolbarDelegate>,
    _target: Retained<Target>,
    observer: Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    registration: Option<gpui_macos::composition::CompositionRegistration>,
    monitor: Option<Retained<AnyObject>>,
    resolver: Resolver,
    events: HashMap<u64, Retained<NSEvent>>,
    next_token: u64,
    sender: async_channel::Sender<Command>,
    pub tokens: Tokens,
    pub palette: Rc<RefCell<Popup>>,
    pub fields: Rc<RefCell<Popup>>,
}
impl Host {
    pub fn install(
        w: &gpui::Window,
        device: &metal::Device,
        t: Tokens,
    ) -> (Rc<RefCell<Self>>, async_channel::Receiver<Command>) {
        let mtm = MainThreadMarker::new().unwrap();
        let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(w).unwrap().as_raw()
        else {
            panic!("macOS required")
        };
        let gpui: Retained<NSView> =
            unsafe { Retained::retain(handle.ns_view.as_ptr().cast()).unwrap() };
        let window = gpui.window().unwrap();
        let content = window.contentView().unwrap();
        appearance(&window, &t);
        window.setTitle(&NSString::from_str("Untitled — Powermove Native"));
        window.setSubtitle(&NSString::from_str("Edited"));
        window.setDocumentEdited(true);
        window.setTabbingIdentifier(&NSString::from_str("com.powermove.native.editor"));
        window.setTabbingMode(NSWindowTabbingMode::Preferred);
        window.setRestorable(true);
        if std::env::var_os("PM_NATIVE_SHELL_STATE").is_none() {
            window.setFrameAutosaveName(&NSString::from_str("PowermoveNative.Editor"));
            window.setFrameUsingName(&NSString::from_str("PowermoveNative.Editor"));
        }
        window.setMinSize(NSSize::new(
            t.metrics.min_width as f64,
            t.metrics.min_height as f64,
        ));
        window.setCollectionBehavior(NSWindowCollectionBehavior::FullScreenPrimary);
        window.setBackgroundColor(Some(&native_color(t.colors.window)));
        let (tx, rx) = async_channel::unbounded();
        ROUTES.with(|r| {
            r.borrow_mut().insert(window.windowNumber(), tx.clone());
        });
        let target = Target::new(
            mtm,
            TOOLBAR.iter().map(|s| s.to_string()).collect(),
            Some(tx.clone()),
        );
        let delegate = ToolbarDelegate::new(mtm, target.clone(), t.clone());
        let toolbar = NSToolbar::initWithIdentifier(
            NSToolbar::alloc(mtm),
            &NSString::from_str("pm.native.main"),
        );
        toolbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
        toolbar.setCenteredItemIdentifiers(&objc2_foundation::NSSet::from_retained_slice(&[
            NSString::from_str("tools"),
        ]));
        window.setToolbar(Some(&toolbar));
        window.setToolbarStyle(NSWindowToolbarStyle::UnifiedCompact);
        window.setTitlebarAppearsTransparent(true);
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        let viewer = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        viewer.setWantsLayer(true);
        viewer.setAccessibilityElement(true);
        viewer.setAccessibilityRole(Some(&NSString::from_str("AXImage")));
        viewer.setAccessibilityLabel(Some(&NSString::from_str("Viewer placeholder")));
        viewer.setAccessibilityIdentifier(Some(&NSString::from_str("shell.viewer")));
        let layer = metal::MetalLayer::new();
        layer.set_device(device);
        layer.set_pixel_format(metal::MTLPixelFormat::RGBA16Float);
        layer.set_opaque(true);
        layer.set_presents_with_transaction(true);
        unsafe {
            let _: () = msg_send![&*viewer,setLayer:layer.as_ptr() as *mut AnyObject];
            let _: () = msg_send![layer.as_ptr() as *mut AnyObject,setWantsExtendedDynamicRangeContent:true];
        }
        if let Some(l) = viewer.layer() {
            l.setBackgroundColor(Some(&native_color(t.colors.viewer).CGColor()));
        }
        content.addSubview_positioned_relativeTo(&viewer, NSWindowOrderingMode::Below, Some(&gpui));
        let choices = menu::menus()
            .into_iter()
            .flat_map(|(_, i)| i)
            .filter(|i| !i.command.is_empty() && i.role.is_none() && i.command != "workspaces")
            .map(|i| Choice {
                label: i.label,
                command: i.command,
            })
            .chain(crate::layout::PRESETS.iter().map(|(id, name)| Choice {
                label: format!("Workspace: {name}"),
                command: format!("workspace:{id}"),
            }))
            .collect();
        let palette = Popup::new(&window, &t, Surface::Palette, choices, tx.clone());
        let fields = Popup::new(
            &window,
            &t,
            Surface::Menu,
            ["Position", "Scale", "Rotation", "Opacity"]
                .iter()
                .map(|n| Choice {
                    label: n.to_string(),
                    command: format!("field:{n}"),
                })
                .collect(),
            tx.clone(),
        );
        let host = Rc::new(RefCell::new(Self {
            window,
            gpui: gpui.clone(),
            content,
            viewer,
            viewer_rect: Rc::new(Cell::new(None)),
            splitter_focus: false,
            layer,
            viewer_transform: None,
            _toolbar: toolbar,
            _delegate: delegate,
            _target: target,
            observer: None,
            registration: None,
            monitor: None,
            resolver: Resolver::new(),
            events: HashMap::new(),
            next_token: 0,
            sender: tx,
            tokens: t,
            palette,
            fields,
        }));
        let weak = Rc::downgrade(&host);
        let hit_rect = host.borrow().viewer_rect.clone();
        let reg = gpui_macos::composition::register(
            &gpui,
            Rc::new(move || {
                if let Some(h) = weak.upgrade() {
                    h.borrow_mut().present();
                }
            }),
            Rc::new(move |p| {
                !hit_rect
                    .get()
                    .is_some_and(|r| r.contains(p.x as f32, p.y as f32))
            }),
        );
        host.borrow_mut().registration = Some(reg);
        let weak = Rc::downgrade(&host);
        let handler =
            block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
                let e = unsafe { event.as_ref() };
                if REPLAYING.with(|b| b.get()) {
                    return event.as_ptr();
                }
                if let Some(host) = weak.upgrade() {
                    let (palette, fields, window, splitter_focus) = {
                        let h = host.borrow();
                        (
                            h.palette.clone(),
                            h.fields.clone(),
                            h.window.clone(),
                            h.splitter_focus,
                        )
                    };
                    if matches!(
                        e.r#type(),
                        NSEventType::LeftMouseDown | NSEventType::RightMouseDown
                    ) {
                        for popup in [&palette, &fields] {
                            let mut p = popup.borrow_mut();
                            if p.model.open && e.windowNumber() != p.window.windowNumber() {
                                p.hide();
                            }
                        }
                        return event.as_ptr();
                    }
                    for popup in [&palette, &fields] {
                        let mut p = popup.borrow_mut();
                        if e.windowNumber() == p.window.windowNumber() && p.model.open {
                            if e.r#type() == NSEventType::KeyDown {
                                let key = chord(e);
                                if matches!(
                                    key.as_str(),
                                    "up" | "down" | "home" | "end" | "enter" | "escape" | "tab"
                                ) {
                                    p.key(&key);
                                    return std::ptr::null_mut();
                                }
                            }
                            return event.as_ptr();
                        }
                    }
                    if e.windowNumber() != window.windowNumber() {
                        return event.as_ptr();
                    }
                    let responder = window.firstResponder();
                    let name = responder
                        .as_ref()
                        .map(|r| r.class().name().to_string_lossy().to_string())
                        .unwrap_or_default();
                    // WebKit keeps its DOM-first key order. S5 forwards unhandled keys explicitly.
                    if name.contains("WK") || name.contains("Web") {
                        return event.as_ptr();
                    }
                    let chord = chord(e);
                    if splitter_focus
                        && matches!(chord.as_str(), "up" | "down" | "left" | "right" | "enter")
                    {
                        return event.as_ptr();
                    }
                    if matches!(chord.as_str(), "enter" | "space")
                        && responder.as_ref().is_some_and(|r| unsafe {
                            msg_send![&**r,isKindOfClass:NSButton::class()]
                        })
                    {
                        return event.as_ptr();
                    }
                    let field = responder.as_ref().is_some_and(|r| unsafe {
                        msg_send![&**r,isKindOfClass:NSTextView::class()]
                    });
                    let mut h = host.borrow_mut();
                    h.next_token += 1;
                    let token = h.next_token;
                    let key = Key {
                        token,
                        chord,
                        field,
                        repeat: e.isARepeat(),
                        up: e.r#type() == NSEventType::KeyUp,
                        compat: false,
                    };
                    h.events
                        .insert(token, unsafe { Retained::retain(event.as_ptr()).unwrap() });
                    let effects = h.resolver.submit(key);
                    h.effects(effects);
                    return std::ptr::null_mut();
                }
                event.as_ptr()
            });
        host.borrow_mut().monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::KeyDown
                    | NSEventMask::KeyUp
                    | NSEventMask::LeftMouseDown
                    | NSEventMask::RightMouseDown,
                &handler,
            )
        };
        let sender = host.borrow().sender.clone();
        let changed = block2::RcBlock::new(
            move |_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
                let _ = sender.try_send(Command {
                    name: "shellAppearanceChanged".into(),
                    args: vec![],
                    token: None,
                });
            },
        );
        let observer = unsafe {
            NSWorkspace::sharedWorkspace()
                .notificationCenter()
                .addObserverForName_object_queue_usingBlock(
                    Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
                    None,
                    Some(&objc2_foundation::NSOperationQueue::mainQueue()),
                    &changed,
                )
        };
        host.borrow_mut().observer = Some(observer);
        (host, rx)
    }
    fn effects(&mut self, effects: Vec<Effect>) {
        for e in effects {
            match e {
                Effect::Dispatch {
                    token,
                    command,
                    args,
                    class,
                } => {
                    let _ = self.sender.try_send(Command {
                        name: command,
                        args,
                        token: Some(token),
                    });
                    if class == Class::NeverDeclines {
                        self.events.remove(&token);
                    }
                }
                Effect::Replay(token) => {
                    if let Some(event) = self.events.remove(&token) {
                        self._target
                            .ivars()
                            .replays
                            .borrow_mut()
                            .insert(Retained::as_ptr(&event) as usize, token);
                        unsafe {
                            let _: () = msg_send![&*self._target,performSelector:sel!(replayEvent:),withObject:&*event,afterDelay:0.0f64];
                        }
                    }
                }
                Effect::Consumed(token) => {
                    self.events.remove(&token);
                }
            }
        }
    }
    pub fn smoke_validate(&self) {
        assert!(!self.window.isVisible(), "Smoke window became visible");
        assert!(!self.palette.borrow().window.isVisible());
        assert!(!self.fields.borrow().window.isVisible());
        let ids: Vec<_> = self
            ._toolbar
            .items()
            .to_vec()
            .iter()
            .map(|i| i.itemIdentifier().to_string())
            .collect();
        for id in TOOLBAR {
            assert!(
                ids.iter().any(|item| item == id),
                "Missing toolbar item: {id}"
            );
        }
        assert!(self.window.isDocumentEdited());
        assert_eq!(
            self.layer.pixel_format(),
            metal::MTLPixelFormat::RGBA16Float
        );
        eprintln!(
            "PASS hidden AppKit initialization: toolbar, pooled search windows, edited state, viewer layer"
        );
    }
    pub fn set_document_metadata(&self, title: &str, edited: bool, path: Option<&std::path::Path>) {
        self.window.setTitle(&NSString::from_str(title));
        self.window.setDocumentEdited(edited);
        self.window
            .setSubtitle(&NSString::from_str(if edited { "Edited" } else { "" }));
        let url = path.map(|p| {
            objc2_foundation::NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))
        });
        self.window.setRepresentedURL(url.as_deref());
        for item in self._toolbar.items().to_vec() {
            if item.itemIdentifier().to_string() == "title"
                && let Some(v) = item.view()
            {
                unsafe {
                    let labels = v.subviews().to_vec();
                    if labels.len() == 2 {
                        let _: () =
                            msg_send![&*labels[0],setStringValue:&*NSString::from_str(title)];
                        let _: () = msg_send![&*labels[1],setStringValue:&*NSString::from_str(if edited{"Edited"}else{""})];
                    }
                }
            }
        }
    }
    pub fn update_tokens(&mut self, t: Tokens) {
        appearance(&self.window, &t);
        self.window
            .setBackgroundColor(Some(&native_color(t.colors.window)));
        if let Some(layer) = self.viewer.layer() {
            layer.setBackgroundColor(Some(&native_color(t.colors.viewer).CGColor()));
        }
        for (popup, surface) in [
            (&self.palette, Surface::Palette),
            (&self.fields, Surface::Menu),
        ] {
            popup.borrow_mut().update_tokens(&t, surface);
        }
        self.tokens = t;
    }
    pub fn replayed(&mut self, token: u64) {
        let effects = self.resolver.replayed(token);
        self.effects(effects);
    }
    pub fn resolve(&mut self, token: u64, handled: bool) {
        let effects = self.resolver.resolve(token, handled);
        self.effects(effects);
    }
    /// Retained below-slot layer; the app configures the service presenter.
    /// Opt in to a clipped content layer. Standalone shell keeps its placeholder.
    pub fn viewer_transform(&mut self, dimensions: [f64; 2], zoom: Option<f64>) {
        if self.viewer_transform.is_none() {
            let root = objc2_quartz_core::CALayer::new();
            root.setMasksToBounds(true);
            root.setBackgroundColor(Some(&native_color(self.tokens.colors.viewer).CGColor()));
            unsafe {
                let _: () = msg_send![&*self.viewer,setLayer:&*root];
                let _: () = msg_send![&*root,addSublayer:self.layer.as_ptr() as *mut AnyObject];
            }
        }
        self.viewer_transform = Some((dimensions, zoom));
    }
    pub fn add_bindings(&mut self, bindings: impl IntoIterator<Item = crate::keyboard::Binding>) {
        self.resolver.bindings.extend(bindings);
        self.resolver
            .bindings
            .sort_by_key(|b| std::cmp::Reverse(b.priority));
    }
    pub fn viewer_layer(&self) -> metal::MetalLayer {
        self.layer.clone()
    }
    pub fn set_viewer_label(&self, label: &str) {
        self.viewer
            .setAccessibilityLabel(Some(&NSString::from_str(label)));
    }
    pub fn toolbar_inset(&self) -> f32 {
        (self.gpui.bounds().size.height - self.window.contentLayoutRect().size.height).max(0.)
            as f32
    }
    fn present(&mut self) {
        let h = self.content.bounds().size.height;
        if let Some(r) = self.viewer_rect.get() {
            self.viewer.setHidden(false);
            self.viewer.setFrame(rect(r, h));
            let scale = self.window.backingScaleFactor();
            let (width, height) = if let Some((dimensions, zoom)) = self.viewer_transform {
                let fit = (r.w as f64 / dimensions[0]).min(r.h as f64 / dimensions[1]);
                let zoom = zoom
                    .unwrap_or(fit)
                    .min(8192. / dimensions[0].max(dimensions[1]) / scale);
                (dimensions[0] * zoom, dimensions[1] * zoom)
            } else {
                (r.w as f64, r.h as f64)
            };
            unsafe {
                let _: () = msg_send![self.layer.as_ptr() as *mut AnyObject,setFrame:NSRect::new(
                    NSPoint::new((r.w as f64-width)/2., (r.h as f64-height)/2.), NSSize::new(width,height))];
            }
            self.layer.set_contents_scale(scale);
            self.layer
                .set_drawable_size(core_graphics::geometry::CGSize::new(
                    width * scale,
                    height * scale,
                ));
        } else {
            self.viewer.setHidden(true);
        }
        gpui_macos::composition::publish_accessibility(
            &self.gpui,
            vec![
                gpui_macos::composition::AccessibilityItem::RemainingGpui,
                gpui_macos::composition::AccessibilityItem::Native(self.viewer.clone()),
            ],
        );
    }
    pub fn popup(&mut self, field: bool) {
        let other = if field { &self.palette } else { &self.fields };
        if other.borrow().model.open {
            other.borrow_mut().hide();
        }
        let p = if field { &self.fields } else { &self.palette };
        p.borrow_mut().toggle();
    }
    pub fn system(&self, name: &str) -> bool {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        match name {
            "quit" => app.terminate(None),
            "closeWindow" | "closeTab" => self.window.performClose(None),
            "nextTab" => unsafe {
                let _: () = msg_send![&*self.window,selectNextTab:std::ptr::null::<AnyObject>()];
            },
            "previousTab" => unsafe {
                let _: () =
                    msg_send![&*self.window,selectPreviousTab:std::ptr::null::<AnyObject>()];
            },
            "moveTabToNewWindow" => unsafe {
                let _: () =
                    msg_send![&*self.window,moveTabToNewWindow:std::ptr::null::<AnyObject>()];
            },
            "blurField" => {
                if !self
                    .window
                    .firstResponder()
                    .as_ref()
                    .is_some_and(|r| unsafe { msg_send![&**r,isKindOfClass:NSTextView::class()] })
                {
                    return false;
                }
                self.window.makeFirstResponder(Some(&self.gpui));
            }
            name if name.starts_with("context") => {
                let responder = self.window.firstResponder();
                if !responder
                    .as_ref()
                    .is_some_and(|r| unsafe { msg_send![&**r,isKindOfClass:NSTextView::class()] })
                {
                    return false;
                }
                let selector = match name {
                    "contextUndo" => sel!(undo:),
                    "contextRedo" => sel!(redo:),
                    "contextCut" => sel!(cut:),
                    "contextCopy" => sel!(copy:),
                    "contextPaste" => sel!(paste:),
                    "contextSelectAll" => sel!(selectAll:),
                    _ => return false,
                };
                return unsafe { app.sendAction_to_from(selector, None, None) };
            }
            _ => return false,
        };
        true
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        if let Some(observer) = &self.observer {
            unsafe {
                NSWorkspace::sharedWorkspace()
                    .notificationCenter()
                    .removeObserver(&*(Retained::as_ptr(observer) as *const AnyObject));
            }
        }
        if let Some(m) = &self.monitor {
            unsafe {
                NSEvent::removeMonitor(m);
            }
        }
        ROUTES.with(|r| {
            r.borrow_mut().remove(&self.window.windowNumber());
        });
        self.registration.take();
        self.viewer.removeFromSuperview();
    }
}
fn chord(e: &NSEvent) -> String {
    let flags = e.modifierFlags();
    let mut s = String::new();
    for (flag, name) in [
        (NSEventModifierFlags::Command, "cmd+"),
        (NSEventModifierFlags::Control, "ctrl+"),
        (NSEventModifierFlags::Option, "alt+"),
        (NSEventModifierFlags::Shift, "shift+"),
    ] {
        if flags.contains(flag) {
            s.push_str(name);
        }
    }
    let key = match e.keyCode() {
        36 | 76 => "enter".into(),
        48 => "tab".into(),
        49 => "space".into(),
        51 => "backspace".into(),
        53 => "escape".into(),
        115 => "home".into(),
        119 => "end".into(),
        116 => "pageup".into(),
        121 => "pagedown".into(),
        117 => "delete".into(),
        123 => "left".into(),
        124 => "right".into(),
        125 => "down".into(),
        126 => "up".into(),
        _ => e
            .charactersIgnoringModifiers()
            .map(|s| s.to_string().to_lowercase())
            .unwrap_or_default(),
    };
    let key = crate::keyboard::physical_fallback(
        key,
        e.keyCode(),
        flags.contains(NSEventModifierFlags::Shift),
    );
    s.push_str(&key);
    s
}

define_class!(
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMShellPopupWindow"]
    struct PopupWindow;
    impl PopupWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_key(&self) -> bool { true }
        #[unsafe(method(canBecomeMainWindow))]
        fn can_main(&self) -> bool { false }
    }
);
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMShellPopupTarget"]
    #[ivars = std::rc::Weak<RefCell<Popup>>]
    struct PopupTarget;
    unsafe impl NSObjectProtocol for PopupTarget {}
    unsafe impl NSControlTextEditingDelegate for PopupTarget {
        #[unsafe(method(controlTextDidChange:))]
        fn changed(&self, _: &objc2_foundation::NSNotification) {
            if let Some(p) = self.ivars().upgrade() {
                let mut p = p.borrow_mut();
                let q = p.search.stringValue().to_string();
                p.model.search(q);
                p.refresh();
            }
        }
    }
    unsafe impl NSTextFieldDelegate for PopupTarget {}
    impl PopupTarget {
        #[unsafe(method(pick:))]
        fn pick(&self, sender: &NSControl) {
            if let Some(p) = self.ivars().upgrade() {
                let mut p = p.borrow_mut();
                let index = sender.tag() as usize;
                if let Some(c) = p.model.choices.get(index) {
                    let _ = p.sender.try_send(Command { name: c.command.clone(), args: vec![], token: None });
                }
                p.hide();
            }
        }
    }
);
pub struct Popup {
    window: Retained<PopupWindow>,
    parent: Retained<NSWindow>,
    search: Retained<NSTextField>,
    body: Retained<NSView>,
    spring: Option<(std::time::Instant, crate::transient::Spring)>,
    rows: Vec<Retained<NSButton>>,
    glider: Retained<NSBox>,
    target: Option<Retained<PopupTarget>>,
    model: Picker,
    sender: async_channel::Sender<Command>,
    tokens: Tokens,
    previous: Option<Retained<NSResponder>>,
}
impl Popup {
    fn new(
        parent: &NSWindow,
        t: &Tokens,
        surface: Surface,
        choices: Vec<Choice>,
        sender: async_channel::Sender<Command>,
    ) -> Rc<RefCell<Self>> {
        let mtm = MainThreadMarker::new().unwrap();
        let m = t.metrics;
        let (width, height) = if surface == Surface::Palette {
            (m.palette_width, m.palette_height)
        } else {
            (m.field_menu_width, m.field_menu_height)
        };
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(width as f64, height as f64));
        let window: Retained<PopupWindow> = unsafe {
            msg_send![PopupWindow::alloc(mtm),initWithContentRect:frame,styleMask:NSWindowStyleMask::Borderless,backing:NSBackingStoreType::Buffered,defer:false]
        };
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        window.setOpaque(false);
        window.setHasShadow(true);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        appearance(&window, t);
        let mat = material(mtm, frame, t, surface);
        let body = contents(&mat);
        window.setContentView(Some(&mat));
        mat.setWantsLayer(true);
        if let Some(layer) = mat.layer() {
            layer.setOpacity(0.);
        }
        let search = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(
                NSPoint::new(
                    m.menu_inset as f64,
                    (height - m.menu_inset - m.control) as f64,
                ),
                NSSize::new((width - m.menu_inset * 2.) as f64, m.control as f64),
            ),
        );
        search.setPlaceholderString(Some(&NSString::from_str(if surface == Surface::Palette {
            "Search commands…"
        } else {
            "Search fields…"
        })));
        search.setFont(Some(&NSFont::systemFontOfSize(t.typography.body as f64)));
        search.setAccessibilityLabel(Some(&NSString::from_str("Search")));
        body.addSubview(&search);
        let glider = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::ZERO);
        glider.setBoxType(NSBoxType::Custom);
        glider.setFillColor(&native_color(t.colors.selected_fill));
        glider.setBorderWidth(0.);
        glider.setCornerRadius(m.control_radius as f64);
        body.addSubview(&glider);
        let popup = Rc::new(RefCell::new(Self {
            window,
            parent: unsafe { Retained::retain(parent as *const _ as *mut _).unwrap() },
            search,
            body,
            spring: None,
            rows: vec![],
            glider,
            target: None,
            model: Picker {
                choices,
                ..Default::default()
            },
            sender,
            tokens: t.clone(),
            previous: None,
        }));
        let target: Retained<PopupTarget> = unsafe {
            msg_send![
                super(PopupTarget::alloc(mtm).set_ivars(Rc::downgrade(&popup))),
                init
            ]
        };
        unsafe {
            popup
                .borrow()
                .search
                .setDelegate(Some(ProtocolObject::from_ref(&*target)));
        }
        popup.borrow_mut().target = Some(target);
        popup.borrow_mut().refresh();
        popup
    }
    fn update_tokens(&mut self, t: &Tokens, surface: Surface) {
        self.tokens = t.clone();
        appearance(&self.window, t);
        let frame = NSRect::new(NSPoint::ZERO, self.window.frame().size);
        let mat = material(MainThreadMarker::new().unwrap(), frame, t, surface);
        let body = contents(&mat);
        self.search.removeFromSuperview();
        self.glider.removeFromSuperview();
        body.addSubview(&self.search);
        body.addSubview(&self.glider);
        self.glider
            .setFillColor(&native_color(t.colors.selected_fill));
        self.window.setContentView(Some(&mat));
        self.body = body;
        self.refresh();
    }
    fn refresh(&mut self) {
        for row in self.rows.drain(..) {
            row.removeFromSuperview();
        }
        let m = self.tokens.metrics;
        let size = self.window.frame().size;
        let visible =
            ((size.height as f32 - m.menu_inset * 3. - m.control) / m.menu_row).floor() as usize;
        let ids = self.model.filtered();
        let start = self
            .model
            .selected
            .saturating_sub(visible.saturating_sub(1));
        let mtm = MainThreadMarker::new().unwrap();
        self.glider.setHidden(ids.is_empty());
        for (i, index) in ids.into_iter().enumerate().skip(start).take(visible) {
            let c = &self.model.choices[index];
            let b = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(&c.label),
                    self.target.as_deref().map(|t| t as &AnyObject),
                    Some(sel!(pick:)),
                    mtm,
                )
            };
            b.setBordered(false);
            b.setAlignment(NSTextAlignment::Left);
            b.setFont(Some(&NSFont::systemFontOfSize(
                self.tokens.typography.body as f64,
            )));
            b.setContentTintColor(Some(&native_color(self.tokens.colors.text)));
            b.setTag(index as isize);
            let r = NSRect::new(
                NSPoint::new(
                    m.menu_inset as f64,
                    size.height
                        - (m.menu_inset * 2. + m.control + (i - start + 1) as f32 * m.menu_row)
                            as f64,
                ),
                NSSize::new(size.width - m.menu_inset as f64 * 2., m.menu_row as f64),
            );
            b.setFrame(r);
            if i == self.model.selected {
                self.glider.setFrame(r);
            }
            self.body.addSubview(&b);
            self.rows.push(b);
        }
    }
    fn toggle(&mut self) {
        if self.model.open {
            self.hide();
            return;
        }
        self.model.open = true;
        self.model.search(String::new());
        self.search.setStringValue(&NSString::from_str(""));
        self.previous = self.parent.firstResponder();
        self.refresh();
        let p = self.parent.frame();
        let s = self.window.frame().size;
        self.window.setFrameOrigin(NSPoint::new(
            p.origin.x + (p.size.width - s.width) / 2.,
            p.origin.y + p.size.height - s.height - self.tokens.metrics.toolbar as f64,
        ));
        unsafe {
            let _: () = msg_send![NSObject::class(),cancelPreviousPerformRequestsWithTarget:&*self.window,selector:sel!(orderOut:),object:std::ptr::null::<AnyObject>()];
            self.parent
                .addChildWindow_ordered(&self.window, NSWindowOrderingMode::Above);
        }
        self.window.makeKeyAndOrderFront(None);
        self.window.makeFirstResponder(Some(&self.search));
        animate(&self.window, true, &self.tokens, &mut self.spring);
    }
    fn hide(&mut self) {
        self.model.open = false;
        let duration = animate(&self.window, false, &self.tokens, &mut self.spring);
        unsafe {
            let _: () = msg_send![&*self.window,performSelector:sel!(orderOut:),withObject:std::ptr::null::<AnyObject>(),afterDelay:duration];
        }
        self.parent.makeKeyWindow();
        self.parent.makeFirstResponder(self.previous.as_deref());
    }
    fn key(&mut self, key: &str) {
        if let Some(name) = self.model.navigate(key) {
            let _ = self.sender.try_send(Command {
                name,
                args: vec![],
                token: None,
            });
        }
        if !self.model.open {
            self.hide();
        } else {
            self.refresh();
        }
    }
}
fn animate(
    w: &NSWindow,
    enter: bool,
    t: &Tokens,
    state: &mut Option<(std::time::Instant, crate::transient::Spring)>,
) -> f64 {
    let duration = if t.accessibility.reduce_motion {
        t.motion.reduced
    } else if enter {
        t.motion.enter
    } else {
        t.motion.exit
    };
    if let Some(v) = w.contentView() {
        v.setWantsLayer(true);
        if let Some(layer) = v.layer() {
            unsafe {
                let p = layer.presentationLayer();
                let from = p
                    .as_ref()
                    .map_or(if enter { 0. } else { 1. }, |p| p.opacity() as f64);
                let a: *mut AnyObject = msg_send![objc2::class!(CABasicAnimation),animationWithKeyPath:&*NSString::from_str("opacity")];
                let f: *mut AnyObject = msg_send![objc2::class!(NSNumber),numberWithDouble:from];
                let to: *mut AnyObject =
                    msg_send![objc2::class!(NSNumber),numberWithDouble:if enter{1.}else{0.}];
                let _: () = msg_send![a,setFromValue:f];
                let _: () = msg_send![a,setToValue:to];
                let _: () = msg_send![a,setDuration:duration];
                layer.setOpacity(if enter { 1. } else { 0. });
                let _: () = msg_send![&*layer,addAnimation:a,forKey:&*NSString::from_str("pm.shell.opacity")];
                if enter && !t.accessibility.reduce_motion {
                    let allocated: *mut AnyObject =
                        msg_send![objc2::class!(CASpringAnimation), alloc];
                    let spring: *mut AnyObject = msg_send![allocated,initWithPerceptualDuration:t.motion.enter,bounce:t.motion.bounce];
                    let spring = Retained::<AnyObject>::from_raw(spring).unwrap();
                    let key = NSString::from_str("transform.scale");
                    let _: () = msg_send![&*spring,setKeyPath:&*key];
                    let current: *mut AnyObject = p.as_ref().map_or(
                        std::ptr::null_mut(),
                        |p| msg_send![&**p,valueForKeyPath:&*key],
                    );
                    let from = if current.is_null() {
                        let n: *mut AnyObject = msg_send![objc2::class!(NSNumber),numberWithDouble:t.motion.start_scale];
                        n
                    } else {
                        current
                    };
                    let from_value: f64 = msg_send![from, doubleValue];
                    let velocity = state.as_ref().map_or(0., |(start, spring)| {
                        spring.sample(start.elapsed().as_secs_f64()).1
                    });
                    let delta = 1. - from_value;
                    let normalized = if delta.abs() > f64::EPSILON {
                        velocity / delta
                    } else {
                        0.
                    };
                    let _: () = msg_send![&*spring,setInitialVelocity:normalized];
                    let mass: f64 = msg_send![&*spring, mass];
                    let stiffness: f64 = msg_send![&*spring, stiffness];
                    let damping: f64 = msg_send![&*spring, damping];
                    *state = Some((
                        std::time::Instant::now(),
                        crate::transient::Spring {
                            from: from_value,
                            velocity,
                            target: 1.,
                            mass,
                            stiffness,
                            damping,
                        },
                    ));
                    let _: () = msg_send![&*spring,setFromValue:from];
                    let to: *mut AnyObject = msg_send![objc2::class!(NSNumber),numberWithDouble:1.];
                    let _: () = msg_send![&*spring,setToValue:to];
                    let settling: f64 = msg_send![&*spring, settlingDuration];
                    let _: () = msg_send![&*spring,setDuration:settling];
                    let _: () = msg_send![&*layer,addAnimation:&*spring,forKey:&*NSString::from_str("pm.shell.scale")];
                }
            }
        }
    }
    duration
}
pub fn install_menus() {
    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    let root = NSMenu::new(mtm);
    let mut commands = vec![];
    for (_, items) in menu::menus() {
        for item in items {
            commands.push(item.command);
        }
    }
    commands.extend(
        crate::layout::PRESETS
            .iter()
            .map(|(id, _)| format!("workspace:{id}")),
    );
    let target = Target::new(mtm, commands, None);
    let mut index = 0;
    for (title, items) in menu::menus() {
        let submenu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
        submenu.setAutoenablesItems(false);
        for spec in items {
            if spec.label.is_empty() {
                submenu.addItem(&NSMenuItem::separatorItem(mtm));
                index += 1;
                continue;
            }
            let (key, flags) = menu::accelerator(&spec.chord);
            let selector = spec
                .role
                .filter(|r| *r != "services")
                .map(|s| {
                    objc2::runtime::Sel::register(std::ffi::CString::new(s).unwrap().as_c_str())
                })
                .unwrap_or(sel!(act:));
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(&spec.label),
                    Some(selector),
                    &NSString::from_str(&key),
                )
            };
            item.setKeyEquivalentModifierMask(NSEventModifierFlags(flags as _));
            item.setTag(index as isize);
            if spec.role.is_none() {
                unsafe {
                    item.setTarget(Some(&target));
                }
            }
            if spec.role == Some("services") {
                let services = NSMenu::new(mtm);
                item.setSubmenu(Some(&services));
                app.setServicesMenu(Some(&services));
            }
            if spec.command == "workspaces" {
                let workspaces = NSMenu::new(mtm);
                for (i, (_, label)) in crate::layout::PRESETS.iter().enumerate() {
                    let row = unsafe {
                        NSMenuItem::initWithTitle_action_keyEquivalent(
                            NSMenuItem::alloc(mtm),
                            &NSString::from_str(label),
                            Some(sel!(act:)),
                            &NSString::from_str(""),
                        )
                    };
                    unsafe {
                        row.setTarget(Some(&target));
                    }
                    row.setTag(
                        (target.ivars().commands.len() - crate::layout::PRESETS.len() + i) as isize,
                    );
                    workspaces.addItem(&row);
                }
                item.setSubmenu(Some(&workspaces));
            }
            if spec.command == "openRecent" {
                let recents = NSMenu::new(mtm);
                let clear = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str("Clear Menu"),
                        Some(sel!(clearRecentDocuments:)),
                        &NSString::from_str(""),
                    )
                };
                recents.addItem(&clear);
                item.setSubmenu(Some(&recents));
            }
            submenu.addItem(&item);
            index += 1;
        }
        let top = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                None,
                &NSString::from_str(""),
            )
        };
        top.setSubmenu(Some(&submenu));
        root.addItem(&top);
        if title == "Help" {
            app.setHelpMenu(Some(&submenu));
        }
        if title == "Window" {
            app.setWindowsMenu(Some(&submenu));
        }
    }
    app.setMainMenu(Some(&root));
    MENU_TARGET.with(|t| *t.borrow_mut() = Some(target));
}
