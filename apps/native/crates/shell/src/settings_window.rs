//! Settings as a window of its own, framed by AppKit (PM-375): the
//! sections in a real sidebar (`NSSplitViewItem(sidebarWithViewController:)`,
//! a source-list `NSOutlineView` under an `NSSearchField`), back and forward
//! as an `NSToolbarItemGroup` in a unified toolbar that shows the section's
//! name as the window title, and the page (the Settings surface's GPUI view)
//! in the detail item below the toolbar. Every material here is the
//! system's; nothing is drawn to look like one.
//!
//! The section icons are the one custom piece: a glass slab tile with its
//! glyph as a layer that is either the drawn glyph or a real SF Symbol
//! ([`GLYPHS`]), rendered once per backing scale.

use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::AnyObject,
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::{
    NSArray, NSIndexSet, NSInteger, NSNotification, NSNumber, NSObject, NSObjectProtocol,
    NSPoint, NSRect, NSSize, NSString, ns_string,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::{Cell, RefCell};

/// The sidebar's width (fixed; the sidebar does not collapse).
pub const SIDEBAR_WIDTH: f64 = 216.;
/// The window's default and minimum content sizes.
pub const DEFAULT_SIZE: (f64, f64) = (820., 640.);
pub const MIN_SIZE: (f64, f64) = (720., 520.);
const ROW_HEIGHT: f64 = 30.;
const ROW_SPACING: f64 = 2.;
/// The space between the project's sections and the app's.
const GROUP_GAP: f64 = 14.;
/// The tile's side in a sidebar row.
const TILE: f64 = 20.;
const FRAME_NAME: &str = "PMSettingsWindow";
/// The Settings window's identifier (its title is the section's name).
const IDENTIFIER: &str = "pm.settings";

/// Whether `window` is the Settings window.
pub fn is_settings(window: &NSWindow) -> bool {
    window
        .identifier()
        .is_some_and(|id| id.to_string() == IDENTIFIER)
}
const NAV: &str = "pm.settings.navigation";

/// What a section's glyph is drawn from: the drawn glyph on the glass tile,
/// or a real SF Symbol on the same tile. One switch for the whole family;
/// `PM_SETTINGS_GLYPHS=symbols` (or `drawn`) overrides it for a comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyphs {
    Drawn,
    Symbols,
}
pub const GLYPHS: Glyphs = Glyphs::Drawn;
fn glyphs() -> Glyphs {
    match std::env::var("PM_SETTINGS_GLYPHS").as_deref() {
        Ok("symbols") => Glyphs::Symbols,
        Ok("drawn") => Glyphs::Drawn,
        _ => GLYPHS,
    }
}

/// One row of the sidebar.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub id: String,
    pub title: String,
    /// The project's own sections (Project, Memory) come first, set apart
    /// from the app's by a gap.
    pub project: bool,
    /// A search is on and nothing in this section matches it.
    pub dim: bool,
}

/// What the person did in the window's chrome.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Select(String),
    Search(String),
    Back,
    Forward,
}

struct Row {
    section: Option<Section>,
    item: Retained<NSNumber>,
}

