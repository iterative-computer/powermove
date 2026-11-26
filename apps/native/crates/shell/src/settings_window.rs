//! Settings as a window of its own, framed by AppKit (PM-375): the
//! sections in a real sidebar (`NSSplitViewItem(sidebarWithViewController:)`,
//! a full-width `NSTableView` under an `NSSearchField`, the account on top,
//! the pages in named groups), a page header (the section's tile and name)
//! on the traffic lights' line, and an AppKit detail scroll view below it.
//!
//! The selection is one view under the rows that glides between them with
//! Radian's spring, its insets equal on every side of the row's tile and its
//! corner concentric with the tile's. The tiles are drawn once per
//! appearance and size from SVG: Pierre's glyphs (ours in its grammar where
//! it has none) on a monochrome lit squircle.

use objc2::{
    AllocAnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class,
    msg_send, rc::Retained, runtime::AnyObject, sel,
};
use objc2_app_kit::*;
use objc2_foundation::{
    NSArray, NSIndexSet, NSInteger, NSNotification, NSNumber, NSObject, NSObjectProtocol, NSPoint,
    NSRect, NSSize, NSString, ns_string,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::{Cell, RefCell};

/// The sidebar's width (fixed; the sidebar does not collapse).
pub const SIDEBAR_WIDTH: f64 = 224.;
/// The window's default and minimum content sizes. The width is fixed (the
/// maximum width is the minimum): the page fills it, never a centred column
/// in a gutter. The height is the person's, and remembered.
pub const DEFAULT_SIZE: (f64, f64) = (780., 700.);
pub const MIN_SIZE: (f64, f64) = (780., 520.);
/// A section row: its tile with an equal inset on all four sides inside the
/// selection (Radian's 38 pt row holds a 22 pt tile 8 pt in on every side).
const ROW_HEIGHT: f64 = 38.;
/// The tile's side in a sidebar row.
const TILE: f64 = 22.;
/// The inset around a tile inside the selected row, on every side.
const TILE_INSET: f64 = (ROW_HEIGHT - TILE) / 2.;
/// The tile's corner (the squircle's equivalent radius), and the selection's
/// corner concentric with it (`Outer R = Inner R + Padding`).
const TILE_RADIUS: f64 = 6.;
const SELECTION_RADIUS: f64 = TILE_RADIUS + TILE_INSET;
/// The selection's inset from the sidebar's sides.
const SIDEBAR_INSET: f64 = 10.;
const LABEL_SIZE: f64 = 14.;
/// The account row: its avatar with the same equal inset as a tile's.
const AVATAR: f64 = 32.;
const ACCOUNT_HEIGHT: f64 = 52.;
const AVATAR_INSET: f64 = (ACCOUNT_HEIGHT - AVATAR) / 2.;
/// A group's header row ("Agents & Providers"): air above, its name low.
const HEADER_HEIGHT: f64 = 32.;
/// The page header's tile, on the traffic lights' line.
const HEADER_TILE: f64 = 24.;
/// Dark mode's black veil over the sidebar material (#4A over the material
/// alone lands near #2E).
const DARK_VEIL: f64 = 0.37;
/// The selection's glide, measured from Radian's (critically damped,
/// settles in about 0.2 s): unit mass, stiffness and damping.
const GLIDE: (f64, f64) = (900., 57.);
const FRAME_NAME: &str = "PMSettingsWindow2";
/// The Settings window's identifier (its title is the section's name).
const IDENTIFIER: &str = "pm.settings";

/// Whether `window` is the Settings window.
pub fn is_settings(window: &NSWindow) -> bool {
    window
        .identifier()
        .is_some_and(|id| id.to_string() == IDENTIFIER)
}

/// The AppKit surface shades that give a Settings group a lighter face.
pub fn form_background(group: bool) -> Retained<NSColor> {
    let window = NSColor::windowBackgroundColor();
    let under_page = NSColor::underPageBackgroundColor();
    let provider = block2::RcBlock::new(move |appearance: std::ptr::NonNull<NSAppearance>| {
        // Under-page is the softer ground in Aqua and the raised shade in Dark Aqua.
        let color: &NSColor = if is_dark(unsafe { appearance.as_ref() }) == group {
            &under_page
        } else {
            &window
        };
        // The block retains both system colours for its lifetime.
        std::ptr::NonNull::from(color)
    });
    unsafe { NSColor::colorWithName_dynamicProvider(None, &provider) }
}

fn is_dark(appearance: &NSAppearance) -> bool {
    let names = unsafe { [NSAppearanceNameAqua, NSAppearanceNameDarkAqua] };
    appearance
        .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&names))
        .is_some_and(|name| &*name == names[1])
}

/// One row of the sidebar.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub account_name: Option<String>,
    /// The signed-in account's email, under its name on the account row.
    pub account_email: Option<String>,
    /// A search is on and nothing in this section matches it.
    pub dim: bool,
}

/// What the person did in the window's chrome.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Select(String),
    Search(String),
    Escape,
    Back,
    Forward,
}

/// The sidebar's named groups. The app's own pages come first, unnamed
/// (as Radian's); a section not listed here belongs with them.
pub fn group_of(id: &str) -> Option<&'static str> {
    match id {
        "providers" | "agents" | "transcription" => Some("Agents & Providers"),
        "extensions" | "privacy" | "about" => Some("Powermove"),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Section(Section),
    /// The air under the account row.
    Gap,
    /// A group's name.
    Header(&'static str),
}
impl Row {
    fn section(&self) -> Option<&Section> {
        match self {
            Row::Section(s) => Some(s),
            _ => None,
        }
    }
}

/// The rows the sidebar lists for `sections`: the account, then each group
/// under its name.
fn rows(sections: &[Section]) -> Vec<Row> {
    let mut rows = vec![];
    let mut group = None;
    for (i, s) in sections.iter().enumerate() {
        if i > 0 && sections[i - 1].id == "account" {
            rows.push(Row::Gap);
        }
        if s.id != "account" {
            let next = group_of(&s.id);
            if next.is_some() && next != group {
                rows.push(Row::Header(next.unwrap_or_default()));
            }
            group = next;
        }
        rows.push(Row::Section(s.clone()));
    }
    rows
}

define_class!(
    #[unsafe(super(NSTableRowView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSettingsSidebarRow"]
    struct SidebarRow;

    impl SidebarRow {
        #[unsafe(method(setEmphasized:))]
        fn set_emphasized(&self, _: bool) {
            // Keep AppKit's unemphasized selection when the window becomes key.
            unsafe { let _: () = msg_send![super(self), setEmphasized: false]; }
        }
        /// The selection is the one gliding view under the rows.
        #[unsafe(method(drawSelectionInRect:))]
        fn draw_selection(&self, _: NSRect) {}
    }
);

define_class!(
    /// The sidebar's list. Its cells span their rows edge to edge (AppKit
    /// insets them otherwise), so a tile's place in its row is ours exactly.
    /// It also draws the hover: a faint wash under the row the pointer is
    /// on, arriving quickly and leaving slowly (quiet-md's hover springs).
    #[unsafe(super(NSTableView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSettingsSidebar"]
    #[ivars = Hover]
    struct Sidebar;

    impl Sidebar {
        #[unsafe(method(frameOfCellAtColumn:row:))]
        fn cell_frame(&self, _: NSInteger, row: NSInteger) -> NSRect {
            self.rectOfRow(row)
        }
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let at = self.convertPoint_fromView(event.locationInWindow(), None);
            self.hover(self.rowAtPoint(at));
        }
        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _: &NSEvent) {
            self.hover(-1);
        }
    }
);

#[derive(Default)]
struct Hover {
    wash: RefCell<Option<Retained<NSView>>>,
    /// The row the pointer is on (-1: none).
    row: Cell<NSInteger>,
}

/// quiet-md's hover springs: in quickly, out slowly (unit mass).
const HOVER_IN: (f64, f64) = (1100., 66.);
const HOVER_OUT: (f64, f64) = (300., 35.);
const HOVER_SWITCH: (f64, f64) = (2500., 100.);
/// The hover wash's opacity over the selection's colour.
const HOVER_OPACITY: f32 = 0.5;

