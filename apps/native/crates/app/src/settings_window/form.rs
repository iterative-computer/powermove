//! AppKit presentation of the Settings panel's semantic tree. The panel
//! still owns its values, search results, actions and change listeners.
use crate::{session::Controller, v4_compile::number_facts};
use gpui::Context;
use gpui_macos::native_text::{NativeTextSlot, TextEvent};
use objc2::{
    AllocAnyThread, MainThreadOnly, Message, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject,
};
use objc2_app_kit::*;
use objc2_foundation::{
    NSArray, NSData, NSEdgeInsets, NSNotification, NSNotificationCenter, NSObject,
    NSObjectProtocol, NSPoint, NSSize, NSString, NSURL,
};
use pm_builtins::Publication;
use pm_shell::native_control::{ControlSpec, FormControl, Kind};
use pm_surface::{Id, Node, Surface, events::EventDetail, kit::Component};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

const PAGE_INSET: f64 = 20.;
const PAGE_MAX_WIDTH: f64 = 640.;
const SECTION_SPACING: f64 = 20.;
const ROW_INSET: f64 = 18.;
const ROW_PADDING: f64 = 8.;
const ROW_HEIGHT: f64 = 44.;
const GROUP_RADIUS: f64 = 12.;
const LABEL_SIZE: f64 = 13.;
const FOOTER_SIZE: f64 = 11.;
const LIST_ROW_HEIGHT: f64 = ROW_HEIGHT + 1.;
const LIST_ROWS: usize = 40;

pub(crate) struct Form {
    scroll: Retained<NSScrollView>,
    document: Pile,
    widgets: HashMap<Id, Widget>,
    publication: Option<Publication>,
    reveal: Option<String>,
    recorder: Option<Id>,
}

impl Form {
    pub(super) fn new(scroll: &NSScrollView) -> Self {
        let mtm = scroll.mtm();
        let wrapper: Retained<SettingsDocument> =
            unsafe { msg_send![SettingsDocument::alloc(mtm), init] };
        wrapper.setTranslatesAutoresizingMaskIntoConstraints(false);
        set_document(scroll, &wrapper);
        let document = Pile::new(mtm, false, SECTION_SPACING, 0., PAGE_INSET);
        wrapper.addSubview(&document.stack);
        let available = -2. * PAGE_INSET;
        let fill = document
            .stack
            .widthAnchor()
            .constraintEqualToAnchor_constant(&wrapper.widthAnchor(), available);
        // Fit the column to the window's size without steering window resizing.
        fill.setPriority(NSLayoutPriorityDragThatCannotResizeWindow);
        constraints(&[
            fill,
            document
                .stack
                .topAnchor()
                .constraintEqualToAnchor(&wrapper.topAnchor()),
            document
                .stack
                .bottomAnchor()
                .constraintEqualToAnchor(&wrapper.bottomAnchor()),
            document
                .stack
                .centerXAnchor()
                .constraintEqualToAnchor(&wrapper.centerXAnchor()),
            document
                .stack
                .widthAnchor()
                .constraintLessThanOrEqualToAnchor_constant(&wrapper.widthAnchor(), available),
            document
                .stack
                .widthAnchor()
                .constraintLessThanOrEqualToConstant(PAGE_MAX_WIDTH),
        ]);
        Self {
            scroll: scroll.retain(),
            document,
            widgets: HashMap::new(),
            publication: None,
            reveal: None,
            recorder: None,
        }
    }

    /// A replacement JS realm can reuse node IDs and its generation counter.
    pub(crate) fn reset(&mut self) {
        self.document.set(&[]);
        self.widgets.clear();
        self.publication = None;
        self.recorder = None;
    }

    pub(crate) fn take_publication(&mut self) -> Option<Publication> {
        let publication = self.publication.take();
        self.reset();
        publication
    }

    pub(crate) fn publish(
        &mut self,
        publication: Publication,
        tokens: &pm_design::Tokens,
        cx: &mut Context<Controller>,
    ) {
        if self
            .publication
            .as_ref()
            .is_some_and(|p| p.generation != publication.generation)
        {
            self.reset();
        }
        let recorder = self.layout(&publication, tokens, Some(cx));
        self.publication = Some(publication);
        self.apply_reveal();
        if self.recorder != recorder
            && let Some(widget) = recorder.and_then(|id| self.widgets.get(&id))
            && let Some(window) = self.scroll.window().filter(|window| window.isKeyWindow())
        {
            window.makeFirstResponder(Some(widget.view()));
        }
        self.recorder = recorder;
    }

    fn layout(
        &mut self,
        publication: &Publication,
        tokens: &pm_design::Tokens,
        cx: Option<&mut Context<Controller>>,
    ) -> Option<Id> {
        let surface = &publication.surface;
        let detail = surface
            .nodes()
            .values()
            .find(|node| semantic(node) == Some("settings.detail"));
        let mut render = Render {
            surface,
            widgets: &mut self.widgets,
            seen: HashSet::new(),
            scroll: &self.scroll,
            generation: publication.generation,
            recorder: None,
            tokens,
            cx,
        };
        let views = detail
            .map(|node| render.children(&node.children))
            .unwrap_or_default();
        self.document.set(&views);
        let Render { seen, recorder, .. } = render;
        self.widgets.retain(|id, _| seen.contains(id));
        recorder
    }

    pub(crate) fn reveal(&mut self, section: &str) {
        self.reveal = Some(format!("settings.section.{section}"));
        self.apply_reveal();
    }

    fn apply_reveal(&mut self) {
        let Some(target) = &self.reveal else { return };
        let Some(view) = self
            .publication
            .as_ref()
            .and_then(|p| {
                p.surface
                    .nodes()
                    .values()
                    .find(|node| semantic(node) == Some(target))
            })
            .and_then(|node| self.widgets.get(&node.id))
            .map(Widget::view)
        else {
            return;
        };
        self.document.stack.layoutSubtreeIfNeeded();
        let clip = self.scroll.contentView();
        let rect = view.convertRect_toView(view.bounds(), Some(&clip));
        clip.scrollToPoint(NSPoint::new(0., rect.origin.y - PAGE_INSET));
        self.scroll.reflectScrolledClipView(&clip);
        self.reveal = None;
    }
}