pub struct State {
    sender: async_channel::Sender<Event>,
    rows: RefCell<Vec<Row>>,
    outline: RefCell<Option<Retained<NSOutlineView>>>,
    search: RefCell<Option<Retained<NSSearchField>>>,
    nav: RefCell<Option<Retained<NSToolbarItemGroup>>>,
    can: Cell<(bool, bool)>,
    /// The window's appearance: which icon set the rows show.
    dark: Cell<bool>,
    ground: Cell<Option<u32>>,
    /// Set while the host changes the selection, so it is not sent back.
    quiet: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSettingsChrome"]
    #[ivars = State]
    pub struct Chrome;
    unsafe impl NSObjectProtocol for Chrome {}
    impl Chrome {
        // NSOutlineViewDataSource: one flat level.
        #[unsafe(method(outlineView:numberOfChildrenOfItem:))]
        fn children(&self, _: &NSOutlineView, item: *mut AnyObject) -> NSInteger {
            if item.is_null() { self.ivars().rows.borrow().len() as NSInteger } else { 0 }
        }
        #[unsafe(method(outlineView:child:ofItem:))]
        fn child(&self, _: &NSOutlineView, index: NSInteger, _: *mut AnyObject) -> *mut AnyObject {
            self.ivars().rows.borrow().get(index as usize)
                .map_or(std::ptr::null_mut(), |r| Retained::as_ptr(&r.item) as *mut AnyObject)
        }
        #[unsafe(method(outlineView:isItemExpandable:))]
        fn expandable(&self, _: &NSOutlineView, _: *mut AnyObject) -> bool { false }
        // NSOutlineViewDelegate.
        #[unsafe(method(outlineView:heightOfRowByItem:))]
        fn height(&self, _: &NSOutlineView, item: *mut AnyObject) -> f64 {
            match self.row_of(item) {
                // The gap row and the spacing on both its sides make the gap.
                Some(None) => GROUP_GAP - 2. * ROW_SPACING,
                _ => ROW_HEIGHT,
            }
        }
        #[unsafe(method(outlineView:shouldSelectItem:))]
        fn selectable(&self, _: &NSOutlineView, item: *mut AnyObject) -> bool {
            matches!(self.row_of(item), Some(Some(_)))
        }
        #[unsafe(method_id(outlineView:viewForTableColumn:item:))]
        fn cell(&self, _: &NSOutlineView, _: *mut AnyObject, item: *mut AnyObject) -> Option<Retained<NSView>> {
            self.row_of(item).flatten().map(|section| cell(MainThreadMarker::from(self), &section, self.ivars().dark.get()))
        }
        #[unsafe(method_id(outlineView:rowViewForItem:))]
        fn row_view(&self, _: &NSOutlineView, _: *mut AnyObject) -> Option<Retained<NSTableRowView>> {
            let mtm = MainThreadMarker::from(self);
            let row: Retained<NeutralRow> = unsafe { msg_send![NeutralRow::alloc(mtm), init] };
            Some(Retained::into_super(row))
        }
        #[unsafe(method(outlineViewSelectionDidChange:))]
        fn selection_changed(&self, _: &NSNotification) {
            if self.ivars().quiet.get() { return; }
            let Some(outline) = self.ivars().outline.borrow().clone() else { return };
            let row = outline.selectedRow();
            if row < 0 { return; }
            let id = self.ivars().rows.borrow().get(row as usize)
                .and_then(|r| r.section.as_ref().map(|s| s.id.clone()));
            if let Some(id) = id {
                let _ = self.ivars().sender.try_send(Event::Select(id));
            }
        }
        // NSToolbarDelegate.
        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_items(&self, _: &NSToolbar) -> Retained<NSArray<NSString>> { items() }
        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_items(&self, _: &NSToolbar) -> Retained<NSArray<NSString>> { items() }
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn toolbar_item(&self, _: &NSToolbar, id: &NSString, _: bool) -> Option<Retained<NSToolbarItem>> {
            let ours = id.to_string() == NAV;
            self.ivars().nav.borrow().clone().filter(|_| ours).map(Retained::into_super)
        }
        // Actions.
        #[unsafe(method(navigate:))]
        fn navigate(&self, sender: &NSToolbarItemGroup) {
            let _ = self.ivars().sender.try_send(if sender.selectedIndex() == 0 { Event::Back } else { Event::Forward });
        }
        #[unsafe(method(searched:))]
        fn searched(&self, sender: &NSSearchField) {
            let _ = self.ivars().sender.try_send(Event::Search(sender.stringValue().to_string()));
        }
        /// The toolbar's items are enabled while there is somewhere to go.
        #[unsafe(method(validateToolbarItem:))]
        fn validate(&self, item: &NSToolbarItem) -> bool {
            let (back, forward) = self.ivars().can.get();
            match item.label().to_string().as_str() { "Back" => back, "Forward" => forward, _ => true }
        }
    }
);