impl Sidebar {
    /// The pointer is on `row`: the wash goes under it, unless the row is
    /// the selected one or no section's.
    fn hover(&self, row: NSInteger) {
        self.hover_with(row, false);
    }
    /// `arriving`: the selection is gliding onto the hovered row, whose wash
    /// leaves at once (in about 80 ms) so the two never read as one band.
    fn hover_with(&self, row: NSInteger, arriving: bool) {
        use objc2_quartz_core::{CAMediaTiming, CATransaction};
        let section = row >= 0 && row < self.numberOfRows() && {
            let delegate: *mut AnyObject = unsafe { msg_send![self, delegate] };
            !delegate.is_null()
                && unsafe { msg_send![delegate, tableView: self, shouldSelectRow: row] }
        };
        let target = if section && row != self.selectedRow() {
            row
        } else {
            -1
        };
        if self.ivars().row.replace(target) == target {
            return;
        }
        let Some(wash) = self.ivars().wash.borrow().clone() else {
            return;
        };
        let Some(layer) = wash.layer() else { return };
        let shown = unsafe { layer.presentationLayer() };
        let mut from = shown.as_deref().unwrap_or(&layer).opacity();
        let to = if target >= 0 { HOVER_OPACITY } else { 0. };
        if target >= 0 {
            let rect = self.rectOfRow(target);
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            wash.setFrame(NSRect::new(
                NSPoint::new(SIDEBAR_INSET, rect.origin.y),
                NSSize::new(
                    self.bounds().size.width - 2. * SIDEBAR_INSET,
                    rect.size.height,
                ),
            ));
            layer.setCornerRadius(if rect.size.height > ROW_HEIGHT {
                rect.size.height / 2.
            } else {
                SELECTION_RADIUS
            });
            CATransaction::commit();
            // A new row's wash arrives from nothing (two rows never share one).
            from = 0.;
        }
        layer.setOpacity(to);
        let (stiffness, damping) = if target >= 0 {
            HOVER_IN
        } else if arriving {
            HOVER_SWITCH
        } else {
            HOVER_OUT
        };
        let spring = objc2_quartz_core::CASpringAnimation::animationWithKeyPath(Some(
            &NSString::from_str("opacity"),
        ));
        spring.setMass(1.);
        spring.setStiffness(stiffness);
        spring.setDamping(damping);
        unsafe {
            spring.setFromValue(Some(&NSNumber::new_f32(from)));
            spring.setToValue(Some(&NSNumber::new_f32(to)));
        }
        spring.setDuration(spring.settlingDuration());
        layer.addAnimation_forKey(&spring, Some(&NSString::from_str("pm.settings.hover")));
    }
}

define_class!(
    /// The selected row's fill: one view under the rows that glides between
    /// them. Clicks go through it to the rows.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSettingsSelection"]
    struct Selection;

    impl Selection {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _: NSPoint) -> *mut NSView { std::ptr::null_mut() }
    }
);

pub struct State {
    sender: async_channel::Sender<Event>,
    rows: RefCell<Vec<Row>>,
    table: RefCell<Option<Retained<NSTableView>>>,
    search: RefCell<Option<Retained<NSSearchField>>>,
    selection: RefCell<Option<Retained<NSView>>>,
    /// The glide in flight: when it began and its curve (for the velocity a
    /// new glide starts with).
    glide: Cell<Option<(std::time::Instant, pm_design::motion::Analytic)>>,
    /// The window's appearance: which tiles the rows show.
    dark: Cell<bool>,
    /// Set while the host changes the selection, so it is not sent back.
    quiet: Cell<bool>,
    /// The row the person just chose, until the page shows it: the panel
    /// may still say the old page once, and the selection must not glide
    /// back to it and out again.
    chosen: RefCell<Option<(String, std::time::Instant)>>,
    /// The sidebar's material and dark mode's veil over it.
    material: RefCell<Option<(Retained<NSVisualEffectView>, Retained<NSView>)>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSettingsChrome"]
    #[ivars = State]
    pub struct Chrome;
    unsafe impl NSObjectProtocol for Chrome {}
    impl Chrome {
        // NSTableViewDataSource.
        #[unsafe(method(numberOfRowsInTableView:))]
        fn count(&self, _: &NSTableView) -> NSInteger {
            self.ivars().rows.borrow().len() as NSInteger
        }
        // NSTableViewDelegate.
        #[unsafe(method(tableView:heightOfRow:))]
        fn height(&self, _: &NSTableView, row: NSInteger) -> f64 {
            match self.ivars().rows.borrow().get(row as usize) {
                Some(Row::Gap) => 8.,
                Some(Row::Header(_)) => HEADER_HEIGHT,
                Some(Row::Section(s)) if s.id == "account" => ACCOUNT_HEIGHT,
                _ => ROW_HEIGHT,
            }
        }
        #[unsafe(method(tableView:shouldSelectRow:))]
        fn selectable(&self, _: &NSTableView, row: NSInteger) -> bool {
            matches!(self.ivars().rows.borrow().get(row as usize), Some(Row::Section(_)))
        }
        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn row_view(&self, _: &NSTableView, _: NSInteger) -> Retained<NSTableRowView> {
            let row: Retained<SidebarRow> = unsafe {
                msg_send![super(SidebarRow::alloc(MainThreadMarker::from(self)).set_ivars(())), initWithFrame: NSRect::ZERO]
            };
            row.setEmphasized(false);
            Retained::into_super(row)
        }
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn cell(&self, _: &NSTableView, _: *mut AnyObject, row: NSInteger) -> Option<Retained<NSView>> {
            let mtm = MainThreadMarker::from(self);
            match self.ivars().rows.borrow().get(row as usize) {
                Some(Row::Section(section)) => Some(cell(mtm, section, self.ivars().dark.get())),
                Some(Row::Header(name)) => Some(header_cell(mtm, name)),
                _ => None,
            }
        }
        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_changed(&self, _: &NSNotification) {
            if self.ivars().quiet.get() { return; }
            self.place_selection(true);
            let Some(table) = self.ivars().table.borrow().clone() else { return };
            if let Some(list) = table.downcast_ref::<Sidebar>() {
                // The row just chosen loses its hover wash to the selection.
                let row = list.ivars().row.replace(-2);
                list.hover_with(row, true);
            }
            let row = table.selectedRow();
            if row < 0 { return; }
            let id = self.ivars().rows.borrow().get(row as usize)
                .and_then(|r| r.section().map(|s| s.id.clone()));
            if let Some(id) = id {
                *self.ivars().chosen.borrow_mut() = Some((id.clone(), std::time::Instant::now()));
                let _ = self.ivars().sender.try_send(Event::Select(id));
            }
        }
        // NSToolbarDelegate: the toolbar only sizes the title bar (the
        // traffic lights and the page header share its line).
        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_items(&self, _: &NSToolbar) -> Retained<NSArray<NSString>> { items() }
        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_items(&self, _: &NSToolbar) -> Retained<NSArray<NSString>> { items() }
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn toolbar_item(&self, _: &NSToolbar, _: &NSString, _: bool) -> Option<Retained<NSToolbarItem>> {
            None
        }
        // Actions.
        #[unsafe(method(searched:))]
        fn searched(&self, sender: &NSSearchField) {
            let _ = self.ivars().sender.try_send(Event::Search(sender.stringValue().to_string()));
        }
    }
);

fn items() -> Retained<NSArray<NSString>> {
    let separator: &NSString = unsafe { NSToolbarSidebarTrackingSeparatorItemIdentifier };
    NSArray::from_slice(&[separator])
}

