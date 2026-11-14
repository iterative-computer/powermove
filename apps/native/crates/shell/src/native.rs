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
use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{CAGradientLayer, CATransaction};
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
    /// View ▸ 3D items with their (group, value), for checkmarks.
    static VIEW3D_ITEMS:RefCell<Vec<(String,String,Retained<NSMenuItem>)>>=const{RefCell::new(Vec::new())};
}
/// Checkmarks for View ▸ 3D (3d-studio.md §2: the same names and checkmarks
/// as the viewer bar): `on(group, value)` says whether a row is current.
pub fn check_view3d(on: &dyn Fn(&str, &str) -> bool) {
    VIEW3D_ITEMS.with(|items| {
        for (group, value, item) in &*items.borrow() {
            let state = if on(group, value) { 1 } else { 0 };
            if item.state() != state {
                item.setState(state);
            }
        }
    });
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
    /// Toggles that are on: their menu rows carry a checkmark.
    pub checked: std::collections::BTreeSet<String>,
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
/// A sidebar's material, `width` wide and `height` tall at the content
/// view's leading edge: liquid glass on macOS 26 and later (clear style, a
/// heavy tint of the window colour), the sidebar visual effect before, the
/// solid window colour under Reduce Transparency. It runs on under the
/// content sheet's leading corners, so its own rim is hidden and the two
/// meet in one step.
fn sidebar_material(
    mtm: MainThreadMarker,
    t: &Tokens,
    width: f32,
    height: f64,
) -> Retained<NSView> {
    let frame = NSRect::new(
        NSPoint::ZERO,
        NSSize::new((width + t.metrics.panel_radius + t.space.xs) as f64, height),
    );
    let view = material(mtm, frame, t, Surface::Sidebar);
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
    view
}
/// Gives a sheet a new content height. The first time, its top edge is put
/// where a sheet `tallest` high stands centred in its parent (the most it
/// will grow to, so it never runs off its parent); from then on the top stays
/// there, so a sheet whose content changes height grows and shrinks downwards
/// and is never centred again on the way (AppKit centres a sheet on every
/// new frame; the window class hangs it from the pinned top instead). The
/// frame is set on a later turn of the run loop, because a view asks for
/// this while its window is being drawn and resizing calls back into it.
pub fn resize_from_top(w: &gpui::Window, height: f32, tallest: f32, cx: &mut gpui::App) {
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(w).unwrap().as_raw()
    else {
        return;
    };
    let view: Retained<NSView> =
        unsafe { Retained::retain(handle.ns_view.as_ptr().cast()).unwrap() };
    let Some(window) = view.window() else {
        return;
    };
    cx.spawn(async move |_| {
        let frame = window.frame();
        let grow = f64::from(height) - window.contentRectForFrameRect(frame).size.height;
        if grow.abs() < 0.25 {
            return;
        }
        let tall = frame.size.height + grow;
        if window.respondsToSelector(sel!(pmPinSheetTop:)) {
            let most = tall.max(tall + f64::from(tallest - height));
            let _: () = unsafe { msg_send![&*window, pmPinSheetTop: most] };
        }
        window.setFrame_display(
            NSRect::new(
                NSPoint::new(frame.origin.x, frame.origin.y - grow),
                NSSize::new(frame.size.width, tall),
            ),
            true,
        );
    })
    .detach();
}
/// A window laid out as Home is: a sidebar on the window's glass and a
/// content sheet beside it (Settings). The GPUI window is transparent over
/// the behind-window blur; the sidebar's material stands in the leading
/// `width`, which the view leaves unpainted.
pub fn sidebar_frame(w: &gpui::Window, t: &Tokens, width: f32) {
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
    appearance(&window, t);
    let blur = backdrop(mtm, &content);
    ground(&window, &blur, t);
    let side = sidebar_material(mtm, t, width, content.bounds().size.height);
    content.addSubview_positioned_relativeTo(&side, NSWindowOrderingMode::Below, Some(&gpui));
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
                show_in_tab_group(&window);
            }
            // This window's own tab, pressed while Home shows over the
            // project: back to the project (v1's title-bar tab).
            let own = unsafe {
                let in_window: bool = msg_send![sender, respondsToSelector: sel!(window)];
                let window: *mut NSWindow = if in_window {
                    msg_send![sender, window]
                } else {
                    std::ptr::null_mut()
                };
                window.as_ref().map(|w| w.windowNumber())
            };
            if own == Some(number) {
                self.send(Command { name: "leaveHome".into(), args: vec![], token: None });
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
                    n => item.setState(if state.checked.contains(n) {
                        NSControlStateValueOn } else { NSControlStateValueOff }),
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
    hide_system_tab_bar(&lead);
    hide_system_tab_bar(window);
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
/// A tool segment's width, and the room AppKit's menu indicator takes in one.
const TOOL_SEGMENT: f64 = 34.;
const TOOL_MENU_INDICATOR: f64 = 12.;
const TOOLBAR: [&str; 8] = [
    "home",
    "title",
    "tools",
    "add",
    "agent",
    "export",
    "panelLibrary",
    "settings",
];
/// The toolbar's Add menu, as v1's "+" beside the tools: media, then the
/// layers that are made by a command, not drawn with a tool. An empty row
/// is a separator.
pub const ADD: [(&str, &str); 5] = [
    ("import", "Import Media…"),
    ("", ""),
    ("newSolid", "New Solid"),
    ("newAdjustment", "New Adjustment Layer"),
    ("newNull", "New Null Object"),
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
    /// A project tab's close button, at the trailing end under the pointer.
    /// Home and the strip's own buttons have none.
    close: Option<Retained<NSButton>>,
    /// The title (a project) or the glyph button (Home, the strip's own
    /// buttons): its ink follows the cell's state.
    label: Retained<NSView>,
    /// The title's mask: its end fades out under the close button.
    fade: Option<Retained<CAGradientLayer>>,
    edited: bool,
    selected: Cell<bool>,
    hovered: Cell<bool>,
    /// Carried along the strip by the pointer.
    lifted: Cell<bool>,
}
define_class!(
    /// One cell of the toolbar's project strip: a project tab (its title,
    /// with the close button or the edited dot in the leading slot) or Home.
    /// The cell draws its own state: the selected one is a raised neutral
    /// pill, concentric with the toolbar's glass (`TAB_INSET` inside it on
    /// every side); the one under the pointer is a wash of the same shape.
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
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _: NSRect) {
            self.draw();
        }
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            self.restyle();
        }
        #[unsafe(method(viewDidMoveToWindow))]
        fn moved_to_window(&self) {
            self.restyle();
        }
    }
);
impl TabView {
    fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        label: Retained<NSView>,
        close: Option<Retained<NSButton>>,
        fade: Option<Retained<CAGradientLayer>>,
        edited: bool,
        selected: bool,
    ) -> Retained<Self> {
        unsafe {
            msg_send![
                super(Self::alloc(mtm).set_ivars(TabIvars {
                    close,
                    label,
                    fade,
                    edited,
                    selected: Cell::new(selected),
                    hovered: Cell::new(false),
                    lifted: Cell::new(false),
                })),
                initWithFrame: frame
            ]
        }
    }
    fn hover(&self, hovered: bool) {
        self.ivars().hovered.set(hovered);
        self.reveal(hovered, true);
        self.restyle();
    }
    /// The close button slides in from the trailing end while the title's
    /// end fades out under it, so the title itself never moves. Reduce
    /// Motion (and a tab just built) places both at once.
    fn reveal(&self, shown: bool, glide: bool) {
        let TabIvars {
            close, fade, label, ..
        } = self.ivars();
        let Some(close) = close else {
            return;
        };
        let glide =
            glide && !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        let seconds = if glide { 0.16 } else { 0. };
        let rest = self.bounds().size.width - TAB_CLOSE_PAD - TAB_CLOSE;
        let to = NSPoint::new(
            if shown { rest } else { rest + TAB_CLOSE_SLIDE },
            close.frame().origin.y,
        );
        NSAnimationContext::beginGrouping();
        NSAnimationContext::currentContext().setDuration(seconds);
        unsafe {
            let animator: Retained<NSButton> = msg_send![&**close, animator];
            animator.setFrameOrigin(to);
            animator.setAlphaValue(if shown { 1. } else { 0. });
        }
        NSAnimationContext::endGrouping();
        if let Some(fade) = fade {
            let frame = label.frame();
            let at = |x: f64| ((x - frame.origin.x) / frame.size.width.max(1.)).clamp(0., 1.);
            let (from, to) = if shown {
                (at(rest - TAB_FADE), at(rest))
            } else {
                (1., 1.)
            };
            let stops = [0., from, to, 1.].map(NSNumber::new_f64);
            CATransaction::begin();
            CATransaction::setAnimationDuration(seconds);
            fade.setLocations(Some(&NSArray::from_retained_slice(&stops)));
            CATransaction::commit();
        }
    }
    fn select(&self, selected: bool) {
        if self.ivars().selected.replace(selected) != selected {
            self.restyle();
        }
    }
    /// The ink of the title or glyph for the cell's state, and a repaint.
    /// The showing project's title is the primary ink; every other title
    /// and glyph is muted, and a step quieter in a background window.
    fn restyle(&self) {
        let TabIvars {
            label,
            close,
            selected,
            ..
        } = self.ivars();
        let active = self.window().is_none_or(|w| w.isMainWindow());
        let ink = match (selected.get(), active) {
            (true, true) => NSColor::labelColor(),
            (true, false) | (false, true) => NSColor::secondaryLabelColor(),
            (false, false) => NSColor::tertiaryLabelColor(),
        };
        if let Some(text) = label.downcast_ref::<NSTextField>() {
            text.setTextColor(Some(&ink));
        } else if let Some(button) = label.downcast_ref::<NSButton>() {
            button.setContentTintColor(Some(&ink));
        }
        if let Some(close) = close {
            close.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
        }
        self.setNeedsDisplay(true);
    }
    /// One soft neutral fill, straight on the toolbar: the showing project
    /// (or Home), the cell under the pointer, and a step deeper for a tab
    /// carried along the strip. No edge, no shadow, never the accent. An
    /// edited project carries a dot before its title.
    fn draw(&self) {
        let TabIvars {
            selected,
            hovered,
            lifted,
            edited,
            ..
        } = self.ivars();
        let dark = dark_appearance(self);
        let active = self.window().is_none_or(|w| w.isMainWindow());
        let bounds = self.bounds();
        if selected.get() || hovered.get() || lifted.get() {
            let shape = || {
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    bounds, TAB_RADIUS, TAB_RADIUS,
                )
            };
            if lifted.get() {
                // Carried over its neighbours: their titles do not show
                // through it.
                NSColor::windowBackgroundColor().setFill();
                shape().fill();
            }
            let ink = if dark { 1. } else { 0. };
            let alpha = match (lifted.get(), active) {
                (true, _) => TAB_FILL * 1.75,
                (false, true) => TAB_FILL,
                (false, false) => TAB_FILL * 0.6,
            };
            NSColor::colorWithSRGBRed_green_blue_alpha(ink, ink, ink, alpha).setFill();
            shape().fill();
        }
        if *edited {
            if active {
                NSColor::secondaryLabelColor()
            } else {
                NSColor::tertiaryLabelColor()
            }
            .setFill();
            NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
                NSPoint::new(TAB_PAD, ((bounds.size.height - TAB_DOT) / 2.).round()),
                NSSize::new(TAB_DOT, TAB_DOT),
            ))
            .fill();
        }
    }
}
/// A colour as a stop of a gradient mask (a `CGColor` in an array).
fn mask_stop(color: &NSColor) -> Retained<AnyObject> {
    let color = color.CGColor();
    unsafe { Retained::retain((&*color as *const _ as *mut AnyObject).cast()).unwrap() }
}
/// How much of a popup's list fades out at its bottom edge while more rows
/// lie below.
const POPUP_FADE: f64 = 20.;
/// Whether `view` draws in a dark appearance.
fn dark_appearance(view: &NSView) -> bool {
    let list = unsafe { [NSAppearanceNameAqua, NSAppearanceNameDarkAqua] };
    view.effectiveAppearance()
        .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&list))
        .is_some_and(|n| n.to_string().contains("Dark"))
}
define_class!(
    /// The project strip, straight on the toolbar with no glass of its own:
    /// Home, the tabs in a row, then the pull-down that lists every project
    /// (once they no longer fit) and New Project. Each is a quiet cell.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMShellTabStrip"]
    struct TabStrip;
    impl TabStrip {}
);
/// The project tabs of `strip`, in the order they stand.
fn strip_tabs(strip: &NSView) -> Vec<Retained<TabView>> {
    let mut tabs: Vec<Retained<TabView>> = strip
        .subviews()
        .to_vec()
        .into_iter()
        .filter_map(|v| v.downcast::<TabView>().ok())
        .collect();
    tabs.sort_by(|a, b| a.frame().origin.x.total_cmp(&b.frame().origin.x));
    tabs
}
define_class!(
    /// A project tab's button: the whole cell, drawing nothing itself (the
    /// cell draws the pill, a label the title). It tracks the pointer itself,
    /// so a tab can be dragged along the strip to reorder it: a button's own
    /// tracking swallows the drag.
    #[unsafe(super(NSButton))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMShellTabButton"]
    struct TabButton;
    impl TabButton {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            track_tab(self, event);
        }
    }
);
/// The pointer on a tab until it lifts. Released in place it selects the tab.
/// Past `TAB_DRAG_SLOP` it carries the tab along the strip: the others slide
/// to make room where it would land, and the drop moves its window to that
/// place in the tab group.
fn track_tab(button: &TabButton, down: &NSEvent) {
    let (Some(window), Some(tab)) = (button.window(), unsafe { button.superview() }) else {
        return;
    };
    let Some(strip) = (unsafe { tab.superview() }) else {
        return;
    };
    let tabs: Vec<Retained<NSView>> = strip
        .subviews()
        .to_vec()
        .into_iter()
        .filter(|v| v.isKindOfClass(TabView::class()))
        .collect();
    let Some(index) = tabs.iter().position(|v| **v == *tab) else {
        return;
    };
    let frames: Vec<NSRect> = tabs.iter().map(|v| v.frame()).collect();
    let (home, width) = (frames[index].origin, frames[index].size.width);
    let (first, last) = (frames[0].origin.x, {
        let f = frames[frames.len() - 1];
        f.origin.x + f.size.width - width
    });
    let start = down.locationInWindow().x;
    let glide = !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
    // The others in their order with a gap where the dragged tab would land.
    let settle = |target: usize, dragged: Option<f64>| {
        NSAnimationContext::beginGrouping();
        NSAnimationContext::currentContext().setDuration(if glide { 0.2 } else { 0. });
        let mut x = first;
        let mut slot = 0;
        for (i, view) in tabs.iter().enumerate() {
            if i == index {
                continue;
            }
            if slot == target {
                x += width + TAB_GAP;
            }
            let to = NSPoint::new(x, home.y);
            unsafe {
                let animator: Retained<NSView> = msg_send![&**view, animator];
                animator.setFrameOrigin(to);
            }
            x += frames[i].size.width + TAB_GAP;
            slot += 1;
        }
        // On the drop the dragged tab glides into the gap left for it.
        if dragged.is_none() {
            let mut x = first;
            let mut slot = 0;
            for (i, frame) in frames.iter().enumerate() {
                if i == index {
                    continue;
                }
                if slot == target {
                    break;
                }
                x += frame.size.width + TAB_GAP;
                slot += 1;
            }
            unsafe {
                let animator: Retained<NSView> = msg_send![&*tab, animator];
                animator.setFrameOrigin(NSPoint::new(x, home.y));
            }
        }
        NSAnimationContext::endGrouping();
    };
    let carry = |on: bool| {
        if let Some(cell) = tab.downcast_ref::<TabView>() {
            cell.ivars().lifted.set(on);
            cell.setNeedsDisplay(true);
        }
    };
    button.highlight(true);
    let mut target = index;
    let mut dragging = false;
    let inside = loop {
        let Some(event) =
            window.nextEventMatchingMask(NSEventMask::LeftMouseDragged | NSEventMask::LeftMouseUp)
        else {
            break false;
        };
        let at = event.locationInWindow();
        if event.r#type() == NSEventType::LeftMouseUp {
            let local = button.convertPoint_fromView(at, None);
            break button.mouse_inRect(local, button.bounds());
        }
        let by = at.x - start;
        if !dragging {
            if tabs.len() < 2 || by.abs() < TAB_DRAG_SLOP {
                continue;
            }
            dragging = true;
            button.highlight(false);
            // Above its neighbours while it travels, raised as the selected
            // tab is, with no separators in the strip.
            carry(true);
            tab.setWantsLayer(true);
            if let Some(layer) = tab.layer() {
                layer.setZPosition(1.);
            }
        }
        let x = (home.x + by).clamp(first, last.max(first));
        tab.setFrameOrigin(NSPoint::new(x, home.y));
        // Tabs are as wide as their titles: it changes places with a
        // neighbour once the edge it leads with passes that tab's middle.
        let landing = frames
            .iter()
            .enumerate()
            .filter(|(i, f)| {
                let middle = f.origin.x + f.size.width / 2.;
                (*i < index && middle <= x) || (*i > index && middle < x + width)
            })
            .count();
        if landing != target {
            target = landing;
            settle(target, Some(x));
        }
    };
    button.highlight(false);
    if !dragging {
        if inside {
            unsafe {
                let _: bool =
                    msg_send![button, sendAction: button.action(), to: button.target().as_deref()];
            }
        }
        return;
    }
    settle(target, None);
    carry(false);
    if let Some(layer) = tab.layer() {
        layer.setZPosition(0.);
    }
    if target != index {
        let app = NSApplication::sharedApplication(window.mtm());
        if let Some(moved) = app.windowWithWindowNumber(button.tag()) {
            move_tab(&moved, target as isize - index as isize);
        }
    }
}
/// Development builds: drags the tab of window `number` one place along
/// `strip`, as a pointer would: a press handed to the tab's own tracking,
/// then drags and a release posted to the app's event queue, where that
/// tracking reads them. Nothing touches the real pointer or the window's
/// order.
#[cfg(debug_assertions)]
fn review_drag(strip: &NSView, number: isize, right: bool, hold: f64, press: bool) {
    use objc2_foundation::{NSProcessInfo, NSRunLoop, NSRunLoopCommonModes, NSTimer};
    let tabs: Vec<Retained<NSView>> = strip
        .subviews()
        .to_vec()
        .into_iter()
        .filter(|v| v.isKindOfClass(TabView::class()))
        .collect();
    let button_of = |tab: &NSView| {
        tab.subviews()
            .to_vec()
            .into_iter()
            .find_map(|v| v.downcast::<TabButton>().ok())
    };
    let Some((index, button)) = tabs
        .iter()
        .enumerate()
        .find_map(|(i, tab)| button_of(tab).filter(|b| b.tag() == number).map(|b| (i, b)))
    else {
        return;
    };
    let neighbour = if right {
        tabs.get(index + 1)
    } else {
        index.checked_sub(1).and_then(|i| tabs.get(i))
    };
    // A press alone (no travel) needs no neighbour: it selects the tab.
    let (Some(neighbour), Some(window)) = (neighbour.or(tabs.get(index)), strip.window()) else {
        return;
    };
    let frame = tabs[index].frame();
    let from = strip.convertPoint_toView(
        NSPoint::new(
            frame.origin.x + frame.size.width / 2.,
            frame.size.height / 2.,
        ),
        None,
    );
    // Just past the neighbour's middle, where the two change places; a held
    // drag goes further, so the capture shows the tab lifted over the next.
    let past = if hold > 1. { 30. } else { 6. };
    let travel = (frame.size.width + neighbour.frame().size.width) / 2. + TAB_GAP + past;
    let travel = match (press, right) {
        (true, _) => 0.,
        (false, true) => travel,
        (false, false) => -travel,
    };
    let number = window.windowNumber();
    let event = move |kind: NSEventType, x: f64| {
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            NSPoint::new(x, from.y),
            NSEventModifierFlags::empty(),
            NSProcessInfo::processInfo().systemUptime(),
            number,
            None,
            0,
            1,
            1.,
        )
        .expect("a pointer event")
    };
    // Timers in the common modes fire inside the tab's tracking loop too.
    let after = |delay: f64, run: Box<dyn Fn()>| {
        let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSTimer>| run());
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(delay, false, &block) };
        unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
    };
    let app = NSApplication::sharedApplication(window.mtm());
    const STEPS: usize = 8;
    for step in 1..=if press { 0 } else { STEPS } {
        let (app, drag) = (
            app.clone(),
            event(
                NSEventType::LeftMouseDragged,
                from.x + travel * step as f64 / STEPS as f64,
            ),
        );
        after(
            0.2 + 0.06 * step as f64,
            Box::new(move || app.postEvent_atStart(&drag, false)),
        );
    }
    let up = event(NSEventType::LeftMouseUp, from.x + travel);
    after(
        0.2 + 0.06 * STEPS as f64 + hold,
        Box::new(move || app.postEvent_atStart(&up, false)),
    );
    let down = event(NSEventType::LeftMouseDown, from.x);
    after(0.05, Box::new(move || button.mouseDown(&down)));
}
/// The widest any tab may be for `natural` widths, each held to it, to fit
/// `room` with the gaps between them: `TAB_MAX` while everything fits, else
/// the long titles narrow together and the short ones keep their width.
fn tab_cap(natural: &[f64], room: f64) -> f64 {
    let mut sorted: Vec<f64> = natural.iter().map(|n| n.min(TAB_MAX)).collect();
    sorted.sort_by(f64::total_cmp);
    let mut left = room - sorted.len().saturating_sub(1) as f64 * TAB_GAP;
    for (i, n) in sorted.iter().enumerate() {
        let rest = (sorted.len() - i) as f64;
        if n * rest > left {
            return (left / rest).floor();
        }
        left -= n;
    }
    TAB_MAX
}
/// The tabs the strip shows: where the run starts in the group, the width of
/// each tab in it, and whether the rest go to the pull-down. A tab is as
/// wide as its title asks, up to `TAB_MAX`; short of room the long ones
/// narrow together down to `TAB_MIN`; past that the strip shows the run that
/// holds the selected tab, widened to fill what the pull-down leaves.
fn tab_layout(natural: &[f64], selected: usize, room: f64) -> (usize, Vec<f64>, bool) {
    let count = natural.len();
    if count == 0 {
        return (0, vec![], false);
    }
    let selected = selected.min(count - 1);
    let held = |cap: f64, run: &[f64]| run.iter().map(|n| n.min(cap)).collect::<Vec<_>>();
    let cap = tab_cap(natural, room);
    if cap >= TAB_MIN || count == 1 {
        return (0, held(cap.max(TAB_MIN), natural), false);
    }
    let room = room - TAB_MORE;
    let least = held(TAB_MIN, natural);
    let fits = |from: usize, to: usize| {
        least[from..=to].iter().sum::<f64>() + (to - from) as f64 * TAB_GAP <= room
    };
    // The run slides only as far as it must to hold the selected tab.
    let mut start = 0;
    while start < selected && !fits(start, selected) {
        start += 1;
    }
    let mut end = selected;
    while end + 1 < count && fits(start, end + 1) {
        end += 1;
    }
    let run = &natural[start..=end];
    (start, held(tab_cap(run, room).max(TAB_MIN), run), true)
}
/// Hides AppKit's own tab bar in `window`'s title bar. The project tabs are
/// drawn in the toolbar from the same tab group; the system's row under the
/// toolbar would show them a second time and push the editor down. The bar
/// is a title bar accessory AppKit adds once a second window joins, and may
/// add again when the group changes, so this runs whenever the tabs are
/// synced. Hiding the accessory's controller also gives its height back.
/// Posted to a window brought forward in its tab group by one of our tabs.
const TAB_SHOWN: &str = "PMShellTabShown";
/// Brings `window` forward in its tab group and tells it so. A window shown
/// this way while the app is inactive gets no activation, so nothing else
/// makes it hide AppKit's tab row or draw its own strip for the new state.
fn show_in_tab_group(window: &NSWindow) {
    unsafe {
        let group: *mut AnyObject = msg_send![window, tabGroup];
        if group.is_null() {
            return;
        }
        let _: () = msg_send![group, setSelectedWindow: window];
        objc2_foundation::NSNotificationCenter::defaultCenter()
            .postNotificationName_object(&NSString::from_str(TAB_SHOWN), Some(window));
    }
}
fn hide_system_tab_bar(window: &NSWindow) {
    fn tab_bar(view: &NSView) -> Option<Retained<NSView>> {
        if view.class().name().to_bytes() == b"NSTabBar" {
            return Some(unsafe { Retained::retain(view as *const _ as *mut NSView).unwrap() });
        }
        view.subviews().to_vec().iter().find_map(|v| tab_bar(v))
    }
    let Some(frame) = window.contentView().and_then(|c| unsafe { c.superview() }) else {
        return;
    };
    let Some(bar) = tab_bar(&frame) else {
        return;
    };
    // The accessory's controller is the next responder of its view.
    let mut view = Some(bar);
    while let Some(v) = view {
        if let Some(controller) = unsafe { v.nextResponder() }
            .and_then(|r| r.downcast::<NSTitlebarAccessoryViewController>().ok())
        {
            if !controller.isHidden() {
                controller.setHidden(true);
            }
            return;
        }
        view = unsafe { v.superview() };
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
    hide_system_tab_bar(window);
}
/// Widest a project tab grows; its title truncates in the middle.
const TAB_MAX: f64 = 180.;
/// Narrowest a long title's tab gets: enough of a name to tell projects apart.
const TAB_MIN: f64 = 96.;
/// Between the strip's cells.
const TAB_GAP: f64 = 4.;
/// A cell's corner, and the alpha of the one neutral ink that fills the
/// showing project's cell and the cell under the pointer.
const TAB_RADIUS: f64 = 8.;
const TAB_FILL: f64 = 0.08;
/// A title stands this far inside each end of its tab.
const TAB_PAD: f64 = 14.;
/// An edited project's dot before its title, and the space after the dot.
const TAB_DOT: f64 = 6.;
const TAB_DOT_GAP: f64 = 6.;
/// The close button: its side, how far it stands from the tab's trailing
/// end, how far it slides in, and how much of the title fades out before it.
const TAB_CLOSE: f64 = 16.;
const TAB_CLOSE_PAD: f64 = 8.;
const TAB_CLOSE_SLIDE: f64 = 6.;
const TAB_FADE: f64 = 14.;
/// The pull-down that lists every project once they no longer fit: a cell
/// of the control height and the gap after it.
const TAB_MORE: f64 = 32.;
/// Where the strip starts before the toolbar has laid it out (after the
/// window buttons), the tools' width before they exist, and the clear space
/// kept between the strip and the tools.
const TAB_LEADING: f64 = 92.;
const TAB_TOOLS: f64 = 300.;
const TAB_CLEAR: f64 = 16.;
/// How far the pointer travels on a tab before it is a drag, not a click.
const TAB_DRAG_SLOP: f64 = 4.;
struct ToolbarState {
    target: Retained<Target>,
    tokens: Tokens,
    /// The project strip (Home, the window's tab group, New Project), in
    /// the leading area.
    tabs: Retained<NSView>,
    tabs_width: Retained<NSLayoutConstraint>,
    /// Home's cell, the strip's first.
    home: Retained<TabView>,
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
    // Home and the tabs lead; the tools and Add stand in the middle between
    // two flexible spaces; Export, a space, then Panels and Settings trail.
    // Home is the strip's first cell, not an item of its own.
    v.remove(0);
    v.insert(1, NSString::from_str(&f.to_string()));
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
                    let tabs: Retained<TabStrip> =
                        msg_send![super(TabStrip::alloc(mtm).set_ivars(())), initWithFrame: NSRect::ZERO];
                    let tabs: Retained<NSView> = Retained::into_super(tabs);
                    let tabs_width = tabs.widthAnchor().constraintEqualToConstant(TAB_MIN);
                    tabs_width.setActive(true);
                    tabs.heightAnchor()
                        .constraintEqualToConstant(tokens.metrics.control as f64)
                        .setActive(true);
                    // Home is the strip's first cell: a glyph button in
                    // a cell that draws the same fill as a tab.
                    let home = NSButton::buttonWithImage_target_action(
                        &toolbar_symbol("house", "Projects", &tokens),
                        Some(&target),
                        Some(sel!(act:)),
                        mtm,
                    );
                    home.setBordered(false);
                    home.setButtonType(NSButtonType::MomentaryChange);
                    home.setTag(TOOLBAR.iter().position(|n| *n == "home").unwrap() as isize);
                    home.setToolTip(Some(&NSString::from_str("Projects")));
                    home.setAccessibilityLabel(Some(&NSString::from_str("Projects")));
                    let height = tokens.metrics.control as f64;
                    let square = NSRect::new(NSPoint::ZERO, NSSize::new(height, height));
                    home.setFrame(square);
                    let home_cell = TabView::new(
                        mtm,
                        square,
                        Retained::into_super(Retained::into_super(home.clone())),
                        None,
                        None,
                        false,
                        false,
                    );
                    home_cell.addSubview(&home);
                    home_cell.restyle();
                    ToolbarState {
                        target,
                        tabs,
                        tabs_width,
                        home: home_cell,
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
                // The Shape segment also holds a menu indicator: it is wider
                // by the indicator, so its glyph is drawn at the size of its
                // neighbours instead of being scaled down to make room.
                let width = if i as isize == SHAPE_SEGMENT {
                    TOOL_SEGMENT + TOOL_MENU_INDICATOR
                } else {
                    TOOL_SEGMENT
                };
                control.setWidth_forSegment(width, i as isize);
                control.setImageScaling_forSegment(NSImageScaling::ScaleNone, i as isize);
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
        if name == "add" {
            let item = objc2_app_kit::NSMenuToolbarItem::initWithItemIdentifier(
                objc2_app_kit::NSMenuToolbarItem::alloc(mtm),
                id,
            );
            let menu = NSMenu::new(mtm);
            // Rows follow the document state (`enable_tools`), not a responder.
            menu.setAutoenablesItems(false);
            for (command, label) in ADD {
                if command.is_empty() {
                    menu.addItem(&NSMenuItem::separatorItem(mtm));
                    continue;
                }
                let row = named_item(mtm, label, command);
                unsafe { row.setTarget(Some(target)) };
                menu.addItem(&row);
            }
            item.setMenu(&menu);
            item.setShowsIndicator(true);
            item.setImage(Some(&toolbar_symbol("plus", "Add", t)));
            item.setBordered(true);
            item.setLabel(&NSString::from_str("Add"));
            item.setToolTip(Some(&NSString::from_str("Add Layer or Media")));
            return Some(Retained::into_super(item));
        }
        if name == "title" {
            // Home and the open projects, as v1 had them in its title bar:
            // quiet cells straight on the bar, so the item is unbordered
            // (the system gives it no glass).
            item.setView(Some(&self.ivars().tabs));
            item.setBordered(false);
            item.setLabel(&NSString::from_str("Projects"));
            return Some(item);
        }
        // Every other item is a plain bordered toolbar item (an image, or an
        // image and title for Export): the system draws its glass, spaces
        // it and sizes it. Export is the bar's one prominent item, tinted
        // with the accent.
        let (sym, label) = match name.as_str() {
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
    /// Studio 3D views: 1, 2 or 4 panes. With more than one, `viewer_rect`
    /// is the first pane and `viewer_tiles` holds every pane (the host
    /// places the others' views), in content coordinates, top-left origin.
    pub viewer_split: Cell<u8>,
    pub viewer_tiles: Rc<std::cell::RefCell<Vec<crate::layout::Rect>>>,
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
    /// Extension rows, kept apart so a new keymap replaces only the base.
    contributed: Vec<crate::keyboard::Binding>,
    /// The next key pressed goes to Settings › Shortcuts, not to a command.
    recording: bool,
    events: HashMap<u64, Retained<NSEvent>>,
    /// Commands owed when a held key comes up, by key code ([`Host::on_release`]).
    releases: Rc<RefCell<HashMap<u16, Command>>>,
    /// Pending keys that came up before their command was answered.
    up_early: std::collections::HashSet<u64>,
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
    /// The tabs last drawn and the room they were laid out in.
    tabs_shown: RefCell<(Vec<Tab>, f64)>,
    /// The window's main-state observations (the strip's quieter look).
    window_observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    /// Home shows over the docks (`set_home`).
    home: Cell<bool>,
    /// Home's sidebar material, under the GPUI view while Home shows.
    sidebar: RefCell<Option<Retained<NSView>>>,
}
/// Static palette rows: every menu command (including display-only menus,
/// tools, rows with arguments and the commands that have no menu row here)
/// plus workspace presets. Dynamic rows are added when it opens.
fn base_choices() -> Vec<Choice> {
    menu::palette_rows()
        .into_iter()
        // A row is listed under the menu it lives in.
        .map(|(title, command, label)| Choice::new(label, command).within(title))
        .collect()
}
/// Palette command → shortcut glyphs, preferring menu accelerators, then the
/// first non-Control W1 chord (the Control aliases are the same command).
fn shortcut_glyphs() -> HashMap<String, String> {
    let mut map = HashMap::new();
    for i in menu::menus()
        .into_iter()
        .flat_map(|(_, i)| i)
        .chain(menu::tools())
        .chain(menu::extras().into_iter().map(|(_, i)| i))
    {
        if !i.chord.is_empty() {
            map.entry(menu::palette_command(&i.command, &i.args))
                .or_insert_with(|| menu::glyphs(&i.chord));
        }
    }
    for b in crate::keymap::current() {
        if !b.chord.starts_with("ctrl+") {
            map.entry(menu::palette_command(&b.command, &b.args))
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
            l.setBackgroundColor(Some(&native_color(t.viewer_surround()).CGColor()));
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
                .map(|n| Choice::new(*n, format!("field:{n}")))
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
            viewer_split: Cell::new(1),
            viewer_tiles: Rc::default(),
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
            resolver: Resolver::with(crate::keymap::current()),
            contributed: vec![],
            recording: false,
            events: HashMap::new(),
            releases: Default::default(),
            up_early: Default::default(),
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
            window_observers: Vec::new(),
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
        let weak = Rc::downgrade(&host);
        gpui_macos::composition::set_pending(
            &gpui,
            // A host that is being changed (borrowed) is asked again by the
            // frame that presents it.
            Rc::new(move || {
                weak.upgrade()
                    .is_some_and(|h| h.try_borrow().is_ok_and(|h| h.pending()))
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
                    if e.r#type() == NSEventType::KeyUp {
                        let mut h = host.borrow_mut();
                        let owed = h.releases.borrow_mut().remove(&e.keyCode());
                        if let Some(command) = owed {
                            let _ = h.sender.try_send(command);
                        }
                        // A tap can be over before its command is answered.
                        let pending: Vec<u64> = h
                            .events
                            .iter()
                            .filter(|(_, down)| down.keyCode() == e.keyCode())
                            .map(|(token, _)| *token)
                            .collect();
                        h.up_early.extend(pending);
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
                                    p.key(&key, e.isARepeat());
                                    return std::ptr::null_mut();
                                }
                            }
                            return event.as_ptr();
                        }
                    }
                    // Settings › Shortcuts is taking a key: it comes from
                    // whichever of the app's windows is key (Settings is its own).
                    if e.r#type() == NSEventType::KeyDown && host.borrow().recording {
                        let key = chord(e);
                        let mut h = host.borrow_mut();
                        h.recording = false;
                        h.send_command("shortcuts.recorded", vec![serde_json::json!(key)]);
                        return std::ptr::null_mut();
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
        // The strip's ink and pill are quieter in a background window.
        let restyle = {
            let (strip, home) = {
                let host = host.borrow();
                (host.tabs.clone(), host._delegate.ivars().home.clone())
            };
            block2::RcBlock::new(
                move |_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
                    home.restyle();
                    for tab in strip_tabs(&strip) {
                        tab.restyle();
                    }
                },
            )
        };
        // Brought forward by a tab while the app is inactive: no activation
        // follows, so the window is told to redraw its strip itself.
        let shown = {
            let (window, sender) = {
                let host = host.borrow();
                (host.window.clone(), host.sender.clone())
            };
            block2::RcBlock::new(
                move |_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
                    hide_system_tab_bar(&window);
                    let _ = sender.try_send(Command {
                        name: "shellTabShown".into(),
                        args: vec![],
                        token: None,
                    });
                },
            )
        };
        let observer = unsafe {
            objc2_foundation::NSNotificationCenter::defaultCenter()
                .addObserverForName_object_queue_usingBlock(
                    Some(&NSString::from_str(TAB_SHOWN)),
                    Some(&host.borrow().window),
                    None,
                    &shown,
                )
        };
        host.borrow_mut().window_observers.push(observer);
        // A key held while the window stops being key never comes up here.
        let released = {
            let (releases, sender) = {
                let host = host.borrow();
                (host.releases.clone(), host.sender.clone())
            };
            block2::RcBlock::new(
                move |_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
                    for (_, command) in releases.borrow_mut().drain() {
                        let _ = sender.try_send(command);
                    }
                },
            )
        };
        let observer = unsafe {
            objc2_foundation::NSNotificationCenter::defaultCenter()
                .addObserverForName_object_queue_usingBlock(
                    Some(NSWindowDidResignKeyNotification),
                    Some(&host.borrow().window),
                    None,
                    &released,
                )
        };
        host.borrow_mut().window_observers.push(observer);
        for name in unsafe {
            [
                NSWindowDidBecomeMainNotification,
                NSWindowDidResignMainNotification,
            ]
        } {
            let observer = unsafe {
                objc2_foundation::NSNotificationCenter::defaultCenter()
                    .addObserverForName_object_queue_usingBlock(
                        Some(name),
                        Some(&host.borrow().window),
                        None,
                        &restyle,
                    )
            };
            host.borrow_mut().window_observers.push(observer);
        }
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
        if group.len() > 1 {
            hide_system_tab_bar(&self.window);
        }
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
                // On Home the lit item is Home: no project tab is selected.
                selected: !self.home.get() && (group.len() == 1 || w.windowNumber() == selected),
            })
            .collect();
        let (mtm, t, target) = (self.window.mtm(), &self.tokens, &self._target);
        let height = t.metrics.control as f64;
        // The tools stay in the middle of the bar however many projects are
        // open: the tabs have what lies between the strip's leading edge and
        // the tools, less Home and New Project.
        let leading = self
            .tabs
            .convertRect_toView(self.tabs.bounds(), None)
            .origin
            .x;
        let tools = self
            ._toolbar
            .items()
            .to_vec()
            .iter()
            .find(|item| item.itemIdentifier().to_string() == "tools")
            .and_then(|item| item.view())
            .map_or(0., |view| view.frame().size.width);
        let room = (self.window.frame().size.width / 2.
            - (tools.max(TAB_TOOLS) / 2. + TAB_CLEAR)
            - leading.max(TAB_LEADING)
            - 2. * (height + TAB_GAP))
            .max(TAB_MIN)
            .floor();
        if *self.tabs_shown.borrow() == (tabs.clone(), room) {
            return;
        }
        for view in self.tabs.subviews().to_vec() {
            view.removeFromSuperview();
        }
        let glyph = |name: &str, label: &str, points: f64| {
            let config =
                NSImageSymbolConfiguration::configurationWithPointSize_weight(points, unsafe {
                    NSFontWeightMedium
                });
            let image = symbol(name, label);
            image.imageWithSymbolConfiguration(&config).unwrap_or(image)
        };
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
        // A cell that is not a tab stands in a holder: the strip's own
        // children are its tabs only (the order a drag reads).
        let square = NSSize::new(height, height);
        let holder = |x: f64, width: f64| {
            let view = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::new(x, 0.), NSSize::new(width, height)),
            );
            self.tabs.addSubview(&view);
            view
        };
        // Home, the strip's first cell: a quiet glyph.
        holder(0., height).addSubview(&self._delegate.ivars().home);
        // A title in one size and weight whatever its state, so a tab is as
        // wide showing as not. A label keeps `LABEL_PAD` clear inside each
        // end of its frame.
        const LABEL_PAD: f64 = 2.;
        let lead = |tab: &Tab| {
            TAB_PAD
                + if tab.edited {
                    TAB_DOT + TAB_DOT_GAP
                } else {
                    0.
                }
        };
        let labels: Vec<(Retained<NSTextField>, NSSize)> = tabs
            .iter()
            .map(|tab| {
                let label = NSTextField::labelWithString(&NSString::from_str(&tab.title), mtm);
                label.setFont(Some(&NSFont::systemFontOfSize_weight(
                    t.typography.title as f64,
                    unsafe { NSFontWeightRegular },
                )));
                label.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
                label.setAccessibilityElement(false);
                let natural = label.fittingSize();
                (label, natural)
            })
            .collect();
        let natural: Vec<f64> = tabs
            .iter()
            .zip(&labels)
            .map(|(tab, (_, size))| (lead(tab) + size.width - 2. * LABEL_PAD + TAB_PAD).ceil())
            .collect();
        // With more projects than fit at the narrowest width, the strip
        // shows the run that holds the selected one and a pull-down lists
        // them all.
        let selected = tabs.iter().position(|t| t.selected).unwrap_or(0);
        let (start, widths, overflow) = tab_layout(&natural, selected, room);
        let mut x = height + TAB_GAP;
        for ((tab, (label, size)), width) in tabs.iter().zip(&labels).skip(start).zip(widths) {
            let title = NSString::from_str(&tab.title);
            let button: Retained<TabButton> = unsafe {
                msg_send![
                    TabButton::class(),
                    buttonWithTitle: &*title,
                    target: &**target,
                    action: sel!(selectTab:)
                ]
            };
            // The button is the whole cell and draws nothing: the cell
            // draws the fill, the label the title.
            button.setTransparent(true);
            button.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(width, height)));
            button.setTag(tab.window);
            button.setToolTip(Some(&title));
            button.setAccessibilityIdentifier(Some(&NSString::from_str("shell.toolbar.tab")));
            button.setAccessibilitySelected(tab.selected);
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
            // The title starts on the tab's inset (after the edited dot)
            // and is cut in the middle when the tab is narrower than it, so
            // the start and the end of a name both stay.
            let from = lead(tab) - LABEL_PAD;
            label.setFrame(NSRect::new(
                NSPoint::new(from, ((height - size.height) / 2.).floor()),
                NSSize::new(
                    (width - from - TAB_PAD + LABEL_PAD).min(size.width),
                    size.height,
                ),
            ));
            // Its end fades out under the close button: a mask, so the
            // title never moves.
            let fade = CAGradientLayer::new();
            let (ink, clear) = (
                mask_stop(&NSColor::blackColor()),
                mask_stop(&NSColor::clearColor()),
            );
            unsafe {
                fade.setColors(Some(&NSArray::from_retained_slice(&[
                    ink.clone(),
                    ink,
                    clear.clone(),
                    clear,
                ])));
            }
            fade.setStartPoint(NSPoint::new(0., 0.5));
            fade.setEndPoint(NSPoint::new(1., 0.5));
            fade.setFrame(NSRect::new(NSPoint::ZERO, label.frame().size));
            label.setWantsLayer(true);
            if let Some(layer) = label.layer() {
                unsafe { layer.setMask(Some(&fade)) };
            }
            let close = unsafe {
                NSButton::buttonWithImage_target_action(
                    &glyph("xmark", "Close Project", 8.5),
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
                NSPoint::new(width - TAB_CLOSE_PAD - TAB_CLOSE, (height - TAB_CLOSE) / 2.),
                NSSize::new(TAB_CLOSE, TAB_CLOSE),
            ));
            let view = TabView::new(
                mtm,
                NSRect::new(NSPoint::new(x, 0.), NSSize::new(width, height)),
                Retained::into_super(Retained::into_super(label.clone())),
                Some(close.clone()),
                Some(fade),
                tab.edited,
                tab.selected,
            );
            view.addSubview(label);
            view.addSubview(&button);
            view.addSubview(&close);
            self.tabs.addSubview(&view);
            view.reveal(false, false);
            view.restyle();
            x += width + TAB_GAP;
        }
        // The strip's own buttons after the last tab: every project (once
        // they no longer fit), then a new one. Each is a quiet glyph in a
        // cell of its own, so the pointer finds the strip's fill under it.
        let tool = |button: &NSButton, x: f64| {
            button.setFrame(NSRect::new(NSPoint::ZERO, square));
            let cell = TabView::new(
                mtm,
                NSRect::new(NSPoint::ZERO, square),
                Retained::into_super(Retained::into_super(objc2::Message::retain(button))),
                None,
                None,
                false,
                false,
            );
            cell.addSubview(button);
            cell.restyle();
            holder(x, height).addSubview(&cell);
        };
        if overflow {
            let more = NSPopUpButton::initWithFrame_pullsDown(
                NSPopUpButton::alloc(mtm),
                NSRect::new(NSPoint::ZERO, square),
                true,
            );
            more.setBordered(false);
            more.addItemWithTitle(&NSString::from_str(""));
            // The glyph stands in the middle of the target, in place of the
            // pull-down's own arrow at its edge.
            if let Some(cell) = more
                .cell()
                .and_then(|c| c.downcast::<NSPopUpButtonCell>().ok())
            {
                cell.setArrowPosition(NSPopUpArrowPosition::NoArrow);
            }
            if let Some(face) = more.itemAtIndex(0) {
                face.setImage(Some(&glyph("chevron.down", "All Projects", 10.)));
            }
            more.setImagePosition(NSCellImagePosition::ImageOnly);
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
            tool(&more, x);
            x += height + TAB_GAP;
        }
        let add = unsafe {
            NSButton::buttonWithImage_target_action(
                &glyph("plus", "New Project", 12.),
                Some(target),
                Some(sel!(newTab:)),
                mtm,
            )
        };
        add.setBordered(false);
        add.setButtonType(NSButtonType::MomentaryChange);
        add.setToolTip(Some(&NSString::from_str("New Project")));
        add.setAccessibilityIdentifier(Some(&NSString::from_str("shell.toolbar.newProject")));
        tool(&add, x);
        self.tabs_width.setConstant(x + height);
        *self.tabs_shown.borrow_mut() = (tabs, room);
    }
    /// Home over the window: the Home item reads selected, and the
    /// document's own controls (tools, sidebars, Export, the title) step
    /// aside, as v1's titlebar hides Export and the panel library on Home.
    pub fn set_home(&self, home: bool) {
        self._delegate.ivars().home.select(home);
        if self.home.get() == home {
            return;
        }
        self.home.set(home);
        self.sync_sidebar();
        self.sync_tabs();
        for item in self._toolbar.items().to_vec() {
            let name = item.itemIdentifier().to_string();
            // The tabs stay, as in v1's title bar on Home.
            if matches!(name.as_str(), "tools" | "add" | "export" | "panelLibrary") {
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
        let view = sidebar_material(
            self.window.mtm(),
            &self.tokens,
            self.tokens.metrics.sidebar,
            self.content.bounds().size.height,
        );
        self.content.addSubview_positioned_relativeTo(
            &view,
            NSWindowOrderingMode::Below,
            Some(&self.gpui),
        );
        *slot = Some(view);
    }
    /// Development builds: a tab action by index, through the same code the
    /// tab's buttons and menu run (`--open reviewTab:close:1`). `press` is
    /// a pointer press and release on the tab; `hover` the pointer over the
    /// strip's tab at that place, `hovertool` over New Project. `dragleft`
    /// and `dragright` carry the tab one place along the strip with pointer
    /// events posted to this app's own queue; `holdleft` and `holdright`
    /// keep it lifted for a few seconds first, for a capture.
    /// Development builds: resizes the window by (`dw`, `dh`) in twelve
    /// steps, two frames apart, its top-left corner kept, and writes
    /// `resize.txt` to `--save-dir`: for every hosted AppKit control, how far
    /// it stood, at the end of the run-loop turn that set a step's frame,
    /// from where the following frames put it. A line names a step AppKit
    /// displayed a turn later (the content and the controls then move
    /// together in that turn); what the screen showed is read from captures
    /// taken during the run.
    #[cfg(debug_assertions)]
    pub fn review_resize(&self, dw: f64, dh: f64) {
        use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
        use std::ffi::c_void;
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            static kCFRunLoopCommonModes: *const c_void;
            fn CFRunLoopGetMain() -> *mut c_void;
            fn CFRunLoopObserverCreateWithHandler(
                allocator: *const c_void,
                activities: usize,
                repeats: u8,
                order: isize,
                block: &block2::Block<dyn Fn(*mut c_void, usize)>,
            ) -> *mut c_void;
            fn CFRunLoopAddObserver(
                run_loop: *mut c_void,
                observer: *mut c_void,
                mode: *const c_void,
            );
            fn CFRelease(object: *mut c_void);
        }
        const BEFORE_WAITING: usize = 1 << 5;
        const STEPS: usize = 12;
        fn controls(view: &NSView, skip: &NSView, out: &mut Vec<(String, NSRect)>) {
            if std::ptr::eq(view, skip) || view.isHidden() {
                return;
            }
            if unsafe { msg_send![view, isKindOfClass: NSControl::class()] } {
                let name = format!(
                    "{} {:?}",
                    view.class().name().to_string_lossy(),
                    view.accessibilityLabel()
                        .map(|l| l.to_string())
                        .unwrap_or_default()
                );
                // From the window's top-left, which the resize keeps.
                let r = view.convertRect_toView(view.bounds(), None);
                let top = view.window().map_or(0., |w| w.frame().size.height);
                out.push((
                    name,
                    NSRect::new(
                        NSPoint::new(r.origin.x, top - r.origin.y - r.size.height),
                        r.size,
                    ),
                ));
                return;
            }
            for child in view.subviews().to_vec() {
                controls(&child, skip, out);
            }
        }
        let (window, gpui) = (self.window.clone(), self.gpui.clone());
        let read = move || {
            let mut out = vec![];
            if let Some(content) = window.contentView() {
                controls(&content, &gpui, &mut out);
            }
            out
        };
        let window = self.window.clone();
        let state = Rc::new(RefCell::new((0usize, read(), String::new(), 0f64)));
        let step = block2::RcBlock::new(move |timer: std::ptr::NonNull<NSTimer>| {
            let mut s = state.borrow_mut();
            let settled = read();
            if s.0 > 0 {
                // The same control in both readings (a list that changed
                // between them is not compared).
                for ((name, at), (same, now)) in s.1.clone().iter().zip(&settled) {
                    if name != same {
                        continue;
                    }
                    let off = (at.origin.x - now.origin.x)
                        .abs()
                        .max((at.origin.y - now.origin.y).abs())
                        .max((at.size.width - now.size.width).abs());
                    s.3 = s.3.max(off);
                    if off > 0.01 {
                        let line = format!("step {} {name}: {off:.1} pt behind\n", s.0);
                        s.2.push_str(&line);
                    }
                }
            }
            if s.0 == STEPS {
                unsafe { timer.as_ref().invalidate() };
                let report = format!(
                    "{} controls, {STEPS} steps of ({:.1}, {:.1}) pt, worst {:.1} pt\n{}",
                    settled.len(),
                    dw / STEPS as f64,
                    dh / STEPS as f64,
                    s.3,
                    s.2
                );
                if let Ok(dir) = std::env::var("PM_NATIVE_REVIEW_SAVE_DIR") {
                    let _ = std::fs::write(std::path::Path::new(&dir).join("resize.txt"), report);
                }
                return;
            }
            s.0 += 1;
            let mut frame = window.frame();
            frame.size.width += dw / STEPS as f64;
            frame.size.height += dh / STEPS as f64;
            frame.origin.y -= dh / STEPS as f64;
            drop(s);
            window.setFrame_display(frame, true);
            // Read once Core Animation has committed this turn of the run
            // loop (its observer runs at order 2,000,000): what the screen
            // gets together with the window's new size.
            let (state, read) = (state.clone(), read.clone());
            let committed = block2::RcBlock::new(move |_: *mut c_void, _: usize| {
                state.borrow_mut().1 = read();
            });
            unsafe {
                let observer = CFRunLoopObserverCreateWithHandler(
                    std::ptr::null(),
                    BEFORE_WAITING,
                    0,
                    2_000_001,
                    &committed,
                );
                CFRunLoopAddObserver(CFRunLoopGetMain(), observer, kCFRunLoopCommonModes);
                CFRelease(observer);
            }
        });
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(1. / 30., true, &step) };
        unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
    }
    #[cfg(debug_assertions)]
    pub fn review_popup(&self, action: &str, value: &str) {
        for popup in [&self.palette, &self.fields] {
            let mut p = popup.borrow_mut();
            if !p.model.open {
                continue;
            }
            match action {
                "hover" | "leave" => {
                    if p.model.point(value.parse().ok()) {
                        p.light(true);
                    }
                }
                "type" => {
                    p.search.setStringValue(&NSString::from_str(value));
                    p.searched();
                }
                "key" => p.key(value, false),
                _ => {}
            }
        }
    }
    #[cfg(debug_assertions)]
    pub fn review_tab(&self, action: &str, index: usize) {
        let windows = tab_windows(&self.window);
        let Some(window) = windows.get(index) else {
            return;
        };
        match action {
            "select" => show_in_tab_group(window),
            "close" => request_close(window),
            // The tab at `index` of the strip as the pointer over it.
            "hover" => {
                if let Some(tab) = strip_tabs(&self.tabs).get(index) {
                    tab.hover(true);
                }
            }
            // The strip's last own button (New Project) as the pointer
            // over it.
            "hovertool" => {
                let cell = self
                    .tabs
                    .subviews()
                    .to_vec()
                    .into_iter()
                    .filter(|v| !v.isKindOfClass(TabView::class()))
                    .flat_map(|holder| holder.subviews().to_vec())
                    .filter_map(|v| v.downcast::<TabView>().ok())
                    .last();
                if let Some(cell) = cell {
                    cell.hover(true);
                }
            }
            "left" => move_tab(window, -1),
            "right" => move_tab(window, 1),
            "dragleft" | "dragright" | "holdleft" | "holdright" | "press" => review_drag(
                &self.tabs,
                window.windowNumber(),
                action.ends_with("right"),
                if action.starts_with("hold") { 4. } else { 0.3 },
                action == "press",
            ),
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
        // Who has the keyboard comes first: a text view that took it would
        // swallow the editor's shortcuts.
        let mut out = format!(
            "first responder: {}\n",
            self.window.firstResponder().map_or("none".into(), |r| r
                .class()
                .name()
                .to_string_lossy()
                .into_owned())
        );
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
            layer.setBackgroundColor(Some(&native_color(t.viewer_surround()).CGColor()));
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
            viewer_slot::clip(&self.viewer, &self.layer, self.tokens.viewer_surround());
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
        self.contributed.extend(bindings);
        self.rebuild_bindings(crate::keymap::current());
    }
    fn rebuild_bindings(&mut self, base: Vec<crate::keyboard::Binding>) {
        self.resolver.bindings = base;
        self.resolver
            .bindings
            .extend(self.contributed.iter().cloned());
        self.resolver
            .bindings
            .sort_by_key(|b| std::cmp::Reverse(b.priority));
    }
    /// A new keymap: keys resolve against `table` (with the extensions'
    /// rows), and the menu bar and palette show its keys.
    pub fn set_keymap(&mut self, table: Vec<crate::keyboard::Binding>) {
        crate::keymap::set_current(table.clone());
        self.rebuild_bindings(table);
        self.palette.borrow_mut().shortcuts = shortcut_glyphs();
        install_menus();
    }
    /// The next key pressed is sent as `shortcuts.recorded` with its chord
    /// (Escape included, which cancels) instead of running anything.
    pub fn record_next_key(&mut self, on: bool) {
        self.recording = on;
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
        self.contributed.retain(|b| !b.id.starts_with(owner));
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
    /// Where the viewer slot stands for what GPUI last laid out.
    fn viewer_geometry(&self) -> Option<SlotGeometry> {
        let h = self.content.bounds().size.height;
        self.viewer_rect.get().map(|r| {
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
        })
    }
    /// Whether `present` would move the viewer slot now (the window asks
    /// before it moves anything; native panel bodies cannot say beforehand).
    fn pending(&self) -> bool {
        self.last_geometry != Some(self.viewer_geometry())
    }
    fn present(&mut self) -> gpui_macos::composition::SlotChange {
        let geometry = self.viewer_geometry();
        let moved = self.last_geometry != Some(geometry);
        if moved {
            if let Some(g) = geometry {
                viewer_slot::show(&self.viewer, &self.layer, true);
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
                viewer_slot::show(&self.viewer, &self.layer, false);
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
            if item.itemIdentifier().to_string() == "add"
                && let Ok(add) = item.clone().downcast::<objc2_app_kit::NSMenuToolbarItem>()
            {
                for row in add.menu().itemArray().to_vec() {
                    let command: Option<Retained<NSString>> =
                        unsafe { row.representedObject().map(|o| Retained::cast_unchecked(o)) };
                    if let Some(command) = command {
                        row.setEnabled(!disabled.contains(&command.to_string()));
                    }
                }
            }
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
        choices.retain(|c| {
            !state
                .disabled
                .contains(menu::split_palette_command(&c.command).0)
        });
        for (id, title) in &state.panels {
            let open = state.open.contains(id);
            if open && !crate::layout::closable(id) {
                continue;
            }
            choices.push(
                Choice::new(
                    format!("{} {title}", if open { "Hide" } else { "Show" }),
                    format!("panel:toggle:{id}"),
                )
                .within("Panels"),
            );
        }
        choices.extend(self.contributed_choices.iter().cloned().map(|c| {
            if c.category.is_empty() {
                c.within("Extensions")
            } else {
                c
            }
        }));
        for (id, name, _) in &state.workspaces {
            choices.push(Choice::new(name.clone(), format!("workspace:{id}")).within("Workspace"));
        }
        for (label, command) in [
            ("Edit Workspace JSON…", "layout:edit"),
            ("Save Workspace…", "layout:save"),
            ("Reset Workspace", "layout:reset"),
        ] {
            choices.push(Choice::new(label, command).within("Workspace"));
        }
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
        if self.window.attachedSheet().is_some()
            || gpui_macos::child_window::sheet_pending(&self.window)
        {
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
        let (on, shown) = (self.window.clone(), sheet.window.clone());
        gpui_macos::child_window::sheet(
            &self.window,
            &sheet.window,
            move || on.beginSheet_completionHandler(&shown, Some(&handler)),
            || {},
        );
        sheet.window.makeFirstResponder(Some(&sheet.field));
        *self.sheet.borrow_mut() = Some(sheet);
    }
    /// Edit Workspace JSON… (S100): the active workspace's definition in a
    /// wide sheet. Apply closes it only when the text is a usable definition
    /// (else the reason shows above the buttons, and nothing changes), then
    /// sends `layout:define` with the text and the definition it started from.
    pub fn edit_workspace_definition(&self, json: &str) {
        if self.window.attachedSheet().is_some()
            || gpui_macos::child_window::sheet_pending(&self.window)
        {
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
        let (on, shown) = (self.window.clone(), window.clone());
        gpui_macos::child_window::sheet(
            &self.window,
            &window,
            move || on.beginSheet_completionHandler(&shown, Some(&handler)),
            || {},
        );
        window.makeFirstResponder(Some(&target.ivars().text));
        *self.definition.borrow_mut() = Some(target);
    }
    /// Sends `name` once when the key that sent the pending `token` comes up
    /// (or the window stops being key). False: the token has no key event.
    pub fn on_release(&self, token: u64, name: &str, args: Vec<serde_json::Value>) -> bool {
        let Some(event) = self.events.get(&token) else {
            return false;
        };
        if self.up_early.contains(&token) {
            return false;
        }
        self.releases.borrow_mut().insert(
            event.keyCode(),
            Command {
                name: name.into(),
                args,
                token: None,
            },
        );
        true
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
                let Some(editor) = self
                    .window
                    .firstResponder()
                    .filter(|r| unsafe { msg_send![&**r,isKindOfClass:NSTextView::class()] })
                else {
                    return false;
                };
                // A search field hears the key itself: it tells its view
                // (which clears the search or closes the field), then gives
                // the keyboard back. Any other field just gives it back.
                let search = unsafe {
                    let delegate: *mut AnyObject = msg_send![&*editor, delegate];
                    let is = |class: &objc2::runtime::AnyClass| -> bool {
                        !delegate.is_null() && msg_send![delegate, isKindOfClass: class]
                    };
                    // One of our own fields that asked for Escape to cancel
                    // (a drawn search pill) hears it the same way.
                    is(NSSearchField::class())
                        || (is(NSControl::class()) && {
                            let tag: isize = msg_send![delegate, tag];
                            tag == gpui_macos::native_text::ESCAPE_CANCELS
                        })
                };
                if search {
                    unsafe {
                        let _: () =
                            msg_send![&*editor, doCommandBySelector: sel!(cancelOperation:)];
                    }
                } else {
                    self.window.makeFirstResponder(Some(&self.gpui));
                }
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
        for observer in &self.window_observers {
            unsafe {
                objc2_foundation::NSNotificationCenter::defaultCenter()
                    .removeObserver(&*(Retained::as_ptr(observer) as *const AnyObject));
            }
        }
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
                p.borrow_mut().searched();
            }
        }
    }
    unsafe impl NSTextFieldDelegate for PopupTarget {}
    impl PopupTarget {
        #[unsafe(method(searched:))]
        fn searched(&self, _: &NSControl) {
            if let Some(p) = self.ivars().upgrade()
                && let Ok(mut p) = p.try_borrow_mut()
            {
                p.searched();
            }
        }
        #[unsafe(method(pick:))]
        fn pick(&self, sender: &NSControl) {
            if let Some(p) = self.ivars().upgrade() {
                let mut p = p.borrow_mut();
                let index = sender.tag() as usize;
                if let Some(c) = p.model.choices.get(index) {
                    let (name, args) = menu::split_palette_command(&c.command);
                    let _ = p.sender.try_send(Command { name: name.to_owned(), args, token: None });
                }
                p.hide();
            }
        }
    }
);
define_class!(
    /// The rows of a palette or menu: the scrolling list, top row first,
    /// that follows the pointer over them.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMShellPopupList"]
    #[ivars = RefCell<std::rc::Weak<RefCell<Popup>>>]
    struct PopupList;
    impl PopupList {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
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
                    NSTrackingAreaOptions::MouseMoved
                        | NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveAlways
                        | NSTrackingAreaOptions::InVisibleRect,
                    Some(self),
                    None,
                )
            };
            self.addTrackingArea(&area);
        }
        // Nothing lights as the menu opens under a resting pointer: only a
        // move does.
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.point(Some(event));
        }
        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _: &NSEvent) {
            self.point(None);
        }
        // The rows move under a resting pointer.
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            unsafe {
                let _: () = msg_send![super(self), scrollWheel: event];
            }
            self.point(Some(event));
        }
    }
);
impl PopupList {
    fn point(&self, event: Option<&NSEvent>) {
        if let Some(popup) = self.ivars().borrow().upgrade()
            && let Ok(mut popup) = popup.try_borrow_mut()
        {
            popup.point(event.map(|e| self.convertPoint_fromView(e.locationInWindow(), None)));
        }
    }
}
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
/// One row of a palette or menu: its place in the filtered list, its frame
/// in the list and the control that takes its press.
struct PopupRow {
    place: usize,
    rect: NSRect,
    button: Retained<NSButton>,
}
/// The shape a palette or menu is built on. Rows stand `inset` inside the
/// surface, so a row's corner is the surface's less that inset.
#[derive(Clone, Copy)]
struct PopupShape {
    radius: f32,
    inset: f32,
    row: f32,
    heading: f32,
}
fn popup_shape(t: &Tokens, surface: Surface) -> PopupShape {
    if surface == Surface::Palette {
        PopupShape {
            radius: t.radius.xl,
            inset: t.space.sm,
            row: t.metrics.menu_row,
            heading: t.metrics.menu_row - t.space.xs,
        }
    } else {
        PopupShape {
            radius: t.radius.lg,
            inset: t.space.xs,
            row: t.metrics.row,
            heading: t.metrics.row - t.space.xs,
        }
    }
}
/// The surface of a palette or menu, in the menu's own corner: glass where
/// the system has it, with the floating colour laid under the rows
/// (`translucency.glass_menu`). Glass alone takes its value from what is
/// behind it, and a tint does not hold it: over a dark viewer in the light
/// appearance the rows would be dark ink on dark glass.
fn popup_material(
    mtm: MainThreadMarker,
    frame: NSRect,
    t: &Tokens,
    surface: Surface,
) -> Retained<NSView> {
    let view = material(mtm, frame, t, surface);
    let radius = popup_shape(t, surface).radius as f64;
    if let Ok(glass) = view.clone().downcast::<NSGlassEffectView>() {
        glass.setCornerRadius(radius);
        let under = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::new(NSPoint::ZERO, frame.size));
        under.setBoxType(NSBoxType::Custom);
        under.setBorderWidth(0.);
        under.setCornerRadius(radius);
        under.setFillColor(
            &native_color(t.colors.floating)
                .colorWithAlphaComponent(t.translucency.glass_menu as f64),
        );
        under.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        contents(&view).addSubview(&under);
    } else if let Ok(solid) = view.clone().downcast::<NSBox>() {
        solid.setCornerRadius(radius);
    }
    view
}
/// The lit row: the ink as a wash, so it reads on glass over any content.
fn popup_highlight(t: &Tokens) -> Retained<NSColor> {
    native_color(t.colors.text).colorWithAlphaComponent(t.translucency.selected as f64)
}
fn popup_label(text: &str, size: f32, ink: u32, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(size as f64)));
    label.setTextColor(Some(&native_color(ink)));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    // The row's button speaks for it.
    label.setAccessibilityElement(false);
    label
}
/// Development builds: `PM_NATIVE_REVIEW_SLOW=8` runs a menu's motion that
/// many times slower, so a capture can catch it on the way.
fn review_slow() -> f64 {
    static SLOW: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *SLOW.get_or_init(|| {
        std::env::var("PM_NATIVE_REVIEW_SLOW")
            .ok()
            .filter(|_| cfg!(debug_assertions))
            .and_then(|v| v.parse().ok())
            .filter(|v| *v >= 1.)
            .unwrap_or(1.)
    })
}
/// Fades `view` to `alpha` over `seconds`.
fn fade(view: &NSView, alpha: f64, seconds: f64) {
    unsafe {
        let context = objc2::class!(NSAnimationContext);
        let _: () = msg_send![context, beginGrouping];
        let current: *mut AnyObject = msg_send![context, currentContext];
        let _: () = msg_send![current, setDuration: seconds];
        let proxy: *mut AnyObject = msg_send![view, animator];
        let _: () = msg_send![proxy, setAlphaValue: alpha];
        let _: () = msg_send![context, endGrouping];
    }
}
/// The command palette and the menus a button of ours opens: a search field
/// over quiet rows in sections, on glass. Nothing is lit when it opens; one
/// highlight follows the pointer and the keyboard, gliding between rows on
/// the layout spring, and leaves with the pointer.
pub struct Popup {
    window: Retained<PopupWindow>,
    parent: Retained<NSWindow>,
    surface: Surface,
    search: Retained<NSSearchField>,
    scroll: Retained<NSScrollView>,
    list: Retained<PopupList>,
    body: Retained<NSView>,
    spring: Option<(std::time::Instant, pm_design::motion::Analytic)>,
    rows: Vec<PopupRow>,
    /// Headings, titles, checks and shortcut hints of the rows.
    parts: Vec<Retained<NSView>>,
    /// command → shortcut glyphs shown beside palette rows.
    shortcuts: HashMap<String, String>,
    glider: Retained<NSBox>,
    /// The highlight is on a row (it fades in where it lands, then glides).
    lit: bool,
    /// The highlight's travel to its row, and the timer that draws it.
    glide: Option<(std::time::Instant, pm_design::motion::Analytic)>,
    timer: Option<Retained<objc2_foundation::NSTimer>>,
    me: std::rc::Weak<RefCell<Popup>>,
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
        let mat = popup_material(mtm, frame, t, surface);
        let body = contents(&mat);
        window.setContentView(Some(&mat));
        mat.setWantsLayer(true);
        if let Some(layer) = mat.layer() {
            layer.setOpacity(0.);
        }
        // The system's search field: its own magnifier and clear button. It
        // is the menu's only input and always has the keyboard, so no ring
        // (owner rule: no accent focus rings).
        let search = NSSearchField::initWithFrame(NSSearchField::alloc(mtm), NSRect::ZERO);
        search.setPlaceholderString(Some(&NSString::from_str(if surface == Surface::Palette {
            "Search commands"
        } else {
            "Search fields"
        })));
        search.setControlSize(NSControlSize::Large);
        search.setFont(Some(&NSFont::systemFontOfSize(t.typography.title as f64)));
        search.setFocusRingType(NSFocusRingType::None);
        search.setAccessibilityLabel(Some(&NSString::from_str("Search")));
        body.addSubview(&search);
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::ZERO);
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setScrollerStyle(NSScrollerStyle::Overlay);
        scroll.setAutohidesScrollers(true);
        scroll.setAutomaticallyAdjustsContentInsets(false);
        let list: Retained<PopupList> = unsafe {
            msg_send![
                super(PopupList::alloc(mtm).set_ivars(RefCell::new(std::rc::Weak::new()))),
                initWithFrame: NSRect::ZERO
            ]
        };
        scroll.setDocumentView(Some(&list));
        body.addSubview(&scroll);
        let glider = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::ZERO);
        glider.setBoxType(NSBoxType::Custom);
        glider.setFillColor(&popup_highlight(t));
        glider.setBorderWidth(0.);
        let shape = popup_shape(t, surface);
        glider.setCornerRadius(t.radius.nested(shape.radius, shape.inset) as f64);
        glider.setAlphaValue(0.);
        list.addSubview(&glider);
        let popup = Rc::new(RefCell::new(Self {
            window,
            parent: unsafe { Retained::retain(parent as *const _ as *mut _).unwrap() },
            surface,
            search,
            scroll,
            list,
            body,
            spring: None,
            rows: vec![],
            parts: vec![],
            shortcuts: HashMap::new(),
            glider,
            lit: false,
            glide: None,
            timer: None,
            me: std::rc::Weak::new(),
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
            let p = popup.borrow();
            let _: () = msg_send![&*p.search, setDelegate: &*target];
            // The clear button empties the field without an edit.
            p.search.setTarget(Some(&target));
            p.search.setAction(Some(sel!(searched:)));
            *p.list.ivars().borrow_mut() = Rc::downgrade(&popup);
        }
        popup.borrow_mut().me = Rc::downgrade(&popup);
        popup.borrow_mut().target = Some(target);
        popup.borrow_mut().refresh();
        popup
    }
    fn update_tokens(&mut self, t: &Tokens, surface: Surface) {
        self.tokens = t.clone();
        self.surface = surface;
        appearance(&self.window, t);
        let frame = NSRect::new(NSPoint::ZERO, self.window.frame().size);
        let mat = popup_material(MainThreadMarker::new().unwrap(), frame, t, surface);
        let body = contents(&mat);
        self.search.removeFromSuperview();
        self.scroll.removeFromSuperview();
        body.addSubview(&self.search);
        body.addSubview(&self.scroll);
        self.glider.setFillColor(&popup_highlight(t));
        self.window.setContentView(Some(&mat));
        self.body = body;
        self.refresh();
    }
    /// Lays the rows out again for the query: sections under their headings,
    /// the panel as tall as its results (to `max_height`), its top edge kept.
    fn refresh(&mut self) {
        for part in self.parts.drain(..) {
            part.removeFromSuperview();
        }
        for row in self.rows.drain(..) {
            row.button.removeFromSuperview();
        }
        let mtm = MainThreadMarker::new().unwrap();
        let t = self.tokens.clone();
        let (m, shape) = (t.metrics, popup_shape(&t, self.surface));
        let chosen: Vec<(usize, Choice)> = self
            .model
            .filtered()
            .into_iter()
            .map(|i| (i, self.model.choices[i].clone()))
            .collect();
        let width = self.window.frame().size.width;
        let (inset, row_h) = (shape.inset as f64, shape.row as f64);
        let checks = chosen.iter().any(|(_, c)| c.checked.is_some());
        let sections = chosen.iter().any(|(_, c)| !c.category.is_empty());
        let edge = inset + t.space.md as f64;
        let lead = edge
            + if checks {
                (m.symbol_inline + t.space.sm) as f64
            } else {
                0.
            };
        let caption = t.text(pm_design::Text::Caption).size;
        let mut y = 0.;
        let mut category: Option<String> = None;
        let centred = |view: &NSView, x: f64, top: f64, height: f64, width: f64| {
            let h = view.fittingSize().height.ceil();
            view.setFrame(NSRect::new(
                NSPoint::new(x, top + ((height - h) / 2.).floor()),
                NSSize::new(width, h),
            ));
        };
        for (place, (index, c)) in chosen.iter().enumerate() {
            if sections && category.as_deref() != Some(c.category.as_str()) {
                category = Some(c.category.clone());
                if c.category.is_empty() {
                    y += if place > 0 { t.space.sm as f64 } else { 0. };
                } else {
                    // A heading sits low in its band, nearer its own rows.
                    let heading = popup_label(&c.category, caption, t.colors.muted, mtm);
                    let h = heading.fittingSize().height.ceil();
                    heading.setFrame(NSRect::new(
                        NSPoint::new(edge, y + shape.heading as f64 - h - t.space.xs as f64),
                        NSSize::new(width - edge * 2., h),
                    ));
                    self.list.addSubview(&heading);
                    self.parts
                        .push(Retained::into_super(Retained::into_super(heading)));
                    y += shape.heading as f64;
                }
            }
            let rect = NSRect::new(
                NSPoint::new(inset, y),
                NSSize::new(width - inset * 2., row_h),
            );
            let mut trailing = rect.origin.x + rect.size.width - t.space.md as f64;
            if let Some(glyphs) = self.shortcuts.get(&c.command) {
                let hint = popup_label(glyphs, caption, t.colors.faint, mtm);
                hint.setAlignment(NSTextAlignment::Right);
                let w = hint.fittingSize().width.ceil();
                centred(&hint, trailing - w, y, row_h, w);
                trailing -= w + t.space.lg as f64;
                self.list.addSubview(&hint);
                self.parts
                    .push(Retained::into_super(Retained::into_super(hint)));
            }
            if c.checked == Some(true) {
                let mark = NSImageView::imageViewWithImage(&symbol("checkmark", "Checked"), mtm);
                mark.setSymbolConfiguration(Some(
                    &NSImageSymbolConfiguration::configurationWithPointSize_weight(
                        (m.symbol_inline - 2.) as f64,
                        unsafe { NSFontWeightSemibold },
                    ),
                ));
                mark.setContentTintColor(Some(&native_color(t.colors.text)));
                mark.setAccessibilityElement(false);
                centred(&mark, edge, y, row_h, m.symbol_inline as f64);
                self.list.addSubview(&mark);
                self.parts
                    .push(Retained::into_super(Retained::into_super(mark)));
            }
            let title = popup_label(&c.label, t.typography.body, t.colors.text, mtm);
            centred(&title, lead, y, row_h, (trailing - lead).max(0.));
            self.list.addSubview(&title);
            self.parts
                .push(Retained::into_super(Retained::into_super(title)));
            // The whole row takes the press; it draws nothing itself.
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(""),
                    self.target.as_deref().map(|t| t as &AnyObject),
                    Some(sel!(pick:)),
                    mtm,
                )
            };
            button.setTransparent(true);
            button.setAccessibilityLabel(Some(&NSString::from_str(&c.label)));
            button.setAccessibilityIdentifier(Some(&NSString::from_str(&format!(
                "shell.palette.{}",
                c.command
            ))));
            if let Some(glyphs) = self.shortcuts.get(&c.command) {
                button.setAccessibilityHelp(Some(&NSString::from_str(glyphs)));
            }
            button.setTag(*index as isize);
            button.setFrame(rect);
            self.list.addSubview(&button);
            self.rows.push(PopupRow {
                place,
                rect,
                button,
            });
            y += row_h;
        }
        if chosen.is_empty() {
            let none = popup_label("No Results", t.typography.body, t.colors.muted, mtm);
            none.setAlignment(NSTextAlignment::Center);
            centred(&none, inset, 0., row_h, width - inset * 2.);
            self.list.addSubview(&none);
            self.parts
                .push(Retained::into_super(Retained::into_super(none)));
            y = row_h;
        }
        let field_h = self.search.fittingSize().height.ceil();
        let above = inset * 2. + field_h;
        let shown = y.min(self.max_height as f64 - above - inset).max(row_h);
        let height = above + shown + inset;
        let mut frame = self.window.frame();
        if (frame.size.height - height).abs() > 0.5 {
            frame.origin.y += frame.size.height - height;
            frame.size.height = height;
            self.window.setFrame_display(frame, self.model.open);
        }
        self.search.setFrame(NSRect::new(
            NSPoint::new(inset, height - inset - field_h),
            NSSize::new(width - inset * 2., field_h),
        ));
        self.scroll.setFrame(NSRect::new(
            NSPoint::new(0., inset),
            NSSize::new(width, shown),
        ));
        // More rows than fit: the list fades out at the bottom edge, so a
        // row is never cut there, and scrolls a fade's length past its end,
        // so the last row comes clear of it.
        let more = y > shown + 0.5;
        self.list.setFrame(NSRect::new(
            NSPoint::ZERO,
            NSSize::new(width, if more { y + POPUP_FADE } else { y }),
        ));
        self.scroll.setWantsLayer(true);
        if let Some(layer) = self.scroll.layer() {
            let fade = more.then(|| {
                let fade = CAGradientLayer::new();
                unsafe {
                    fade.setColors(Some(&NSArray::from_retained_slice(&[
                        mask_stop(&NSColor::clearColor()),
                        mask_stop(&NSColor::blackColor()),
                    ])));
                }
                fade.setLocations(Some(&NSArray::from_retained_slice(&[
                    NSNumber::new_f64(0.),
                    NSNumber::new_f64(POPUP_FADE / shown),
                ])));
                // The scroll view's layer is flipped: its bottom edge is 1.
                fade.setStartPoint(NSPoint::new(0.5, 1.));
                fade.setEndPoint(NSPoint::new(0.5, 0.));
                fade.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(width, shown)));
                fade
            });
            unsafe { layer.setMask(fade.as_deref().map(|f| &**f)) };
        }
        self.list.scrollPoint(NSPoint::ZERO);
        self.light(false);
    }
    /// Puts the highlight on the lit row. It appears where it lands when
    /// nothing was lit, glides there on the layout spring when `glide` (the
    /// pointer, or one key press), and fades when nothing is lit any more.
    fn light(&mut self, glide: bool) {
        let lit = self.model.lit();
        for row in &self.rows {
            unsafe {
                row.button.setAccessibilityValue(Some(&NSString::from_str(
                    if Some(row.place) == lit {
                        "Selected"
                    } else {
                        ""
                    },
                )));
            }
        }
        let (reduce, spring) = (
            self.tokens.accessibility.reduce_motion,
            pm_design::motion::Preset::Layout.spring(&self.tokens.motion),
        );
        let quick = if reduce {
            self.tokens.motion.reduced
        } else {
            self.tokens.motion.interaction
        };
        let Some(rect) = lit.and_then(|lit| {
            self.rows
                .iter()
                .find(|row| row.place == lit)
                .map(|row| row.rect)
        }) else {
            self.rest();
            if self.lit {
                fade(&self.glider, 0., quick);
            }
            self.lit = false;
            return;
        };
        if !self.lit || !glide || reduce {
            self.rest();
            self.glider.setFrame(rect);
            if !self.lit {
                fade(&self.glider, 1., quick);
            }
            self.lit = true;
            return;
        }
        // Retargeted from where it is, with the speed it has.
        let velocity = self.glide.map_or(0., |(start, spring)| {
            spring
                .sample(start.elapsed().as_secs_f64() / review_slow())
                .1
        });
        let mut from = self.glider.frame();
        self.glide = Some((
            std::time::Instant::now(),
            pm_design::motion::Analytic {
                from: from.origin.y,
                velocity,
                target: rect.origin.y,
                mass: 1.,
                stiffness: spring.stiffness as f64,
                damping: spring.damping as f64,
            },
        ));
        // Rows share a width and a height: only the place along the list moves.
        from.origin.x = rect.origin.x;
        from.size = rect.size;
        self.glider.setFrame(from);
        if self.timer.is_none() {
            let me = self.me.clone();
            let step =
                block2::RcBlock::new(move |_: std::ptr::NonNull<objc2_foundation::NSTimer>| {
                    if let Some(popup) = me.upgrade()
                        && let Ok(mut popup) = popup.try_borrow_mut()
                    {
                        popup.step();
                    }
                });
            // In the common modes, so it keeps moving while a scroll tracks.
            let timer = unsafe {
                objc2_foundation::NSTimer::timerWithTimeInterval_repeats_block(
                    1. / 120.,
                    true,
                    &step,
                )
            };
            unsafe {
                objc2_foundation::NSRunLoop::mainRunLoop()
                    .addTimer_forMode(&timer, objc2_foundation::NSRunLoopCommonModes)
            };
            self.timer = Some(timer);
        }
    }
    /// One frame of the highlight's travel.
    fn step(&mut self) {
        let Some((start, spring)) = self.glide else {
            self.rest();
            return;
        };
        let (y, velocity) = spring.sample(start.elapsed().as_secs_f64() / review_slow());
        let there = (y - spring.target).abs() < 0.2 && velocity.abs() < 2.;
        let mut origin = self.glider.frame().origin;
        origin.y = if there { spring.target } else { y };
        self.glider.setFrameOrigin(origin);
        if there {
            self.rest();
        }
    }
    fn rest(&mut self) {
        self.glide = None;
        if let Some(timer) = self.timer.take() {
            timer.invalidate();
        }
    }
    /// The pointer is at `at` in the list, or has left it.
    fn point(&mut self, at: Option<NSPoint>) {
        let row = at.and_then(|at| {
            self.rows
                .iter()
                .find(|row| {
                    at.x >= row.rect.origin.x
                        && at.x < row.rect.origin.x + row.rect.size.width
                        && at.y >= row.rect.origin.y
                        && at.y < row.rect.origin.y + row.rect.size.height
                })
                .map(|row| row.place)
        });
        if self.model.point(row) {
            self.light(true);
        }
    }
    /// Scrolls the keyboard's row into sight (the first row with its heading).
    fn reveal(&self) {
        let Some(row) = self
            .model
            .lit()
            .and_then(|lit| self.rows.iter().find(|row| row.place == lit))
        else {
            return;
        };
        let mut rect = row.rect;
        if row.place == 0 {
            rect.size.height += rect.origin.y;
            rect.origin.y = 0.;
        }
        self.list.scrollRectToVisible(rect);
        if self.row_focus {
            self.window.makeFirstResponder(Some(&row.button));
        }
    }
    fn searched(&mut self) {
        let q = self.search.stringValue().to_string();
        if q != self.model.query {
            self.model.search(q);
            self.refresh();
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
        self.lit = false;
        self.glider.setAlphaValue(0.);
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
        #[cfg(debug_assertions)]
        gpui_macos::child_window::review_show(&self.window, &self.parent);
        self.window.makeFirstResponder(Some(&self.search));
        animate(&self.window, true, &self.tokens, &mut self.spring);
    }
    fn hide(&mut self) {
        self.model.open = false;
        self.rest();
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
    /// A key the menu owns. One press glides the highlight a row; a held
    /// key steps it without the glide.
    fn key(&mut self, key: &str, repeat: bool) {
        if matches!(key, "tab" | "shift+tab") {
            let n = self.rows.len();
            if n == 0 {
                return;
            }
            let reverse = key == "shift+tab";
            // Tab walks the rows and comes back to the field past either end.
            let next = match self.model.lit().filter(|_| self.row_focus) {
                None => Some(if reverse { n - 1 } else { 0 }),
                Some(0) if reverse => None,
                Some(i) if !reverse && i + 1 >= n => None,
                Some(i) => Some(if reverse { i - 1 } else { i + 1 }),
            };
            self.row_focus = next.is_some();
            if next.is_some() {
                self.model.keyed = next;
                self.model.hover = None;
            } else {
                self.window.makeFirstResponder(Some(&self.search));
            }
            self.light(false);
            self.reveal();
            return;
        }
        let key = if key == "space" && self.row_focus {
            "enter"
        } else if key == "space" {
            return;
        } else {
            key
        };
        let before = self.model.lit();
        if let Some(name) = self.model.navigate(key) {
            let _ = self.sender.try_send(Command {
                name,
                args: vec![],
                token: None,
            });
        }
        if !self.model.open {
            self.hide();
        } else if self.model.lit() != before {
            self.light(!repeat);
            self.reveal();
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
    } * review_slow();
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
                    let _: () = msg_send![&*spring,setSpeed:(1. / review_slow()) as f32];
                    let _: () = msg_send![&*layer,addAnimation:&*spring,forKey:&*NSString::from_str("pm.shell.scale")];
                }
            }
        }
    }
    duration
}
pub fn install_menus() {
    let mtm = MainThreadMarker::new().unwrap();
    // Built again when the keymap changes: the old menus' delegates go.
    MENU_DELEGATES.with(|v| v.borrow_mut().clear());
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
    let new3d_start = commands.len();
    commands.extend(menu::new3d().into_iter().map(|i| (i.command, i.args)));
    let view3d_start = commands.len();
    commands.extend(menu::view3d().into_iter().map(|i| (i.command, i.args)));
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
        // These menus dim a row whose command cannot run now (`validateMenuItem`).
        submenu.setAutoenablesItems(
            title == "Edit"
                || title == "Composition"
                || title == "Playback"
                || menu::DISPLAY_ONLY.contains(&title)
                || title == "View",
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
                "new3d" => {
                    let sub = NSMenu::new(mtm);
                    for (i, t) in menu::new3d().iter().enumerate() {
                        if t.label.is_empty() {
                            sub.addItem(&NSMenuItem::separatorItem(mtm));
                        } else {
                            sub.addItem(&row(&t.label, &t.chord, new3d_start + i));
                        }
                    }
                    delegate(&sub, MenuKind::DisplayOnly);
                    item.setSubmenu(Some(&sub));
                }
                "view3d" => {
                    let sub = NSMenu::new(mtm);
                    VIEW3D_ITEMS.with(|v| v.borrow_mut().clear());
                    for (i, t) in menu::view3d().iter().enumerate() {
                        if t.label.is_empty() {
                            sub.addItem(&NSMenuItem::separatorItem(mtm));
                        } else {
                            let item = row(&t.label, &t.chord, view3d_start + i);
                            let arg = |k: usize| {
                                t.args
                                    .get(k)
                                    .and_then(|a| a.as_str())
                                    .unwrap_or("")
                                    .to_owned()
                            };
                            VIEW3D_ITEMS.with(|v| v.borrow_mut().push((arg(0), arg(1), item.clone())));
                            sub.addItem(&item);
                        }
                    }
                    delegate(&sub, MenuKind::DisplayOnly);
                    item.setSubmenu(Some(&sub));
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
            if matches!(spec.command.as_str(), "tools" | "new3d" | "view3d") {
                // In an auto-enabling menu an item with no action is disabled;
                // its submenu's own action keeps the parent enabled.
                if let Some(sub) = item.submenu() {
                    unsafe {
                        item.setTarget(Some(&sub));
                        item.setAction(Some(sel!(submenuAction:)));
                    }
                }
            } else if menu::SUBMENUS.contains(&spec.command.as_str()) {
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
/// The fill of a selected segment, in place of the accent an active window
/// gives an "on" control: selection is neutral, the selected tab's pill
/// (paper on a light ground, a lift of the ground on a dark one). AppKit
/// would draw a title or template glyph white on it, so the control host
/// gives each segment a face in the label ink (`native_control`).
pub fn selected_color() -> Retained<NSColor> {
    const LIGHT: (f64, f64) = (1., 0.96);
    const DARK: (f64, f64) = (1., 0.16);
    let of = |(white, alpha): (f64, f64)| {
        NSColor::colorWithSRGBRed_green_blue_alpha(white, white, white, alpha)
    };
    let (light, dark) = (of(LIGHT), of(DARK));
    let provider = block2::RcBlock::new(
        move |appearance: std::ptr::NonNull<NSAppearance>| -> std::ptr::NonNull<NSColor> {
            let appearance = unsafe { appearance.as_ref() };
            let list = unsafe { [NSAppearanceNameAqua, NSAppearanceNameDarkAqua] };
            let dark_match = appearance
                .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&list))
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

/// The viewer slot's AppKit side: a view whose picture is the engine's Metal
/// layer, first as the view's own backing layer, then (once the app gives it
/// a transform) under a clipping root that pans and zooms it.
pub mod viewer_slot {
    use objc2::{msg_send, runtime::AnyObject};
    use objc2_app_kit::NSView;
    /// Puts `content` under a clipping root in the viewer's ground colour,
    /// as the view's layer.
    pub fn clip(view: &NSView, content: &metal::MetalLayerRef, ground: u32) {
        let root = objc2_quartz_core::CALayer::new();
        root.setMasksToBounds(true);
        root.setBackgroundColor(Some(&super::native_color(ground).CGColor()));
        unsafe {
            let _: () = msg_send![view, setLayer: &*root];
            let _: () = msg_send![&*root, addSublayer: foreign_types::ForeignTypeRef::as_ptr(content) as *mut AnyObject];
        }
    }
    /// Shows or hides the slot. Hiding a view hides its backing layer, and
    /// the content layer is that layer until it is clipped: a slot hidden
    /// before its first transform (a window that opened on Home) would keep
    /// its picture hidden under a shown root. Showing shows the content too.
    pub fn show(view: &NSView, content: &metal::MetalLayerRef, shown: bool) {
        view.setHidden(!shown);
        if shown {
            unsafe {
                let _: () = msg_send![foreign_types::ForeignTypeRef::as_ptr(content) as *mut AnyObject, setHidden: false];
            }
        }
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

#[cfg(test)]
mod tab_layout_tests {
    use super::{TAB_GAP, TAB_MAX, TAB_MIN, TAB_MORE, tab_layout};

    fn span(widths: &[f64]) -> f64 {
        widths.iter().sum::<f64>() + (widths.len() - 1) as f64 * TAB_GAP
    }

    #[test]
    fn tabs_with_room_are_as_wide_as_their_titles_ask() {
        assert_eq!(
            tab_layout(&[90., 120., 60.], 1, 600.),
            (0, vec![90., 120., 60.], false)
        );
        // A long name is held to the widest a tab grows.
        assert_eq!(tab_layout(&[400.], 0, 600.), (0, vec![TAB_MAX], false));
    }

    #[test]
    fn long_titles_narrow_together_and_short_ones_keep_their_width() {
        let room = 520.;
        let (start, widths, overflow) = tab_layout(&[180., 180., 60., 180.], 2, room);
        assert_eq!((start, overflow), (0, false));
        assert_eq!(widths[2], 60.);
        assert!(widths[0] < TAB_MAX && widths[0] >= TAB_MIN);
        assert_eq!((widths[0], widths[1]), (widths[1], widths[3]));
        assert!(span(&widths) <= room && span(&widths) > room - 4.);
    }

    #[test]
    fn more_projects_than_fit_show_the_run_that_holds_the_selected_one() {
        let (room, natural) = (366., [150.; 11]);
        for selected in [0, 5, 10] {
            let (start, widths, overflow) = tab_layout(&natural, selected, room);
            assert!(overflow);
            assert!((start..start + widths.len()).contains(&selected));
            assert!(start + widths.len() <= 11);
            assert!(widths.iter().all(|w| *w >= TAB_MIN));
            assert!(
                span(&widths) <= room - TAB_MORE,
                "the pull-down has its place"
            );
            assert!(
                span(&widths) > room - TAB_MORE - widths.len() as f64,
                "the run fills what the pull-down leaves"
            );
        }
        // The run slides only as far as it must: the selected tab is its last.
        let (start, widths, _) = tab_layout(&natural, 5, room);
        assert_eq!(start + widths.len(), 6);
    }

    #[test]
    fn the_pull_down_appears_only_when_the_narrowest_tabs_do_not_fit() {
        let fits = span(&[TAB_MIN; 4]);
        assert!(!tab_layout(&[200.; 4], 0, fits).2);
        assert!(tab_layout(&[200.; 5], 0, fits).2);
        // Short titles need no pull-down in the same room.
        assert!(!tab_layout(&[70.; 5], 0, fits).2);
    }

    #[test]
    fn one_tab_in_no_room_is_still_a_tab() {
        assert_eq!(tab_layout(&[150.], 0, 10.), (0, vec![TAB_MIN], false));
    }
}