define_class!(
    /// A sidebar row whose selection is the system's neutral source-list
    /// highlight, never the accent (the build note).
    #[unsafe(super(NSTableRowView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSettingsRowView"]
    struct NeutralRow;
    impl NeutralRow {
        #[unsafe(method(isEmphasized))]
        fn emphasized(&self) -> bool { false }
    }
);

fn items() -> Retained<NSArray<NSString>> {
    let separator: &NSString = unsafe { NSToolbarSidebarTrackingSeparatorItemIdentifier };
    NSArray::from_slice(&[separator, &*NSString::from_str(NAV)])
}

impl Chrome {
    /// `Some(None)` is the gap row; `None` is no row of ours.
    fn row_of(&self, item: *mut AnyObject) -> Option<Option<Section>> {
        self.ivars()
            .rows
            .borrow()
            .iter()
            .find(|r| Retained::as_ptr(&r.item) as *mut AnyObject == item)
            .map(|r| r.section.clone())
    }
}

/// The chrome of an open Settings window. Dropping it leaves the window as
/// it is; the window's views keep what they show.
pub struct SettingsChrome {
    chrome: Retained<Chrome>,
    window: Retained<NSWindow>,
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
                outline: RefCell::new(None),
                search: RefCell::new(None),
                nav: RefCell::new(None),
                can: Cell::new((false, false)),
                dark: Cell::new(true),
                ground: Cell::new(None),
                quiet: Cell::new(false),
            });
            unsafe { msg_send![super(this), init] }
        };

        // The window: titled, resizable, its content under a unified
        // toolbar that shows the section's name.
        window.setStyleMask(
            window.styleMask()
                | NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable
                | NSWindowStyleMask::FullSizeContentView,
        );
        window.setIdentifier(Some(&NSString::from_str(IDENTIFIER)));
        window.setTitlebarAppearsTransparent(false);
        window.setTitleVisibility(NSWindowTitleVisibility::Visible);
        let frame = window.frame();

        // The sidebar: the search field over the source list.
        let sidebar = NSView::new(mtm);
        let search = NSSearchField::new(mtm);
        search.setPlaceholderString(Some(ns_string!("Search")));
        search.setSendsSearchStringImmediately(true);
        unsafe {
            search.setTarget(Some(&chrome));
            search.setAction(Some(sel!(searched:)));
        }
        search.setTranslatesAutoresizingMaskIntoConstraints(false);
        let outline = NSOutlineView::new(mtm);
        let column = NSTableColumn::initWithIdentifier(
            NSTableColumn::alloc(mtm),
            ns_string!("section"),
        );
        outline.addTableColumn(&column);
        unsafe { outline.setOutlineTableColumn(Some(&column)) };
        outline.setHeaderView(None);
        outline.setStyle(NSTableViewStyle::SourceList);
        outline.setRowSizeStyle(NSTableViewRowSizeStyle::Custom);
        outline.setIntercellSpacing(NSSize::new(0., ROW_SPACING));
        outline.setIndentationPerLevel(0.);
        outline.setFloatsGroupRows(false);
        outline.setAllowsEmptySelection(false);
        unsafe {
            let _: () = msg_send![&outline, setDataSource: &*chrome];
            let _: () = msg_send![&outline, setDelegate: &*chrome];
        }
        let scroll = NSScrollView::new(mtm);
        scroll.setDocumentView(Some(&outline));
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setTranslatesAutoresizingMaskIntoConstraints(false);
        sidebar.addSubview(&search);
        sidebar.addSubview(&scroll);
        let guide = sidebar.safeAreaLayoutGuide();
        activate(&[
            search.topAnchor().constraintEqualToAnchor(&guide.topAnchor()),
            search.leadingAnchor().constraintEqualToAnchor_constant(&sidebar.leadingAnchor(), 10.),
            search.trailingAnchor().constraintEqualToAnchor_constant(&sidebar.trailingAnchor(), -10.),
            scroll.topAnchor().constraintEqualToAnchor_constant(&search.bottomAnchor(), 12.),
            scroll.leadingAnchor().constraintEqualToAnchor(&sidebar.leadingAnchor()),
            scroll.trailingAnchor().constraintEqualToAnchor(&sidebar.trailingAnchor()),
            scroll.bottomAnchor().constraintEqualToAnchor(&sidebar.bottomAnchor()),
        ]);

        // The detail: GPUI's view below the toolbar (the safe area).
        gpui.setTranslatesAutoresizingMaskIntoConstraints(false);
        activate(&[
            gpui.topAnchor().constraintEqualToAnchor(&content.safeAreaLayoutGuide().topAnchor()),
            gpui.leadingAnchor().constraintEqualToAnchor(&content.leadingAnchor()),
            gpui.trailingAnchor().constraintEqualToAnchor(&content.trailingAnchor()),
            gpui.bottomAnchor().constraintEqualToAnchor(&content.bottomAnchor()),
        ]);
        content.setWantsLayer(true);

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
        split.addSplitViewItem(&side);
        split.addSplitViewItem(&detail);
        window.setContentViewController(Some(&split));

        // The toolbar: back and forward, over the detail.
        let images = NSArray::from_retained_slice(&[
            symbol("chevron.left", "Back")?,
            symbol("chevron.right", "Forward")?,
        ]);
        let labels = NSArray::from_slice(&[ns_string!("Back"), ns_string!("Forward")]);
        let nav = unsafe {
            NSToolbarItemGroup::groupWithItemIdentifier_images_selectionMode_labels_target_action(
                &NSString::from_str(NAV),
                &images,
                NSToolbarItemGroupSelectionMode::Momentary,
                Some(&labels),
                Some(&chrome),
                Some(sel!(navigate:)),
                mtm,
            )
        };
        nav.setNavigational(true);
        nav.setLabel(ns_string!("Back/Forward"));
        *chrome.ivars().nav.borrow_mut() = Some(nav);
        let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), ns_string!("PMSettings"));
        unsafe {
            let _: () = msg_send![&toolbar, setDelegate: &*chrome];
        }
        toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
        toolbar.setAllowsUserCustomization(false);
        window.setToolbar(Some(&toolbar));
        window.setToolbarStyle(NSWindowToolbarStyle::Unified);

        // Standard window behaviour: the size it was left at, a minimum.
        window.setContentMinSize(NSSize::new(MIN_SIZE.0, MIN_SIZE.1));
        window.setFrame_display(frame, true);
        window.setFrameAutosaveName(&NSString::from_str(FRAME_NAME));
        window.setAutorecalculatesKeyViewLoop(true);
        window.makeFirstResponder(Some(&outline));

        *chrome.ivars().outline.borrow_mut() = Some(outline);
        *chrome.ivars().search.borrow_mut() = Some(search);
        // GPUI's view takes its place in the detail now, so GPUI lays the
        // page out at the detail's size from the first frame.
        if let Some(content) = window.contentView() {
            content.layoutSubtreeIfNeeded();
        }
        Some(SettingsChrome { chrome, window })
    }

    /// The sidebar lists `sections` with `active` selected; the title names it.
    pub fn show(&self, sections: &[Section], active: &str) {
        let state = self.chrome.ivars();
        let Some(outline) = state.outline.borrow().clone() else {
            return;
        };
        let same = {
            let rows = state.rows.borrow();
            rows.iter().filter_map(|r| r.section.as_ref()).eq(sections.iter())
        };
        if !same {
            let mut rows = vec![];
            for (i, s) in sections.iter().enumerate() {
                if i > 0 && sections[i - 1].project && !s.project {
                    rows.push(Row { section: None, item: NSNumber::new_isize(-1) });
                }
                rows.push(Row { section: Some(s.clone()), item: NSNumber::new_isize(i as isize) });
            }
            *state.rows.borrow_mut() = rows;
            state.quiet.set(true);
            outline.reloadData();
            state.quiet.set(false);
        }
        let row = state
            .rows
            .borrow()
            .iter()
            .position(|r| r.section.as_ref().is_some_and(|s| s.id == active));
        if let Some(row) = row
            && outline.selectedRow() != row as NSInteger
        {
            state.quiet.set(true);
            outline.selectRowIndexes_byExtendingSelection(
                &NSIndexSet::indexSetWithIndex(row as usize),
                false,
            );
            state.quiet.set(false);
        }
        if let Some(title) = sections.iter().find(|s| s.id == active).map(|s| &s.title) {
            self.window.setTitle(&NSString::from_str(title));
        }
    }

    /// Back and forward are enabled while there is somewhere to go.
    pub fn set_history(&self, back: bool, forward: bool) {
        let state = self.chrome.ivars();
        state.can.set((back, forward));
        if let Some(nav) = state.nav.borrow().as_ref() {
            for (i, item) in nav.subitems().iter().enumerate() {
                item.setEnabled(if i == 0 { back } else { forward });
            }
        }
    }

    /// The window's appearance (live): the rows' icon set, and the ground
    /// the detail shows under the toolbar, as the page's own.
    pub fn set_scheme(&self, dark: bool, ground: u32) {
        let state = self.chrome.ivars();
        if state.ground.replace(Some(ground)) != Some(ground) {
            let c = |shift: u32| f64::from((ground >> shift) & 0xff) / 255.;
            let colour = NSColor::colorWithSRGBRed_green_blue_alpha(c(16), c(8), c(0), 1.);
            self.window.setBackgroundColor(Some(&colour));
        }
        if state.dark.replace(dark) == dark {
            return;
        }
        if let Some(outline) = state.outline.borrow().clone() {
            let selected = outline.selectedRowIndexes();
            state.quiet.set(true);
            outline.reloadData();
            outline.selectRowIndexes_byExtendingSelection(&selected, false);
            state.quiet.set(false);
        }
    }

    /// ⌘F: the keyboard goes to the search field.
    pub fn focus_search(&self) {
        if let Some(search) = self.chrome.ivars().search.borrow().as_ref() {
            self.window.makeFirstResponder(Some(search));
        }
    }

    /// The search field shows `text` (a search ended from the page).
    pub fn set_search(&self, text: &str) {
        if let Some(search) = self.chrome.ivars().search.borrow().as_ref()
            && search.stringValue().to_string() != text
        {
            search.setStringValue(&NSString::from_str(text));
        }
    }
}

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