impl Chrome {
    /// Puts the selection's fill under the selected row: gliding there with
    /// Radian's spring when `animate` (interrupted glides keep their speed),
    /// at once otherwise; a cross-fade with Reduce Motion.
    fn place_selection(&self, animate: bool) {
        use objc2_quartz_core::{CAMediaTiming, CATransaction};
        let state = self.ivars();
        let (Some(table), Some(fill)) = (
            state.table.borrow().clone(),
            state.selection.borrow().clone(),
        ) else {
            return;
        };
        let row = table.selectedRow();
        let kind = (row >= 0)
            .then(|| {
                state
                    .rows
                    .borrow()
                    .get(row as usize)
                    .and_then(Row::section)
                    .map(|s| s.id == "account")
            })
            .flatten();
        let Some(account) = kind else {
            fill.setHidden(true);
            return;
        };
        let rect = table.rectOfRow(row);
        let target = NSRect::new(
            NSPoint::new(SIDEBAR_INSET, rect.origin.y),
            NSSize::new(
                table.bounds().size.width - 2. * SIDEBAR_INSET,
                rect.size.height,
            ),
        );
        // The avatar is round: its selection is a capsule around it.
        let radius = if account {
            ACCOUNT_HEIGHT / 2.
        } else {
            SELECTION_RADIUS
        };
        let appeared = fill.isHidden();
        fill.setHidden(false);
        let Some(layer) = fill.layer() else {
            fill.setFrame(target);
            return;
        };
        // What is on screen now (mid-glide, the presentation).
        let shown = unsafe { layer.presentationLayer() };
        let from = shown.as_deref().unwrap_or(&layer);
        let (from_y, from_h, from_r) = (
            from.position().y,
            from.bounds().size.height,
            from.cornerRadius(),
        );
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        fill.setFrame(target);
        layer.setCornerRadius(radius);
        CATransaction::commit();
        let to_y = layer.position().y;
        let moved = (from_y - to_y).abs() > 0.25 || (from_h - rect.size.height).abs() > 0.25;
        if !animate || appeared || !moved {
            layer.removeAllAnimations();
            state.glide.set(None);
            return;
        }
        let key = |k: &str| NSString::from_str(k);
        let number = |v: f64| NSNumber::new_f64(v);
        if NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion() {
            layer.removeAllAnimations();
            state.glide.set(None);
            let fade =
                objc2_quartz_core::CABasicAnimation::animationWithKeyPath(Some(&key("opacity")));
            unsafe {
                fade.setFromValue(Some(&number(0.)));
                fade.setToValue(Some(&number(1.)));
            }
            fade.setDuration(0.15);
            layer.addAnimation_forKey(&fade, Some(&key("pm.settings.fade")));
            return;
        }
        let (stiffness, damping) = GLIDE;
        // The speed it moves at now carries into the new glide.
        let velocity = state.glide.get().map_or(0., |(start, curve)| {
            curve.sample(start.elapsed().as_secs_f64()).1
        });
        state.glide.set(Some((
            std::time::Instant::now(),
            pm_design::motion::Analytic {
                from: from_y,
                velocity,
                target: to_y,
                mass: 1.,
                stiffness,
                damping,
            },
        )));
        for (path, from, to, speed) in [
            ("position.y", from_y, to_y, velocity),
            ("bounds.size.height", from_h, rect.size.height, 0.),
            ("cornerRadius", from_r, radius, 0.),
        ] {
            if (from - to).abs() < 0.01 {
                continue;
            }
            let spring =
                objc2_quartz_core::CASpringAnimation::animationWithKeyPath(Some(&key(path)));
            spring.setMass(1.);
            spring.setStiffness(stiffness);
            spring.setDamping(damping);
            // Core Animation's initial velocity is in distances per second.
            spring.setInitialVelocity(speed / (to - from));
            unsafe {
                spring.setFromValue(Some(&number(from)));
                spring.setToValue(Some(&number(to)));
            }
            spring.setDuration(spring.settlingDuration());
            layer.addAnimation_forKey(&spring, Some(&key(&format!("pm.settings.{path}"))));
        }
    }

    /// The selection's fill in the window's appearance.
    fn paint_selection(&self, appearance: &NSAppearance) {
        let Some(fill) = self.ivars().selection.borrow().clone() else {
            return;
        };
        let wash = self.ivars().table.borrow().as_ref().and_then(|t| {
            t.downcast_ref::<Sidebar>()
                .and_then(|s| s.ivars().wash.borrow().clone())
        });
        let layers = [fill.layer(), wash.and_then(|w| w.layer())];
        appearance.performAsCurrentDrawingAppearance(&block2::RcBlock::new(|| {
            let colour = NSColor::quaternaryLabelColor().CGColor();
            for layer in layers.iter().flatten() {
                layer.setBackgroundColor(Some(&colour));
            }
        }));
    }
}

/// The chrome of an open Settings window. Dropping it leaves the window as
/// it is; the window's views keep what they show.
pub struct SettingsChrome {
    chrome: Retained<Chrome>,
    window: Retained<NSWindow>,
    /// The page's ground, beside the sidebar (under the toolbar too): nothing
    /// opaque sits under the sidebar's glass.
    ground: Retained<NSBox>,
    page: Retained<NSScrollView>,
    /// The page's header on the traffic lights' line: its tile and name.
    header_tile: Retained<NSImageView>,
    header_title: Retained<NSTextField>,
    header_id: RefCell<String>,
    /// The window's own keys (⌘F, ⌘[ ⌘], typing in the sidebar).
    monitor: Option<Retained<AnyObject>>,
}

impl Drop for SettingsChrome {
    fn drop(&mut self) {
        if let Some(monitor) = self.monitor.take() {
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
    }
}

/// Settings' own keys, as System Settings has them: ⌘F goes to the search
/// field, and typing while the sidebar has the keyboard starts a search
/// with what was typed (never the list's jump to a row by name); ⌘[ and ⌘]
/// go back and forward.
fn keys(
    window: &NSWindow,
    search: &NSSearchField,
    table: &NSTableView,
    sender: async_channel::Sender<Event>,
) -> Option<Retained<AnyObject>> {
    let window = objc2::rc::Weak::from(window);
    let search = objc2::rc::Weak::from(search);
    let table = objc2::rc::Weak::from(table);
    let handler = block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
        let e = unsafe { event.as_ref() };
        let (Some(window), Some(search), Some(table)) =
            (window.load(), search.load(), table.load())
        else {
            return event.as_ptr();
        };
        if e.windowNumber() != window.windowNumber() {
            return event.as_ptr();
        }
        let flags = e.modifierFlags();
        let command = flags.contains(NSEventModifierFlags::Command);
        let chars = e
            .charactersIgnoringModifiers()
            .map(|c| c.to_string())
            .unwrap_or_default();
        let plain_command = command
            && !flags.intersects(
                NSEventModifierFlags::Shift
                    | NSEventModifierFlags::Option
                    | NSEventModifierFlags::Control,
            );
        if plain_command && chars == "f" {
            window.makeFirstResponder(Some(&search));
            return std::ptr::null_mut();
        }
        // Back and forward through the pages visited (the window has no
        // toolbar buttons for them).
        if plain_command && (chars == "[" || chars == "]") {
            let _ = sender.try_send(if chars == "[" {
                Event::Back
            } else {
                Event::Forward
            });
            return std::ptr::null_mut();
        }
        if e.keyCode() == 53
            && !window
                .firstResponder()
                .is_some_and(|r| r.isKindOfClass(NSTextView::class()))
        {
            let _ = sender.try_send(Event::Escape);
            return std::ptr::null_mut();
        }
        let typed = e.characters().map(|c| c.to_string()).unwrap_or_default();
        let sidebar = window.firstResponder().is_some_and(|r| {
            std::ptr::eq(
                Retained::as_ptr(&r).cast::<AnyObject>(),
                Retained::as_ptr(&table).cast(),
            )
        });
        let printable = !typed.is_empty()
            && typed
                .chars()
                .all(|c| !c.is_control() && !('\u{f700}'..='\u{f8ff}').contains(&c));
        if sidebar
            && printable
            && !flags.intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Control)
        {
            window.makeFirstResponder(Some(&search));
            if let Some(editor) = search.currentEditor() {
                unsafe { editor.insertText(&NSString::from_str(&typed)) };
            }
            return std::ptr::null_mut();
        }
        event.as_ptr()
    });
    unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler) }
}