/// Captures the Settings form from synthetic wire publications, offscreen.
#[cfg(debug_assertions)]
pub(crate) fn capture(inputs: &std::path::Path, output: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use objc2_foundation::{NSDictionary, NSRect};
    use pm_design::{Accessibility, Scheme, Tokens};

    let mtm = objc2::MainThreadMarker::new().context("Settings capture needs the main thread")?;
    std::fs::create_dir_all(output)?;
    let mut hidden_titles = 0;
    for entry in std::fs::read_dir(inputs)? {
        let file = entry?.path();
        if file.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let name = file
            .file_stem()
            .context("Settings fixture name")?
            .to_string_lossy();
        let commits: Vec<pm_surface::Commit> = serde_json::from_slice(&std::fs::read(&file)?)?;
        let mut surface = Surface::default();
        for commit in commits {
            surface
                .apply(commit)
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        }
        let publication = Publication {
            panel: "settings".into(),
            generation: 1,
            surface: std::sync::Arc::new(surface),
        };
        for (theme, scheme, appearance_name) in [
            ("light", Scheme::Light, unsafe { NSAppearanceNameAqua }),
            ("dark", Scheme::Dark, unsafe { NSAppearanceNameDarkAqua }),
        ] {
            objc2::rc::autoreleasepool(|_| -> anyhow::Result<()> {
                let appearance = NSAppearance::appearanceNamed(appearance_name)
                    .context("Settings capture appearance")?;
                let bounds = NSRect::new(
                    NSPoint::new(0., 0.),
                    NSSize::new(PAGE_MAX_WIDTH + 2. * PAGE_INSET, 870.),
                );
                let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), bounds);
                scroll.setAppearance(Some(&appearance));
                scroll.setDrawsBackground(true);
                scroll.setBackgroundColor(&pm_shell::settings_window::form_background(false));
                let form = std::cell::RefCell::new(Form::new(&scroll));
                let tokens = Tokens::new(scheme, Accessibility::default());
                appearance.performAsCurrentDrawingAppearance(&block2::StackBlock::new(|| {
                    form.borrow_mut().layout(&publication, &tokens, None);
                    scroll.layoutSubtreeIfNeeded();
                }));
                if let Some(filter) = publication.surface.nodes().values().find(|node| {
                    label(node) == "Extension status"
                        && Component::of(node) == Some(Component::SegmentedControl)
                }) {
                    let form = form.borrow();
                    let control = form.widgets.get(&filter.id)
                        .and_then(|widget| widget.view().downcast_ref::<NSSegmentedControl>())
                        .context("Settings status filter needs the system segmented control")?;
                    let (_, selected, _) = control_value(
                        &publication.surface,
                        filter,
                        Some(Component::SegmentedControl),
                    );
                    anyhow::ensure!(
                        selected.as_i64() == Some(control.selectedSegment() as i64),
                        "Settings status filter lost its selected segment",
                    );
                }
                capture_row_titles(&form.borrow(), &publication.surface, &name, theme, "initial");
                appearance.performAsCurrentDrawingAppearance(&block2::StackBlock::new(|| {
                    scroll.setNeedsLayout(true);
                    scroll.layoutSubtreeIfNeeded();
                    scroll.displayIfNeeded();
                }));
                hidden_titles += capture_row_titles(
                    &form.borrow(),
                    &publication.surface,
                    &name,
                    theme,
                    "settled",
                );
                let bitmap = scroll
                    .bitmapImageRepForCachingDisplayInRect(bounds)
                    .context("offscreen Settings bitmap")?;
                appearance.performAsCurrentDrawingAppearance(&block2::StackBlock::new(|| {
                    scroll.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
                }));
                let png = unsafe {
                    bitmap.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                }
                .context("encode Settings bitmap")?;
                std::fs::write(output.join(format!("settings-{name}-{theme}.png")), unsafe {
                    png.as_bytes_unchecked()
                })?;
                Ok(())
            })?;
        }
    }
    anyhow::ensure!(hidden_titles == 0, "Settings review found {hidden_titles} unusable row titles");
    Ok(())
}

/// Regresses the leading labels on the same AppKit view tree the capture paints.
#[cfg(debug_assertions)]
fn capture_row_titles(form: &Form, surface: &Surface, fixture: &str, theme: &str, pass: &str) -> usize {
    let mut unusable = 0;
    for expected in ["Sign-Ins from the Original App", "Powermove Store"] {
        let field = form.widgets.iter().find_map(|(id, widget)| match widget {
            Widget::Label(field) if surface.node(*id).is_some_and(|node| text(surface, node) == expected) => {
                Some(field)
            }
            _ => None,
        });
        let mut record = json!({
            "fixture":fixture, "theme":theme, "pass":pass, "title":expected,
            "present":field.is_some()
        });
        let visible = field.is_some_and(|field| {
            let value = field.stringValue().to_string();
            let frame = field.frame();
            let hidden = field.isHiddenOrHasHiddenAncestor();
            let mut alpha = field.alphaValue();
            // The retained view hierarchy stays on AppKit's main thread.
            let mut ancestor = unsafe { field.superview() };
            while let Some(view) = ancestor {
                alpha *= view.alphaValue();
                ancestor = unsafe { view.superview() };
            }
            record["stringValue"] = json!(value);
            record["frame"] = json!([frame.origin.x,frame.origin.y,frame.size.width,frame.size.height]);
            record["hidden"] = json!(hidden);
            record["alpha"] = json!(alpha);
            value == expected && !hidden && alpha > 0. && frame.size.width > 1. && frame.size.height > 1.
        });
        record["usable"] = json!(visible);
        eprintln!("SETTINGS_ROW_TITLE {record}");
        unusable += usize::from(!visible);
    }
    unusable
}

pub(super) fn semantic(node: &Node) -> Option<&str> {
    node.props.get("id").and_then(Value::as_str)
}