fn symbol(name: &str, label: &str) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        Some(&NSString::from_str(label)),
    )
}

/// A sidebar row: the section's icon and its name.
fn cell(mtm: MainThreadMarker, section: &Section, dark: bool) -> Retained<NSView> {
    let view = NSTableCellView::new(mtm);
    let icon = NSImageView::new(mtm);
    if let Some(image) = icon_image(&section.id, dark) {
        icon.setImage(Some(&image));
    }
    icon.setImageScaling(NSImageScaling::ScaleNone);
    let label = NSTextField::labelWithString(&NSString::from_str(&section.title), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(13.)));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    if section.dim {
        label.setTextColor(Some(&NSColor::tertiaryLabelColor()));
        icon.setAlphaValue(0.45);
    }
    icon.setTranslatesAutoresizingMaskIntoConstraints(false);
    label.setTranslatesAutoresizingMaskIntoConstraints(false);
    view.addSubview(&icon);
    view.addSubview(&label);
    // The image carries the tile's shadow margin; the tile itself is
    // `TILE` square at the row's leading padding.
    let (w, h) = ICON_SIZE;
    activate(&[
        icon.widthAnchor().constraintEqualToConstant(w),
        icon.heightAnchor().constraintEqualToConstant(h),
        icon.leadingAnchor().constraintEqualToAnchor_constant(&view.leadingAnchor(), 7. - MARGIN.0),
        // The tile's centre on the row's: the image's is 2.8 − 1.2 = 0.8 pt lower.
        icon.centerYAnchor().constraintEqualToAnchor_constant(&view.centerYAnchor(), (MARGIN.3 - MARGIN.1) / 2.),
        label.leadingAnchor().constraintEqualToAnchor_constant(&view.leadingAnchor(), 7. + TILE + 8.),
        label.trailingAnchor().constraintLessThanOrEqualToAnchor_constant(&view.trailingAnchor(), -7.),
        label.centerYAnchor().constraintEqualToAnchor(&view.centerYAnchor()),
    ]);
    unsafe {
        view.setImageView(Some(&icon));
        view.setTextField(Some(&label));
    }
    Retained::into_super(view)
}