impl SettingsChrome {
    /// Frames the GPUI window `w` as Settings: its content view (GPUI's view
    /// and the accessibility host that holds it) becomes the detail item.
    ///
    /// Call it outside any update of that GPUI window (take the view with
    /// [`gpui_view`] inside one): the frame ends with a layout, and GPUI must
    /// be free to hear its view's new size.
    pub fn install(
        gpui: Retained<NSView>,
        sender: async_channel::Sender<Event>,
    ) -> Option<SettingsChrome> {
        let mtm = MainThreadMarker::new()?;
        let window = gpui.window()?;
        let content = unsafe { gpui.superview() }?;
        let chrome: Retained<Chrome> = {
            let this = Chrome::alloc(mtm).set_ivars(State {
                sender,
                rows: RefCell::new(vec![]),
                table: RefCell::new(None),
                search: RefCell::new(None),
                selection: RefCell::new(None),
                glide: Cell::new(None),
                dark: Cell::new(true),
                quiet: Cell::new(false),
                chosen: RefCell::new(None),
                material: RefCell::new(None),
            });
            unsafe { msg_send![super(this), init] }
        };

        // The window: titled, its content under a unified title bar whose
        // line the page's header shares with the traffic lights. The width
        // is fixed; the height is the person's.
        window.setStyleMask(
            window.styleMask()
                | NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable
                | NSWindowStyleMask::FullSizeContentView,
        );
        window.setIdentifier(Some(&NSString::from_str(IDENTIFIER)));
        window.setTabbingMode(NSWindowTabbingMode::Disallowed);
        window.setTitlebarAppearsTransparent(true);
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        let mut frame = window.frame();
        frame.size.width = frame.size.width.max(DEFAULT_SIZE.0);

        // The sidebar: the search field over the list.
        let sidebar = NSView::new(mtm);
        let search = NSSearchField::new(mtm);
        search.setControlSize(NSControlSize::Large);
        search.setFocusRingType(NSFocusRingType::None);
        search.setPlaceholderString(Some(ns_string!("Search")));
        search.setSendsSearchStringImmediately(true);
        unsafe {
            search.setTarget(Some(&chrome));
            search.setAction(Some(sel!(searched:)));
        }
        search.setTranslatesAutoresizingMaskIntoConstraints(false);
        let table: Retained<Sidebar> = unsafe {
            msg_send![super(Sidebar::alloc(mtm).set_ivars(Hover::default())), initWithFrame: NSRect::ZERO]
        };
        let sidebar_list = table.clone();
        let table: Retained<NSTableView> = Retained::into_super(table);
        table.setFocusRingType(NSFocusRingType::None);
        let column =
            NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("section"));
        column.setResizingMask(NSTableColumnResizingOptions::AutoresizingMask);
        table.addTableColumn(&column);
        table.setHeaderView(None);
        // Full width: rows run edge to edge, so the selection's insets are
        // ours, equal on every side of the tile (AppKit's source-list
        // selection was wider at the sides than above and below).
        table.setStyle(NSTableViewStyle::FullWidth);
        table.setBackgroundColor(&NSColor::clearColor());
        table.setColumnAutoresizingStyle(
            NSTableViewColumnAutoresizingStyle::UniformColumnAutoresizingStyle,
        );
        table.setRowSizeStyle(NSTableViewRowSizeStyle::Custom);
        table.setIntercellSpacing(NSSize::new(0., 0.));
        table.setAllowsEmptySelection(false);
        table.setGridStyleMask(NSTableViewGridLineStyle::GridNone);
        unsafe {
            let _: () = msg_send![&table, setDataSource: &*chrome];
            let _: () = msg_send![&table, setDelegate: &*chrome];
        }
        // The selected row's fill, under every row.
        let selection: Retained<Selection> = unsafe {
            msg_send![super(Selection::alloc(mtm).set_ivars(())), initWithFrame: NSRect::ZERO]
        };
        let selection: Retained<NSView> = Retained::into_super(selection);
        selection.setWantsLayer(true);
        if let Some(layer) = selection.layer() {
            layer.setCornerRadius(SELECTION_RADIUS);
            layer.setCornerCurve(unsafe { objc2_quartz_core::kCACornerCurveContinuous });
        }
        selection.setHidden(true);
        table.addSubview_positioned_relativeTo(&selection, NSWindowOrderingMode::Below, None);
        // The hover wash, under the selection; the pointer is tracked over
        // the list while the window is open, key or not.
        let wash: Retained<Selection> = unsafe {
            msg_send![super(Selection::alloc(mtm).set_ivars(())), initWithFrame: NSRect::ZERO]
        };
        let wash: Retained<NSView> = Retained::into_super(wash);
        wash.setWantsLayer(true);
        if let Some(layer) = wash.layer() {
            layer.setCornerCurve(unsafe { objc2_quartz_core::kCACornerCurveContinuous });
            layer.setOpacity(0.);
        }
        table.addSubview_positioned_relativeTo(&wash, NSWindowOrderingMode::Below, None);
        *sidebar_list.ivars().wash.borrow_mut() = Some(wash);
        sidebar_list.ivars().row.set(-1);
        let tracking = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::ZERO,
                NSTrackingAreaOptions::MouseMoved
                    | NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(&table),
                None,
            )
        };
        table.addTrackingArea(&tracking);
        let scroll = NSScrollView::new(mtm);
        scroll.setDocumentView(Some(&table));
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setScrollerStyle(NSScrollerStyle::Overlay);
        scroll.setTranslatesAutoresizingMaskIntoConstraints(false);
        // The system's sidebar material under the search and the list: on
        // its own the floating glass shows whatever is behind the window,
        // sharp, in light mode (a seam where another window ends).
        let material = NSVisualEffectView::new(mtm);
        material.setMaterial(NSVisualEffectMaterial::Sidebar);
        material.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        material.setState(NSVisualEffectState::FollowsWindowActiveState);
        material.setTranslatesAutoresizingMaskIntoConstraints(false);
        sidebar.addSubview(&material);
        // Dark mode's sidebar is held near #2E2E2E (Raycast's depth): the
        // material alone lifts to a washed-out grey when the window takes
        // focus. A black veil over it, and the material held active, so
        // focus changes nothing. Light mode keeps the material as it is.
        let veil = NSView::new(mtm);
        veil.setWantsLayer(true);
        veil.setTranslatesAutoresizingMaskIntoConstraints(false);
        sidebar.addSubview(&veil);
        sidebar.addSubview(&search);
        sidebar.addSubview(&scroll);
        let guide = sidebar.safeAreaLayoutGuide();
        activate(&[
            material
                .topAnchor()
                .constraintEqualToAnchor(&sidebar.topAnchor()),
            material
                .bottomAnchor()
                .constraintEqualToAnchor(&sidebar.bottomAnchor()),
            material
                .leadingAnchor()
                .constraintEqualToAnchor(&sidebar.leadingAnchor()),
            material
                .trailingAnchor()
                .constraintEqualToAnchor(&sidebar.trailingAnchor()),
            veil.topAnchor()
                .constraintEqualToAnchor(&sidebar.topAnchor()),
            veil.bottomAnchor()
                .constraintEqualToAnchor(&sidebar.bottomAnchor()),
            veil.leadingAnchor()
                .constraintEqualToAnchor(&sidebar.leadingAnchor()),
            veil.trailingAnchor()
                .constraintEqualToAnchor(&sidebar.trailingAnchor()),
            search
                .topAnchor()
                .constraintEqualToAnchor(&guide.topAnchor()),
            search
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&sidebar.leadingAnchor(), SIDEBAR_INSET),
            search
                .trailingAnchor()
                .constraintEqualToAnchor_constant(&sidebar.trailingAnchor(), -SIDEBAR_INSET),
            scroll
                .topAnchor()
                .constraintEqualToAnchor_constant(&search.bottomAnchor(), 10.),
            scroll
                .leadingAnchor()
                .constraintEqualToAnchor(&sidebar.leadingAnchor()),
            scroll
                .trailingAnchor()
                .constraintEqualToAnchor(&sidebar.trailingAnchor()),
            scroll
                .bottomAnchor()
                .constraintEqualToAnchor(&sidebar.bottomAnchor()),
        ]);

        // GPUI still owns the window's lifetime. AppKit owns the whole detail.
        gpui.setHidden(true);
        let ground = NSBox::new(mtm);
        ground.setBoxType(NSBoxType::Custom);
        ground.setTitlePosition(NSTitlePosition::NoTitle);
        ground.setBorderWidth(0.);
        ground.setFillColor(&form_background(false));
        ground.setTranslatesAutoresizingMaskIntoConstraints(false);
        content.addSubview(&ground);
        let page = NSScrollView::new(mtm);
        page.setDrawsBackground(false);
        page.setHasVerticalScroller(true);
        page.setAutohidesScrollers(true);
        page.setTranslatesAutoresizingMaskIntoConstraints(false);
        content.addSubview(&page);
        // The page's header: its tile and name on the traffic lights' line,
        // in the title bar's band above the page.
        let header_tile = NSImageView::new(mtm);
        header_tile.setImageScaling(NSImageScaling::ScaleNone);
        header_tile.setTranslatesAutoresizingMaskIntoConstraints(false);
        let header_title = NSTextField::labelWithString(ns_string!(""), mtm);
        header_title.setFont(Some(&NSFont::systemFontOfSize_weight(16., unsafe {
            NSFontWeightBold
        })));
        header_title.setTranslatesAutoresizingMaskIntoConstraints(false);
        content.addSubview(&header_tile);
        content.addSubview(&header_title);
        let safe = content.safeAreaLayoutGuide();
        let band = NSLayoutGuide::new();
        content.addLayoutGuide(&band);
        let (tile_w, tile_h) = tile_size(HEADER_TILE);
        let margin = tile_margin(HEADER_TILE);
        activate(&[
            ground
                .topAnchor()
                .constraintEqualToAnchor(&content.topAnchor()),
            ground
                .leadingAnchor()
                .constraintEqualToAnchor(&safe.leadingAnchor()),
            ground
                .trailingAnchor()
                .constraintEqualToAnchor(&content.trailingAnchor()),
            ground
                .bottomAnchor()
                .constraintEqualToAnchor(&content.bottomAnchor()),
            page.topAnchor().constraintEqualToAnchor(&safe.topAnchor()),
            page.leadingAnchor()
                .constraintEqualToAnchor(&safe.leadingAnchor()),
            page.trailingAnchor()
                .constraintEqualToAnchor(&content.trailingAnchor()),
            page.bottomAnchor()
                .constraintEqualToAnchor(&content.bottomAnchor()),
            band.topAnchor()
                .constraintEqualToAnchor(&content.topAnchor()),
            band.bottomAnchor()
                .constraintEqualToAnchor(&safe.topAnchor()),
            header_tile.widthAnchor().constraintEqualToConstant(tile_w),
            header_tile.heightAnchor().constraintEqualToConstant(tile_h),
            // The tile on the page's leading edge (the cards'), its centre on
            // the band's (the image carries the shadow's margin below).
            header_tile
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&safe.leadingAnchor(), PAGE_INSET - margin),
            header_tile
                .centerYAnchor()
                .constraintEqualToAnchor_constant(
                    &band.centerYAnchor(),
                    tile_centre_offset(HEADER_TILE),
                ),
            header_title
                .leadingAnchor()
                .constraintEqualToAnchor_constant(
                    &header_tile.leadingAnchor(),
                    margin + HEADER_TILE + 8.,
                ),
            header_title
                .centerYAnchor()
                .constraintEqualToAnchor(&band.centerYAnchor()),
        ]);
        // The sidebar is the system's own material, blended with what is
        // behind the window: GPUI made the window opaque with an opaque
        // background (the window server then has nothing behind it to show),
        // so the window is clear and only the page's ground is painted.
        window.setOpaque(false);
        window.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
            0., 0., 0., 0.0001, // not clearColor: that drops the window's shadow
        )));

        // The split view: the content view leaves the window for the detail
        // item (the window holds a stand-in meanwhile).
        window.setContentView(Some(&NSView::new(mtm)));
        let split = NSSplitViewController::new(mtm);
        let side = NSViewController::new(mtm);
        side.setView(&sidebar);
        let side = NSSplitViewItem::sidebarWithViewController(&side);
        side.setCanCollapse(false);
        side.setMinimumThickness(SIDEBAR_WIDTH);
        side.setMaximumThickness(SIDEBAR_WIDTH);
        side.setAllowsFullHeightLayout(true);
        let detail = NSViewController::new(mtm);
        detail.setView(&content);
        let detail = NSSplitViewItem::splitViewItemWithViewController(&detail);
        // The system's sidebar sits over the detail, which runs under it
        // (macOS 26 and later) unpainted: its glass shows what is behind the
        // window, as System Settings' does.
        if detail.respondsToSelector(sel!(setAutomaticallyAdjustsSafeAreaInsets:)) {
            detail.setAutomaticallyAdjustsSafeAreaInsets(true);
        }
        split.addSplitViewItem(&side);
        split.addSplitViewItem(&detail);
        window.setContentViewController(Some(&split));

        // A toolbar with no items: it gives the title bar its unified height,
        // so the traffic lights, the search field and the page header line up.
        let toolbar =
            NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), ns_string!("PMSettings"));
        unsafe {
            let _: () = msg_send![&toolbar, setDelegate: &*chrome];
        }
        toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
        toolbar.setAllowsUserCustomization(false);
        window.setToolbar(Some(&toolbar));
        window.setToolbarStyle(NSWindowToolbarStyle::Unified);

        // Standard window behaviour: the size it was left at, its limits.
        window.setContentMinSize(NSSize::new(MIN_SIZE.0, MIN_SIZE.1));
        window.setContentMaxSize(NSSize::new(MIN_SIZE.0, f64::MAX));
        window.setFrame_display(frame, true);
        window.setFrameAutosaveName(&NSString::from_str(FRAME_NAME));
        window.setAutorecalculatesKeyViewLoop(true);
        window.makeFirstResponder(Some(&table));
        let monitor = keys(&window, &search, &table, chrome.ivars().sender.clone());

        *chrome.ivars().table.borrow_mut() = Some(table);
        *chrome.ivars().search.borrow_mut() = Some(search);
        *chrome.ivars().selection.borrow_mut() = Some(selection);
        *chrome.ivars().material.borrow_mut() = Some((material, veil));
        chrome.paint_selection(&window.effectiveAppearance());
        // GPUI's view takes its place in the detail now, so GPUI lays the
        // page out at the detail's size from the first frame.
        if let Some(content) = window.contentView() {
            content.layoutSubtreeIfNeeded();
        }
        Some(SettingsChrome {
            chrome,
            window,
            ground,
            page,
            header_tile,
            header_title,
            header_id: RefCell::default(),
            monitor,
        })
    }

    /// The sidebar lists `sections` with `active` selected; the header and
    /// the title name it.
    pub fn show(&self, sections: &[Section], active: &str) {
        let state = self.chrome.ivars();
        let Some(table) = state.table.borrow().clone() else {
            return;
        };
        let rows = rows(sections);
        if *state.rows.borrow() != rows {
            *state.rows.borrow_mut() = rows;
            state.quiet.set(true);
            table.reloadData();
            state.quiet.set(false);
            self.chrome.place_selection(false);
        }
        // A page the person did not choose, said while their choice is on its
        // way, leaves the selection where they put it.
        let stale = {
            let mut chosen = state.chosen.borrow_mut();
            match chosen.as_ref() {
                Some((id, _)) if id == active => {
                    *chosen = None;
                    false
                }
                Some((_, at)) => at.elapsed() < std::time::Duration::from_secs(1),
                None => false,
            }
        };
        let row = state
            .rows
            .borrow()
            .iter()
            .position(|r| r.section().is_some_and(|s| s.id == active))
            .filter(|_| !stale);
        if let Some(row) = row
            && table.selectedRow() != row as NSInteger
        {
            state.quiet.set(true);
            table.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row), false);
            state.quiet.set(false);
            // Back, forward and a search's pick glide as a click does.
            self.chrome.place_selection(true);
        }
        if let Some(title) = sections.iter().find(|s| s.id == active).map(|s| &s.title) {
            self.window.setTitle(&NSString::from_str(title));
            self.header_title.setStringValue(&NSString::from_str(title));
            *self.header_id.borrow_mut() = active.to_owned();
            self.header_tile
                .setImage(tile_image(active, state.dark.get(), HEADER_TILE).as_deref());
        }
    }

    pub fn page(&self) -> &NSScrollView {
        &self.page
    }

    /// What the window is made of, for a check against any build (the test,
    /// and `PM_NATIVE_SETTINGS_PROBE=<file>` in a release bundle): whether
    /// the window is clear, whether the sidebar is inside the system's glass,
    /// whether anything under the sidebar is painted, and the selection's
    /// insets around the selected row's tile.
    pub fn report(&self) -> serde_json::Value {
        let alpha = |c: &NSColor| {
            c.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
                .map_or(1., |c| c.alphaComponent())
        };
        let painted = |v: &NSView| {
            v.layer()
                .and_then(|l| l.backgroundColor())
                .and_then(|c| NSColor::colorWithCGColor(&c))
                .is_some_and(|c| alpha(&c) > 0.01)
        };
        let mut glass = false;
        let mut detail_painted = false;
        let table = self.chrome.ivars().table.borrow().clone();
        if let Some(table) = table.as_ref()
            && let Some(class) = objc2::runtime::AnyClass::get(c"NSGlassEffectView")
        {
            let mut up = unsafe { table.superview() };
            while let Some(v) = up {
                glass |= v.isKindOfClass(class);
                up = unsafe { v.superview() };
            }
        }
        if let Some(detail) = unsafe { self.ground.superview() } {
            detail_painted = painted(&detail);
        }
        let ground = self.ground.convertRect_toView(self.ground.bounds(), None);
        // The selection's insets around the selected tile: left, top, bottom.
        let insets = table.as_ref().and_then(|table| {
            let fill = self.chrome.ivars().selection.borrow().clone()?;
            let row = table.selectedRow();
            // AppKit raises for a row out of range (nothing selected yet).
            if row < 0 || row >= table.numberOfRows() {
                return None;
            }
            let cell = table.viewAtColumn_row_makeIfNecessary(0, row, false)?;
            let tile = cell
                .subviews()
                .iter()
                .find_map(|v| v.downcast::<NSImageView>().ok())?;
            let f = fill.frame();
            let t = tile.convertRect_toView(tile.bounds(), Some(table));
            let m = tile_margin(TILE);
            let (tx, ty) = (t.origin.x + m, t.origin.y + tile_top_margin(TILE));
            Some([
                tx - f.origin.x,
                ty - f.origin.y,
                f.origin.y + f.size.height - (ty + TILE),
            ])
        });
        serde_json::json!({
            "opaque": self.window.isOpaque(),
            "backgroundAlpha": alpha(&self.window.backgroundColor()),
            "sidebarInGlass": glass,
            "glassAvailable": objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some(),
            "detailPainted": detail_painted,
            "groundX": ground.origin.x,
            "sidebarWidth": SIDEBAR_WIDTH,
            "searchHeight": self.chrome.ivars().search.borrow().as_ref().map(|search| search.frame().size.height),
            "selectionInsets": insets,
            "header": self.header_title.stringValue().to_string(),
        })
    }
    pub fn update_sidebar_appearance(&self) {
        let state = self.chrome.ivars();
        let appearance = self.window.effectiveAppearance();
        let dark = is_dark(&appearance);
        if let Some((material, veil)) = state.material.borrow().as_ref() {
            material.setState(if dark {
                NSVisualEffectState::Active
            } else {
                NSVisualEffectState::FollowsWindowActiveState
            });
            veil.setHidden(!dark);
            if let Some(layer) = veil.layer() {
                layer.setBackgroundColor(Some(
                    &NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., DARK_VEIL).CGColor(),
                ));
            }
        }
        self.chrome.paint_selection(&appearance);
        if state.dark.replace(dark) != dark
            && let Some(table) = state.table.borrow().as_ref()
        {
            let selected = table.selectedRowIndexes();
            state.quiet.set(true);
            table.reloadData();
            table.selectRowIndexes_byExtendingSelection(&selected, false);
            state.quiet.set(false);
            let id = self.header_id.borrow().clone();
            self.header_tile
                .setImage(tile_image(&id, dark, HEADER_TILE).as_deref());
        }
    }

    /// ⌘F: the keyboard goes to the search field.
    pub fn focus_search(&self) {
        if let Some(search) = self.chrome.ivars().search.borrow().as_ref() {
            self.window.makeFirstResponder(Some(search));
        }
    }

    /// The search field shows `text` (a search ended from the page).
    /// The field shows the panel's query, except while the person is typing
    /// in it: what they typed is newer than any answer (an answer to "pr"
    /// arriving after "pro" was typed dropped the "o").
    pub fn set_search(&self, text: &str) {
        if let Some(search) = self.chrome.ivars().search.borrow().as_ref()
            && search.currentEditor().is_none()
            && search.stringValue().to_string() != text
        {
            search.setStringValue(&NSString::from_str(text));
        }
    }
}