fn label(node: &Node) -> &str {
    node.props
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn metadata(node: &Node) -> &Value {
    node.props.get("metadata").unwrap_or(&Value::Null)
}

fn layout(node: &Node) -> &str {
    metadata(node)["settingsLayout"].as_str().unwrap_or("")
}

fn disabled(node: &Node) -> bool {
    node.props
        .get("disabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn constraints(items: &[Retained<NSLayoutConstraint>]) {
    NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(items));
}

fn set_document(scroll: &NSScrollView, document: &NSView) {
    scroll.setAutomaticallyAdjustsContentInsets(false);
    scroll.setDocumentView(Some(document));
    let clip = scroll.contentView();
    constraints(&[
        document
            .leadingAnchor()
            .constraintEqualToAnchor(&clip.leadingAnchor()),
        document
            .topAnchor()
            .constraintEqualToAnchor(&clip.topAnchor()),
        document
            .widthAnchor()
            .constraintEqualToAnchor(&clip.widthAnchor()),
    ]);
}

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSettingsDocument"]
    struct SettingsDocument;

    impl SettingsDocument {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
    }
);

define_class!(
    #[unsafe(super(NSStackView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSettingsStack"]
    struct SettingsStack;

    impl SettingsStack {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
    }
);

struct Pile {
    stack: Retained<NSStackView>,
    horizontal: bool,
}

impl Pile {
    fn new(
        mtm: objc2::MainThreadMarker,
        horizontal: bool,
        spacing: f64,
        inset_x: f64,
        inset_y: f64,
    ) -> Self {
        let flipped: Retained<SettingsStack> =
            unsafe { msg_send![SettingsStack::alloc(mtm), init] };
        let stack: Retained<NSStackView> = Retained::into_super(flipped);
        stack.setTranslatesAutoresizingMaskIntoConstraints(false);
        stack.setOrientation(if horizontal {
            NSUserInterfaceLayoutOrientation::Horizontal
        } else {
            NSUserInterfaceLayoutOrientation::Vertical
        });
        stack.setAlignment(if horizontal {
            NSLayoutAttribute::CenterY
        } else {
            NSLayoutAttribute::Leading
        });
        stack.setDistribution(NSStackViewDistribution::Fill);
        stack.setEdgeInsets(NSEdgeInsets {
            top: inset_y,
            left: inset_x,
            bottom: inset_y,
            right: inset_x,
        });
        stack.setSpacing(spacing);
        Self { stack, horizontal }
    }

    fn set(&self, views: &[Retained<NSView>]) {
        let old = self.stack.arrangedSubviews();
        if old.iter().zip(views).all(|(a, b)| std::ptr::eq(&*a, &**b)) && old.len() == views.len() {
            return;
        }
        for previous in old.iter() {
            if !views.iter().any(|view| std::ptr::eq(&*previous, &**view)) {
                self.stack.removeArrangedSubview(&previous);
                previous.removeFromSuperview();
            }
        }
        for (index, view) in views.iter().enumerate() {
            let current = self.stack.arrangedSubviews();
            if index < current.len() && std::ptr::eq(&*current.objectAtIndex(index), &**view) {
                continue;
            }
            let already = current.iter().any(|at| std::ptr::eq(&*at, &**view));
            if already {
                self.stack.removeArrangedSubview(view);
            }
            self.stack
                .insertArrangedSubview_atIndex(view, index as isize);
            if !self.horizontal && !already {
                let inset = self.stack.edgeInsets();
                view.widthAnchor()
                    .constraintEqualToAnchor_constant(
                        &self.stack.widthAnchor(),
                        -(inset.left + inset.right),
                    )
                    .setActive(true);
            }
        }
    }
}

struct Group {
    box_view: Retained<NSBox>,
    rows: Pile,
    separators: Vec<Retained<NSView>>,
}

impl Group {
    fn new(mtm: objc2::MainThreadMarker) -> Self {
        let box_view = NSBox::new(mtm);
        box_view.setBoxType(NSBoxType::Custom);
        box_view.setTitlePosition(NSTitlePosition::NoTitle);
        box_view.setBorderWidth(0.);
        box_view.setCornerRadius(GROUP_RADIUS);
        box_view.setFillColor(&pm_shell::settings_window::form_background(true));
        box_view.setTranslatesAutoresizingMaskIntoConstraints(false);
        let rows = Pile::new(mtm, false, 0., 0., 0.);
        let content = box_view.contentView().expect("NSBox has a content view");
        box_view.setContentViewMargins(NSSize::ZERO);
        content.addSubview(&rows.stack);
        constraints(&[
            rows.stack
                .leadingAnchor()
                .constraintEqualToAnchor(&content.leadingAnchor()),
            rows.stack
                .trailingAnchor()
                .constraintEqualToAnchor(&content.trailingAnchor()),
            rows.stack
                .topAnchor()
                .constraintEqualToAnchor(&content.topAnchor()),
            rows.stack
                .bottomAnchor()
                .constraintEqualToAnchor(&content.bottomAnchor()),
        ]);
        Self {
            box_view,
            rows,
            separators: Vec::new(),
        }
    }

    fn set(&mut self, views: Vec<Retained<NSView>>) {
        let count = views.len().saturating_sub(1);
        self.separators.truncate(count);
        while self.separators.len() < count {
            self.separators.push(separator(self.box_view.mtm()));
        }
        let mut spaced = Vec::with_capacity(views.len() + count);
        for (i, view) in views.into_iter().enumerate() {
            if i != 0 {
                spaced.push(self.separators[i - 1].clone());
            }
            spaced.push(view);
        }
        self.rows.set(&spaced);
    }
}

fn separator(mtm: objc2::MainThreadMarker) -> Retained<NSView> {
    let view = NSView::new(mtm);
    let line = NSBox::new(mtm);
    line.setBoxType(NSBoxType::Separator);
    line.setTranslatesAutoresizingMaskIntoConstraints(false);
    view.setTranslatesAutoresizingMaskIntoConstraints(false);
    view.addSubview(&line);
    constraints(&[
        line.leadingAnchor()
            .constraintEqualToAnchor_constant(&view.leadingAnchor(), ROW_INSET),
        line.trailingAnchor()
            .constraintEqualToAnchor_constant(&view.trailingAnchor(), -ROW_INSET),
        line.topAnchor().constraintEqualToAnchor(&view.topAnchor()),
        line.bottomAnchor()
            .constraintEqualToAnchor(&view.bottomAnchor()),
        line.heightAnchor().constraintEqualToConstant(1.),
    ]);
    view
}

