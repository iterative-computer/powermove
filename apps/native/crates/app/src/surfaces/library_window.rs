//! S017 AppKit window and token-bound card rendering. The shell retains one
//! instance and binds its native table/collection delegates to LibraryController.
//! No image decoding, sorting, file IO or JS runs on this thread.
use super::{model::*, sheet_layout::native::colour};
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
use objc2_app_kit::*;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSSortDescriptor, NSString};
pub struct LibraryWindow {
    pub window: Retained<NSWindow>,
    pub split: Retained<NSSplitViewController>,
    pub sidebar: Retained<NSTableView>,
    pub grid: Retained<NSCollectionView>,
    pub list: Retained<NSTableView>,
    pub toolbar: Retained<NSToolbar>,
}
impl LibraryWindow {
    pub fn new(tokens: &pm_design::Tokens, mtm: MainThreadMarker) -> Self {
        let metrics = WindowMetrics::resolve(tokens);
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(
                    NSPoint::ZERO,
                    NSSize::new(metrics.min_width.into(), metrics.min_height.into()),
                ),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str(TITLE));
        window.setMinSize(NSSize::new(
            metrics.min_width.into(),
            metrics.min_height.into(),
        ));
        window.setFrameAutosaveName(&NSString::from_str("Project Library"));
        let toolbar = NSToolbar::initWithIdentifier(
            NSToolbar::alloc(mtm),
            &NSString::from_str("pm.project-library"),
        );
        window.setToolbar(Some(&toolbar));
        window.setToolbarStyle(NSWindowToolbarStyle::Unified);
        let split = NSSplitViewController::new(mtm);
        let sidebar = NSTableView::new(mtm);
        sidebar.setHeaderView(None);
        sidebar.setStyle(NSTableViewStyle::SourceList);
        let column = NSTableColumn::initWithIdentifier(
            NSTableColumn::alloc(mtm),
            &NSString::from_str("section"),
        );
        sidebar.addTableColumn(&column);
        let source = NSViewController::new(mtm);
        source.setView(&sidebar);
        let item = NSSplitViewItem::sidebarWithViewController(&source);
        item.setMinimumThickness(metrics.sidebar.into());
        item.setMaximumThickness(metrics.sidebar.into());
        split.addSplitViewItem(&item);
        let grid = NSCollectionView::new(mtm);
        let layout = NSCollectionViewGridLayout::new(mtm);
        layout.setMinimumInteritemSpacing(tokens.metrics.gap.into());
        layout.setMinimumLineSpacing(tokens.metrics.gap.into());
        // Width is specified. Height comes from the shell's measured thumbnail
        // slot; until then the native collection stays an empty placeholder.
        // DESIGN-TODO(opus): specify the Library thumbnail slot aspect/row height
        // and badge insets for unknown/mixed-aspect projects; no guessed geometry.
        layout.setMinimumItemSize(NSSize::new(metrics.thumbnail_min.into(), 0.));
        grid.setCollectionViewLayout(Some(&layout));
        grid.setAccessibilityLabel(Some(&NSString::from_str(TITLE)));
        let scroll = NSScrollView::new(mtm);
        scroll.setHasVerticalScroller(true);
        scroll.setDocumentView(Some(&grid));
        let content = NSViewController::new(mtm);
        content.setView(&scroll);
        split.addSplitViewItem(&NSSplitViewItem::splitViewItemWithViewController(&content));
        window.setContentViewController(Some(&split));
        let list = NSTableView::new(mtm);
        list.setRowHeight(tokens.metrics.row.into());
        for label in COLUMNS {
            let column = NSTableColumn::initWithIdentifier(
                NSTableColumn::alloc(mtm),
                &NSString::from_str(label),
            );
            column.setTitle(&NSString::from_str(label));
            column.setSortDescriptorPrototype(Some(
                &NSSortDescriptor::sortDescriptorWithKey_ascending(
                    Some(&NSString::from_str(label)),
                    *label == "Name",
                ),
            ));
            list.addTableColumn(&column);
        }
        Self {
            window,
            split,
            sidebar,
            grid,
            list,
            toolbar,
        }
    }
    /// Install the exact measured slot from the design-approved native host.
    pub fn grid_size(&self, size: NSSize, tokens: &pm_design::Tokens) -> bool {
        if !size.width.is_finite()
            || !size.height.is_finite()
            || size.width < f64::from(tokens.metrics.thumbnail_min)
            || size.height <= 0.
        {
            return false;
        }
        if let Some(layout) = self
            .grid
            .collectionViewLayout()
            .and_then(|l| l.downcast::<NSCollectionViewGridLayout>().ok())
        {
            layout.setMinimumItemSize(size);
            layout.setMaximumItemSize(size);
            true
        } else {
            false
        }
    }
}
/// Render a real native grid item from a measured thumbnail rectangle. Geometry
/// is supplied by the host layout; facts and names remain exact publication data.
/// `image` is predecoded on a worker. Missing thumbnails use the specified film
/// symbol. Duration text is supplied by the system time formatter only if known.
pub struct Card<'a> {
    pub row: &'a Row,
    pub relative_date: &'a str,
    pub duration: Option<&'a str>,
    pub image: Option<&'a NSImage>,
    pub thumbnail: NSSize,
    /// Design-approved badge frame. None leaves a neutral empty slot.
    pub badge_frame: Option<NSRect>,
    pub selected: bool,
}
pub fn grid_card(
    card: Card<'_>,
    tokens: &pm_design::Tokens,
    mtm: MainThreadMarker,
) -> Option<Retained<NSCollectionViewItem>> {
    let Card {
        row,
        relative_date,
        duration,
        image,
        thumbnail,
        badge_frame,
        selected,
    } = card;
    if !thumbnail.width.is_finite()
        || !thumbnail.height.is_finite()
        || thumbnail.width < f64::from(tokens.metrics.thumbnail_min)
        || thumbnail.height <= 0.
    {
        return None;
    }
    let facts = row.facts(relative_date);
    let row_height = f64::from(tokens.metrics.row);
    let text_height = row_height * (1. + f64::from(facts.is_some()));
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::ZERO,
            NSSize::new(thumbnail.width, thumbnail.height + text_height),
        ),
    );
    view.setAccessibilityElement(true);
    view.setAccessibilityLabel(
        row.ax_label(relative_date)
            .as_deref()
            .map(NSString::from_str)
            .as_deref(),
    );
    view.setIdentifier(Some(&NSString::from_str(&row.id)));
    let well = NSBox::initWithFrame(
        NSBox::alloc(mtm),
        NSRect::new(NSPoint::new(0., text_height), thumbnail),
    );
    well.setBoxType(NSBoxType::Custom);
    well.setTitlePosition(NSTitlePosition::NoTitle);
    well.setCornerRadius(tokens.metrics.control_radius.into());
    well.setFillColor(&colour(tokens.colors.field));
    well.setBorderWidth(if selected { 2. } else { 0. });
    well.setBorderColor(&colour(tokens.colors.accent));
    let picture = NSImageView::new(mtm);
    picture.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    if let Some(image) = image {
        picture.setImage(Some(image));
    } else {
        picture.setImage(
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str("film"),
                None,
            )
            .as_deref(),
        );
        picture.setContentTintColor(Some(&colour(tokens.colors.muted)));
    }
    well.setContentView(Some(&picture));
    view.addSubview(&well);
    let name = NSTextField::labelWithString(&NSString::from_str(&row.name), mtm);
    name.setFont(Some(&NSFont::systemFontOfSize(
        tokens.typography.body.into(),
    )));
    name.setTextColor(Some(&colour(tokens.colors.text)));
    name.setFrame(NSRect::new(
        NSPoint::new(0., text_height - row_height),
        NSSize::new(thumbnail.width, row_height),
    ));
    name.setDrawsBackground(selected);
    name.setBackgroundColor(Some(&colour(tokens.colors.selected_fill)));
    view.addSubview(&name);
    if let Some(facts) = facts {
        let field = NSTextField::labelWithString(&NSString::from_str(&facts), mtm);
        field.setFont(Some(&NSFont::systemFontOfSize(
            tokens.typography.minimum.into(),
        )));
        field.setTextColor(Some(&colour(tokens.colors.muted)));
        field.setFrame(NSRect::new(
            NSPoint::ZERO,
            NSSize::new(thumbnail.width, row_height),
        ));
        view.addSubview(&field);
    }
    if let Some((duration, frame)) = duration.filter(|s| !s.is_empty()).zip(badge_frame) {
        let dark = pm_design::Tokens::new(pm_design::Scheme::Dark, tokens.accessibility);
        let badge = NSTextField::labelWithString(&NSString::from_str(duration), mtm);
        badge.setFont(Some(&NSFont::systemFontOfSize(
            tokens.typography.minimum.into(),
        )));
        badge.setTextColor(Some(&colour(dark.colors.text)));
        badge.sizeToFit();
        let capsule = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
        capsule.setBoxType(NSBoxType::Custom);
        capsule.setTitlePosition(NSTitlePosition::NoTitle);
        capsule.setBorderWidth(0.);
        capsule.setCornerRadius(frame.size.height / 2.);
        capsule.setFillColor(
            &colour(tokens.colors.media_badge)
                .colorWithAlphaComponent(tokens.metrics.media_badge_opacity.into()),
        );
        capsule.setContentView(Some(&badge));
        view.addSubview(&capsule);
    }
    let item = NSCollectionViewItem::new(mtm);
    item.setView(&view);
    Some(item)
}