/// The page's side inset: the header's tile and the cards start here.
pub const PAGE_INSET: f64 = 20.;

/// GPUI's view of window `w`.
pub fn gpui_view(w: &gpui::Window) -> Option<Retained<NSView>> {
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(w).ok()?.as_raw() else {
        return None;
    };
    unsafe { Retained::retain(handle.ns_view.as_ptr().cast()) }
}

fn activate(constraints: &[Retained<NSLayoutConstraint>]) {
    NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(constraints));
}

/// A sidebar row: the section's tile and its name. Rows run the table's
/// full width; the tile sits `TILE_INSET` inside the selection on every side.
fn cell(mtm: MainThreadMarker, section: &Section, dark: bool) -> Retained<NSView> {
    if section.id == "account" {
        return account_cell(mtm, section, dark);
    }
    let view = NSTableCellView::new(mtm);
    let icon = NSImageView::new(mtm);
    icon.setImage(tile_image(&section.id, dark, TILE).as_deref());
    icon.setImageScaling(NSImageScaling::ScaleNone);
    let label = NSTextField::labelWithString(&NSString::from_str(&section.title), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(LABEL_SIZE)));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    if section.dim {
        label.setTextColor(Some(&NSColor::tertiaryLabelColor()));
        icon.setAlphaValue(0.45);
    }
    icon.setTranslatesAutoresizingMaskIntoConstraints(false);
    label.setTranslatesAutoresizingMaskIntoConstraints(false);
    view.addSubview(&icon);
    view.addSubview(&label);
    let (w, h) = tile_size(TILE);
    let tile_x = SIDEBAR_INSET + TILE_INSET;
    activate(&[
        icon.widthAnchor().constraintEqualToConstant(w),
        icon.heightAnchor().constraintEqualToConstant(h),
        icon.leadingAnchor()
            .constraintEqualToAnchor_constant(&view.leadingAnchor(), tile_x - tile_margin(TILE)),
        icon.centerYAnchor()
            .constraintEqualToAnchor_constant(&view.centerYAnchor(), tile_centre_offset(TILE)),
        label
            .leadingAnchor()
            .constraintEqualToAnchor_constant(&view.leadingAnchor(), tile_x + TILE + 9.),
        label
            .trailingAnchor()
            .constraintLessThanOrEqualToAnchor_constant(
                &view.trailingAnchor(),
                -SIDEBAR_INSET - 6.,
            ),
        label
            .centerYAnchor()
            .constraintEqualToAnchor(&view.centerYAnchor()),
    ]);
    unsafe {
        view.setImageView(Some(&icon));
        view.setTextField(Some(&label));
    }
    Retained::into_super(view)
}