fn flexible_space(mtm: objc2::MainThreadMarker) -> Retained<NSView> {
    let space = NSView::new(mtm);
    space.setTranslatesAutoresizingMaskIntoConstraints(false);
    space.setContentHuggingPriority_forOrientation(1., NSLayoutConstraintOrientation::Horizontal);
    space
}

struct Row {
    body: Pile,
    line: Pile,
    space: Retained<NSView>,
}

impl Row {
    fn new(mtm: objc2::MainThreadMarker) -> Self {
        let body = Pile::new(mtm, false, 2., ROW_INSET, ROW_PADDING);
        let line = Pile::new(mtm, true, 8., 0., 0.);
        line.stack
            .heightAnchor()
            .constraintGreaterThanOrEqualToConstant(ROW_HEIGHT - 2. * ROW_PADDING)
            .setActive(true);
        Self {
            body,
            line,
            space: flexible_space(mtm),
        }
    }

    fn set(
        &self,
        leading: Option<Retained<NSView>>,
        trailing: Vec<Retained<NSView>>,
        footers: Vec<Retained<NSView>>,
    ) {
        let mut views = Vec::new();
        if let Some(leading) = leading {
            leading.setContentCompressionResistancePriority_forOrientation(
                NSLayoutPriorityDefaultHigh - 1.,
                NSLayoutConstraintOrientation::Horizontal,
            );
            views.push(leading);
        }
        views.push(self.space.clone());
        views.extend(trailing);
        self.line.set(&views);
        self.body.set(
            &std::iter::once(Retained::into_super(self.line.stack.clone()))
                .chain(footers)
                .collect::<Vec<_>>(),
        );
    }
}

struct VirtualList {
    scroll: Retained<NSScrollView>,
    rows: Group,
    before_height: Retained<NSLayoutConstraint>,
    after_height: Retained<NSLayoutConstraint>,
    height: Retained<NSLayoutConstraint>,
    observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    count: Rc<std::cell::Cell<usize>>,
    anchor: Option<(String, usize)>,
}

impl VirtualList {
    fn new(mtm: objc2::MainThreadMarker, sender: async_channel::Sender<(usize, usize)>) -> Self {
        let scroll = NSScrollView::new(mtm);
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setTranslatesAutoresizingMaskIntoConstraints(false);
        let document = Pile::new(mtm, false, 0., 0., 0.);
        set_document(&scroll, &document.stack);
        let rows = Group::new(mtm);
        rows.box_view.setFillColor(&NSColor::clearColor());
        let before = NSView::new(mtm);
        let after = NSView::new(mtm);
        for view in [&before, &after] {
            view.setTranslatesAutoresizingMaskIntoConstraints(false);
        }
        document.set(&[
            before.clone(),
            Retained::into_super(rows.box_view.clone()),
            after.clone(),
        ]);
        let before_height = before.heightAnchor().constraintEqualToConstant(0.);
        let after_height = after.heightAnchor().constraintEqualToConstant(0.);
        let height = scroll.heightAnchor().constraintEqualToConstant(0.);
        constraints(&[before_height.clone(), after_height.clone(), height.clone()]);
        let clip = scroll.contentView();
        clip.setPostsBoundsChangedNotifications(true);
        let weak = objc2::rc::Weak::from(&*clip);
        let count = Rc::new(std::cell::Cell::new(0));
        let rows_count = count.clone();
        let sent = std::cell::Cell::new(None);
        let notify = block2::RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
            // Bounds notifications of this main-thread clip view run synchronously.
            let Some(clip) = weak.load() else { return };
            let count = rows_count.get();
            if count <= LIST_ROWS {
                return;
            }
            let bounds = clip.bounds();
            let row = (bounds.origin.y.max(0.) / LIST_ROW_HEIGHT) as usize;
            let end = (row + LIST_ROWS).min(count);
            let range = (row.min(end), end);
            if sent.replace(Some(range)) != Some(range) {
                let _ = sender.try_send(range);
            }
        });
        let observer = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(NSViewBoundsDidChangeNotification),
                Some(&clip),
                None,
                &notify,
            )
        };
        Self {
            scroll,
            rows,
            before_height,
            after_height,
            height,
            observer,
            count,
            anchor: None,
        }
    }

    fn update(&mut self, node: &Node, surface: &Surface, views: Vec<Retained<NSView>>) {
        let count = node.props.get("count").and_then(Value::as_u64).unwrap_or(0) as usize;
        self.count.set(count);
        let start = node
            .props
            .get("range")
            .and_then(|v| v[0].as_u64())
            .unwrap_or(0) as usize;
        self.height
            .setConstant(count.min(LIST_ROWS) as f64 * LIST_ROW_HEIGHT - 1.);
        let after = count.saturating_sub(start + views.len());
        self.before_height
            .setConstant(start as f64 * LIST_ROW_HEIGHT);
        self.after_height
            .setConstant(after as f64 * LIST_ROW_HEIGHT);
        self.scroll.setHasVerticalScroller(count > LIST_ROWS);
        self.rows.set(views);

        let anchor = metadata(node)["anchorId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .and_then(|id| {
                let key = format!("settings.extension.{id}");
                node.children
                    .iter()
                    .position(|id| surface.node(*id).and_then(semantic) == Some(&key))
                    .map(|index| (id.to_owned(), start + index))
            });
        let clip = self.scroll.contentView();
        let offset = match (&self.anchor, &anchor) {
            (Some((old_id, old_index)), Some((new_id, new_index)))
                if old_id == new_id && old_index != new_index =>
            {
                Some(
                    clip.bounds().origin.y
                        + (*new_index as f64 - *old_index as f64) * LIST_ROW_HEIGHT,
                )
            }
            (Some(_), None) => Some(0.),
            _ => None,
        };
        self.anchor = anchor;
        if let Some(offset) = offset {
            self.scroll.layoutSubtreeIfNeeded();
            clip.scrollToPoint(NSPoint::new(0., offset.max(0.)));
            self.scroll.reflectScrolledClipView(&clip);
        }
    }
}

