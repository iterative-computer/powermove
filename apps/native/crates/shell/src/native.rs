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
    pub open: Vec<String>,
    pub saved: Vec<String>,
    pub workspace: String,
    pub sidebar: bool,
    pub inspector: bool,
    pub tool: String,
    /// Commands that cannot run now: menu rows dim, the palette omits them.
    pub disabled: std::collections::BTreeSet<String>,
}
/// The editor window commands should go to: the key window, else the main window
/// (a key palette/field-menu child panel is not an editor window).
fn editor_window_number() -> Option<isize> {
    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    ROUTES.with(|r| {
        let r = r.borrow();
        [app.keyWindow(), app.mainWindow()]
            .into_iter()
            .flatten()
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
    Accessibility {
        reduce_transparency: w.accessibilityDisplayShouldReduceTransparency(),
        increase_contrast: w.accessibilityDisplayShouldIncreaseContrast(),
        reduce_motion: w.accessibilityDisplayShouldReduceMotion(),
    }
}
/// Drawable pixels for a layer of `size` points at the preview resolution.
pub fn preview_drawable(size: (f64, f64), backing: f64, preview: f64) -> (f64, f64) {
    (
        (size.0 * backing * preview).round().max(2.),
        (size.1 * backing * preview).round().max(2.),
    )
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
                return (!state.disabled.contains(name)).into();
            }
            true.into()
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
    window.makeKeyAndOrderFront(None);
    // A group of two or more must show its tab bar. AppKit can leave it hidden
    // (unified-compact toolbars, windows grouped before they were on screen).
    unsafe {
        let group: *mut AnyObject = msg_send![&*lead, tabGroup];
        if !group.is_null() {
            let visible: bool = msg_send![group, isTabBarVisible];
            if !visible {
                let _: () = msg_send![&*lead, toggleTabBar: std::ptr::null::<AnyObject>()];
            }
        }
    }
    for w in lead.tabbedWindows().map(|t| t.to_vec()).unwrap_or_default() {
        reveal_tab_bar(&w);
    }
    true
}
/// GPUI-CE hides AppKit's tab bar (Zed draws its own): its
/// `addTitlebarAccessoryViewController:` override hides the accessory and
/// sets its height to 0, so a grouped window showed no tabs (shell 2's open
/// item). Powermove uses the native tab bar: un-hide it and give it back its
/// height, then let AppKit lay the titlebar out again.
pub fn reveal_tab_bar(window: &NSWindow) -> bool {
    let mut revealed = false;
    for vc in window.titlebarAccessoryViewControllers().to_vec() {
        let view = vc.view();
        let subviews = view.subviews().to_vec();
        let is_tab_bar = subviews
            .iter()
            .any(|s| s.class().name().to_string_lossy().contains("NSTabBar"));
        if !is_tab_bar {
            continue;
        }
        view.setHidden(false);
        let mut f = view.frame();
        f.size.height = TAB_BAR_HEIGHT;
        view.setFrame(f);
        // AppKit's own layout: a 28 pt bar 8 pt above the accessory's bottom edge.
        for bar in subviews
            .iter()
            .filter(|s| s.class().name().to_string_lossy().contains("NSTabBar"))
        {
            bar.setFrame(NSRect::new(
                NSPoint::new(0., TAB_BAR_INSET),
                NSSize::new(f.size.width, TAB_BAR_HEIGHT - TAB_BAR_INSET),
            ));
        }
        // AppKit sized the titlebar from the hidden 0 pt accessory; re-adding
        // it through NSWindow's own implementation (not GPUI's override) makes
        // the titlebar grow to fit the bar.
        if let Some(index) = window
            .titlebarAccessoryViewControllers()
            .to_vec()
            .iter()
            .position(|c| Retained::as_ptr(c) == Retained::as_ptr(&vc))
        {
            window.removeTitlebarAccessoryViewControllerAtIndex(index as isize);
            unsafe {
                let _: () = msg_send![super(window, NSWindow::class()), addTitlebarAccessoryViewController: &*vc];
            }
        }
        revealed = true;
    }
    if revealed {
        relayout_titlebar(window);
    }
    revealed
}
/// AppKit's tab bar accessory height (28 pt bar + 8 pt inset) on macOS 26–27.
const TAB_BAR_HEIGHT: f64 = 36.;
const TAB_BAR_INSET: f64 = 8.;
/// A 1 pt frame round trip makes AppKit lay the titlebar out again.
fn relayout_titlebar(window: &NSWindow) {
    let f = window.frame();
    let mut taller = f;
    taller.size.height += 1.;
    window.setFrame_display(taller, false);
    window.setFrame_display(f, true);
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
            for id in crate::layout::PANELS {
                let item = named_item(
                    mtm,
                    crate::layout::panel_title(id),
                    &format!("panel:toggle:{id}"),
                );
                item.setState(if state.open.iter().any(|o| o == id) {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                menu.addItem(&item);
            }
        }
    }
}
/// Window ▸ Workspace: presets (current one checked), saved workspaces, then
/// Save / Delete / Reset. One vocabulary: a workspace is a panel arrangement.
fn workspace_menu(state: &MenuState) -> Vec<Retained<NSMenuItem>> {
    let mtm = MainThreadMarker::new().unwrap();
    let mut items = vec![];
    for (id, label) in crate::layout::PRESETS {
        let item = named_item(mtm, label, &format!("workspace:{id}"));
        item.setState(if id == state.workspace {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        items.push(item);
    }
    items.push(NSMenuItem::separatorItem(mtm));
    for name in &state.saved {
        items.push(named_item(mtm, name, &format!("layout:apply:{name}")));
    }
    if !state.saved.is_empty() {
        items.push(NSMenuItem::separatorItem(mtm));
    }
    items.push(named_item(mtm, "Save Workspace…", "layout:save"));
    let delete = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Delete Workspace"),
            None,
            &NSString::from_str(""),
        )
    };
    let names = NSMenu::new(mtm);
    for name in &state.saved {
        names.addItem(&named_item(mtm, name, &format!("layout:delete:{name}")));
    }
    delete.setSubmenu(Some(&names));
    delete.setEnabled(!state.saved.is_empty());
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
    title: Retained<NSTextField>,
    subtitle: Retained<NSTextField>,
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
                super(Self::alloc(mtm).set_ivars(ToolbarState {
                    target,
                    title: NSTextField::labelWithString(&NSString::from_str("Untitled"), mtm),
                    subtitle: NSTextField::labelWithString(&NSString::from_str(""), mtm),
                    tokens,
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
            control.setAccessibilityLabel(Some(&NSString::from_str("Tools")));
            control.setAccessibilityIdentifier(Some(&NSString::from_str("shell.toolbar.tools")));
            for (i, (_, _, l)) in TOOLS.iter().enumerate() {
                control.setToolTip_forSegment(Some(&NSString::from_str(l)), i as isize);
            }
            item.setView(Some(&control));
            item.setLabel(&NSString::from_str("Tools"));
            return Some(item);
        }
        if name == "title" {
            let (title, subtitle) = (&self.ivars().title, &self.ivars().subtitle);
            title.setFont(Some(&NSFont::systemFontOfSize_weight(
                t.typography.title as f64,
                unsafe { NSFontWeightSemibold },
            )));
            title.setTextColor(Some(&native_color(t.colors.text)));
            title.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            subtitle.setFont(Some(&NSFont::systemFontOfSize(t.typography.minimum as f64)));
            subtitle.setTextColor(Some(&native_color(t.colors.muted)));
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
            view.addSubview(title);
            view.addSubview(subtitle);
            layout_title(title, subtitle, t);
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
            "toggleSidebar" => ("sidebar.leading", "Sidebar"),
            "agent" => ("sparkles", "Agent"),
            "export" => ("square.and.arrow.up", "Export"),
            "toggleInspector" => ("sidebar.trailing", "Inspector"),
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
        b.setAccessibilityIdentifier(Some(&NSString::from_str(&format!("shell.toolbar.{name}"))));
        b.setTag(TOOLBAR.iter().position(|n| *n == name).unwrap() as isize);
        if name == "toggleSidebar" || name == "toggleInspector" {
            // Apple's sidebar toggles: a plain toolbar item, never a filled tint.
            item.setImage(Some(&symbol(sym, label)));
            unsafe {
                item.setTarget(Some(target));
                item.setAction(Some(sel!(act:)));
            }
            item.setTag(TOOLBAR.iter().position(|n| *n == name).unwrap() as isize);
            item.setBordered(true);
            item.setLabel(&NSString::from_str(label));
            item.setToolTip(Some(&NSString::from_str(&format!("Hide {label}"))));
            return Some(item);
        }
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

/// Title alone is centred vertically in the toolbar item; with a subtitle
/// (`Edited`, `Saving…`) the two lines stack around the centre.
fn layout_title(title: &NSTextField, subtitle: &NSTextField, t: &Tokens) {
    let m = t.metrics;
    let (w, h, line) = (
        m.toolbar_title_width as f64,
        m.control as f64,
        m.toolbar_title_line as f64,
    );
    let has_subtitle = !subtitle.stringValue().to_string().is_empty();
    subtitle.setHidden(!has_subtitle);
    let title_h = title.fittingSize().height.max(1.);
    if has_subtitle {
        title.setFrame(NSRect::new(
            NSPoint::new(0., line),
            NSSize::new(w, h - line),
        ));
        subtitle.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(w, line)));
    } else {
        title.setFrame(NSRect::new(
            NSPoint::new(0., ((h - title_h) / 2.).floor()),
            NSSize::new(w, title_h),
        ));
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
}
pub struct Host {
    pub window: Retained<NSWindow>,
    pub gpui: Retained<NSView>,
    content: Retained<NSView>,
    viewer: Retained<NSView>,
    pub viewer_rect: Rc<Cell<Option<crate::layout::Rect>>>,
    pub splitter_focus: bool,
    pub keyboard_control_focus: Cell<bool>,
    pub canvas_focus: Cell<bool>,
    flowchart: Option<flowchart::Popover>,
    layer: metal::MetalLayer,
    viewer_transform: Option<([f64; 2], Option<f64>, [f64; 2])>,
    preview_scale: f64,
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
    menu_state: RefCell<Option<MenuState>>,
    /// The open Save Workspace sheet (kept alive while attached).
    sheet: RefCell<Option<Sheet>>,
    /// Title and subtitle of the toolbar's document item.
    title: Retained<NSTextField>,
    subtitle: Retained<NSTextField>,
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
        .chain(crate::layout::PRESETS.iter().map(|(id, name)| Choice {
            label: format!("Workspace: {name}"),
            command: format!("workspace:{id}"),
        }))
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
        window.setBackgroundColor(Some(&native_color(t.colors.window)));
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
        window.setToolbarStyle(NSWindowToolbarStyle::UnifiedCompact);
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
        let (title, subtitle) = (
            delegate.ivars().title.clone(),
            delegate.ivars().subtitle.clone(),
        );
        let host = Rc::new(RefCell::new(Self {
            window,
            gpui: gpui.clone(),
            content,
            viewer,
            viewer_rect: Rc::new(Cell::new(None)),
            splitter_focus: false,
            keyboard_control_focus: Cell::new(false),
            canvas_focus: Cell::new(false),
            flowchart: None,
            layer,
            viewer_transform: None,
            preview_scale: 1.,
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
            menu_state: RefCell::new(None),
            sheet: RefCell::new(None),
            title,
            subtitle,
        }));
        let weak = Rc::downgrade(&host);
        let hit_rect = host.borrow().viewer_rect.clone();
        let reg = gpui_macos::composition::register_tracked(
            &gpui,
            Rc::new(move || {
                weak.upgrade()
                    .map_or(gpui_macos::composition::SlotChange::Unchanged, |h| {
                        h.borrow_mut().present()
                    })
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
        self.title.setStringValue(&NSString::from_str(title));
        self.subtitle.setStringValue(&NSString::from_str(subtitle));
        layout_title(&self.title, &self.subtitle, &self.tokens);
    }
    /// Current toolbar title and subtitle (tests, accessibility checks).
    pub fn document_title(&self) -> (String, String) {
        (
            self.title.stringValue().to_string(),
            self.subtitle.stringValue().to_string(),
        )
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
        self.title.setTextColor(Some(&native_color(t.colors.text)));
        self.subtitle
            .setTextColor(Some(&native_color(t.colors.muted)));
        layout_title(&self.title, &self.subtitle, &t);
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
    pub fn add_bindings(&mut self, bindings: impl IntoIterator<Item = crate::keyboard::Binding>) {
        self.resolver.bindings.extend(bindings);
        self.resolver
            .bindings
            .sort_by_key(|b| std::cmp::Reverse(b.priority));
    }
    /// Viewer render resolution as a fraction of the layer's pixels. The layer
    /// keeps its size; AppKit scales the smaller drawable up. The engine follows
    /// the drawable through its OutputBinding, so a change costs one reconfigure.
    pub fn set_preview_scale(&mut self, scale: f64) {
        if self.preview_scale != scale {
            self.preview_scale = scale;
            self.last_geometry = None;
        }
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
    /// P-120: returns `Moved` only when the viewer slot's frame, layer frame,
    /// drawable size or visibility changed, so GPUI blocks on the composition
    /// transaction only on those frames.
    fn present(&mut self) -> gpui_macos::composition::SlotChange {
        let h = self.content.bounds().size.height;
        let geometry = self.viewer_rect.get().map(|r| {
            let scale = self.window.backingScaleFactor();
            let (width, height, pan) = if let Some((dimensions, zoom, pan)) = self.viewer_transform
            {
                let fit = (r.w as f64 / dimensions[0]).min(r.h as f64 / dimensions[1]);
                let zoom = zoom
                    .unwrap_or(fit)
                    .min(8192. / dimensions[0].max(dimensions[1]) / scale);
                (dimensions[0] * zoom, dimensions[1] * zoom, pan)
            } else {
                (r.w as f64, r.h as f64, [0., 0.])
            };
            SlotGeometry {
                frame: rect(r, h),
                layer: NSRect::new(
                    NSPoint::new(
                        (r.w as f64 - width) / 2. + pan[0],
                        (r.h as f64 - height) / 2. - pan[1],
                    ),
                    NSSize::new(width, height),
                ),
                scale,
            }
        });
        let moved = self.last_geometry != Some(geometry);
        if moved {
            if let Some(g) = geometry {
                self.viewer.setHidden(false);
                self.viewer.setFrame(g.frame);
                unsafe {
                    let _: () = msg_send![self.layer.as_ptr() as *mut AnyObject,setFrame:g.layer];
                }
                self.layer.set_contents_scale(g.scale);
                let (width, height) = preview_drawable(
                    (g.layer.size.width, g.layer.size.height),
                    g.scale,
                    self.preview_scale,
                );
                self.layer
                    .set_drawable_size(core_graphics::geometry::CGSize::new(width, height));
            } else {
                self.viewer.setHidden(true);
            }
            self.last_geometry = Some(geometry);
        }
        gpui_macos::composition::publish_accessibility(
            &self.gpui,
            vec![
                gpui_macos::composition::AccessibilityItem::RemainingGpui,
                gpui_macos::composition::AccessibilityItem::Native(self.viewer.clone()),
            ],
        );
        moved.into()
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
        if self.menu_state.borrow().as_ref() == Some(state) {
            return;
        }
        MENU_STATES.with(|m| {
            m.borrow_mut()
                .insert(self.window.windowNumber(), state.clone())
        });
        for item in self._toolbar.items().to_vec() {
            let id = item.itemIdentifier().to_string();
            let on = match id.as_str() {
                "toggleSidebar" => state.sidebar,
                "toggleInspector" => state.inspector,
                _ => continue,
            };
            let label = item.label().to_string();
            let tip = format!("{} {label}", if on { "Hide" } else { "Show" });
            item.setToolTip(Some(&NSString::from_str(&tip)));
        }
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
        for id in crate::layout::PANELS {
            let open = state.open.iter().any(|o| o == id);
            if open && id == "viewer" {
                continue;
            }
            choices.push(Choice {
                label: format!(
                    "{} {}",
                    if open { "Hide" } else { "Show" },
                    crate::layout::panel_title(id)
                ),
                command: format!("panel:toggle:{id}"),
            });
        }
        for name in &state.saved {
            choices.push(Choice {
                label: format!("Layout: {name}"),
                command: format!("layout:apply:{name}"),
            });
        }
        choices.push(Choice {
            label: "Save Layout…".into(),
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
        let preset = crate::layout::PRESETS
            .iter()
            .find(|p| p.0 == workspace)
            .map_or("Custom", |p| p.1);
        let sheet = workspace_sheet(&self.tokens, &format!("{preset} Copy"));
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
    body: Retained<NSView>,
    spring: Option<(std::time::Instant, crate::transient::Spring)>,
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
        // NSSearchField: the system magnifying-glass glyph and clear button.
        let search = NSSearchField::initWithFrame(
            NSSearchField::alloc(mtm),
            NSRect::new(
                NSPoint::new(
                    m.menu_inset as f64,
                    (height - m.menu_inset - m.control) as f64,
                ),
                NSSize::new((width - m.menu_inset * 2.) as f64, m.control as f64),
            ),
        );
        let search: Retained<NSTextField> = Retained::into_super(search);
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
            self.search.setFrameOrigin(NSPoint::new(
                m.menu_inset as f64,
                height - (m.menu_inset + m.control) as f64,
            ));
        }
        let size = frame.size;
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
            b.setFrame(r);
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