/// A group's name over its rows, on the tiles' leading edge.
fn header_cell(mtm: MainThreadMarker, name: &str) -> Retained<NSView> {
    let view = NSTableCellView::new(mtm);
    let label = NSTextField::labelWithString(&NSString::from_str(name), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize_weight(12., unsafe {
        NSFontWeightSemibold
    })));
    label.setTextColor(Some(&NSColor::secondaryLabelColor()));
    label.setTranslatesAutoresizingMaskIntoConstraints(false);
    view.addSubview(&label);
    activate(&[
        label
            .leadingAnchor()
            .constraintEqualToAnchor_constant(&view.leadingAnchor(), SIDEBAR_INSET + TILE_INSET),
        label
            .bottomAnchor()
            .constraintEqualToAnchor_constant(&view.bottomAnchor(), -5.),
    ]);
    unsafe { view.setTextField(Some(&label)) };
    Retained::into_super(view)
}

/// The account at the top of the sidebar: the avatar, the name and
/// "Account"; signed out, the account glyph and "Sign In".
fn account_cell(mtm: MainThreadMarker, section: &Section, dark: bool) -> Retained<NSView> {
    let view = NSTableCellView::new(mtm);
    let name = section.account_name.as_deref().unwrap_or("Sign In");
    let title = NSTextField::labelWithString(&NSString::from_str(name), mtm);
    title.setFont(Some(&NSFont::systemFontOfSize_weight(LABEL_SIZE, unsafe {
        NSFontWeightSemibold
    })));
    title.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    // Under the name, the account's email (a name that is the email says
    // "Account").
    let email = section
        .account_email
        .as_deref()
        .filter(|email| !email.is_empty() && Some(*email) != section.account_name.as_deref());
    let subtitle =
        NSTextField::labelWithString(&NSString::from_str(email.unwrap_or("Account")), mtm);
    subtitle.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    subtitle.setFont(Some(&NSFont::systemFontOfSize(
        NSFont::smallSystemFontSize(),
    )));
    subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
    let labels = NSStackView::new(mtm);
    labels.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    labels.setAlignment(NSLayoutAttribute::Leading);
    labels.setSpacing(1.);
    labels.addArrangedSubview(&title);
    if section.account_name.is_some() {
        labels.addArrangedSubview(&subtitle);
    }
    let initial = section
        .account_name
        .as_deref()
        .and_then(|name| name.chars().find(|letter| letter.is_alphanumeric()));
    let avatar = NSImageView::new(mtm);
    avatar.setImageScaling(NSImageScaling::ScaleNone);
    avatar.setImage(avatar_image(initial.is_some(), dark).as_deref());
    avatar.setAccessibilityElement(false);
    avatar.setTranslatesAutoresizingMaskIntoConstraints(false);
    labels.setTranslatesAutoresizingMaskIntoConstraints(false);
    view.addSubview(&avatar);
    if let Some(initial) = initial {
        let letter = NSTextField::labelWithString(
            &NSString::from_str(&initial.to_uppercase().to_string()),
            mtm,
        );
        letter.setFont(Some(&NSFont::systemFontOfSize_weight(15., unsafe {
            NSFontWeightSemibold
        })));
        letter.setTextColor(Some(&NSColor::labelColor()));
        letter.setTranslatesAutoresizingMaskIntoConstraints(false);
        view.addSubview(&letter);
        activate(&[
            letter
                .centerXAnchor()
                .constraintEqualToAnchor(&avatar.centerXAnchor()),
            letter
                .centerYAnchor()
                .constraintEqualToAnchor(&avatar.centerYAnchor()),
        ]);
    }
    view.addSubview(&labels);
    activate(&[
        avatar
            .leadingAnchor()
            .constraintEqualToAnchor_constant(&view.leadingAnchor(), SIDEBAR_INSET + AVATAR_INSET),
        avatar
            .centerYAnchor()
            .constraintEqualToAnchor(&view.centerYAnchor()),
        avatar.widthAnchor().constraintEqualToConstant(AVATAR),
        avatar.heightAnchor().constraintEqualToConstant(AVATAR),
        labels
            .leadingAnchor()
            .constraintEqualToAnchor_constant(&avatar.trailingAnchor(), 9.),
        labels
            .trailingAnchor()
            .constraintLessThanOrEqualToAnchor_constant(
                &view.trailingAnchor(),
                -SIDEBAR_INSET - 6.,
            ),
        labels
            .centerYAnchor()
            .constraintEqualToAnchor(&view.centerYAnchor()),
    ]);
    unsafe { view.setTextField(Some(&title)) };
    Retained::into_super(view)
}