impl Drop for VirtualList {
    fn drop(&mut self) {
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(self.observer.as_ref()) };
    }
}

enum Widget {
    Pile(Pile),
    Group(Group),
    Header(Pile, Retained<NSView>, usize),
    Row(Row),
    Virtual(VirtualList),
    Control(FormControl),
    Text {
        slot: Rc<NativeTextSlot>,
        container: Retained<NSView>,
    },
    Label(Retained<NSTextField>),
    Path(Retained<NSPathControl>),
    Progress(Retained<NSProgressIndicator>),
    Image {
        view: Retained<NSImageView>,
        resource: Rc<std::cell::RefCell<Option<String>>>,
    },
    Secure(Retained<NSSecureTextField>),
}

impl Widget {
    fn view(&self) -> &NSView {
        match self {
            Self::Pile(value) | Self::Header(value, ..) => &value.stack,
            Self::Group(value) => &value.box_view,
            Self::Row(value) => &value.body.stack,
            Self::Virtual(value) => &value.scroll,
            Self::Control(value) => value.view(),
            Self::Text { container, .. } => container,
            Self::Label(value) => value,
            Self::Path(value) => value,
            Self::Progress(value) => value,
            Self::Image { view, .. } => view,
            Self::Secure(value) => value,
        }
    }
}

struct Render<'a, 'cx> {
    surface: &'a Surface,
    widgets: &'a mut HashMap<Id, Widget>,
    seen: HashSet<Id>,
    scroll: &'a NSScrollView,
    generation: u64,
    recorder: Option<Id>,
    tokens: &'a pm_design::Tokens,
    cx: Option<&'a mut Context<'cx, Controller>>,
}

impl Render<'_, '_> {
    fn children(&mut self, ids: &[Id]) -> Vec<Retained<NSView>> {
        ids.iter().filter_map(|&id| self.node(id)).collect()
    }

