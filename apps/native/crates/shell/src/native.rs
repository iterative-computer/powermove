//! AppKit windows and toolbar. No project/document state is authored here.
#[path = "flowchart.rs"]
mod flowchart;
pub use flowchart::FlowchartRow;

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
    /// Latest shell state per editor window number, read by menus and the palette.
    static MENU_STATES:RefCell<HashMap<isize,MenuState>>=RefCell::new(HashMap::new());
    /// AppKit window number → session window id.
    static WINDOW_IDS:RefCell<HashMap<isize,u64>>=RefCell::new(HashMap::new());
    /// Menu delegates must outlive their menus (NSMenu.delegate is weak).
    static MENU_DELEGATES:RefCell<Vec<Retained<MenuDelegate>>>=const{RefCell::new(Vec::new())};
    /// The context menu being tracked, so a test driver can cancel it.
    static OPEN_MENU:RefCell<Option<Retained<NSMenu>>>=const{RefCell::new(None)};
}
/// A context-menu row. An empty label is a separator.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuEntry {
    pub label: String,
    pub command: String,
    /// Shows a checkmark (current state, e.g. the panel's dock).
    pub checked: bool,
    pub enabled: bool,
    pub children: Vec<MenuEntry>,
}
impl MenuEntry {
    pub fn item(label: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            command: command.into(),
            checked: false,
            enabled: true,
            children: vec![],
        }
    }
    pub fn submenu(label: impl Into<String>, children: Vec<MenuEntry>) -> Self {
        Self {
            enabled: !children.is_empty(),
            children,
            ..Self::item(label, "")
        }
    }
    pub fn separator() -> Self {
        Self::default()
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
}
/// Snapshot of an editor window's shell state for menus, toolbar and palette.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuState {
    /// Window ▸ Panels: id and title.
    pub panels: Vec<(String, String)>,
    pub open: Vec<String>,
    /// Window ▸ Workspace: id, name, built-in (built-ins first).
    pub workspaces: Vec<(String, String, bool)>,
    pub workspace: String,
    pub sidebar: bool,
    pub inspector: bool,
    pub tool: String,
    /// Commands that cannot run now: menu rows dim, the palette omits them.
    pub disabled: std::collections::BTreeSet<String>,
}
/// The editor window commands should go to: the key window, else the main window
/// (a key palette/field-menu child panel is not an editor window), else the
/// frontmost editor (the app is inactive, e.g. a menu item invoked through
/// accessibility while another app is frontmost).
fn editor_window_number() -> Option<isize> {
    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    ROUTES.with(|r| {
        let r = r.borrow();
        [app.keyWindow(), app.mainWindow()]
            .into_iter()
            .flatten()
            .chain(app.orderedWindows().to_vec())
            .map(|w| w.windowNumber())
            .find(|n| r.contains_key(n))
    })
}
fn active_send(command: Command) {
    if let Some(n) = editor_window_number() {
        ROUTES.with(|r| {
            if let Some(tx) = r.borrow().get(&n) {
                let _ = tx.try_send(command);
            }
        });
    }
}
/// Sends `name` to the current editor exactly as its menu item would.
pub fn send_menu_command(name: &str) {
    active_send(Command {
        name: name.into(),
        args: vec![],
        token: None,
    });
}
/// Frame of the current editor window (key, else main), for cascading new windows.
pub fn editor_frame() -> Option<[f64; 4]> {
    let mtm = MainThreadMarker::new().unwrap();
    let w = NSApplication::sharedApplication(mtm).windowWithWindowNumber(editor_window_number()?)?;
    let f = w.frame();
    Some([f.origin.x, f.origin.y, f.size.width, f.size.height])
}
fn active_state() -> MenuState {
    editor_window_number()
        .and_then(|n| MENU_STATES.with(|m| m.borrow().get(&n).cloned()))
        .unwrap_or_default()
}
pub fn settings() -> Accessibility {
    let w = NSWorkspace::sharedWorkspace();
    // Review captures may force settings for this process (comma-separated
    // `increase-contrast`, `reduce-transparency`, `reduce-motion`).
    // Development builds only; release builds follow the system alone.
    let forced = if cfg!(debug_assertions) {
        std::env::var("PM_NATIVE_ACCESSIBILITY").unwrap_or_default()
    } else {
        String::new()
    };
    let on = |name: &str| forced.split(',').any(|f| f.trim() == name);
    Accessibility {
        reduce_transparency: on("reduce-transparency")
            || w.accessibilityDisplayShouldReduceTransparency(),
        increase_contrast: on("increase-contrast")
            || w.accessibilityDisplayShouldIncreaseContrast(),
        reduce_motion: on("reduce-motion") || w.accessibilityDisplayShouldReduceMotion(),
    }
}
/// The highest zoom whose layer stays inside Metal's largest drawable.
pub fn zoom_limit(dimensions: [f64; 2], backing: f64) -> f64 {
    8192. / dimensions[0].max(dimensions[1]) / backing
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
/// Applies a forced scheme (`PM_NATIVE_APPEARANCE`) app-wide, so context
/// menus match the windows.
pub fn app_appearance(t: &Tokens) {
    let name = unsafe {
        if t.scheme == Scheme::Light {
            NSAppearanceNameAqua
        } else {
            NSAppearanceNameDarkAqua
        }
    };
    NSApplication::sharedApplication(MainThreadMarker::new().unwrap())
        .setAppearance(NSAppearance::appearanceNamed(name).as_deref());
}
/// The window ground: clear over a behind-window blur while translucent,
/// the solid `window` colour under Reduce Transparency.
fn ground(window: &NSWindow, backdrop: &NSView, t: &Tokens) {
    let translucent = t.translucent();
    backdrop.setHidden(!translucent);
    window.setOpaque(!translucent);
    let colour = if translucent {
        NSColor::clearColor()
    } else {
        native_color(t.colors.window)
    };
    window.setBackgroundColor(Some(&colour));
}
/// Behind-window blur under everything in the content view; GPUI tints it
/// with `Tokens::ground`.
fn backdrop(mtm: MainThreadMarker, content: &NSView) -> Retained<NSVisualEffectView> {
    let v = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), content.bounds());
    v.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
    v.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    v.setState(NSVisualEffectState::Active);
    v.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    content.addSubview_positioned_relativeTo(&v, NSWindowOrderingMode::Below, None);
    v
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
/// A transparent GPUI window over the shell's behind-window blur, tinted by
/// its own content (the Project Library sidebar), as the editor's ground.
/// Solid `window` under Reduce Transparency. GPUI's own `Blurred` background
/// panics in its layer update, so windows use this instead.
pub fn window_backdrop(w: &gpui::Window, t: &Tokens) {
    let mtm = MainThreadMarker::new().unwrap();
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(w).unwrap().as_raw()
    else {
        return;
    };
    let gpui: Retained<NSView> =
        unsafe { Retained::retain(handle.ns_view.as_ptr().cast()).unwrap() };
    let (Some(window), Some(content)) =
        (gpui.window(), gpui.window().and_then(|w| w.contentView()))
    else {
        return;
    };
    let view = backdrop(mtm, &content);
    ground(&window, &view, t);
}
/// Frames a borderless GPUI window as a popover: `Surface::Popover` material
/// (glass on macOS 26, a visual-effect popover before, solid `floating`
/// under Reduce Transparency) under the transparent GPUI view, rounded to
/// `popover_radius`, with the system shadow. The GPUI content stays clear.
pub fn popover_frame(w: &gpui::Window, t: &Tokens) {
    let mtm = MainThreadMarker::new().unwrap();
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(w).unwrap().as_raw()
    else {
        return;
    };
    let gpui: Retained<NSView> =
        unsafe { Retained::retain(handle.ns_view.as_ptr().cast()).unwrap() };
    let Some(window) = gpui.window() else {
        return;
    };
    let Some(content) = window.contentView() else {
        return;
    };
    appearance(&window, t);
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(true);
    let radius = t.metrics.popover_radius as f64;
    let mat = material(mtm, content.bounds(), t, Surface::Popover);
    mat.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    if let Ok(glass) = mat.clone().downcast::<NSGlassEffectView>() {
        glass.setCornerRadius(radius);
    } else if let Ok(solid) = mat.clone().downcast::<NSBox>() {
        solid.setCornerRadius(radius);
    }
    content.setWantsLayer(true);
    if let Some(layer) = content.layer() {
        layer.setCornerRadius(radius);
        layer.setMasksToBounds(true);
    }
    content.addSubview_positioned_relativeTo(&mat, NSWindowOrderingMode::Below, Some(&gpui));
    window.invalidateShadow();
    // The half-bounce entrance (plan §3.10), as the palette and field menu.
    animate(&window, true, t, &mut None);
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
/// Menu, location and view for a deferred context-menu pop-up.
type PendingMenu = (Retained<NSMenu>, NSPoint, Retained<NSView>);
struct TargetState {
    replays: RefCell<HashMap<usize, u64>>,
    commands: Vec<(String, Vec<serde_json::Value>)>,
    sender: Option<async_channel::Sender<Command>>,
    /// A context menu to pop up on the next run-loop turn (outside GPUI's event dispatch).
    pending_menu: RefCell<Option<PendingMenu>>,
    /// A tab-group join (leader session id, this window) for the next run-loop turn.
    join: RefCell<Option<(u64, Retained<NSWindow>)>>,
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
            if let Some((name, args)) = self.ivars().commands.get(tag as usize) {
                if name == "settings" {
                    let app = NSApplication::sharedApplication(self.mtm());
                    if let Some(w) = app.keyWindow() && w.title().to_string() == "Settings" {
                        w.performClose(None);
                        return;
                    }
                }
                self.send(Command { name: name.clone(), args: args.clone(), token: None });
            }
        }
        /// Dynamic menu rows carry their command in `representedObject`.
        #[unsafe(method(actNamed:))]
        fn act_named(&self, sender: &NSMenuItem) {
            let name: Option<Retained<NSString>> = unsafe {
                sender.representedObject().map(|o| Retained::cast_unchecked(o))
            };
            if let Some(name) = name {
                self.send(Command { name: name.to_string(), args: vec![], token: None });
            }
        }
        #[unsafe(method(showPendingMenu:))]
        fn show_pending_menu(&self, _: Option<&AnyObject>) {
            let pending = self.ivars().pending_menu.borrow_mut().take();
            if let Some((menu, at, view)) = pending {
                OPEN_MENU.with(|m| *m.borrow_mut() = Some(menu.clone()));
                let _ = menu.popUpMenuPositioningItem_atLocation_inView(None, at, Some(&view));
                OPEN_MENU.with(|m| m.borrow_mut().take());
            }
        }
        /// A project tab in the toolbar: its window comes forward in the group.
        #[unsafe(method(selectTab:))]
        fn select_tab(&self, sender: &AnyObject) {
            let number: isize = unsafe { msg_send![sender, tag] };
            let app = NSApplication::sharedApplication(self.mtm());
            if let Some(window) = app.windowWithWindowNumber(number) {
                unsafe {
                    let group: *mut AnyObject = msg_send![&*window, tabGroup];
                    if !group.is_null() {
                        let _: () = msg_send![group, setSelectedWindow: &*window];
                    }
                }
            }
        }
        #[unsafe(method(closeTab:))]
        fn close_tab(&self, sender: &AnyObject) {
            let number: isize = unsafe { msg_send![sender, tag] };
            let app = NSApplication::sharedApplication(self.mtm());
            if let Some(window) = app.windowWithWindowNumber(number) {
                request_close(&window);
            }
        }
        #[unsafe(method(closeOtherTabs:))]
        fn close_other_tabs(&self, sender: &AnyObject) {
            let number: isize = unsafe { msg_send![sender, tag] };
            let app = NSApplication::sharedApplication(self.mtm());
            if let Some(window) = app.windowWithWindowNumber(number) {
                for other in tab_windows(&window) {
                    if other.windowNumber() != number {
                        request_close(&other);
                    }
                }
            }
        }
        #[unsafe(method(moveTabLeft:))]
        fn move_tab_left(&self, sender: &AnyObject) {
            let number: isize = unsafe { msg_send![sender, tag] };
            let app = NSApplication::sharedApplication(self.mtm());
            if let Some(window) = app.windowWithWindowNumber(number) {
                move_tab(&window, -1);
            }
        }
        #[unsafe(method(moveTabRight:))]
        fn move_tab_right(&self, sender: &AnyObject) {
            let number: isize = unsafe { msg_send![sender, tag] };
            let app = NSApplication::sharedApplication(self.mtm());
            if let Some(window) = app.windowWithWindowNumber(number) {
                move_tab(&window, 1);
            }
        }
        #[unsafe(method(moveTabOut:))]
        fn move_tab_out(&self, sender: &AnyObject) {
            let number: isize = unsafe { msg_send![sender, tag] };
            let app = NSApplication::sharedApplication(self.mtm());
            if let Some(window) = app.windowWithWindowNumber(number) {
                window.moveTabToNewWindow(None);
            }
        }
        #[unsafe(method(newTab:))]
        fn new_tab(&self, _: &AnyObject) {
            self.send(Command { name: "newProject".into(), args: vec![], token: None });
        }
        #[unsafe(method(joinPendingTab:))]
        fn join_pending_tab(&self, _: Option<&AnyObject>) {
            if let Some((leader, window)) = self.ivars().join.borrow_mut().take() {
                join_tab_group(leader, &window);
            }
        }
        #[unsafe(method(validateMenuItem:))]
        fn validate(&self, item: &NSMenuItem) -> bool {
            // Dynamic rows (representedObject) set their own state when rebuilt.
            let state = active_state();
            let tag = item.tag() as usize;
            let fixed = item.representedObject().is_none();
            if let Some((name, _)) = self.ivars().commands.get(tag).filter(|_| fixed) {
                // HIG: toggles read as the action they perform.
                match name.as_str() {
                    "toggleSidebar" => item.setTitle(&NSString::from_str(
                        if state.sidebar { "Hide Sidebar" } else { "Show Sidebar" })),
                    "toggleInspector" => item.setTitle(&NSString::from_str(
                        if state.inspector { "Hide Inspector" } else { "Show Inspector" })),
                    n if n.starts_with("tool") => item.setState(if *n == state.tool {
                        NSControlStateValueOn } else { NSControlStateValueOff }),
                    _ => {}
                }
                !state.disabled.contains(name)
            } else {
                true
            }
        }

        #[unsafe(method(replayEvent:))]
        fn replay_event(&self,event:&NSEvent){
            let token=self.ivars().replays.borrow_mut().remove(&(event as *const NSEvent as usize));
            REPLAYING.with(|guard|{guard.set(true);NSApplication::sharedApplication(self.mtm()).sendEvent(event);guard.set(false);});
            if let (Some(token),Some(sender))=(token,&self.ivars().sender){let _=sender.try_send(Command{name:"shellReplayComplete".into(),args:vec![],token:Some(token)});}
        }
        /// A Shape variant from the Shape segment's menu.
        #[unsafe(method(shapeVariant:))]
        fn shape_variant(&self, sender: &NSMenuItem) {
            let variant: Option<Retained<NSString>> = unsafe {
                sender.representedObject().map(|o| Retained::cast_unchecked(o))
            };
            if let (Some(variant), Some(tx)) = (variant, &self.ivars().sender) {
                let args = vec![serde_json::Value::from(variant.to_string())];
                let _ = tx.try_send(Command { name: "toolShape".into(), args, token: None });
            }
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
        commands: Vec<(String, Vec<serde_json::Value>)>,
        sender: Option<async_channel::Sender<Command>>,
    ) -> Retained<Self> {
        unsafe {
            msg_send![
                super(Self::alloc(mtm).set_ivars(TargetState {
                    replays: RefCell::new(HashMap::new()),
                    commands,
                    sender,
                    pending_menu: RefCell::new(None),
                    join: RefCell::new(None),
                })),
                init
            ]
        }
    }
    fn send(&self, cmd: Command) {
        if let Some(tx) = &self.ivars().sender {
            let _ = tx.try_send(cmd);
        } else {
            active_send(cmd);
        }
    }
}
fn join_tab_group(leader: u64, window: &NSWindow) -> bool {
    let mtm = MainThreadMarker::new().unwrap();
    let number = WINDOW_IDS.with(|m| {
        m.borrow()
            .iter()
            .find(|(_, id)| **id == leader)
            .map(|(n, _)| *n)
    });
    let Some(number) = number else { return false };
    let app = NSApplication::sharedApplication(mtm);
    let Some(lead) = app.windowWithWindowNumber(number) else {
        return false;
    };
    if lead.windowNumber() == window.windowNumber() {
        return false;
    }
    lead.addTabbedWindow_ordered(window, NSWindowOrderingMode::Above);
    gpui_macos::child_window::present(window, true);
    true
}
/// `SEL *` out-parameter of `menuHasKeyEquivalent:forEvent:target:action:`
/// (objc2's `Sel` has no `RefEncode`, so `*mut Sel` cannot be declared directly).
#[repr(transparent)]
struct SelOut(*mut std::ffi::c_void);
unsafe impl objc2::encode::Encode for SelOut {
    const ENCODING: objc2::encode::Encoding =
        objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Sel);
}
/// Kinds of menus with a delegate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuKind {
    /// Shortcuts are displayed but never matched (resolver-only W1 chords).
    DisplayOnly,
    Panels,
    Workspaces,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMShellMenuDelegate"]
    #[ivars = MenuKind]
    struct MenuDelegate;
    unsafe impl NSObjectProtocol for MenuDelegate {}
    unsafe impl NSMenuDelegate for MenuDelegate {
        /// Returning NO stops AppKit from matching this menu's key equivalents.
        /// The shell's ordered resolver is the only key path for these chords,
        /// so a declined key replays to the focused control instead of a menu.
        #[unsafe(method(menuHasKeyEquivalent:forEvent:target:action:))]
        fn has_key_equivalent(
            &self,
            _: &NSMenu,
            _: &NSEvent,
            _: *mut *mut AnyObject,
            _: SelOut,
        ) -> bool {
            false
        }
        #[unsafe(method(menuNeedsUpdate:))]
        fn needs_update(&self, menu: &NSMenu) {
            rebuild_menu(menu, *self.ivars());
        }
    }
);
impl MenuDelegate {
    fn new(mtm: MainThreadMarker, kind: MenuKind) -> Retained<Self> {
        unsafe { msg_send![super(Self::alloc(mtm).set_ivars(kind)), init] }
    }
}
fn named_item(mtm: MainThreadMarker, label: &str, command: &str) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(label),
            Some(sel!(actNamed:)),
            &NSString::from_str(""),
        )
    };
    unsafe {
        item.setRepresentedObject(Some(&NSString::from_str(command)));
    }
    MENU_TARGET.with(|t| {
        if let Some(t) = &*t.borrow() {
            unsafe { item.setTarget(Some(t)) };
        }
    });
    item
}
fn rebuild_menu(menu: &NSMenu, kind: MenuKind) {
    let mtm = MainThreadMarker::new().unwrap();
    let state = active_state();
    match kind {
        MenuKind::DisplayOnly => {}
        MenuKind::Workspaces => {
            menu.removeAllItems();
            for item in workspace_menu(&state) {
                menu.addItem(&item);
            }
        }
        MenuKind::Panels => {
            menu.removeAllItems();
            for (id, title) in &state.panels {
                let item = named_item(mtm, title, &format!("panel:toggle:{id}"));
                item.setState(if state.open.contains(id) {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                menu.addItem(&item);
            }
        }
    }
}
/// Window ▸ Workspace: built-ins, then saved workspaces (current one
/// checked), then Save / Delete / Reset.
fn workspace_menu(state: &MenuState) -> Vec<Retained<NSMenuItem>> {
    let mtm = MainThreadMarker::new().unwrap();
    let mut items = vec![];
    let mut builtin = true;
    for (id, name, b) in &state.workspaces {
        if builtin && !b {
            items.push(NSMenuItem::separatorItem(mtm));
        }
        builtin = *b;
        let item = named_item(mtm, name, &format!("workspace:{id}"));
        item.setState(if *id == state.workspace {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        items.push(item);
    }
    items.push(NSMenuItem::separatorItem(mtm));
    items.push(named_item(mtm, "Save Workspace…", "layout:save"));
    items.push(named_item(mtm, "Edit Workspace JSON…", "layout:edit"));
    let delete = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Delete Workspace"),
            None,
            &NSString::from_str(""),
        )
    };
    let names = NSMenu::new(mtm);
    for (id, name, _) in state.workspaces.iter().filter(|w| !w.2) {
        names.addItem(&named_item(mtm, name, &format!("layout:delete:{id}")));
    }
    delete.setSubmenu(Some(&names));
    delete.setEnabled(state.workspaces.iter().any(|w| !w.2));
    items.push(delete);
    items.push(named_item(mtm, "Reset Workspace", "layout:reset"));
    items
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
/// The Shape tool's variants: id, menu label, symbol.
pub const SHAPES: [(&str, &str, &str); 5] = [
    ("rect", "Rectangle", "square"),
    ("rounded", "Rounded Rectangle", "app"),
    ("ellipse", "Ellipse", "circle"),
    ("polygon", "Polygon", "hexagon"),
    ("star", "Star", "star"),
];
const SHAPE_SEGMENT: isize = 5;
const TOOLBAR: [&str; 7] = [
    "home",
    "title",
    "tools",
    "agent",
    "export",
    "panelLibrary",
    "settings",
];
/// One open project in the toolbar's tab strip.
#[derive(Clone, Debug, PartialEq)]
struct Tab {
    window: isize,
    title: String,
    edited: bool,
    selected: bool,
}
struct TabIvars {
    close: Retained<NSButton>,
    edited: bool,
}
define_class!(
    /// One project tab in the toolbar: its title button with the close
    /// button in the leading slot. The close button shows under the pointer;
    /// an edited project shows a dot there instead, as the system's tabs do.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMShellTab"]
    #[ivars = TabIvars]
    struct TabView;
    impl TabView {
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            unsafe {
                let _: () = msg_send![super(self), updateTrackingAreas];
            }
            for area in self.trackingAreas().to_vec() {
                self.removeTrackingArea(&area);
            }
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    <NSTrackingArea as objc2::AllocAnyThread>::alloc(),
                    self.bounds(),
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveAlways
                        | NSTrackingAreaOptions::InVisibleRect,
                    Some(self),
                    None,
                )
            };
            self.addTrackingArea(&area);
        }
        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _: &NSEvent) {
            self.hover(true);
        }
        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _: &NSEvent) {
            self.hover(false);
        }
    }
);
impl TabView {
    fn hover(&self, hovered: bool) {
        let TabIvars { close, edited } = self.ivars();
        let (name, points) = if hovered || !edited {
            ("xmark", 9.)
        } else {
            ("circle.fill", 6.)
        };
        let config =
            NSImageSymbolConfiguration::configurationWithPointSize_weight(points, unsafe {
                NSFontWeightSemibold
            });
        let image = symbol(name, "Close Project");
        close.setImage(Some(
            &image.imageWithSymbolConfiguration(&config).unwrap_or(image),
        ));
        close.setHidden(!hovered && !edited);
    }
}
/// The windows of `window`'s tab group, in tab order (itself when alone).
fn tab_windows(window: &NSWindow) -> Vec<Retained<NSWindow>> {
    window
        .tabbedWindows()
        .map(|t| t.to_vec())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| {
            vec![unsafe { Retained::retain(window as *const _ as *mut NSWindow).unwrap() }]
        })
}
/// Asks `window` to close on the next turn of the run loop, as its close
/// button does. Never `performClose` directly: inside a GPUI update the
/// should-close handler cannot run, so the window would close without its
/// save question and GPUI's close callback would abort the app.
pub fn request_close(window: &NSWindow) {
    unsafe {
        let _: () = msg_send![
            window,
            performSelector: sel!(performClose:),
            withObject: std::ptr::null::<AnyObject>(),
            afterDelay: 0.0f64
        ];
    }
}
/// Moves `window` `by` places along its tab group and keeps it selected.
fn move_tab(window: &NSWindow, by: isize) {
    let windows = tab_windows(window);
    let Some(at) = windows
        .iter()
        .position(|w| w.windowNumber() == window.windowNumber())
    else {
        return;
    };
    let to = at as isize + by;
    if to < 0 || to >= windows.len() as isize {
        return;
    }
    unsafe {
        let group: *mut AnyObject = msg_send![window, tabGroup];
        if group.is_null() {
            return;
        }
        let _: () = msg_send![group, removeWindow: window];
        let _: () = msg_send![group, insertWindow: window, atIndex: to];
        let _: () = msg_send![group, setSelectedWindow: window];
    }
}
/// Widest a project tab grows; its title truncates in the middle.
const TAB_MAX: f64 = 168.;
const TAB_MIN: f64 = 64.;
struct ToolbarState {
    target: Retained<Target>,
    tokens: Tokens,
    /// The project tabs (the window's tab group), in the leading area.
    tabs: Retained<NSView>,
    tabs_width: Retained<NSLayoutConstraint>,
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
        /// Home is the toolbar's one selectable item: selected while it shows.
        #[unsafe(method_id(toolbarSelectableItemIdentifiers:))]
        fn selectable(&self, _: &NSToolbar) -> Retained<NSArray<NSString>> {
            NSArray::from_retained_slice(&[NSString::from_str("home")])
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
    // Home and the tabs lead; the tools are the centred item between two
    // flexible spaces; Export, a space, then Panels and Settings trail.
    v.insert(2, NSString::from_str(&f.to_string()));
    v.insert(4, NSString::from_str(&f.to_string()));
    v.insert(
        7,
        NSString::from_str(&unsafe { NSToolbarSpaceItemIdentifier }.to_string()),
    );
    NSArray::from_retained_slice(&v)
}
fn symbol(s: &str, label: &str) -> Retained<NSImage> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(s),
        Some(&NSString::from_str(label)),
    )
    .expect("SF Symbol")
}
/// A toolbar glyph: every item draws its symbol at `metrics.symbol_toolbar`
/// in one weight, whichever control hosts it.
fn toolbar_symbol(s: &str, label: &str, t: &Tokens) -> Retained<NSImage> {
    let image = symbol(s, label);
    let config = objc2_app_kit::NSImageSymbolConfiguration::configurationWithPointSize_weight(
        t.metrics.symbol_toolbar as f64,
        unsafe { objc2_app_kit::NSFontWeightRegular },
    );
    image.imageWithSymbolConfiguration(&config).unwrap_or(image)
}
impl ToolbarDelegate {
    fn new(mtm: MainThreadMarker, target: Retained<Target>, tokens: Tokens) -> Retained<Self> {
        unsafe {
            msg_send![
                super(Self::alloc(mtm).set_ivars({
                    let tabs = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
                    let tabs_width = tabs.widthAnchor().constraintEqualToConstant(TAB_MIN);
                    tabs_width.setActive(true);
                    tabs.heightAnchor()
                        .constraintEqualToConstant(tokens.metrics.control as f64)
                        .setActive(true);
                    ToolbarState {
                        target,
                        tabs,
                        tabs_width,
                        tokens,
                    }
                })),
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
            let images: Vec<_> = TOOLS
                .iter()
                .map(|(_, s, l)| toolbar_symbol(s, l, t))
                .collect();
            let control = unsafe {
                NSSegmentedControl::segmentedControlWithImages_trackingMode_target_action(
                    &NSArray::from_retained_slice(&images),
                    NSSegmentSwitchTracking::SelectOne,
                    Some(target),
                    Some(sel!(tool:)),
                    mtm,
                )
            };
            for i in 0..TOOLS.len() {
                control.setWidth_forSegment(34., i as isize);
            }
            control.setSelectedSegment(0);
            control.setAccessibilityLabel(Some(&NSString::from_str("Tools")));
            control.setAccessibilityIdentifier(Some(&NSString::from_str("shell.toolbar.tools")));
            for (i, (_, _, l)) in TOOLS.iter().enumerate() {
                control.setToolTip_forSegment(Some(&NSString::from_str(l)), i as isize);
            }
            // Holding the Shape segment lists its variants.
            let menu = NSMenu::new(mtm);
            for (variant, label, glyph) in SHAPES {
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(label),
                        Some(sel!(shapeVariant:)),
                        &NSString::from_str(""),
                    )
                };
                unsafe {
                    item.setRepresentedObject(Some(&NSString::from_str(variant)));
                    item.setTarget(Some(target));
                }
                item.setImage(Some(&symbol(glyph, label)));
                item.setState(if variant == "rect" {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                menu.addItem(&item);
            }
            unsafe {
                let _: () = msg_send![&*control, setMenu: &*menu, forSegment: SHAPE_SEGMENT];
                let _: () =
                    msg_send![&*control, setShowsMenuIndicator: true, forSegment: SHAPE_SEGMENT];
            }
            item.setView(Some(&control));
            item.setLabel(&NSString::from_str("Tools"));
            return Some(item);
        }
        if name == "title" {
            // The open projects, as v1 had them in its title bar.
            item.setView(Some(&self.ivars().tabs));
            item.setLabel(&NSString::from_str("Projects"));
            return Some(item);
        }
        // Every other item is a plain bordered toolbar item (an image, or an
        // image and title for Export): the system draws its glass, spaces
        // it and sizes it. Export is the bar's one prominent item, tinted
        // with the accent.
        let (sym, label) = match name.as_str() {
            "home" => ("house", "Projects"),
            "agent" => ("sparkles", "Agent"),
            "export" => ("square.and.arrow.up", "Export"),
            "panelLibrary" => ("square.grid.2x2", "Panels"),
            "settings" => ("gearshape", "Settings"),
            _ => return None,
        };
        item.setImage(Some(&toolbar_symbol(sym, label, t)));
        unsafe {
            item.setTarget(Some(target));
            item.setAction(Some(sel!(act:)));
        }
        item.setTag(TOOLBAR.iter().position(|n| *n == name).unwrap() as isize);
        item.setBordered(true);
        item.setLabel(&NSString::from_str(label));
        item.setToolTip(Some(&NSString::from_str(label)));
        if name == "export" {
            item.setTitle(&NSString::from_str(label));
            item.setStyle(NSToolbarItemStyle::Prominent);
            item.setBackgroundTintColor(Some(&native_color(t.colors.accent)));
        }
        Some(item)
    }
}

/// Toolbar subtitle for a document: `Saving…` while a save runs, else
/// `Edited` when there are unsaved changes, else nothing.
pub fn document_subtitle(edited: bool, saving: bool) -> &'static str {
    if saving {
        "Saving…"
    } else if edited {
        "Edited"
    } else {
        ""
    }
}
/// Native viewer slot placement published in one composition frame.
#[derive(Clone, Copy, PartialEq)]
struct SlotGeometry {
    frame: NSRect,
    layer: NSRect,
    scale: f64,
    crop: Option<[u32; 4]>,
}
/// The viewer layer's frame in a `slot` points wide and high, for a canvas
/// zoomed to `canvas` points, centred on the slot and moved by `pan` (points,
/// y down), plus, when part of the canvas lies outside the slot, the
/// drawable's window onto the whole canvas in device pixels:
/// `[x, y, canvas width, canvas height]`. The engine renders only that
/// window, so zooming in or panning away never renders pixels nobody sees.
pub fn viewer_window(
    slot: [f64; 2],
    canvas: [f64; 2],
    scale: f64,
    pan: [f64; 2],
) -> (NSRect, Option<[u32; 4]>) {
    // Canvas edges in the slot, top-left origin.
    let start = [
        (slot[0] - canvas[0]) / 2. + pan[0],
        (slot[1] - canvas[1]) / 2. + pan[1],
    ];
    // The part of the canvas inside the slot. A canvas panned wholly out keeps
    // a sliver at its nearest edge (the slot clips it) so the window is never empty.
    let span = |axis: usize| {
        let (lo, hi) = (start[axis], start[axis] + canvas[axis]);
        let (mut a, mut b) = (0f64.clamp(lo, hi), slot[axis].clamp(lo, hi));
        let least = canvas[axis].min(1.);
        if b - a < least {
            if a + least <= hi {
                b = a + least;
            } else {
                a = b - least;
            }
        }
        (a, b)
    };
    let ((x0, x1), (y0, y1)) = (span(0), span(1));
    let frame = NSRect::new(
        NSPoint::new(x0, slot[1] - y1),
        NSSize::new(x1 - x0, y1 - y0),
    );
    if [x1 - x0, y1 - y0] == canvas {
        return (frame, None);
    }
    // The drawable is `visible × scale` rounded, as the layer sizes it; the
    // window starts on a whole pixel and always lies on its canvas.
    let px = |v: f64| (v * scale).round().max(1.) as u32;
    let (cw, ch) = (px(canvas[0]), px(canvas[1]));
    let (w, h) = (px(x1 - x0).min(cw), px(y1 - y0).min(ch));
    let at = |v: f64, room: u32| ((v * scale + 1e-6).floor().max(0.) as u32).min(room);
    (
        frame,
        Some([at(x0 - start[0], cw - w), at(y0 - start[1], ch - h), cw, ch]),
    )
}
pub struct Host {
    pub window: Retained<NSWindow>,
    pub gpui: Retained<NSView>,
    content: Retained<NSView>,
    backdrop: Retained<NSVisualEffectView>,
    viewer: Retained<NSView>,
    pub viewer_rect: Rc<Cell<Option<crate::layout::Rect>>>,
    pub splitter_focus: bool,
    pub keyboard_control_focus: Cell<bool>,
    pub canvas_focus: Cell<bool>,
    flowchart: Option<flowchart::Popover>,
    layer: metal::MetalLayer,
    viewer_transform: Option<([f64; 2], Option<f64>, [f64; 2])>,
    /// The drawable's window onto the zoomed canvas ([`viewer_window`]), set
    /// before the drawable size so its observers read the matching window.
    pub viewer_crop: Rc<Cell<Option<[u32; 4]>>>,
    /// Last published viewer geometry (`Some(None)` = hidden); `None` before the first frame.
    last_geometry: Option<Option<SlotGeometry>>,
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
    base_choices: Vec<Choice>,
    /// Commands contributed at runtime, listed in the palette after built-ins.
    pub contributed_choices: Vec<Choice>,
    /// Rectangles of panel bodies a native surface below GPUI draws.
    pub native_panels: Rc<RefCell<Vec<(String, crate::layout::Rect)>>>,
    /// Runs inside the composition transaction, with the window size.
    pub native_present: Option<Rc<dyn Fn(f64, f64) -> bool>>,
    /// Native views to expose in reading order after GPUI's.
    pub native_accessibility: Vec<Retained<NSView>>,
    /// The project this window showed when the app last ran, and whether the
    /// system keeps documents open across quits (NSQuitAlwaysKeepsWindows).
    pub restored_project: Option<(String, bool)>,
    menu_state: RefCell<Option<MenuState>>,
    /// The open Save Workspace sheet (kept alive while attached).
    sheet: RefCell<Option<Sheet>>,
    /// The open Workspace Definition sheet, kept alive while it shows.
    definition: RefCell<Option<Retained<DefinitionTarget>>>,
    /// Title and subtitle of the toolbar's document item.
    /// The document's name and state line (`document_subtitle`).
    document: RefCell<(String, String)>,
    /// The toolbar's project tabs, their width, and what they last showed.
    tabs: Retained<NSView>,
    tabs_width: Retained<NSLayoutConstraint>,
    tabs_shown: RefCell<Vec<Tab>>,
    /// Home shows over the docks (`set_home`).
    home: Cell<bool>,
    /// Home's sidebar material, under the GPUI view while Home shows.
    sidebar: RefCell<Option<Retained<NSView>>>,
}
/// Static palette rows: every menu command (including display-only menus and
/// tools) plus workspace presets. Dynamic rows are added when it opens.
fn base_choices() -> Vec<Choice> {
    let mut seen = std::collections::HashSet::new();
    menu::menus()
        .into_iter()
        .flat_map(|(_, i)| i)
        .chain(menu::tools())
        .filter(|i| {
            !i.command.is_empty()
                && i.role.is_none()
                && i.args.is_empty()
                && !menu::SUBMENUS.contains(&i.command.as_str())
        })
        .filter(|i| seen.insert(i.command.clone()))
        .map(|i| Choice {
            label: i.label,
            command: i.command,
        })
        .collect()
}
/// command → shortcut glyphs, preferring menu accelerators, then the first
/// non-Control W1 chord (the Control aliases are the same command).
fn shortcut_glyphs() -> HashMap<String, String> {
    let mut map = HashMap::new();
    for i in menu::menus()
        .into_iter()
        .flat_map(|(_, i)| i)
        .chain(menu::tools())
    {
        if !i.chord.is_empty() && i.args.is_empty() {
            map.entry(i.command.clone())
                .or_insert_with(|| menu::glyphs(&i.chord));
        }
    }
    for b in crate::keyboard::shipped() {
        if b.args.is_empty() && !b.chord.starts_with("ctrl+") {
            map.entry(b.command.clone())
                .or_insert_with(|| menu::glyphs(&b.chord));
        }
    }
    map
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
        window.setTitle(&NSString::from_str("Untitled"));
        window.setSubtitle(&NSString::from_str(""));
        window.setTabbingIdentifier(&NSString::from_str("com.powermove.native.editor"));
        window.setTabbingMode(NSWindowTabbingMode::Preferred);
        // Frames, tabs and workspaces restore from the shell session (multiple
        // windows); AppKit's autosave name is one-per-window and would collide.
        window.setRestorable(false);
        window.setMinSize(NSSize::new(
            t.metrics.min_width as f64,
            t.metrics.min_height as f64,
        ));
        window.setCollectionBehavior(NSWindowCollectionBehavior::FullScreenPrimary);
        let (tx, rx) = async_channel::unbounded();
        ROUTES.with(|r| {
            r.borrow_mut().insert(window.windowNumber(), tx.clone());
        });
        let target = Target::new(
            mtm,
            TOOLBAR.iter().map(|s| (s.to_string(), vec![])).collect(),
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
        // The regular unified bar at the system's own height, spacing and
        // control size; nothing is hand-packed.
        window.setToolbarStyle(NSWindowToolbarStyle::Unified);
        window.setTitlebarAppearsTransparent(true);
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        let viewer = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        viewer.setWantsLayer(true);
        viewer.setAccessibilityElement(true);
        viewer.setAccessibilityRole(Some(&NSString::from_str("AXImage")));
        viewer.setAccessibilityLabel(Some(&NSString::from_str("Viewer")));
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
        let backdrop = backdrop(mtm, &content);
        ground(&window, &backdrop, &t);
        let choices = base_choices();
        let palette = Popup::new(&window, &t, Surface::Palette, choices.clone(), tx.clone());
        palette.borrow_mut().shortcuts = shortcut_glyphs();
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
        let (tabs, tabs_width) = (
            delegate.ivars().tabs.clone(),
            delegate.ivars().tabs_width.clone(),
        );
        let host = Rc::new(RefCell::new(Self {
            window,
            gpui: gpui.clone(),
            content,
            backdrop,
            viewer,
            viewer_rect: Rc::new(Cell::new(None)),
            splitter_focus: false,
            keyboard_control_focus: Cell::new(false),
            canvas_focus: Cell::new(false),
            flowchart: None,
            layer,
            viewer_transform: None,
            viewer_crop: Rc::new(Cell::new(None)),
            last_geometry: None,
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
            base_choices: choices,
            contributed_choices: vec![],
            native_panels: Rc::default(),
            native_present: None,
            native_accessibility: vec![],
            restored_project: None,
            menu_state: RefCell::new(None),
            sheet: RefCell::new(None),
            definition: RefCell::new(None),
            document: RefCell::new(("Untitled".into(), String::new())),
            tabs,
            tabs_width,
            tabs_shown: RefCell::default(),
            home: Cell::new(false),
            sidebar: RefCell::new(None),
        }));
        let weak = Rc::downgrade(&host);
        let native_rects = host.borrow().native_panels.clone();
        let reg = gpui_macos::composition::register_tracked(
            &gpui,
            Rc::new(move || {
                weak.upgrade()
                    .map_or(gpui_macos::composition::SlotChange::Unchanged, |h| {
                        h.borrow_mut().present()
                    })
            }),
            // The viewer view below only shows pixels: GPUI paints a hole over it
            // but owns the pointer there, for gizmos and viewer controls. Native
            // panel bodies (extension pages) take their own events.
            Rc::new(move |p| {
                !native_rects
                    .borrow()
                    .iter()
                    .any(|(_, r)| r.contains(p.x as f32, p.y as f32))
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
                    if e.r#type() == NSEventType::KeyDown {
                        let key = chord(e);
                        if let Some(sheet) = host.borrow().sheet.borrow().as_ref()
                            && e.windowNumber() == sheet.window.windowNumber()
                            && matches!(key.as_str(), "tab" | "shift+tab")
                        {
                            sheet.tab(key == "shift+tab");
                            return std::ptr::null_mut();
                        }
                    }
                    if e.r#type() == NSEventType::KeyDown {
                        let key = chord(e);
                        let mut h = host.borrow_mut();
                        if h.flowchart.as_ref().is_some_and(|p| {
                            p.owns_event(e)
                                || (h.flowchart_open() && e.windowNumber() == window.windowNumber())
                        }) && (key == "escape"
                            || (key == "tab"
                                && !NSApplication::sharedApplication(h.window.mtm())
                                    .isFullKeyboardAccessEnabled()))
                        {
                            h.close_flowchart();
                            return std::ptr::null_mut();
                        }
                    }
                    for popup in [&palette, &fields] {
                        let mut p = popup.borrow_mut();
                        if e.windowNumber() == p.window.windowNumber() && p.model.open {
                            if p.search
                                .currentEditor()
                                .is_some_and(|r| unsafe { msg_send![&*r, hasMarkedText] })
                            {
                                return event.as_ptr();
                            }
                            if e.r#type() == NSEventType::KeyDown {
                                let key = chord(e);
                                if matches!(
                                    key.as_str(),
                                    "up" | "down"
                                        | "home"
                                        | "end"
                                        | "enter"
                                        | "escape"
                                        | "tab"
                                        | "shift+tab"
                                ) || (key == "space" && p.row_focus)
                                {
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
                    if e.r#type() == NSEventType::KeyDown {
                        if matches!(
                            chord.as_str(),
                            "ctrl+f6" | "ctrl+shift+f6" | "ctrl+tab" | "ctrl+shift+tab"
                        ) {
                            {
                                let h = host.borrow();
                                h.window.makeFirstResponder(Some(&h.gpui));
                            }
                            host.borrow().send_command(
                                if chord.contains("shift") {
                                    "shellPanelPrevious"
                                } else {
                                    "shellPanelNext"
                                },
                                vec![],
                            );
                            return std::ptr::null_mut();
                        }
                        let canvas_tab =
                            chord == "tab" && host.borrow().composition_canvas_focused();
                        if matches!(chord.as_str(), "tab" | "shift+tab")
                            && !name.contains("Text")
                            && !canvas_tab
                        {
                            let h = host.borrow();
                            let reverse = chord == "shift+tab";
                            if !h.advance_toolbar(reverse) {
                                let native = responder.as_ref().is_some_and(|r| {
                                    !std::ptr::eq(
                                        &**r as *const _,
                                        &*h.gpui as *const _ as *const NSResponder,
                                    )
                                });
                                h.window.makeFirstResponder(Some(&h.gpui));
                                h.send_command(
                                    if reverse {
                                        "shellFocusPrevious"
                                    } else {
                                        "shellFocusNext"
                                    },
                                    vec![serde_json::json!(native)],
                                );
                            }
                            return std::ptr::null_mut();
                        }
                    }
                    if host.borrow().keyboard_control_focus.get()
                        && matches!(
                            chord.as_str(),
                            "enter" | "space" | "up" | "down" | "left" | "right"
                        )
                    {
                        return event.as_ptr();
                    }
                    if splitter_focus
                        && matches!(chord.as_str(), "up" | "down" | "left" | "right" | "enter")
                    {
                        return event.as_ptr();
                    }
                    if matches!(
                        chord.as_str(),
                        "enter" | "space" | "up" | "down" | "left" | "right"
                    ) && responder.as_ref().is_some_and(|r| unsafe {
                        msg_send![&**r,isKindOfClass:NSControl::class()]
                    }) {
                        return event.as_ptr();
                    }
                    let field = responder.as_ref().is_some_and(|r| unsafe {
                        msg_send![&**r,isKindOfClass:NSTextView::class()]
                    });
                    // P-TEXT: composition keystrokes belong to the OS input method.
                    // Never queue/replay a key that changes an active marked range.
                    if field
                        && responder
                            .as_ref()
                            .is_some_and(|r| unsafe { msg_send![&**r, hasMarkedText] })
                    {
                        return event.as_ptr();
                    }
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
    fn toolbar_controls(&self) -> Vec<Retained<NSView>> {
        fn collect(view: &NSView, out: &mut Vec<Retained<NSView>>) {
            if view.isHidden() {
                return;
            }
            let control: bool = unsafe { msg_send![view, isKindOfClass: NSControl::class()] };
            if control && view.acceptsFirstResponder() {
                out.push(unsafe { Retained::retain(view as *const _ as *mut _).unwrap() });
            } else {
                for child in view.subviews().to_vec() {
                    collect(&child, out);
                }
            }
        }
        fn toolbar(view: &NSView, out: &mut Vec<Retained<NSView>>) {
            if view.class().name().to_string_lossy().as_ref() == "NSToolbarView" {
                collect(view, out);
            } else {
                for child in view.subviews().to_vec() {
                    toolbar(&child, out);
                }
            }
        }
        let mut out = vec![];
        if let Some(frame) = unsafe { self.content.superview() } {
            toolbar(&frame, &mut out);
        }
        out.sort_by(|a, b| {
            a.convertRect_toView(a.bounds(), None)
                .origin
                .x
                .total_cmp(&b.convertRect_toView(b.bounds(), None).origin.x)
        });
        out
    }
    pub fn focus_toolbar(&self, reverse: bool) -> bool {
        let controls = self.toolbar_controls();
        let control = if reverse {
            controls.last()
        } else {
            controls.first()
        };
        control.is_some_and(|v| self.window.makeFirstResponder(Some(v)))
    }
    fn advance_toolbar(&self, reverse: bool) -> bool {
        let controls = self.toolbar_controls();
        let Some(responder) = self.window.firstResponder() else {
            return self.focus_toolbar(reverse);
        };
        let current = controls
            .iter()
            .position(|v| std::ptr::eq(&**v as *const _ as *const NSResponder, &*responder));
        // An unfocused GPUI surface starts at the toolbar (initial Tab).
        if current.is_none() {
            return false;
        }
        let index = current.unwrap() as isize + if reverse { -1 } else { 1 };
        controls
            .get(index as usize)
            .is_some_and(|v| self.window.makeFirstResponder(Some(v)))
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
        assert_eq!(
            self.layer.pixel_format(),
            metal::MTLPixelFormat::RGBA16Float
        );
        eprintln!(
            "PASS hidden AppKit initialization: toolbar, pooled search windows, edited state, viewer layer"
        );
    }
    /// Window title, toolbar title and subtitle (`document_subtitle`), the
    /// close-button edited dot and the proxy icon.
    pub fn set_document_metadata(
        &self,
        title: &str,
        subtitle: &str,
        edited: bool,
        path: Option<&std::path::Path>,
    ) {
        self.window.setTitle(&NSString::from_str(title));
        self.window.setDocumentEdited(edited);
        self.window.setSubtitle(&NSString::from_str(subtitle));
        let url = path.map(|p| {
            objc2_foundation::NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))
        });
        self.window.setRepresentedURL(url.as_deref());
        *self.document.borrow_mut() = (title.into(), subtitle.into());
        self.sync_tabs();
    }
    /// The toolbar's tabs are the windows of this window's tab group (a lone
    /// window is one tab), then New Project. AppKit's own tab bar stays
    /// hidden; the group remains the model, so a tab is selected, closed and
    /// moved to its own window exactly as before. Rebuilt only when the
    /// group, a title, an edited state or the selection changes.
    pub fn sync_tabs(&self) {
        let own = self.window.windowNumber();
        let group = tab_windows(&self.window);
        let selected = unsafe {
            let tab_group: *mut AnyObject = msg_send![&*self.window, tabGroup];
            let window: *mut NSWindow = if tab_group.is_null() {
                std::ptr::null_mut()
            } else {
                msg_send![tab_group, selectedWindow]
            };
            window.as_ref().map_or(own, |w| w.windowNumber())
        };
        let tabs: Vec<Tab> = group
            .iter()
            .map(|w| Tab {
                window: w.windowNumber(),
                title: w.title().to_string(),
                edited: w.isDocumentEdited(),
                selected: if group.len() == 1 {
                    true
                } else {
                    w.windowNumber() == selected
                },
            })
            .collect();
        if *self.tabs_shown.borrow() == tabs {
            return;
        }
        let (mtm, t, target) = (self.window.mtm(), &self.tokens, &self._target);
        for view in self.tabs.subviews().to_vec() {
            view.removeFromSuperview();
        }
        let height = t.metrics.control as f64;
        // Tabs share what half the window leaves; titles truncate in the middle.
        let room = (self.window.frame().size.width / 2. - 160.).max(TAB_MIN);
        let widest = (room / tabs.len() as f64).clamp(TAB_MIN, TAB_MAX);
        let glyph = |name: &str, label: &str, points: f64| {
            let config =
                NSImageSymbolConfiguration::configurationWithPointSize_weight(points, unsafe {
                    NSFontWeightMedium
                });
            let image = symbol(name, label);
            image.imageWithSymbolConfiguration(&config).unwrap_or(image)
        };
        // With more projects than fit at the narrowest width, the strip
        // shows as many as fit (the selected one among them) and a pull-down
        // lists them all.
        let fits = ((room / TAB_MIN).floor() as usize).max(1);
        let mut shown: Vec<&Tab> = tabs.iter().take(fits).collect();
        if let Some(selected) = tabs.iter().find(|t| t.selected)
            && !shown.iter().any(|t| t.window == selected.window)
            && let Some(last) = shown.last_mut()
        {
            *last = selected;
        }
        let item = |title: &str, action: objc2::runtime::Sel, tag: isize| {
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(action),
                    &NSString::from_str(""),
                )
            };
            unsafe { item.setTarget(Some(target)) };
            item.setTag(tag);
            item
        };
        let blank = NSImage::initWithSize(
            <NSImage as objc2::AllocAnyThread>::alloc(),
            NSSize::new(10., 10.),
        );
        let mut x = 0.;
        for tab in shown {
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(&tab.title),
                    Some(target),
                    Some(sel!(selectTab:)),
                    mtm,
                )
            };
            // The system's toolbar bezel: the selected tab keeps it, the
            // others show it under the pointer. A blank leading image keeps
            // the close button's slot inside the bezel.
            button.setBezelStyle(NSBezelStyle::Toolbar);
            button.setButtonType(NSButtonType::PushOnPushOff);
            button.setState(if tab.selected {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            button.setShowsBorderOnlyWhileMouseInside(!tab.selected);
            button.setImage(Some(&blank));
            button.setImagePosition(NSCellImagePosition::ImageLeading);
            button.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
            button.setFont(Some(&NSFont::systemFontOfSize_weight(
                t.typography.title as f64,
                unsafe {
                    if tab.selected {
                        NSFontWeightMedium
                    } else {
                        NSFontWeightRegular
                    }
                },
            )));
            button.setTag(tab.window);
            button.setToolTip(Some(&NSString::from_str(&tab.title)));
            button.setAccessibilityIdentifier(Some(&NSString::from_str("shell.toolbar.tab")));
            // Right-click: the tab's own menu (reorder, detach, close).
            let menu = NSMenu::new(mtm);
            menu.addItem(&item("Close Tab", sel!(closeTab:), tab.window));
            menu.addItem(&item("Close Other Tabs", sel!(closeOtherTabs:), tab.window));
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            menu.addItem(&item("Move Tab Left", sel!(moveTabLeft:), tab.window));
            menu.addItem(&item("Move Tab Right", sel!(moveTabRight:), tab.window));
            menu.addItem(&item(
                "Move Tab to New Window",
                sel!(moveTabOut:),
                tab.window,
            ));
            unsafe { button.setMenu(Some(&menu)) };
            let width = (button.fittingSize().width + 8.).clamp(TAB_MIN, widest);
            button.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(width, height)));
            let close = unsafe {
                NSButton::buttonWithImage_target_action(
                    &glyph("xmark", "Close Project", 9.),
                    Some(target),
                    Some(sel!(closeTab:)),
                    mtm,
                )
            };
            close.setBordered(false);
            close.setTag(tab.window);
            close.setToolTip(Some(&NSString::from_str("Close Project")));
            close.setAccessibilityIdentifier(Some(&NSString::from_str("shell.toolbar.tab.close")));
            close.setFrame(NSRect::new(
                NSPoint::new(6., (height - 16.) / 2.),
                NSSize::new(16., 16.),
            ));
            let view: Retained<TabView> = unsafe {
                msg_send![
                    super(TabView::alloc(mtm).set_ivars(TabIvars {
                        close: close.clone(),
                        edited: tab.edited,
                    })),
                    initWithFrame: NSRect::new(NSPoint::new(x, 0.), NSSize::new(width, height))
                ]
            };
            view.addSubview(&button);
            view.addSubview(&close);
            view.hover(false);
            self.tabs.addSubview(&view);
            x += width + 2.;
        }
        if tabs.len() > fits {
            let more = NSPopUpButton::initWithFrame_pullsDown(
                NSPopUpButton::alloc(mtm),
                NSRect::new(NSPoint::new(x, 0.), NSSize::new(34., height)),
                true,
            );
            more.setBezelStyle(NSBezelStyle::Toolbar);
            more.setShowsBorderOnlyWhileMouseInside(true);
            more.addItemWithTitle(&NSString::from_str(""));
            if let Some(menu) = more.menu() {
                for tab in &tabs {
                    let row = item(&tab.title, sel!(selectTab:), tab.window);
                    row.setState(if tab.selected {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                    menu.addItem(&row);
                }
            }
            more.setToolTip(Some(&NSString::from_str("All Projects")));
            more.setAccessibilityIdentifier(Some(&NSString::from_str("shell.toolbar.tabs.all")));
            self.tabs.addSubview(&more);
            x += 36.;
        }
        let add = unsafe {
            NSButton::buttonWithImage_target_action(
                &glyph("plus", "New Project", 12.),
                Some(target),
                Some(sel!(newTab:)),
                mtm,
            )
        };
        add.setBezelStyle(NSBezelStyle::Toolbar);
        add.setShowsBorderOnlyWhileMouseInside(true);
        add.setToolTip(Some(&NSString::from_str("New Project")));
        add.setAccessibilityIdentifier(Some(&NSString::from_str("shell.toolbar.newProject")));
        add.setFrame(NSRect::new(
            NSPoint::new(x + 2., 0.),
            NSSize::new(height, height),
        ));
        self.tabs.addSubview(&add);
        self.tabs_width.setConstant(x + 2. + height);
        *self.tabs_shown.borrow_mut() = tabs;
    }
    /// Home over the window: the Home item reads selected, and the
    /// document's own controls (tools, sidebars, Export, the title) step
    /// aside, as v1's titlebar hides Export and the panel library on Home.
    pub fn set_home(&self, home: bool) {
        if self.home.get() == home {
            return;
        }
        self.home.set(home);
        self.sync_sidebar();
        self.sync_tabs();
        let id = NSString::from_str("home");
        self._toolbar
            .setSelectedItemIdentifier(home.then_some(&*id));
        for item in self._toolbar.items().to_vec() {
            let name = item.itemIdentifier().to_string();
            // The tabs stay, as in v1's title bar on Home.
            if matches!(name.as_str(), "tools" | "export" | "panelLibrary") {
                item.setHidden(home);
            }
        }
    }
    /// Home's sidebar is real system material in a column under the GPUI
    /// view, which leaves that column unpainted: liquid glass on macOS 26 and
    /// later, the sidebar visual effect before, the solid window colour under
    /// Reduce Transparency. The glass carries a light tint of the window
    /// colour (`translucency.glass_tint`), so labels drawn on it keep their
    /// contrast whatever is behind the window.
    fn sync_sidebar(&self) {
        let mut slot = self.sidebar.borrow_mut();
        if !self.home.get() {
            if let Some(view) = slot.take() {
                view.removeFromSuperview();
            }
            return;
        }
        if slot.is_some() {
            return;
        }
        let t = &self.tokens;
        let frame = NSRect::new(
            NSPoint::ZERO,
            NSSize::new(t.metrics.sidebar as f64, self.content.bounds().size.height),
        );
        let view = material(self.window.mtm(), frame, t, Surface::Sidebar);
        if let Ok(glass) = view.clone().downcast::<NSGlassEffectView>() {
            glass.setCornerRadius(0.);
            glass.setStyle(NSGlassEffectViewStyle::Clear);
            glass.setTintColor(Some(
                &native_color(t.colors.window)
                    .colorWithAlphaComponent(t.translucency.glass_tint as f64),
            ));
        } else if let Ok(effect) = view.clone().downcast::<NSVisualEffectView>() {
            effect.setMaterial(NSVisualEffectMaterial::Sidebar);
            effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        } else if let Ok(solid) = view.clone().downcast::<NSBox>() {
            solid.setCornerRadius(0.);
            solid.setBorderWidth(0.);
            solid.setFillColor(&native_color(t.colors.window));
        }
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
        self.content.addSubview_positioned_relativeTo(
            &view,
            NSWindowOrderingMode::Below,
            Some(&self.gpui),
        );
        *slot = Some(view);
    }
    /// Development builds: a tab action by index, through the same code the
    /// tab's buttons and menu run (`--open reviewTab:close:1`).
    #[cfg(debug_assertions)]
    pub fn review_tab(&self, action: &str, index: usize) {
        let windows = tab_windows(&self.window);
        let Some(window) = windows.get(index) else {
            return;
        };
        match action {
            "select" => unsafe {
                let group: *mut AnyObject = msg_send![&**window, tabGroup];
                if !group.is_null() {
                    let _: () = msg_send![group, setSelectedWindow: &**window];
                }
            },
            "close" => request_close(window),
            "left" => move_tab(window, -1),
            "right" => move_tab(window, 1),
            "out" => window.moveTabToNewWindow(None),
            // Every other editor window joins this one's group again, as
            // dragging its tab back in (or Window ▸ Merge All Windows) does.
            "merge" => {
                let app = NSApplication::sharedApplication(self.window.mtm());
                let own = self.window.tabbingIdentifier();
                for other in app.windows().to_vec() {
                    if other.windowNumber() != self.window.windowNumber()
                        && other.tabbingIdentifier() == own
                        && other.isVisible()
                        && !windows
                            .iter()
                            .any(|w| w.windowNumber() == other.windowNumber())
                    {
                        self.window
                            .addTabbedWindow_ordered(&other, NSWindowOrderingMode::Above);
                    }
                }
            }
            _ => {}
        }
    }
    /// Development builds: the window's whole AppKit view tree (class, frame,
    /// hidden), from the theme frame down, so a review can show which system
    /// materials and controls really exist without activating the window.
    #[cfg(debug_assertions)]
    pub fn view_tree(&self) -> String {
        fn walk(view: &NSView, depth: usize, out: &mut String) {
            let f = view.frame();
            out.push_str(&format!(
                "{}{} [{:.0},{:.0} {:.0}x{:.0}]{}\n",
                "  ".repeat(depth),
                view.class().name().to_string_lossy(),
                f.origin.x,
                f.origin.y,
                f.size.width,
                f.size.height,
                if view.isHidden() { " hidden" } else { "" },
            ));
            for child in view.subviews().to_vec() {
                walk(&child, depth + 1, out);
            }
        }
        let mut out = String::new();
        if let Some(root) = self
            .window
            .contentView()
            .and_then(|c| unsafe { c.superview() })
        {
            walk(&root, 0, &mut out);
        }
        out
    }
    /// Current toolbar title and subtitle (tests, accessibility checks).
    pub fn document_title(&self) -> (String, String) {
        self.document.borrow().clone()
    }
    pub fn update_tokens(&mut self, t: Tokens) {
        appearance(&self.window, &t);
        ground(&self.window, &self.backdrop, &t);
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
        // The sidebar material follows the appearance and transparency setting.
        if let Some(view) = self.sidebar.borrow_mut().take() {
            view.removeFromSuperview();
        }
        self.sync_sidebar();
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
    pub fn viewer_transform(&mut self, dimensions: [f64; 2], zoom: Option<f64>, pan: [f64; 2]) {
        if self.viewer_transform.is_none() {
            let root = objc2_quartz_core::CALayer::new();
            root.setMasksToBounds(true);
            root.setBackgroundColor(Some(&native_color(self.tokens.colors.viewer).CGColor()));
            unsafe {
                let _: () = msg_send![&*self.viewer,setLayer:&*root];
                let _: () = msg_send![&*root,addSublayer:self.layer.as_ptr() as *mut AnyObject];
            }
        }
        if self.viewer_transform != Some((dimensions, zoom, pan)) {
            // Re-place the content layer on the next composition frame.
            self.last_geometry = None;
        }
        self.viewer_transform = Some((dimensions, zoom, pan));
    }
    /// The highest zoom the display can show `dimensions` at.
    pub fn viewer_zoom_limit(&self, dimensions: [f64; 2]) -> f64 {
        zoom_limit(dimensions, self.window.backingScaleFactor())
    }
    pub fn add_bindings(&mut self, bindings: impl IntoIterator<Item = crate::keyboard::Binding>) {
        self.resolver.bindings.extend(bindings);
        self.resolver
            .bindings
            .sort_by_key(|b| std::cmp::Reverse(b.priority));
    }
    /// The view native surfaces install beside the viewer, below GPUI.
    pub fn native_container(&self) -> Retained<NSView> {
        self.content.clone()
    }
    pub fn set_contributed_bindings(
        &mut self,
        owner: &str,
        bindings: impl IntoIterator<Item = crate::keyboard::Binding>,
    ) {
        self.resolver.bindings.retain(|b| !b.id.starts_with(owner));
        self.add_bindings(bindings);
    }
    pub fn viewer_layer(&self) -> metal::MetalLayer {
        self.layer.clone()
    }
    /// A closed document must not leave its drawable pool in the Home
    /// window. The presenter and placement binding release their old layer
    /// separately; the next layout sizes this fresh, configured layer.
    pub fn replace_viewer_layer(&mut self, layer: metal::MetalLayer) {
        unsafe {
            let old = self.layer.as_ptr() as *mut AnyObject;
            let _: () = msg_send![old, removeFromSuperlayer];
            if self.viewer_transform.is_some() {
                if let Some(root) = self.viewer.layer() {
                    let _: () = msg_send![&*root, addSublayer:layer.as_ptr() as *mut AnyObject];
                }
            } else {
                let _: () = msg_send![&*self.viewer, setLayer:layer.as_ptr() as *mut AnyObject];
            }
        }
        self.layer = layer;
        self.last_geometry = None;
    }
    pub fn set_viewer_label(&self, label: &str) {
        self.viewer
            .setAccessibilityLabel(Some(&NSString::from_str(label)));
    }
    pub fn toolbar_inset(&self) -> f32 {
        (self.gpui.bounds().size.height - self.window.contentLayoutRect().size.height).max(0.)
            as f32
    }
    /// P-120: returns `Moved` only when the viewer slot's frame, layer frame,
    /// drawable size or visibility changed, so GPUI blocks on the composition
    /// transaction only on those frames.
    fn present(&mut self) -> gpui_macos::composition::SlotChange {
        let h = self.content.bounds().size.height;
        let geometry = self.viewer_rect.get().map(|r| {
            let scale = self.window.backingScaleFactor();
            let (canvas, pan) = if let Some((dimensions, zoom, pan)) = self.viewer_transform {
                let fit = (r.w as f64 / dimensions[0]).min(r.h as f64 / dimensions[1]);
                let zoom = zoom.unwrap_or(fit).min(zoom_limit(dimensions, scale));
                ([dimensions[0] * zoom, dimensions[1] * zoom], pan)
            } else {
                ([r.w as f64, r.h as f64], [0., 0.])
            };
            let (layer, crop) = viewer_window([r.w as f64, r.h as f64], canvas, scale, pan);
            SlotGeometry {
                frame: rect(r, h),
                layer,
                scale,
                crop,
            }
        });
        let moved = self.last_geometry != Some(geometry);
        if moved {
            if let Some(g) = geometry {
                self.viewer.setHidden(false);
                self.viewer.setFrame(g.frame);
                // Without a transform the Metal layer is the view's own backing
                // layer and AppKit sizes it; framing it here would move it to
                // the content view's origin (the stray lower-left rectangle).
                if self.viewer_transform.is_some() {
                    unsafe {
                        let _: () =
                            msg_send![self.layer.as_ptr() as *mut AnyObject,setFrame:g.layer];
                    }
                }
                self.layer.set_contents_scale(g.scale);
                self.viewer_crop.set(g.crop);
                // CAMetalLayer posts no KVO for drawableSize; the engine's
                // output binding observes it, so announce the change here.
                let key = NSString::from_str("drawableSize");
                let layer = self.layer.as_ptr() as *mut AnyObject;
                unsafe {
                    let _: () = msg_send![layer, willChangeValueForKey: &*key];
                }
                self.layer
                    .set_drawable_size(core_graphics::geometry::CGSize::new(
                        g.layer.size.width * g.scale,
                        g.layer.size.height * g.scale,
                    ));
                unsafe {
                    let _: () = msg_send![layer, didChangeValueForKey: &*key];
                }
            } else {
                self.viewer.setHidden(true);
            }
            self.last_geometry = Some(geometry);
        }
        let bounds = self.content.bounds().size;
        let native = self
            .native_present
            .as_ref()
            .is_some_and(|present| present(bounds.width, bounds.height));
        let mut order = vec![
            gpui_macos::composition::AccessibilityItem::RemainingGpui,
            gpui_macos::composition::AccessibilityItem::Native(self.viewer.clone()),
        ];
        order.extend(
            self.native_accessibility
                .iter()
                .cloned()
                .map(gpui_macos::composition::AccessibilityItem::Native),
        );
        gpui_macos::composition::publish_accessibility(&self.gpui, order);
        (moved || native).into()
    }
    pub fn frame(&self) -> [f64; 4] {
        let f = self.window.frame();
        [f.origin.x, f.origin.y, f.size.width, f.size.height]
    }
    pub fn is_fullscreen(&self) -> bool {
        self.window
            .styleMask()
            .contains(NSWindowStyleMask::FullScreen)
    }
    pub fn is_key(&self) -> bool {
        self.window.isKeyWindow()
    }
    pub fn set_window_id(&self, id: u64) {
        WINDOW_IDS.with(|m| m.borrow_mut().insert(self.window.windowNumber(), id));
    }
    /// Session id of the first window in this window's native tab group, if this
    /// window is a non-leading tab.
    pub fn tab_leader(&self) -> Option<u64> {
        let tabs = self.window.tabbedWindows()?;
        let first = tabs.to_vec().into_iter().next()?;
        if first.windowNumber() == self.window.windowNumber() {
            return None;
        }
        WINDOW_IDS.with(|m| m.borrow().get(&first.windowNumber()).copied())
    }
    /// Restores a saved frame, kept on screen.
    pub fn apply_frame(&self, frame: [f64; 4], fullscreen: bool) {
        let mtm = MainThreadMarker::new().unwrap();
        let screens: Vec<[f64; 4]> = NSScreen::screens(mtm)
            .to_vec()
            .iter()
            .map(|s| {
                let v = s.visibleFrame();
                [v.origin.x, v.origin.y, v.size.width, v.size.height]
            })
            .collect();
        let f = crate::session::constrain_to_screens(crate::session::sane_frame(frame), &screens);
        self.window.setFrame_display(
            NSRect::new(NSPoint::new(f[0], f[1]), NSSize::new(f[2], f[3])),
            true,
        );
        if fullscreen && !self.is_fullscreen() {
            unsafe {
                let _: () = msg_send![&*self.window,performSelector:sel!(toggleFullScreen:),withObject:std::ptr::null::<AnyObject>(),afterDelay:0.0f64];
            }
        }
    }
    /// New windows cascade from `from` (the current key editor window).
    pub fn cascade_from(&self, from: [f64; 4]) {
        let top_left = NSPoint::new(from[0], from[1] + from[3]);
        let next = self.window.cascadeTopLeftFromPoint(top_left);
        self.window.cascadeTopLeftFromPoint(next);
    }
    /// Joins the native tab group of the window with session id `leader`.
    /// Deferred to the next run-loop turn: during launch/restore neither window
    /// is on screen yet, and AppKit then groups them without showing the tab
    /// bar (shell 2's restored-tab-group bug).
    pub fn join_tabs(&self, leader: u64) {
        *self._target.ivars().join.borrow_mut() = Some((leader, self.window.clone()));
        unsafe {
            let _: () = msg_send![&*self._target,performSelector:sel!(joinPendingTab:),withObject:std::ptr::null::<AnyObject>(),afterDelay:0.0f64];
        }
    }
    /// Number of windows in this window's native tab group and whether the
    /// tab bar shows (1 / false for a lone window).
    pub fn tab_info(&self) -> (usize, bool) {
        let count = self.window.tabbedWindows().map_or(1, |t| t.count());
        let visible = unsafe {
            let group: *mut AnyObject = msg_send![&*self.window, tabGroup];
            !group.is_null() && msg_send![group, isTabBarVisible]
        };
        (count, visible)
    }
    /// Publishes the window's shell state to menus/palette and syncs toolbar toggles.
    pub fn publish_menu_state(&self, state: &MenuState) {
        self.sync_tabs();
        if self.menu_state.borrow().as_ref() == Some(state) {
            return;
        }
        MENU_STATES.with(|m| {
            m.borrow_mut()
                .insert(self.window.windowNumber(), state.clone())
        });
        self.enable_tools(&state.disabled);
        *self.menu_state.borrow_mut() = Some(state.clone());
    }
    /// Selects the toolbar tool for a `tool*` command (keyboard/menu/palette).
    /// A tool with no gesture yet dims in the toolbar like its menu row.
    fn enable_tools(&self, disabled: &std::collections::BTreeSet<String>) {
        for item in self._toolbar.items().to_vec() {
            if item.itemIdentifier().to_string() == "tools"
                && let Some(view) = item.view()
            {
                for (index, (command, _, _)) in TOOLS.iter().enumerate() {
                    let on = !disabled.contains(*command);
                    unsafe {
                        let _: () = msg_send![&*view,setEnabled:on,forSegment:index as isize];
                    }
                }
            }
        }
    }
    /// Checks the Shape variant in its segment menu and shows its symbol.
    pub fn select_shape(&self, variant: &str) {
        let Some((_, label, glyph)) = SHAPES.iter().find(|(v, _, _)| *v == variant) else {
            return;
        };
        for item in self._toolbar.items().to_vec() {
            if item.itemIdentifier().to_string() != "tools" {
                continue;
            }
            let Some(view) = item.view() else { continue };
            unsafe {
                let menu: Option<Retained<NSMenu>> =
                    msg_send![&*view, menuForSegment: SHAPE_SEGMENT];
                for entry in menu.map(|m| m.itemArray().to_vec()).unwrap_or_default() {
                    let id: Option<Retained<NSString>> = entry
                        .representedObject()
                        .map(|o| Retained::cast_unchecked(o));
                    let on = id.is_some_and(|id| id.to_string() == variant);
                    entry.setState(if on {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                }
                let image = symbol(glyph, &format!("Shape: {label} (Q)"));
                let _: () = msg_send![&*view, setImage: &*image, forSegment: SHAPE_SEGMENT];
                let tip = NSString::from_str(&format!("{label} (Q)"));
                let _: () = msg_send![&*view, setToolTip: &*tip, forSegment: SHAPE_SEGMENT];
            }
        }
    }
    pub fn select_tool(&self, command: &str) {
        let Some(index) = TOOLS.iter().position(|(c, _, _)| *c == command) else {
            return;
        };
        for item in self._toolbar.items().to_vec() {
            if item.itemIdentifier().to_string() == "tools"
                && let Some(v) = item.view()
            {
                unsafe {
                    let _: () = msg_send![&*v,setSelectedSegment:index as isize];
                }
            }
        }
    }
    /// Opens the command palette with this window's dynamic rows.
    pub fn open_palette(&mut self, state: &MenuState) {
        let mut choices = self.base_choices.clone();
        choices.retain(|c| !state.disabled.contains(&c.command));
        for (id, title) in &state.panels {
            let open = state.open.contains(id);
            if open && !crate::layout::closable(id) {
                continue;
            }
            choices.push(Choice {
                label: format!("{} {title}", if open { "Hide" } else { "Show" }),
                command: format!("panel:toggle:{id}"),
            });
        }
        choices.extend(self.contributed_choices.iter().cloned());
        for (id, name, _) in &state.workspaces {
            choices.push(Choice {
                label: format!("Workspace: {name}"),
                command: format!("workspace:{id}"),
            });
        }
        choices.push(Choice {
            label: "Edit Workspace JSON…".into(),
            command: "layout:edit".into(),
        });
        choices.push(Choice {
            label: "Save Workspace…".into(),
            command: "layout:save".into(),
        });
        choices.push(Choice {
            label: "Reset Workspace".into(),
            command: "layout:reset".into(),
        });
        if self.fields.borrow().model.open {
            self.fields.borrow_mut().hide();
        }
        let mut p = self.palette.borrow_mut();
        if !p.model.open {
            p.model.choices = choices;
        }
        p.toggle();
    }
    /// Native context menu at a GPUI point. Empty labels are separators.
    /// Shown on the next run-loop turn so menu tracking never nests inside
    /// GPUI's event dispatch.
    pub fn context_menu(&self, items: &[MenuEntry], x: f32, y: f32) {
        // An agent's panel action never leaves a menu on screen.
        let model = menu_model(items);
        if let Some(choice) = crate::agent_menu::claim(&agent_menu_windows(&self.window), &model) {
            if let Some(entry) = choice.and_then(|i| entry_at(items, i)) {
                self._target.send(Command {
                    name: entry.command.clone(),
                    args: vec![],
                    token: None,
                });
            }
            return;
        }
        let mtm = MainThreadMarker::new().unwrap();
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        // Match the window (a forced scheme, or the system's).
        menu.setAppearance(Some(&self.window.effectiveAppearance()));
        append_menu(&menu, items, &self._target, mtm);
        let h = self.gpui.bounds().size.height;
        let at = if self.gpui.isFlipped() {
            NSPoint::new(x as f64, y as f64)
        } else {
            NSPoint::new(x as f64, h - y as f64)
        };
        *self._target.ivars().pending_menu.borrow_mut() = Some((menu, at, self.gpui.clone()));
        unsafe {
            let _: () = msg_send![&*self._target,performSelector:sel!(showPendingMenu:),withObject:std::ptr::null::<AnyObject>(),afterDelay:0.0f64];
        }
    }
    /// Save Workspace… : a window-modal sheet attached below the toolbar.
    /// Return = Save (the accent default button), Esc = Cancel. Save sends
    /// `layout:save:<name>`.
    pub fn prompt_workspace_name(&self, workspace: &str) {
        if self.window.attachedSheet().is_some() {
            return;
        }
        let sheet = workspace_sheet(&self.tokens, &format!("{workspace} Copy"));
        let field = sheet.field.clone();
        let sender = self.sender.clone();
        let handler = block2::RcBlock::new(move |response: NSModalResponse| {
            if response == NSModalResponseOK {
                let mut name = field.stringValue().to_string();
                if name.trim().is_empty() {
                    name = field
                        .placeholderString()
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                }
                let _ = sender.try_send(Command {
                    name: format!("layout:save:{name}"),
                    args: vec![],
                    token: None,
                });
            }
        });
        self.window
            .beginSheet_completionHandler(&sheet.window, Some(&handler));
        sheet.window.makeFirstResponder(Some(&sheet.field));
        *self.sheet.borrow_mut() = Some(sheet);
    }
    /// Edit Workspace JSON… (S100): the active workspace's definition in a
    /// wide sheet. Apply closes it only when the text is a usable definition
    /// (else the reason shows above the buttons, and nothing changes), then
    /// sends `layout:define` with the text and the definition it started from.
    pub fn edit_workspace_definition(&self, json: &str) {
        if self.window.attachedSheet().is_some() {
            return;
        }
        let target = definition_sheet(&self.tokens, json);
        let window = target.ivars().window.clone();
        let text = target.ivars().text.clone();
        let original = json.to_owned();
        let sender = self.sender.clone();
        let handler = block2::RcBlock::new(move |response: NSModalResponse| {
            if response == NSModalResponseOK {
                let _ = sender.try_send(Command {
                    name: "layout:define".into(),
                    args: vec![text.string().to_string().into(), original.clone().into()],
                    token: None,
                });
            }
        });
        self.window
            .beginSheet_completionHandler(&window, Some(&handler));
        window.makeFirstResponder(Some(&target.ivars().text));
        *self.definition.borrow_mut() = Some(target);
    }
    /// Queues a command for this window's editor, as a menu item would.
    pub fn send_command(&self, name: &str, args: Vec<serde_json::Value>) {
        let _ = self.sender.try_send(Command {
            name: name.into(),
            args,
            token: None,
        });
    }
    /// Test driver: closes an open context menu and any attached sheet.
    #[doc(hidden)]
    pub fn dismiss_transients(&self) {
        OPEN_MENU.with(|m| {
            if let Some(menu) = &*m.borrow() {
                menu.cancelTracking();
            }
        });
        if let Some(sheet) = self.window.attachedSheet() {
            self.window
                .endSheet_returnCode(&sheet, NSModalResponseCancel);
        }
        for popup in [&self.palette, &self.fields] {
            if popup.borrow().model.open {
                popup.borrow_mut().hide();
            }
        }
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
            "closeWindow" | "closeTab" => request_close(&self.window),
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
                // The field being edited may sit in another window (the
                // command palette, a popup): the key window's responder decides.
                let responder = app
                    .keyWindow()
                    .and_then(|w| w.firstResponder())
                    .or_else(|| self.window.firstResponder());
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
        MENU_STATES.with(|m| m.borrow_mut().remove(&self.window.windowNumber()));
        WINDOW_IDS.with(|m| m.borrow_mut().remove(&self.window.windowNumber()));
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
define_class!(
    /// Sheet buttons: end the sheet they sit in with OK (Save) or Cancel.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMShellSheetTarget"]
    struct SheetTarget;
    impl SheetTarget {
        #[unsafe(method(save:))]
        fn save(&self, sender: &NSControl) {
            end_sheet(sender, NSModalResponseOK);
        }
        #[unsafe(method(cancel:))]
        fn cancel(&self, sender: &NSControl) {
            end_sheet(sender, NSModalResponseCancel);
        }
    }
);
fn end_sheet(sender: &NSControl, response: NSModalResponse) {
    if let Some(sheet) = sender.window()
        && let Some(parent) = sheet.sheetParent()
    {
        parent.endSheet_returnCode(&sheet, response);
    }
}
pub struct DefinitionState {
    window: Retained<NSWindow>,
    text: Retained<NSTextView>,
    error: Retained<NSTextField>,
}
define_class!(
    /// The Workspace Definition sheet's buttons. Apply ends the sheet only
    /// when its text is a usable definition.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMShellDefinitionTarget"]
    #[ivars = DefinitionState]
    pub struct DefinitionTarget;
    impl DefinitionTarget {
        #[unsafe(method(apply:))]
        fn apply(&self, sender: &NSControl) {
            match crate::workspace::check_definition(&self.ivars().text.string().to_string()) {
                Ok(_) => end_sheet(sender, NSModalResponseOK),
                Err(why) => self.ivars().error.setStringValue(&NSString::from_str(&why)),
            }
        }
        #[unsafe(method(cancel:))]
        fn cancel(&self, sender: &NSControl) {
            end_sheet(sender, NSModalResponseCancel);
        }
    }
);
/// S100 Workspace Definition: heading, a monospaced editor, an error line,
/// then Cancel and Apply (the default button).
fn definition_sheet(t: &Tokens, json: &str) -> Retained<DefinitionTarget> {
    let mtm = MainThreadMarker::new().unwrap();
    let (m, ty, c) = (t.metrics, t.typography, &t.colors);
    let pad = (m.inset * 2.) as f64;
    let width = m.sheet_width_wide as f64;
    let inner = width - pad * 2.;
    let heading_h = (ty.heading * ty.dense_leading).ceil() as f64;
    let editor_h = m.palette_height as f64;
    let error_h = (ty.reading * ty.reading_leading * 2.).ceil() as f64;
    let footer_h = (m.control + m.inset * 2.) as f64;
    let height = pad + heading_h + m.gap as f64 + editor_h + m.gap as f64 + error_h + footer_h;
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(width, height)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    appearance(&window, t);
    window.setBackgroundColor(Some(&native_color(c.panel)));
    window.setTitle(&NSString::from_str("Workspace Definition"));
    let content = window.contentView().unwrap();
    let mut y = height - pad - heading_h;
    let title = NSTextField::labelWithString(&NSString::from_str("Workspace Definition"), mtm);
    title.setFont(Some(&NSFont::systemFontOfSize_weight(
        ty.heading as f64,
        unsafe { NSFontWeightSemibold },
    )));
    title.setTextColor(Some(&native_color(c.text)));
    title.setFrame(NSRect::new(
        NSPoint::new(pad, y),
        NSSize::new(inner, heading_h),
    ));
    content.addSubview(&title);
    y -= m.gap as f64 + editor_h;
    let frame = NSRect::new(NSPoint::new(pad, y), NSSize::new(inner, editor_h));
    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
    scroll.setHasVerticalScroller(true);
    scroll.setBorderType(NSBorderType::NoBorder);
    scroll.setDrawsBackground(true);
    scroll.setBackgroundColor(&native_color(c.field));
    scroll.setWantsLayer(true);
    if let Some(layer) = scroll.layer() {
        layer.setCornerRadius(m.control_radius as f64);
        layer.setMasksToBounds(true);
    }
    let text = NSTextView::initWithFrame(
        NSTextView::alloc(mtm),
        NSRect::new(NSPoint::ZERO, NSSize::new(inner, editor_h)),
    );
    text.setRichText(false);
    text.setDrawsBackground(false);
    text.setVerticallyResizable(true);
    text.setHorizontallyResizable(false);
    text.setAutomaticQuoteSubstitutionEnabled(false);
    text.setAutomaticDashSubstitutionEnabled(false);
    text.setAutomaticTextReplacementEnabled(false);
    text.setAutomaticSpellingCorrectionEnabled(false);
    text.setContinuousSpellCheckingEnabled(false);
    text.setTextContainerInset(NSSize::new(m.text_padding as f64, m.text_padding as f64));
    text.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
        ty.body as f64,
        unsafe { NSFontWeightRegular },
    )));
    text.setTextColor(Some(&native_color(c.text)));
    text.setString(&NSString::from_str(json));
    text.setAccessibilityLabel(Some(&NSString::from_str("Workspace definition")));
    scroll.setDocumentView(Some(&text));
    content.addSubview(&scroll);
    y -= m.gap as f64 + error_h;
    let error = NSTextField::wrappingLabelWithString(&NSString::from_str(""), mtm);
    error.setFont(Some(&NSFont::systemFontOfSize(ty.reading as f64)));
    error.setTextColor(Some(&native_color(c.danger)));
    error.setFrame(NSRect::new(
        NSPoint::new(pad, y),
        NSSize::new(inner, error_h),
    ));
    content.addSubview(&error);
    let line = NSBox::initWithFrame(
        NSBox::alloc(mtm),
        NSRect::new(
            NSPoint::new(0., footer_h),
            NSSize::new(width, m.border as f64),
        ),
    );
    line.setBoxType(NSBoxType::Custom);
    line.setBorderWidth(0.);
    line.setFillColor(&native_color(c.separator));
    content.addSubview(&line);
    let target: Retained<DefinitionTarget> = unsafe {
        msg_send![
            super(DefinitionTarget::alloc(mtm).set_ivars(DefinitionState {
                window: window.clone(),
                text: text.clone(),
                error,
            })),
            init
        ]
    };
    let button = |title: &str, action: objc2::runtime::Sel, key: &str| {
        let b = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(title),
                Some(&target),
                Some(action),
                mtm,
            )
        };
        b.setKeyEquivalent(&NSString::from_str(key));
        b.setControlSize(NSControlSize::Large);
        b
    };
    // Return types a newline in the editor; ⌘Return applies.
    let apply = button("Apply", sel!(apply:), "\r");
    apply.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
    apply.setBezelColor(Some(&native_color(c.accent)));
    let cancel = button("Cancel", sel!(cancel:), "\u{1b}");
    let bw = (m.control * 3.) as f64;
    let by = (footer_h - m.control as f64) / 2.;
    apply.setFrame(NSRect::new(
        NSPoint::new(width - pad - bw, by),
        NSSize::new(bw, m.control as f64),
    ));
    cancel.setFrame(NSRect::new(
        NSPoint::new(width - pad - bw * 2. - m.inset as f64, by),
        NSSize::new(bw, m.control as f64),
    ));
    content.addSubview(&cancel);
    content.addSubview(&apply);
    target
}
/// A sheet window and the controls the completion handler reads.
pub struct Sheet {
    pub window: Retained<NSWindow>,
    pub field: Retained<NSTextField>,
    cancel: Retained<NSButton>,
    save: Retained<NSButton>,
    _target: Retained<SheetTarget>,
}
/// The Save Workspace sheet (language §9 sheets): `heading` title, one
/// `reading` sentence, a labelled name field, then Cancel and the accent
/// default button named for the action. Content sits on the panel material.
pub fn workspace_sheet(t: &Tokens, placeholder: &str) -> Sheet {
    let mtm = MainThreadMarker::new().unwrap();
    let (m, ty, c) = (t.metrics, t.typography, &t.colors);
    let pad = (m.inset * 2.) as f64;
    let width = m.sheet_width as f64;
    let inner = width - pad * 2.;
    let heading_h = (ty.heading * ty.dense_leading).ceil() as f64;
    let reading_h = (ty.reading * ty.reading_leading * 2.).ceil() as f64;
    let row_h = (m.control + m.gap) as f64;
    let footer_h = (m.control + m.inset * 2.) as f64;
    let body_h = pad + heading_h + m.gap as f64 + reading_h + m.inset as f64 + row_h + pad;
    let height = body_h + footer_h;
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(width, height)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    appearance(&window, t);
    window.setBackgroundColor(Some(&native_color(c.panel)));
    window.setTitle(&NSString::from_str("Save Workspace"));
    let content = window.contentView().unwrap();
    // Top-down layout in AppKit's bottom-up coordinates.
    let mut y = height - pad;
    let label = |text: &str, size: f32, weight: f64, color: u32| {
        let l = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
        l.setFont(Some(&NSFont::systemFontOfSize_weight(size as f64, weight)));
        l.setTextColor(Some(&native_color(color)));
        l
    };
    let title = label(
        "Save Workspace",
        ty.heading,
        unsafe { NSFontWeightSemibold },
        c.text,
    );
    y -= heading_h;
    title.setFrame(NSRect::new(
        NSPoint::new(pad, y),
        NSSize::new(inner, heading_h),
    ));
    content.addSubview(&title);
    y -= m.gap as f64 + reading_h;
    let reading = label(
        "Saved workspaces are available in every project from Window ▸ Workspace.",
        ty.reading,
        unsafe { NSFontWeightRegular },
        c.secondary,
    );
    reading.setFrame(NSRect::new(
        NSPoint::new(pad, y),
        NSSize::new(inner, reading_h),
    ));
    content.addSubview(&reading);
    y -= m.inset as f64 + row_h;
    let name = label(
        "Name",
        ty.reading,
        unsafe { NSFontWeightRegular },
        c.secondary,
    );
    let text_h = (ty.reading * ty.dense_leading).ceil() as f64;
    name.setFrame(NSRect::new(
        NSPoint::new(pad, y + (row_h - text_h) / 2.),
        NSSize::new(inner / 3., text_h),
    ));
    content.addSubview(&name);
    let field_w = inner * 2. / 3.;
    let field = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(
            NSPoint::new(pad + inner - field_w, y + (row_h - m.control as f64) / 2.),
            NSSize::new(field_w, m.control as f64),
        ),
    );
    field.setFont(Some(&NSFont::systemFontOfSize(ty.reading as f64)));
    field.setPlaceholderString(Some(&NSString::from_str(placeholder)));
    field.setAccessibilityLabel(Some(&NSString::from_str("Workspace Name")));
    content.addSubview(&field);
    // Footer: a separator, then Cancel and Save trailing.
    let line = NSBox::initWithFrame(
        NSBox::alloc(mtm),
        NSRect::new(
            NSPoint::new(0., footer_h),
            NSSize::new(width, m.border as f64),
        ),
    );
    line.setBoxType(NSBoxType::Custom);
    line.setBorderWidth(0.);
    line.setFillColor(&native_color(c.separator));
    content.addSubview(&line);
    let target: Retained<SheetTarget> = unsafe { msg_send![SheetTarget::alloc(mtm), init] };
    let button = |title: &str, action: objc2::runtime::Sel, key: &str| {
        let b = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(title),
                Some(&target),
                Some(action),
                mtm,
            )
        };
        b.setKeyEquivalent(&NSString::from_str(key));
        b.setControlSize(NSControlSize::Large);
        b
    };
    let save = button("Save", sel!(save:), "\r");
    // The accent default button (AccentColor tints it; set explicitly for
    // unbundled dev runs).
    save.setBezelColor(Some(&native_color(c.accent)));
    let cancel = button("Cancel", sel!(cancel:), "\u{1b}");
    let bw = (m.control * 3.) as f64;
    let by = (footer_h - m.control as f64) / 2.;
    save.setFrame(NSRect::new(
        NSPoint::new(width - pad - bw, by),
        NSSize::new(bw, m.control as f64),
    ));
    cancel.setFrame(NSRect::new(
        NSPoint::new(width - pad - bw * 2. - m.inset as f64, by),
        NSSize::new(bw, m.control as f64),
    ));
    content.addSubview(&cancel);
    content.addSubview(&save);
    unsafe {
        field.setNextKeyView(Some(&cancel));
        cancel.setNextKeyView(Some(&save));
        save.setNextKeyView(Some(&field));
    }
    window.setAutorecalculatesKeyViewLoop(false);
    window.setInitialFirstResponder(Some(&field));
    Sheet {
        window,
        field,
        cancel,
        save,
        _target: target,
    }
}
impl Sheet {
    fn tab(&self, reverse: bool) {
        let responder = self.window.firstResponder();
        let current = if responder
            .as_ref()
            .is_some_and(|r| std::ptr::eq(&**r, &*self.cancel as *const _ as *const NSResponder))
        {
            1
        } else if responder
            .as_ref()
            .is_some_and(|r| std::ptr::eq(&**r, &*self.save as *const _ as *const NSResponder))
        {
            2
        } else {
            0
        };
        let next = (current + if reverse { 2 } else { 1 }) % 3;
        let view: &NSView = match next {
            1 => &self.cancel,
            2 => &self.save,
            _ => &self.field,
        };
        self.window.makeFirstResponder(Some(view));
    }
}
pub struct Popup {
    window: Retained<PopupWindow>,
    parent: Retained<NSWindow>,
    search: Retained<NSTextField>,
    /// The magnifying glass before the query, and the hairline under them.
    glyph: Retained<NSImageView>,
    rule: Retained<NSBox>,
    body: Retained<NSView>,
    spring: Option<(std::time::Instant, pm_design::motion::Analytic)>,
    rows: Vec<Retained<NSButton>>,
    /// Right-aligned shortcut labels for the visible rows.
    hints: Vec<Retained<NSTextField>>,
    /// command → shortcut glyphs shown beside palette rows.
    shortcuts: HashMap<String, String>,
    glider: Retained<NSBox>,
    target: Option<Retained<PopupTarget>>,
    model: Picker,
    sender: async_channel::Sender<Command>,
    tokens: Tokens,
    previous: Option<Retained<NSResponder>>,
    /// Tallest the panel may grow (`palette_height` / `field_menu_height`).
    max_height: f32,
    row_focus: bool,
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
        // A child of the editor window while shown, never above other apps (P-CHILD).
        gpui_macos::child_window::apply(&window);
        window.setTitle(&NSString::from_str(if surface == Surface::Palette {
            "Command Palette"
        } else {
            "Field Menu"
        }));
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
        let search = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
        search.setPlaceholderString(Some(&NSString::from_str(if surface == Surface::Palette {
            "Search commands…"
        } else {
            "Search fields…"
        })));
        search.setFont(Some(&NSFont::systemFontOfSize(t.typography.heading as f64)));
        // The field is the menu's only input and always focused: no ring (owner rule:
        // no accent focus rings) and no bezel; a quiet query row over a hairline,
        // as Spotlight's (v1's palette too).
        search.setFocusRingType(NSFocusRingType::None);
        search.setBezeled(false);
        search.setBordered(false);
        search.setDrawsBackground(false);
        search.setAccessibilityLabel(Some(&NSString::from_str("Search")));
        body.addSubview(&search);
        let glyph = NSImageView::imageViewWithImage(&symbol("magnifyingglass", "Search"), mtm);
        glyph.setSymbolConfiguration(Some(
            &NSImageSymbolConfiguration::configurationWithPointSize_weight(
                t.typography.heading as f64,
                unsafe { NSFontWeightRegular },
            ),
        ));
        glyph.setContentTintColor(Some(&native_color(t.colors.muted)));
        body.addSubview(&glyph);
        let rule = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::ZERO);
        rule.setBoxType(NSBoxType::Separator);
        body.addSubview(&rule);
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
            glyph,
            rule,
            body,
            spring: None,
            rows: vec![],
            hints: vec![],
            shortcuts: HashMap::new(),
            glider,
            target: None,
            model: Picker {
                choices,
                ..Default::default()
            },
            sender,
            tokens: t.clone(),
            previous: None,
            max_height: height,
            row_focus: false,
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
        self.glyph.removeFromSuperview();
        self.rule.removeFromSuperview();
        self.glider.removeFromSuperview();
        body.addSubview(&self.search);
        body.addSubview(&self.glyph);
        body.addSubview(&self.rule);
        body.addSubview(&self.glider);
        self.glyph
            .setContentTintColor(Some(&native_color(t.colors.muted)));
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
        for hint in self.hints.drain(..) {
            hint.removeFromSuperview();
        }
        let m = self.tokens.metrics;
        let ids = self.model.filtered();
        // The panel shrinks to its results (max `max_height`), keeping its top edge.
        let chrome = m.menu_inset * 3. + m.control;
        let max_rows = ((self.max_height - chrome) / m.menu_row).floor().max(1.) as usize;
        let height = (chrome + ids.len().clamp(1, max_rows) as f32 * m.menu_row) as f64;
        let mut frame = self.window.frame();
        if (frame.size.height - height).abs() > 0.5 {
            frame.origin.y += frame.size.height - height;
            frame.size.height = height;
            self.window.setFrame_display(frame, self.model.open);
        }
        let size = frame.size;
        // The query row: glyph in a control-square at the rows' text inset,
        // the field after it, the text centred on the row.
        let (row_y, control) = (
            size.height - (m.menu_inset + m.control) as f64,
            m.control as f64,
        );
        // The glyph's ink, not its square, starts at the rows' text inset.
        let glyph_x = (m.menu_inset + m.gap) as f64
            - ((control - self.tokens.typography.heading as f64) / 2.).floor();
        self.glyph.setFrame(NSRect::new(
            NSPoint::new(glyph_x, row_y),
            NSSize::new(control, control),
        ));
        let text_h = self.search.fittingSize().height.ceil();
        let field_x = glyph_x + control + m.gap as f64;
        self.search.setFrame(NSRect::new(
            NSPoint::new(field_x, row_y + ((control - text_h) / 2.).round()),
            NSSize::new(size.width - field_x - m.menu_inset as f64, text_h),
        ));
        self.rule.setFrame(NSRect::new(
            NSPoint::new(
                0.,
                (size.height - (m.menu_inset * 1.5 + m.control) as f64).round(),
            ),
            NSSize::new(size.width, 1.),
        ));
        let visible = max_rows;
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
            b.setAccessibilityLabel(Some(&NSString::from_str(&c.label)));
            b.setAccessibilityIdentifier(Some(&NSString::from_str(&format!(
                "shell.palette.{}",
                c.command
            ))));
            unsafe {
                b.setAccessibilityValue(Some(&NSString::from_str(if i == self.model.selected {
                    "Selected"
                } else {
                    ""
                })));
            }
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
            // The glider spans the row; the title sits `gap` inside it.
            b.setFrame(NSRect::new(
                NSPoint::new(r.origin.x + m.gap as f64, r.origin.y),
                NSSize::new(r.size.width - m.gap as f64 * 2., r.size.height),
            ));
            if i == self.model.selected {
                self.glider.setFrame(r);
            }
            self.body.addSubview(&b);
            if let Some(glyphs) = self.shortcuts.get(&c.command) {
                let hint = NSTextField::labelWithString(&NSString::from_str(glyphs), mtm);
                hint.setFont(Some(&NSFont::systemFontOfSize(
                    self.tokens.typography.body as f64,
                )));
                hint.setTextColor(Some(&native_color(self.tokens.colors.muted)));
                hint.setAlignment(NSTextAlignment::Right);
                // Fixed trailing column, `menu_inset` from the row's right edge.
                let column = (m.menu_row * 4.) as f64;
                let text_h = hint.fittingSize().height;
                let hr = NSRect::new(
                    NSPoint::new(
                        r.origin.x + r.size.width - m.menu_inset as f64 - column,
                        r.origin.y + ((r.size.height - text_h) / 2.).floor(),
                    ),
                    NSSize::new(column, text_h),
                );
                hint.setFrame(hr);
                b.setAccessibilityHelp(Some(&NSString::from_str(glyphs)));
                self.body.addSubview(&hint);
                self.hints.push(hint);
            }
            if self.row_focus && i == self.model.selected {
                self.window.makeFirstResponder(Some(&b));
            }
            self.rows.push(b);
        }
    }
    fn toggle(&mut self) {
        if self.model.open {
            self.hide();
            return;
        }
        self.row_focus = false;
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
        }
        gpui_macos::child_window::attach(&self.window, &self.parent);
        // Only while the app is active, never over other apps (P-CHILD).
        gpui_macos::child_window::show(&self.window, true);
        self.window.makeFirstResponder(Some(&self.search));
        animate(&self.window, true, &self.tokens, &mut self.spring);
    }
    fn hide(&mut self) {
        self.model.open = false;
        gpui_macos::child_window::forget(&self.window);
        let duration = animate(&self.window, false, &self.tokens, &mut self.spring);
        unsafe {
            let _: () = msg_send![&*self.window,performSelector:sel!(orderOut:),withObject:std::ptr::null::<AnyObject>(),afterDelay:duration];
        }
        // Becoming key can synchronously render GPUI. Restore after the event
        // monitor releases its popup borrow; a command may already have opened
        // a sheet/settings/new window, whose focus must take precedence.
        let (parent, popup, previous) = (
            self.parent.clone(),
            self.window.clone(),
            self.previous.clone(),
        );
        let restore = block2::RcBlock::new(move || {
            let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
            let another_window = app.keyWindow().is_some_and(|key| {
                key.windowNumber() != popup.windowNumber()
                    && key.windowNumber() != parent.windowNumber()
            });
            if !another_window && parent.isVisible() && parent.attachedSheet().is_none() {
                parent.makeKeyWindow();
                parent.makeFirstResponder(previous.as_deref());
            }
        });
        unsafe {
            objc2_foundation::NSOperationQueue::mainQueue().addOperationWithBlock(&restore);
        }
    }
    fn key(&mut self, key: &str) {
        if matches!(key, "tab" | "shift+tab") {
            let n = self.model.filtered().len();
            if n == 0 {
                return;
            }
            let reverse = key == "shift+tab";
            if !self.row_focus {
                self.row_focus = true;
                self.model.selected = if reverse { n - 1 } else { 0 };
            } else if (!reverse && self.model.selected + 1 == n)
                || (reverse && self.model.selected == 0)
            {
                self.row_focus = false;
            } else if reverse {
                self.model.selected -= 1;
            } else {
                self.model.selected += 1;
            }
            self.refresh();
            if !self.row_focus {
                self.window.makeFirstResponder(Some(&self.search));
            }
            return;
        }
        let key = if key == "space" && self.row_focus {
            "enter"
        } else if key == "space" {
            return;
        } else {
            key
        };
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
    state: &mut Option<(std::time::Instant, pm_design::motion::Analytic)>,
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
                        pm_design::motion::Analytic {
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
    let mut commands: Vec<(String, Vec<serde_json::Value>)> = vec![];
    let menus = menu::menus();
    for (_, items) in &menus {
        for item in items {
            commands.push((item.command.clone(), item.args.clone()));
        }
    }
    let tools_start = commands.len();
    commands.extend(menu::tools().into_iter().map(|i| (i.command, i.args)));
    let target = Target::new(mtm, commands, None);
    // Dynamic rows look the target up through MENU_TARGET.
    MENU_TARGET.with(|t| *t.borrow_mut() = Some(target.clone()));
    let delegate = |menu: &NSMenu, kind: MenuKind| {
        let d = MenuDelegate::new(mtm, kind);
        menu.setDelegate(Some(ProtocolObject::from_ref(&*d)));
        MENU_DELEGATES.with(|v| v.borrow_mut().push(d));
    };
    let row = |label: &str, chord: &str, tag: usize| {
        let (key, flags) = menu::accelerator(chord);
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(label),
                Some(sel!(act:)),
                &NSString::from_str(&key),
            )
        };
        item.setKeyEquivalentModifierMask(NSEventModifierFlags(flags as _));
        item.setTag(tag as isize);
        unsafe { item.setTarget(Some(&target)) };
        item
    };
    let mut index = 0;
    for (title, items) in menus {
        let submenu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
        submenu.setAutoenablesItems(
            title == "Edit" || menu::DISPLAY_ONLY.contains(&title) || title == "View",
        );
        if menu::DISPLAY_ONLY.contains(&title) {
            delegate(&submenu, MenuKind::DisplayOnly);
        }
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
            match spec.command.as_str() {
                "workspaces" => {
                    let workspaces = NSMenu::new(mtm);
                    delegate(&workspaces, MenuKind::Workspaces);
                    item.setSubmenu(Some(&workspaces));
                }
                "tools" => {
                    let tools = NSMenu::new(mtm);
                    for (i, t) in menu::tools().iter().enumerate() {
                        tools.addItem(&row(&t.label, &t.chord, tools_start + i));
                    }
                    delegate(&tools, MenuKind::DisplayOnly);
                    item.setSubmenu(Some(&tools));
                }
                "panels" => {
                    let dynamic = NSMenu::new(mtm);
                    delegate(&dynamic, MenuKind::Panels);
                    item.setSubmenu(Some(&dynamic));
                }
                "openRecent" => {
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
                _ => {}
            }
            if menu::SUBMENUS.contains(&spec.command.as_str()) {
                unsafe { item.setAction(None) };
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
}
/// App name for unbundled dev runs. A bundle's Info.plist names the app; a bare
/// binary would otherwise show its executable name — or the launching
/// process's — in the menu bar. Must run before NSApplication finishes launching.
pub fn install_identity() {
    use objc2_foundation::{NSBundle, NSProcessInfo};
    NSProcessInfo::processInfo().setProcessName(&NSString::from_str("Powermove"));
    let bundle = NSBundle::mainBundle();
    if let Some(info) = bundle.infoDictionary() {
        let key = NSString::from_str("CFBundleName");
        let named = info.objectForKey(&key).is_some_and(|v| {
            unsafe { Retained::cast_unchecked::<NSString>(v) }.to_string() == "Powermove"
        });
        // The bundle dictionary is mutable in practice (unbundled executables get
        // a synthesised one); only touch it when it really is.
        let mutable: bool = unsafe {
            msg_send![&*info, isKindOfClass: objc2_foundation::NSMutableDictionary::<AnyObject, AnyObject>::class()]
        };
        if !named && mutable {
            unsafe {
                let _: () =
                    msg_send![&*info, setObject: &*NSString::from_str("Powermove"), forKey: &*key];
            }
        }
    }
}
/// Name of the accent colour asset the bundle declares (`NSAccentColorName`).
fn bundle_accent_name() -> Option<String> {
    let info = objc2_foundation::NSBundle::mainBundle().infoDictionary()?;
    let value = info.objectForKey(&NSString::from_str("NSAccentColorName"))?;
    Some(unsafe { Retained::cast_unchecked::<NSString>(value) }.to_string())
}
/// Powermove orange as a dynamic colour: `accent` of the light or dark scheme,
/// resolved for whichever appearance draws it.
pub fn accent_color() -> Retained<NSColor> {
    let light = Tokens::new(Scheme::Light, Accessibility::default())
        .colors
        .accent;
    let dark = Tokens::new(Scheme::Dark, Accessibility::default())
        .colors
        .accent;
    let (light, dark) = (native_color(light), native_color(dark));
    let provider = block2::RcBlock::new(
        move |appearance: std::ptr::NonNull<NSAppearance>| -> std::ptr::NonNull<NSColor> {
            let appearance = unsafe { appearance.as_ref() };
            let list = unsafe { [NSAppearanceNameAqua, NSAppearanceNameDarkAqua] };
            let names = NSArray::from_slice(&list);
            let dark_match = appearance
                .bestMatchFromAppearancesWithNames(&names)
                .is_some_and(|n| n.to_string().contains("Dark"));
            // Both colours live as long as the block (captured).
            let color: &NSColor = if dark_match { &dark } else { &light };
            std::ptr::NonNull::from(color)
        },
    );
    unsafe { NSColor::colorWithName_dynamicProvider(None, &provider) }
}
/// The app's AccentColor is Powermove orange, so system controls (focus rings,
/// default buttons, switches, selection) never show the user's blue next to
/// our orange. A bundle does this with `NSAccentColorName` + the asset catalog
/// (`apps/native/bundle`); unbundled dev and test runs set the same colour on
/// NSApp at launch.
pub fn install_accent() -> bool {
    if bundle_accent_name().is_some() {
        return true;
    }
    let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
    let set = sel!(_setAccentColor:);
    let responds: bool = unsafe { msg_send![&*app, respondsToSelector: set] };
    if responds {
        let color = accent_color();
        unsafe {
            let _: () = msg_send![&*app, _setAccentColor: &*color];
        }
    }
    responds
}
/// The accent AppKit currently tints controls with, as 0xRRGGBB in sRGB for
/// the given scheme (tests and the perf gate check it is not system blue).
pub fn system_accent(scheme: Scheme) -> Option<u32> {
    let name = unsafe {
        if scheme == Scheme::Light {
            NSAppearanceNameAqua
        } else {
            NSAppearanceNameDarkAqua
        }
    };
    let appearance = NSAppearance::appearanceNamed(name)?;
    let out = Cell::new(None);
    let block = block2::StackBlock::new(|| {
        let c = NSColor::controlAccentColor();
        if let Some(c) = c.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()) {
            let ch = |v: f64| (v.clamp(0., 1.) * 255.).round() as u32;
            out.set(Some(
                (ch(c.redComponent()) << 16)
                    | (ch(c.greenComponent()) << 8)
                    | ch(c.blueComponent()),
            ));
        }
    });
    appearance.performAsCurrentDrawingAppearance(&block);
    out.get()
}

/// The window keys an agent's panel action registers under: a menu's own
/// window and the editor window it is a child of.
pub fn agent_menu_windows(window: &NSWindow) -> Vec<usize> {
    let mut keys = vec![window as *const NSWindow as usize];
    if let Some(parent) = window.parentWindow() {
        keys.push(Retained::as_ptr(&parent) as usize);
    }
    keys
}
/// Context-menu rows as `agent_menu` items (preorder, separators included).
fn menu_model(items: &[MenuEntry]) -> Vec<serde_json::Value> {
    items
        .iter()
        .map(|e| {
            serde_json::json!({"label": e.label, "enabled": e.enabled, "checked": e.checked,
                "children": menu_model(&e.children)})
        })
        .collect()
}
fn entry_at(items: &[MenuEntry], index: usize) -> Option<&MenuEntry> {
    fn find<'a>(items: &'a [MenuEntry], index: usize, next: &mut usize) -> Option<&'a MenuEntry> {
        for item in items {
            if *next == index {
                return Some(item);
            }
            *next += 1;
            if let Some(found) = find(&item.children, index, next) {
                return Some(found);
            }
        }
        None
    }
    find(items, index, &mut 0)
}
fn append_menu(menu: &NSMenu, items: &[MenuEntry], target: &AnyObject, mtm: MainThreadMarker) {
    for entry in items {
        if entry.label.is_empty() {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        }
        let item = named_item(mtm, &entry.label, &entry.command);
        unsafe { item.setTarget(Some(target)) };
        item.setState(if entry.checked {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        item.setEnabled(entry.enabled);
        if !entry.children.is_empty() {
            let submenu = NSMenu::new(mtm);
            submenu.setAutoenablesItems(false);
            append_menu(&submenu, &entry.children, target, mtm);
            item.setSubmenu(Some(&submenu));
        }
        menu.addItem(&item);
    }
}

#[cfg(test)]
mod agent_menu_tests {
    use super::{MenuEntry, entry_at, menu_model};
    use crate::agent_menu;

    #[test]
    fn an_agent_picks_context_menu_rows_by_label_including_submenus() {
        let items = vec![
            MenuEntry::item("Rename", "layer:rename"),
            MenuEntry::separator(),
            MenuEntry::submenu(
                "Move to Dock",
                vec![
                    MenuEntry::item("Left", "dock:left"),
                    MenuEntry::item("Right", "dock:right"),
                ],
            ),
        ];
        for (want, command) in [
            ("Rename", "layer:rename"),
            ("Move to Dock ▸ Right", "dock:right"),
        ] {
            let scope = agent_menu::begin(11, Some(want.into()), || {});
            let choice = agent_menu::claim(&[11], &menu_model(&items)).unwrap();
            assert_eq!(entry_at(&items, choice.unwrap()).unwrap().command, command);
            drop(scope);
        }
    }
}

#[cfg(test)]
mod viewer_window_tests {
    use super::viewer_window;

    #[test]
    fn a_canvas_inside_its_slot_is_whole_and_centred() {
        let (frame, crop) = viewer_window([800., 600.], [640., 360.], 2., [0., 0.]);
        assert_eq!(crop, None);
        assert_eq!((frame.origin.x, frame.origin.y), (80., 120.));
        assert_eq!((frame.size.width, frame.size.height), (640., 360.));
    }

    #[test]
    fn a_zoomed_canvas_draws_only_the_window_in_its_slot() {
        // 1920×1080 at 400 %, Retina, in an 800×600 pt slot.
        let (frame, crop) = viewer_window([800., 600.], [7680., 4320.], 2., [0., 0.]);
        assert_eq!((frame.origin.x, frame.origin.y), (0., 0.));
        assert_eq!((frame.size.width, frame.size.height), (800., 600.));
        let [x, y, cw, ch] = crop.unwrap();
        assert_eq!((cw, ch), (15360, 8640));
        assert_eq!((x, y), ((15360 - 1600) / 2, (8640 - 1200) / 2));
        // Wider than the slot only: the full height stays, centred across.
        let (frame, crop) = viewer_window([800., 600.], [1000.5, 300.], 2., [0., 0.]);
        assert_eq!((frame.size.width, frame.size.height), (800., 300.));
        assert_eq!(frame.origin.y, 150.);
        let [x, y, cw, ch] = crop.unwrap();
        assert_eq!((x, y, cw, ch), ((2001 - 1600) / 2, 0, 2001, 600));
    }

    #[test]
    fn a_panned_canvas_moves_its_layer_and_windows_what_leaves_the_slot() {
        // Pan is in points, y down; the layer frame is in AppKit's y-up space.
        let (frame, crop) = viewer_window([800., 600.], [640., 360.], 2., [50., 50.]);
        assert_eq!(crop, None);
        assert_eq!((frame.origin.x, frame.origin.y), (130., 70.));
        // Pushed past the right edge: only the part still in the slot renders.
        let (frame, crop) = viewer_window([800., 600.], [640., 360.], 2., [300., -200.]);
        assert_eq!((frame.origin.x, frame.size.width), (380., 420.));
        assert_eq!((frame.origin.y, frame.size.height), (320., 280.));
        assert_eq!(crop, Some([0, 160, 1280, 720]));
        // Zoomed and panned: the window follows the pan across the canvas.
        let (_, crop) = viewer_window([800., 600.], [7680., 4320.], 2., [-1000., 0.]);
        let [x, y, ..] = crop.unwrap();
        assert_eq!((x, y), ((15360 - 1600) / 2 + 2000, (8640 - 1200) / 2));
        // Panned wholly out: a sliver remains, still on its canvas.
        let (frame, crop) = viewer_window([800., 600.], [640., 360.], 2., [2000., 0.]);
        assert_eq!(frame.size.width, 1.);
        let [x, _, cw, _] = crop.unwrap();
        assert!(x + 2 <= cw);
    }
}