// Tiles. One drawing for every place a section's tile shows (the sidebar,
// the page header), rendered from SVG once per appearance and size.

/// The SVG's room around the tile for its shadow, in tile units (22).
const TILE_UNITS: f64 = 22.;
const MARGIN_UNITS: f64 = 3.;
fn tile_margin(side: f64) -> f64 {
    MARGIN_UNITS * side / TILE_UNITS
}
fn tile_top_margin(side: f64) -> f64 {
    tile_margin(side)
}
fn tile_size(side: f64) -> (f64, f64) {
    let s = side + 2. * tile_margin(side);
    (s, s)
}
/// The image is centred on the tile, so its centre is the tile's.
fn tile_centre_offset(_side: f64) -> f64 {
    0.
}

macro_rules! pierre {
    ($name:literal) => {
        include_str!(concat!("../icons/pierre/Icon", $name, ".svg"))
    };
}
macro_rules! drawn {
    ($name:literal) => {
        include_str!(concat!("../icons/powermove/", $name, ".svg"))
    };
}

/// A section's glyph, all in one filled style: Pierre's filled glyphs, or
/// ours drawn in its grammar where it has none or only an outline (film,
/// photo, keyboard, drive, blocks, prompt, window).
fn glyph(id: &str) -> Option<&'static str> {
    Some(match id {
        "general" => pierre!("GearFill"),
        "project" => drawn!("film"),
        "media" => drawn!("photo"),
        "shortcuts" => drawn!("keyboard"),
        "transcription" => pierre!("Microphone"),
        "storage" => drawn!("drive"),
        "providers" => pierre!("KeyFill"),
        "agents" => pierre!("Sparkles"),
        "extensions" => drawn!("blocks"),
        "privacy" => pierre!("ShieldFill"),
        "about" => pierre!("InfoFill"),
        "account" => drawn!("account"),
        // The sign-in sheet's tile.
        "powermove" => drawn!("powermove"),
        // Settings › Agents' apps with no mark of their own.
        "terminal" => drawn!("prompt"),
        "window" => drawn!("window"),
        _ => return None,
    })
}

/// The inside of a 16-unit glyph file, painted with `paint`. A mask's own
/// black and white stay as they are (painting them would fill the glyph's
/// knock-outs).
fn glyph_body(svg: &str, paint: &str) -> String {
    let start = svg
        .find("<svg")
        .and_then(|at| svg[at..].find('>').map(|end| at + end + 1));
    let end = svg.rfind("</svg>");
    let inner = match (start, end) {
        (Some(start), Some(end)) if start < end => &svg[start..end],
        _ => "",
    };
    let recolour = |part: &str| {
        part.replace("fill=\"black\"", &format!("fill=\"{paint}\""))
            .replace("stroke=\"black\"", &format!("stroke=\"{paint}\""))
    };
    let mut out = String::new();
    let mut rest = inner;
    while let Some(at) = rest.find("<mask") {
        out += &recolour(&rest[..at]);
        let close = rest[at..]
            .find("</mask>")
            .map_or(rest.len(), |c| at + c + "</mask>".len());
        out += &rest[at..close];
        rest = &rest[close..];
    }
    out + &recolour(rest)
}

/// A continuous-corner square (a superellipse) of side `s` at the origin,
/// inset by `inset`.
fn squircle(s: f64, inset: f64) -> String {
    let a = s / 2. - inset;
    let n = 5.;
    let steps = 120;
    let mut d = String::new();
    for i in 0..steps {
        let t = std::f64::consts::TAU * i as f64 / steps as f64;
        let (sin, cos) = t.sin_cos();
        let x = s / 2. + a * cos.signum() * cos.abs().powf(2. / n);
        let y = s / 2. + a * sin.signum() * sin.abs().powf(2. / n);
        d.push_str(&format!("{}{x:.3} {y:.3}", if i == 0 { "M" } else { "L" }));
    }
    d.push('Z');
    d
}