    fn node(&mut self, id: Id) -> Option<Retained<NSView>> {
        let node = self.surface.node(id)?;
        let component = Component::of(node);
        let layout = layout(node);
        if node.r#type == "text" && node.text.is_empty() && node.children.is_empty() {
            return None;
        }
        let mut widget = self
            .widgets
            .remove(&id)
            .unwrap_or_else(|| self.create(node, component));
        match &mut widget {
            Widget::Group(group) => group.set(self.children(&node.children)),
            Widget::Virtual(list) => list.update(node, self.surface, self.children(&node.children)),
            Widget::Row(row) => {
                let mut leading = None;
                let mut footers = Vec::new();
                let mut trailing = Vec::new();
                for child in node.children.iter().filter_map(|id| self.surface.node(*id)) {
                    match self::layout(child) {
                        "copy" => {
                            let mut contents = self.children(&child.children).into_iter();
                            leading = contents.next();
                            footers.extend(contents);
                        }
                        "nameButton" => leading = self.node(child.id),
                        "error" => footers.extend(self.node(child.id)),
                        _ => trailing.extend(self.node(child.id)),
                    }
                }
                row.set(leading, trailing, footers);
            }
            Widget::Pile(pile) => pile.set(&self.children(&node.children)),
            Widget::Header(pile, space, leading) => {
                let mut views = self.children(&node.children);
                if views.len() > *leading {
                    views.insert(*leading, space.clone());
                }
                pile.set(&views);
            }
            Widget::Control(control) => {
                let (kind, value, options) = control_value(self.surface, node, component);
                control.update(&value, &options, disabled(node));
                if matches!(kind, Some(Kind::Push)) {
                    let title = button_title(self.surface, node);
                    control.set_title(&title);
                }
                if matches!(layout, "shortcutButton" | "shortcutRecorder") {
                    control
                        .view()
                        .setAccessibilityLabel(Some(&NSString::from_str(label(node))));
                    if layout == "shortcutRecorder" {
                        self.recorder = Some(id);
                    }
                }
            }
            Widget::Text { slot: field, .. } => {
                let value = field_value(node);
                if field.value() != value {
                    field.set_value(&value);
                }
                if let Some(native) = field.native_view().downcast_ref::<NSTextField>() {
                    native.setEnabled(!disabled(node));
                }
            }
            Widget::Label(field) => {
                let title = text(self.surface, node);
                let title = NSString::from_str(&title);
                if field.stringValue() != title {
                    field.setStringValue(&title);
                }
            }
            Widget::Path(path) => {
                let value = text(self.surface, node);
                let value = NSString::from_str(&value);
                let expanded = value.stringByExpandingTildeInPath();
                if path.URL().and_then(|url| url.path()).as_deref() != Some(&*expanded) {
                    path.setURL(Some(&NSURL::fileURLWithPath(&expanded)));
                    path.setToolTip(Some(&expanded));
                }
            }
            Widget::Progress(progress) => {
                let value = node.props.get("aria-valuenow").and_then(Value::as_f64);
                progress.setIndeterminate(value.is_none());
                if let Some(value) = value {
                    progress.setDoubleValue(value);
                } else {
                    unsafe { progress.startAnimation(None) };
                }
            }
            Widget::Secure(field) => field.setEnabled(!disabled(node)),
            Widget::Image { view, resource } => {
                let key = node.props.get("resource").and_then(Value::as_str);
                if let Some(key) = key.filter(|key| resource.borrow().as_deref() != Some(*key))
                    && let Some(cx) = self.cx.as_deref_mut()
                {
                    *resource.borrow_mut() = Some(key.to_owned());
                    load_poster(view, resource.clone(), key.to_owned(), cx);
                }
            }
        }
        let view = widget.view().retain();
        if let Some(id) = semantic(node) {
            view.setAccessibilityIdentifier(Some(&NSString::from_str(id)));
        }
        if layout == "section" {
            view.setAccessibilityLabel(Some(&NSString::from_str(label(node))));
        }
        self.seen.insert(id);
        self.widgets.insert(id, widget);
        Some(view)
    }

    fn create(&mut self, node: &Node, component: Option<Component>) -> Widget {
        let mtm = self.scroll.mtm();
        if node.r#type == "virtual-list" {
            let (tx, rx) = async_channel::unbounded();
            let (id, generation) = (node.id, self.generation);
            if let Some(cx) = self.cx.as_deref_mut() {
                cx
                    .spawn(async move |owner, cx| {
                        while let Ok(mut range) = rx.recv().await {
                            while let Ok(latest) = rx.try_recv() {
                                range = latest;
                            }
                            if owner
                                .update(cx, |owner, cx| {
                                    if owner.settings_node(generation, id).is_some() {
                                        owner.settings_event(
                                            id,
                                            "rangechange",
                                            EventDetail {
                                                range: Some(range),
                                                ..Default::default()
                                            },
                                            cx,
                                        );
                                    }
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
            }
            return Widget::Virtual(VirtualList::new(mtm, tx));
        }
        match component {
            Some(Component::FormGroup) => return Widget::Group(Group::new(mtm)),
            Some(Component::NumberField | Component::TextField)
                if !metadata(node)["search"].as_bool().unwrap_or(false) =>
            {
                return self.text_field(node, component == Some(Component::NumberField));
            }
            Some(Component::SecureField) => {
                let field = NSSecureTextField::new(mtm);
                field.setControlSize(NSControlSize::Regular);
                field.setFocusRingType(NSFocusRingType::None);
                field.setFont(Some(&NSFont::systemFontOfSize(LABEL_SIZE)));
                field.setAccessibilityLabel(Some(&NSString::from_str(label(node))));
                field.setTranslatesAutoresizingMaskIntoConstraints(false);
                field
                    .widthAnchor()
                    .constraintEqualToConstant(160.)
                    .setActive(true);
                return Widget::Secure(field);
            }
            Some(Component::ProgressRow) => {
                let progress = NSProgressIndicator::new(mtm);
                progress.setTranslatesAutoresizingMaskIntoConstraints(false);
                progress.setControlSize(NSControlSize::Regular);
                progress.setStyle(NSProgressIndicatorStyle::Bar);
                progress.setMinValue(0.);
                progress.setMaxValue(1.);
                progress.setAccessibilityLabel(Some(&NSString::from_str(label(node))));
                progress
                    .widthAnchor()
                    .constraintEqualToConstant(96.)
                    .setActive(true);
                return Widget::Progress(progress);
            }
            _ => {}
        }
        let (kind, value, options) = control_value(self.surface, node, component);
        if let Some(kind) = kind {
            return self.control(node, kind, &value, options);
        }
        match layout(node) {
            "row" => Widget::Row(Row::new(mtm)),
            "header" => Widget::Header(
                Pile::new(mtm, true, ROW_INSET, ROW_INSET, ROW_INSET),
                flexible_space(mtm),
                2,
            ),
            "headingLine" => Widget::Header(
                Pile::new(mtm, true, ROW_INSET, ROW_INSET, 0.),
                flexible_space(mtm),
                1,
            ),
            "line" => Widget::Pile(Pile::new(mtm, true, 6., 0., 0.)),
            "group" => Widget::Pile(Pile::new(mtm, false, 8., 0., 0.)),
            "section" => Widget::Pile(Pile::new(mtm, false, SECTION_SPACING, 0., 0.)),
            "note" => Widget::Pile(Pile::new(mtm, false, 3., ROW_INSET, 8.)),
            "path" => {
                let path = NSPathControl::new(mtm);
                path.setFocusRingType(NSFocusRingType::None);
                path.setTranslatesAutoresizingMaskIntoConstraints(false);
                path.setPathStyle(NSPathStyle::PopUp);
                path.setEditable(false);
                path.setControlSize(NSControlSize::Regular);
                path.setAccessibilityLabel(Some(&NSString::from_str("Location")));
                path.widthAnchor()
                    .constraintLessThanOrEqualToConstant(210.)
                    .setActive(true);
                Widget::Path(path)
            }
            _ if layout(node) == "poster" || node.r#type == "img" => Widget::Image {
                view: settings_image(mtm, node),
                resource: Rc::default(),
            },
            _ if node.r#type == "text"
                || node.props.get("role").and_then(Value::as_str) == Some("status") =>
            {
                Widget::Label(text_label(mtm, node))
            }
            _ => {
                let inset = if component == Some(Component::EmptyState) {
                    ROW_INSET
                } else {
                    0.
                };
                Widget::Pile(Pile::new(mtm, false, 4., inset, inset))
            }
        }
    }

    fn control(&mut self, node: &Node, kind: Kind, value: &Value, options: Vec<Value>) -> Widget {
        let name = label(node);
        let title = if matches!(kind, Kind::Push) {
            button_title(self.surface, node)
        } else {
            name.to_owned()
        };
        let identifier = semantic(node).unwrap_or(name);
        let meta = metadata(node);
        let (tx, rx) = async_channel::unbounded();
        let button_role = meta["pmKit"]["buttonRole"].as_str();
        let control = FormControl::new(
            &ControlSpec {
                kind,
                label: if title.is_empty() { name } else { &title },
                identifier,
                value,
                options,
                disabled: disabled(node),
                default: button_role == Some("default"),
                destructive: button_role == Some("destructive"),
                inline: false,
                quiet: false,
                pull: node.props.get("data-pm-variant").and_then(Value::as_str) == Some("pulldown"),
                tint: None,
                symbol: meta["symbol"].as_str(),
                glass: false,
                well: false,
            },
            self.tokens,
            tx,
        );
        let view = control.view();
        view.setTranslatesAutoresizingMaskIntoConstraints(false);
        view.setAccessibilityLabel(Some(&NSString::from_str(name)));
        if let Some(help) = node
            .props
            .get("aria-description")
            .and_then(Value::as_str)
            .filter(|help| !help.is_empty())
        {
            view.setAccessibilityHelp(Some(&NSString::from_str(help)));
        }
        if let Some(placeholder) = meta["placeholder"].as_str() {
            control.set_placeholder(placeholder);
        }
        if layout(node) == "nameButton"
            && let Some(button) = view.downcast_ref::<NSButton>()
        {
            button.setBordered(false);
            button.setAlignment(NSTextAlignment::Left);
            button.setRefusesFirstResponder(true);
        }
        if layout(node) == "filterField" {
            view.widthAnchor()
                .constraintGreaterThanOrEqualToConstant(175.)
                .setActive(true);
            view.setContentHuggingPriority_forOrientation(
                NSLayoutPriorityDefaultLow - 1.,
                NSLayoutConstraintOrientation::Horizontal,
            );
        } else if matches!(kind, Kind::Search | Kind::Slider) {
            view.widthAnchor()
                .constraintEqualToConstant(175.)
                .setActive(true);
        } else if matches!(layout(node), "shortcutButton" | "shortcutRecorder") {
            view.widthAnchor()
                .constraintGreaterThanOrEqualToConstant(88.)
                .setActive(true);
        }
        let (id, generation) = (node.id, self.generation);
        if let Some(cx) = self.cx.as_deref_mut() {
            cx
                .spawn(async move |owner, cx| {
                    while let Ok(value) = rx.recv().await {
                        if owner
                            .update(cx, |owner, cx| {
                                owner.settings_control(generation, id, kind, value, cx);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
        }
        Widget::Control(control)
    }

    fn text_field(&mut self, node: &Node, number: bool) -> Widget {
        let slot = Rc::new(NativeTextSlot::new_form(
            self.scroll,
            label(node),
            &field_value(node),
            number,
            metadata(node)["stepper"].as_bool().unwrap_or(false),
        ));
        if let Some(placeholder) = metadata(node)["placeholder"].as_str() {
            slot.set_placeholder(placeholder);
        }
        let field = slot.form_view();
        field.setTranslatesAutoresizingMaskIntoConstraints(false);
        field
            .widthAnchor()
            .constraintEqualToConstant(if number { 98. } else { 190. })
            .setActive(true);
        let (tx, rx) = async_channel::unbounded();
        slot.on_event(Rc::new(move || {
            let _ = tx.try_send(());
        }));
        let weak = Rc::downgrade(&slot);
        let (id, generation) = (node.id, self.generation);
        if let Some(cx) = self.cx.as_deref_mut() {
            cx
                .spawn(async move |owner, cx| {
                    while rx.recv().await.is_ok() {
                        let Some(slot) = weak.upgrade() else { break };
                        for event in slot.take_events() {
                            if owner
                                .update(cx, |owner, cx| {
                                    owner.settings_text(generation, id, &slot, event, cx);
                                })
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                })
                .detach();
        }
        let container = if let Some(unit) = node.props.get("unit").and_then(Value::as_str) {
            let label = NSTextField::labelWithString(&NSString::from_str(unit), field.mtm());
            label.setTranslatesAutoresizingMaskIntoConstraints(false);
            label.setFont(Some(&NSFont::systemFontOfSize(FOOTER_SIZE)));
            label.setTextColor(Some(&NSColor::secondaryLabelColor()));
            let with_unit = Pile::new(field.mtm(), true, 5., 0., 0.);
            with_unit.set(&[field.retain(), (label.as_ref() as &NSView).retain()]);
            Retained::into_super(with_unit.stack)
        } else {
            field.retain()
        };
        Widget::Text { slot, container }
    }
}

fn button_title(surface: &Surface, node: &Node) -> String {
    let content = text(surface, node);
    if content.is_empty() {
        label(node).to_owned()
    } else {
        content
    }
}

fn text(surface: &Surface, node: &Node) -> String {
    if node.children.is_empty() {
        return node.text.clone();
    }
    node.children
        .iter()
        .filter_map(|id| surface.node(*id))
        .map(|child| text(surface, child))
        .collect()
}

fn control_value(
    surface: &Surface,
    node: &Node,
    component: Option<Component>,
) -> (Option<Kind>, Value, Vec<Value>) {
    let mut value = node.props.get("value").cloned().unwrap_or(Value::Null);
    let mut options = Vec::new();
    let kind = match component {
        Some(Component::PopUpButton) => {
            options = metadata(node)["pmKit"]["menu"]
                .as_array()
                .or_else(|| node.props.get("options").and_then(Value::as_array))
                .cloned()
                .unwrap_or_default();
            Kind::Popup
        }
        Some(Component::SegmentedControl) => {
            for (index, child) in node
                .children
                .iter()
                .filter_map(|id| surface.node(*id))
                .enumerate()
            {
                options.push(json!({"label": label(child), "v": index}));
                if child.props.get("aria-checked") == Some(&Value::Bool(true)) {
                    value = json!(index);
                }
            }
            Kind::Segmented
        }
        Some(Component::Switch) => Kind::Switch,
        Some(Component::ColourWell) => Kind::Well,
        Some(Component::Slider) => {
            options.push(json!({
                "min": node.props.get("min"),
                "max": node.props.get("max"),
                "step": node.props.get("step"),
            }));
            Kind::Slider
        }
        Some(Component::TextField) if metadata(node)["search"] == Value::Bool(true) => Kind::Search,
        Some(Component::PushButton) => Kind::Push,
        _ if node.r#type == "button" => Kind::Push,
        _ => return (None, value, options),
    };
    (Some(kind), value, options)
}

fn text_label(mtm: objc2::MainThreadMarker, node: &Node) -> Retained<NSTextField> {
    let heading = node.props.get("role").and_then(Value::as_str) == Some("heading");
    let style = layout(node);
    let title = NSString::from_str(&node.text);
    let nowrap = node
        .props
        .get("style")
        .and_then(|style| style.get("whiteSpace"))
        .and_then(Value::as_str) == Some("nowrap");
    let field = if nowrap {
        let field = NSTextField::labelWithString(&title, mtm);
        field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        if style == "error" {
            field.setContentCompressionResistancePriority_forOrientation(
                NSLayoutPriorityDefaultHigh + 1.,
                NSLayoutConstraintOrientation::Horizontal,
            );
        }
        field
    } else {
        NSTextField::wrappingLabelWithString(&title, mtm)
    };
    field.setTranslatesAutoresizingMaskIntoConstraints(false);
    if heading {
        field.setFont(Some(&NSFont::systemFontOfSize_weight(LABEL_SIZE, unsafe {
            NSFontWeightSemibold
        })));
    } else {
        let size = if matches!(style, "footer" | "secondary" | "error") {
            FOOTER_SIZE
        } else {
            LABEL_SIZE
        };
        field.setFont(Some(&NSFont::systemFontOfSize(size)));
    }
    if style == "error" {
        field.setTextColor(Some(&NSColor::systemRedColor()));
    } else if matches!(style, "footer" | "secondary" | "mutedHeading") {
        field.setTextColor(Some(&NSColor::secondaryLabelColor()));
    }
    field
}

fn settings_image(mtm: objc2::MainThreadMarker, node: &Node) -> Retained<NSImageView> {
    let image_view = NSImageView::new(mtm);
    image_view.setTranslatesAutoresizingMaskIntoConstraints(false);
    let glyph = layout(node) == "glyph";
    let symbol = metadata(node)["symbol"].as_str().unwrap_or("film");
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(symbol),
        node.props
            .get("alt")
            .and_then(Value::as_str)
            .map(NSString::from_str)
            .as_deref(),
    );
    image_view.setImage(image.as_deref());
    image_view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    constraints(&[
        image_view.widthAnchor().constraintEqualToConstant(if glyph { LABEL_SIZE } else { 100. }),
        image_view.heightAnchor().constraintEqualToConstant(if glyph { LABEL_SIZE } else { 56. }),
    ]);
    image_view
}

fn load_poster(
    view: &NSImageView,
    resource: Rc<std::cell::RefCell<Option<String>>>,
    key: String,
    cx: &mut Context<Controller>,
) {
    let view = objc2::rc::Weak::from(view);
    cx.spawn(async move |_, cx| {
        let requested = key.clone();
        let bytes = cx
            .background_executor()
            .spawn(async move { std::fs::read(crate::media_thumbs::resolve(&requested)?).ok() })
            .await;
        if resource.borrow().as_deref() == Some(&key)
            && let (Some(view), Some(bytes)) = (view.load(), bytes)
        {
            let data = NSData::with_bytes(&bytes);
            if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
                view.setImage(Some(&image));
            }
        }
    })
    .detach();
}

fn field_value(node: &Node) -> String {
    match node.props.get("value") {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => number_facts(node).text(value.as_f64().unwrap_or_default()),
        _ => String::new(),
    }
}

impl Controller {
    fn settings_node(&self, generation: u64, id: Id) -> Option<&Node> {
        let publication = self.builtins.settings_form.as_ref()?.publication.as_ref()?;
        (publication.generation == generation).then_some(())?;
        publication.surface.node(id)
    }

    fn settings_control(
        &mut self,
        generation: u64,
        id: Id,
        kind: Kind,
        value: Value,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.settings_node(generation, id) else {
            return;
        };
        match kind {
            Kind::Push => self.settings_event(id, "click", EventDetail::default(), cx),
            Kind::Segmented => {
                if let Some(&child) = value
                    .as_u64()
                    .and_then(|index| node.children.get(index as usize))
                {
                    self.settings_event(child, "click", EventDetail::default(), cx);
                }
            }
            Kind::Search => self.settings_event(
                id,
                "input",
                EventDetail {
                    value: Some(value),
                    ..Default::default()
                },
                cx,
            ),
            _ => self.settings_event(
                id,
                "change",
                EventDetail {
                    value: Some(value),
                    ..Default::default()
                },
                cx,
            ),
        }
    }

    fn settings_text(
        &mut self,
        generation: u64,
        id: Id,
        slot: &NativeTextSlot,
        event: TextEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.settings_node(generation, id) else {
            return;
        };
        let number = Component::of(node) == Some(Component::NumberField);
        let (event, value) = match event {
            TextEvent::Changed(value) => ("input", json!(value)),
            TextEvent::Committed(value) if number => {
                let Ok(value) = value.trim().parse::<f64>() else {
                    return;
                };
                ("change", json!(value))
            }
            TextEvent::Committed(value) => ("change", json!(value)),
            TextEvent::Key(key @ ("up" | "down" | "shiftUp" | "shiftDown")) if number => {
                let facts = number_facts(node);
                let value = crate::v4_view::arrow_step(&slot.value(), facts, key);
                slot.replace_value(&facts.text(value));
                ("change", json!(value))
            }
            _ => return,
        };
        self.settings_event(
            id,
            event,
            EventDetail {
                value: Some(value),
                ..Default::default()
            },
            cx,
        );
    }

    pub(super) fn settings_event(
        &mut self,
        id: Id,
        event: &str,
        detail: EventDetail,
        cx: &mut Context<Self>,
    ) {
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        let Some(p) = self
            .builtins
            .settings_form
            .as_ref()
            .and_then(|form| form.publication.as_ref())
        else {
            return;
        };
        let Some(node) = p.surface.node(id).filter(|node| !disabled(node)) else {
            return;
        };
        let (generation, revision, semantic) = (
            p.generation,
            p.surface.revision(),
            semantic(node).map(str::to_owned),
        );
        let listen = node.listeners.contains_key(event);
        let binding = node
            .props
            .get("binding")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let event = event.to_owned();
        cx.spawn(async move |this, cx| {
            let ticket = cx
                .background_executor()
                .spawn(async move {
                    if listen {
                        runtime.activate(
                            "settings",
                            generation,
                            revision,
                            id,
                            semantic.as_deref(),
                            &event,
                            detail,
                        )
                    } else if event == "change"
                        && let (Some(binding), Some(value)) = (binding, detail.value)
                    {
                        runtime.write("settings", &binding, value)
                    } else {
                        return None;
                    }
                    .ok()
                })
                .await;
            if let Some(ticket) = ticket {
                if let Err(error) = ticket.await {
                    eprintln!("Settings: {error}");
                }
                let _ = this.update(cx, |owner, cx| owner.drain_builtins(cx));
            }
        })
        .detach();
    }
}