/// The SVGs' view box is the tile (0…100) with room for its shadow:
/// −8…108 across, −6…114 down. In points at `TILE`: (left, top, right, bottom).
const MARGIN: (f64, f64, f64, f64) = (8. * TILE / 100., 6. * TILE / 100., 8. * TILE / 100., 14. * TILE / 100.);
const ICON_SIZE: (f64, f64) = (116. * TILE / 100., 120. * TILE / 100.);

/// Where the fallback's SF Symbol is centred down the tile
/// (`sf-symbols.json`: x 50 %, y 49 %). The drawn glyphs sit where their
/// SVGs place them.
const SYMBOL_CENTRE: f64 = 0.49;

/// One section's art in one appearance: dark is the glass slab, light its
/// pastel variant (the owner's choices). Glyphs are placed alike in both.
struct Art {
    drawn: &'static str,
    tile: &'static str,
    symbol: &'static str,
    /// The fallback symbol's glyph, lit edge and deep copy (`#rrggbbaa`).
    colours: [&'static str; 3],
}

fn art(id: &str, dark: bool) -> Option<Art> {
    macro_rules! art {
        ($name:literal, $symbol:literal, [$($d:literal),*], [$($l:literal),*]) => {
            if dark {
                Art {
                    drawn: include_str!(concat!("../assets/settings/dark/", $name, ".svg")),
                    tile: include_str!(concat!("../assets/settings/dark/tiles/", $name, ".svg")),
                    symbol: $symbol,
                    colours: [$($d),*],
                }
            } else {
                Art {
                    drawn: include_str!(concat!("../assets/settings/light/", $name, ".svg")),
                    tile: include_str!(concat!("../assets/settings/light/tiles/", $name, ".svg")),
                    symbol: $symbol,
                    colours: [$($l),*],
                }
            }
        };
    }
    Some(match id {
        "project" => art!("project", "movieclapper.fill",
            ["#ffeee2", "#ffe8d7", "#2606008c"], ["#c05621", "#ffffff", "#7232138c"]),
        "memory" => art!("memory", "brain.head.profile.fill",
            ["#f9f0ff", "#f8ebff", "#1709278c"], ["#8c61c3", "#ffffff", "#5238748c"]),
        "general" => art!("general", "gearshape.fill",
            ["#f7f5f3", "#f4f1ee", "#14110f8c"], ["#817971", "#ffffff", "#4c47428c"]),
        "shortcuts" => art!("shortcuts", "command",
            ["#f1f5fc", "#edf2fc", "#0e12198c"], ["#6d7b94", "#ffffff", "#3f48578c"]),
        "media" => art!("media", "photo.on.rectangle.angled.fill",
            ["#e6f7ff", "#dbf5ff", "#00122a8c"], ["#2f7dcb", "#ffffff", "#1b49798c"]),
        "storage" => art!("storage", "externaldrive.fill",
            ["#e5fcee", "#dbfbe7", "#0018088c"], ["#1b905c", "#ffffff", "#1155368c"]),
        "accounts" => art!("accounts", "person.crop.circle.fill",
            ["#e1faff", "#d4f9ff", "#0016218c"], ["#0089b0", "#ffffff", "#0051688c"]),
        "transcription" => art!("transcription", "captions.bubble.fill",
            ["#ffecf1", "#ffe5ec", "#25040d8c"], ["#be506a", "#ffffff", "#712e3e8c"]),
        "extensions" => art!("extensions", "puzzlepiece.extension.fill",
            ["#fff4de", "#fff0d0", "#1d0e008c"], ["#a17000", "#ffffff", "#6042008c"]),
        _ => return None,
    })
}

thread_local! {
    static ICONS: RefCell<std::collections::HashMap<(String, bool), Retained<NSImage>>> = RefCell::default();
}

/// A section's icon for the dark or light appearance, rendered once at 1×
/// and 2×.
pub fn icon_image(id: &str, dark: bool) -> Option<Retained<NSImage>> {
    let key = (id.to_owned(), dark);
    if let Some(image) = ICONS.with(|icons| icons.borrow().get(&key).cloned()) {
        return Some(image);
    }
    let art = art(id, dark)?;
    let size = NSSize::new(ICON_SIZE.0, ICON_SIZE.1);
    let image = match glyphs() {
        Glyphs::Drawn => bitmaps(art.drawn, size)?,
        Glyphs::Symbols => {
            let tile = bitmaps(art.tile, size)?;
            symbol_tile(tile, &art, size)?
        }
    };
    ICONS.with(|icons| icons.borrow_mut().insert(key, image.clone()));
    Some(image)
}

/// `svg` rasterised at 1× and 2× into one image of `size` points.
fn bitmaps(svg: &str, size: NSSize) -> Option<Retained<NSImage>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    for scale in [1., 2.] {
        let (w, h) = ((size.width * scale).round() as u32, (size.height * scale).round() as u32);
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

/// The glyph-free tile with a real SF Symbol on it: the symbol at 0.46 of
/// the tile, its box centred at `SYMBOL_CENTRE`, with a lit copy 1.2 %
/// above and a deep copy 2.6 % below, both unblurred.
fn symbol_tile(tile: Retained<NSImage>, art: &Art, size: NSSize) -> Option<Retained<NSImage>> {
    let name = NSString::from_str(art.symbol);
    let base = NSImage::imageWithSystemSymbolName_accessibilityDescription(&name, None)?;
    let tint = |hex: &str| {
        let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xffffffff);
        let c = |shift: u32| f64::from((v >> shift) & 0xff) / 255.;
        NSColor::colorWithSRGBRed_green_blue_alpha(c(24), c(16), c(8), c(0))
    };
    let [glyph, light, deep] = art.colours.map(|hex| {
        let colours = NSArray::from_retained_slice(&[tint(hex)]);
        let config = NSImageSymbolConfiguration::configurationWithPointSize_weight_scale(
            TILE * 0.46,
            unsafe { NSFontWeightSemibold },
            NSImageSymbolScale::Medium,
        )
        .configurationByApplyingConfiguration(
            &NSImageSymbolConfiguration::configurationWithPaletteColors(&colours),
        );
        base.imageWithSymbolConfiguration(&config)
    });
    let (glyph, light, deep) = (glyph?, light?, deep?);
    let draw = block2::RcBlock::new(move |rect: NSRect| -> objc2::runtime::Bool {
        tile.drawInRect(rect);
        let s = glyph.size();
        // Unflipped: the tile's bottom edge is `MARGIN.3` up.
        let centre = NSPoint::new(MARGIN.0 + TILE * 0.5, MARGIN.3 + TILE * (1. - SYMBOL_CENTRE));
        let at = |dy: f64| {
            NSRect::new(NSPoint::new(centre.x - s.width / 2., centre.y - s.height / 2. + dy), s)
        };
        let over = NSCompositingOperation::SourceOver;
        deep.drawInRect_fromRect_operation_fraction(at(-TILE * 0.026), NSRect::ZERO, over, 1.);
        light.drawInRect_fromRect_operation_fraction(at(TILE * 0.012), NSRect::ZERO, over, 0.95);
        glyph.drawInRect_fromRect_operation_fraction(at(0.), NSRect::ZERO, over, 1.);
        objc2::runtime::Bool::YES
    });
    Some(NSImage::imageWithSize_flipped_drawingHandler(size, false, &draw))
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