/// The colours of one tile family in one appearance.
struct Look {
    body: (&'static str, &'static str),
    /// The inner edge: its colour and opacity at top and bottom.
    edge: (&'static str, f64, f64),
    /// The soft top light's opacity.
    light: f64,
    glyph: (&'static str, &'static str),
    /// The tile's drop shadow: offset, blur, opacity.
    shadow: (f64, f64, f64),
    /// The glyph's own offset shadow (pressed in): colour and opacity.
    press: (&'static str, f64),
}
/// The tile: a dark (light in Aqua) squircle with a soft top light, an
/// inner edge and a monochrome glyph pressed into it.
fn look(dark: bool) -> Look {
    if dark {
        Look {
            body: ("#55555c", "#26262b"),
            edge: ("#ffffff", 0.42, 0.06),
            light: 0.24,
            glyph: ("#ffffff", "#d4d4da"),
            shadow: (0.6, 0.7, 0.6),
            press: ("#000000", 0.5),
        }
    } else {
        Look {
            body: ("#ffffff", "#e2e2e6"),
            edge: ("#000000", 0.08, 0.2),
            light: 0.9,
            glyph: ("#55555b", "#232327"),
            shadow: (0.6, 0.7, 0.2),
            press: ("#ffffff", 0.9),
        }
    }
}

/// An SVG's view box size (width, height).
fn view_box(svg: &str) -> Option<(f64, f64)> {
    let at = svg.find("viewBox=\"")? + "viewBox=\"".len();
    let end = svg[at..].find('"')? + at;
    let n: Vec<f64> = svg[at..end]
        .split_whitespace()
        .filter_map(|v| v.parse().ok())
        .collect();
    (n.len() == 4).then(|| (n[2], n[3]))
}

fn tile_svg(id: &str, dark: bool) -> Option<String> {
    let source = glyph(id)?;
    let glyph = glyph_body(source, "url(#g)");
    let Look {
        body,
        edge,
        light,
        glyph: ink,
        shadow,
        press,
    } = look(dark);
    let s = TILE_UNITS;
    let m = MARGIN_UNITS;
    let full = s + 2. * m;
    // The glyph: 14 of the tile's 22 units across its longer side, centred
    // (Pierre's are 16 units; the Powermove mark has its own box).
    let (gw, gh) = view_box(source).unwrap_or((16., 16.));
    let k = 14. / gw.max(gh);
    let (ax, ay) = ((s - gw * k) / 2., (s - gh * k) / 2.);
    Some(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{full}" height="{full}" viewBox="{nm} {nm} {full} {full}">
<defs>
<linearGradient id="body" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{b0}"/><stop offset="1" stop-color="{b1}"/></linearGradient>
<linearGradient id="edge" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{ec}" stop-opacity="{e0}"/><stop offset="1" stop-color="{ec}" stop-opacity="{e1}"/></linearGradient>
<radialGradient id="light" cx="0.5" cy="0" r="0.9"><stop offset="0" stop-color="#ffffff" stop-opacity="{light}"/><stop offset="1" stop-color="#ffffff" stop-opacity="0"/></radialGradient>
<linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="0" y2="{gh}"><stop offset="0" stop-color="{g0}"/><stop offset="1" stop-color="{g1}"/></linearGradient>
<filter id="drop" x="-30%" y="-30%" width="160%" height="160%"><feDropShadow dx="0" dy="{sd}" stdDeviation="{sb}" flood-color="#000000" flood-opacity="{so}"/></filter>
<filter id="press" x="-30%" y="-30%" width="160%" height="160%"><feDropShadow dx="0" dy="0.55" stdDeviation="0.3" flood-color="{pc}" flood-opacity="{po}"/></filter>
</defs>
<path d="{outer}" fill="url(#body)" filter="url(#drop)"/>
<path d="{outer}" fill="url(#light)"/>
<path d="{inner}" fill="none" stroke="url(#edge)" stroke-width="0.5"/>
<g filter="url(#press)"><g transform="translate({ax} {ay}) scale({k})">{glyph}</g></g>
</svg>"##,
        nm = -m,
        b0 = body.0,
        b1 = body.1,
        ec = edge.0,
        e0 = edge.1,
        e1 = edge.2,
        g0 = ink.0,
        g1 = ink.1,
        sd = shadow.0,
        sb = shadow.1,
        so = shadow.2,
        pc = press.0,
        po = press.1,
        outer = squircle(s, 0.),
        inner = squircle(s, 0.25),
    ))
}

thread_local! {
    static TILE_CACHE: RefCell<std::collections::HashMap<(String, bool, u32), Retained<NSImage>>> = RefCell::default();
}

/// A section's tile at `side` points for the dark or light appearance,
/// rendered once at 1× and 2× (the image carries `tile_margin` around it).
pub fn tile_image(id: &str, dark: bool, side: f64) -> Option<Retained<NSImage>> {
    let key = (id.to_owned(), dark, (side * 100.) as u32);
    if let Some(image) = TILE_CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return Some(image);
    }
    let svg = tile_svg(id, dark)?;
    let (w, h) = tile_size(side);
    let image = svg_image(&svg, NSSize::new(w, h))?;
    TILE_CACHE.with(|cache| cache.borrow_mut().insert(key, image.clone()));
    Some(image)
}

/// The account's avatar: the tile's material as a disc (the initial is a
/// label over it), or, signed out, the account glyph in the secondary grey.
fn avatar_image(signed_in: bool, dark: bool) -> Option<Retained<NSImage>> {
    let size = NSSize::new(AVATAR, AVATAR);
    if !signed_in {
        let grey = if dark { "#98989f" } else { "#6a6a70" };
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">{}</svg>"#,
            glyph_body(glyph("account")?, grey)
        );
        return svg_image(&svg, size);
    }
    let Look {
        body, edge, light, ..
    } = look(dark);
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 32 32">
<defs>
<linearGradient id="body" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{}"/></linearGradient>
<radialGradient id="light" cx="0.5" cy="0" r="0.9"><stop offset="0" stop-color="#ffffff" stop-opacity="{light}"/><stop offset="1" stop-color="#ffffff" stop-opacity="0"/></radialGradient>
</defs>
<circle cx="16" cy="16" r="15.5" fill="url(#body)"/>
<circle cx="16" cy="16" r="15.5" fill="url(#light)"/>
<circle cx="16" cy="16" r="15.25" fill="none" stroke="{}" stroke-opacity="{}" stroke-width="0.5"/>
</svg>"##,
        body.0, body.1, edge.0, edge.1
    );
    svg_image(&svg, size)
}

/// A section's tile as PNG bytes at `side` points drawn at `scale` (for a
/// GPUI view: the sign-in sheet's Powermove tile), its shadow margin around it.
pub fn tile_png(id: &str, dark: bool, side: f64, scale: f64) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let svg = tile_svg(id, dark)?;
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
    let (w, h) = tile_size(side);
    let (w, h) = ((w * scale).round() as u32, (h * scale).round() as u32);
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    let view = tree.size();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(w as f32 / view.width(), h as f32 / view.height()),
        &mut pixmap.as_mut(),
    );
    // tiny-skia's pixels are premultiplied; PNG's are not.
    let mut rgba = Vec::with_capacity(pixmap.data().len());
    for px in pixmap.pixels() {
        let c = px.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    let image = image::RgbaImage::from_raw(w, h, rgba)?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).ok()?;
    Some(bytes.into_inner())
}
/// The room a tile's image keeps around the tile, in points.
pub fn tile_image_size(side: f64) -> (f64, f64) {
    tile_size(side)
}

/// `svg` rasterised at 1× and 2× into one image of `size` points.
pub fn svg_image(svg: &str, size: NSSize) -> Option<Retained<NSImage>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    for scale in [1., 2.] {
        let (w, h) = (
            (size.width * scale).round() as u32,
            (size.height * scale).round() as u32,
        );
        let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
        let view = tree.size();
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(w as f32 / view.width(), h as f32 / view.height()),
            &mut pixmap.as_mut(),
        );
        let rep: Retained<NSBitmapImageRep> = unsafe {
            msg_send![
                NSBitmapImageRep::alloc(),
                initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>(),
                pixelsWide: w as NSInteger,
                pixelsHigh: h as NSInteger,
                bitsPerSample: 8 as NSInteger,
                samplesPerPixel: 4 as NSInteger,
                hasAlpha: true,
                isPlanar: false,
                colorSpaceName: NSDeviceRGBColorSpace,
                bytesPerRow: (w * 4) as NSInteger,
                bitsPerPixel: 32 as NSInteger
            ]
        };
        let data: *mut u8 = unsafe { msg_send![&rep, bitmapData] };
        if data.is_null() {
            return None;
        }
        // tiny-skia's pixels are premultiplied RGBA, as the rep's.
        unsafe { std::ptr::copy_nonoverlapping(pixmap.data().as_ptr(), data, pixmap.data().len()) };
        let srgb = NSColorSpace::sRGBColorSpace();
        let rep: Retained<NSBitmapImageRep> =
            unsafe { msg_send![&rep, bitmapImageRepByRetaggingWithColorSpace: &*srgb] };
        rep.setSize(size);
        image.addRepresentation(&rep);
    }
    Some(image)
}

const SECTION_KEY: &str = "PMSettingsSection";

/// The section Settings was last left on (standard window behaviour: it
/// opens where it was).
pub fn remembered_section() -> Option<String> {
    let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
    defaults
        .stringForKey(&NSString::from_str(SECTION_KEY))
        .map(|s| s.to_string())
}
pub fn remember_section(id: &str) {
    let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
    unsafe {
        defaults.setObject_forKey(
            Some(&NSString::from_str(id)),
            &NSString::from_str(SECTION_KEY),
        )
    };
}
