//! The Agent composer's chooser (PM-727): one native `NSPopover` off the
//! pill, on Capy's layout (the owner's reference). A rail of harness marks
//! with a tile on the chosen one; the models that harness runs, searchable
//! and scrolling, under their account headings, with ⌘1…⌘9 local to the
//! popover; a card for the chosen model: its mark and name, its context when
//! known, Reasoning as a snapping slider with the level's name, and one
//! existing action. The panel's TypeScript sends the rows (`kind` says where
//! each stands) and gets the chosen row's index back as `agentPick:i`, as
//! its menus do; a row that `stays` keeps the popover open for the rows that
//! follow it.
use super::*;
use objc2::Message;
use objc2::runtime::Bool;
use objc2_foundation::NSNotification;

/// Where a row of the chooser stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Harness,
    Heading,
    Model,
    Effort,
    Action,
    /// One quiet line in an empty list (a harness that couldn't start).
    Status,
}
impl Part {
    fn of(kind: &str) -> Option<Self> {
        Some(match kind {
            "harness" => Self::Harness,
            "heading" => Self::Heading,
            "model" => Self::Model,
            "effort" => Self::Effort,
            "action" => Self::Action,
            "status" => Self::Status,
            _ => return None,
        })
    }
}

/// One row as the panel sent it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub part: Option<Part>,
    pub label: String,
    pub detail: String,
    pub icon: Option<String>,
    pub checked: bool,
    pub default: bool,
    pub disabled: bool,
    pub stays: bool,
}

/// The chooser's rows and its card, from the panel's request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spec {
    pub rows: Vec<Row>,
    /// The card's title (the chosen model, or the harness while signed
    /// out) and its mark.
    pub title: Option<(String, Option<String>)>,
    /// The card's facts as (label, value), only those the provider reports.
    pub facts: Vec<(String, String)>,
    /// A quiet line on the card (where to sign in).
    pub note: String,
    pub placeholder: String,
}
impl Spec {
    pub fn from_request(input: &serde_json::Value) -> Self {
        let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_owned();
        let rows = input["options"]
            .as_array()
            .map(|options| {
                options
                    .iter()
                    .map(|o| Row {
                        part: o["kind"].as_str().and_then(Part::of),
                        label: text(&o["label"]),
                        detail: text(&o["detail"]),
                        icon: o["icon"].as_str().map(str::to_owned),
                        checked: o["checked"] == true,
                        default: o["default"] == true,
                        disabled: o["disabled"] == true,
                        stays: o["stays"] == true,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let card = &input["chooser"];
        Self {
            rows,
            title: card["title"]
                .as_str()
                .map(|t| (t.to_owned(), card["icon"].as_str().map(str::to_owned))),
            facts: card["facts"]
                .as_array()
                .map(|facts| {
                    facts
                        .iter()
                        .filter_map(|f| {
                            Some((f[0].as_str()?.to_owned(), f[1].as_str()?.to_owned()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            note: text(&card["note"]),
            placeholder: text(&input["placeholder"]),
        }
    }
    /// The middle list's rows for `query`, in order: a heading stands only
    /// over rows that match; a disabled heading (a source coming later)
    /// shows only while nothing is searched.
    pub fn listed(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        let mut out = vec![];
        let mut heading: Option<usize> = None;
        for (i, row) in self.rows.iter().enumerate() {
            match row.part {
                Some(Part::Heading) => {
                    heading = Some(i);
                    if row.disabled && query.is_empty() {
                        out.push(i);
                        heading = None;
                    }
                }
                Some(Part::Model)
                    if query.is_empty() || row.label.to_lowercase().contains(&query) =>
                {
                    out.extend(heading.take());
                    out.push(i);
                }
                _ => {}
            }
        }
        out
    }
    /// The model rows ⌘1…⌘9 choose, in listed order.
    pub fn numbered(&self, query: &str) -> Vec<usize> {
        self.listed(query)
            .into_iter()
            .filter(|&i| self.rows[i].part == Some(Part::Model) && !self.rows[i].disabled)
            .take(9)
            .collect()
    }
    /// The list after `next` is the same list (the same harness chosen): a
    /// model or a level picked in it keeps the list's scroll position.
    pub fn keeps_place(&self, next: &Spec) -> bool {
        let harness = |spec: &Spec| {
            spec.of(Part::Harness)
                .find(|(_, r)| r.checked)
                .map(|(_, r)| r.label.clone())
        };
        harness(self).is_some() && harness(self) == harness(next)
    }
    fn of(&self, part: Part) -> impl Iterator<Item = (usize, &Row)> {
        self.rows
            .iter()
            .enumerate()
            .filter(move |(_, r)| r.part == Some(part))
    }
}

/// The popover's sizes, all from tokens (measured against Capy's picker in
/// the lane notes).
struct Shape {
    /// The gutter round and between the three surfaces.
    gutter: f64,
    rail: f64,
    tile: f64,
    tile_gap: f64,
    rail_icon: f64,
    list: f64,
    card: f64,
    /// The most the list surface grows to; it scrolls past it.
    most: f64,
    search: f64,
    heading: f64,
    row: f64,
    fact: f64,
    control: f64,
    mark: f64,
    title_mark: f64,
    /// A surface's leading content inset (the mark column starts here).
    inset: f64,
    gap: f64,
    pad: f64,
    radius: f64,
    line: f64,
}
impl Shape {
    fn new(t: &Tokens) -> Self {
        let m = &t.metrics;
        // A hairline: half a point on a Retina screen, a point at 1x.
        let scale = MainThreadMarker::new()
            .and_then(NSScreen::mainScreen)
            .map_or(2., |s| s.backingScaleFactor());
        Self {
            gutter: t.space.sm as f64,
            rail: m.chooser_rail as f64,
            tile: m.control as f64,
            tile_gap: t.space.hair as f64,
            rail_icon: m.chooser_mark as f64,
            list: m.chooser_list as f64,
            card: m.chooser_card as f64,
            most: m.chooser_height as f64,
            search: (m.control + t.space.md) as f64,
            heading: m.hit as f64,
            row: m.row as f64,
            fact: m.field as f64,
            control: m.control as f64,
            mark: m.chooser_mark as f64,
            title_mark: m.symbol_panel as f64,
            inset: t.space.lg as f64,
            gap: t.space.sm as f64,
            pad: t.space.md as f64,
            radius: t.radius.nested(m.popover_radius, t.space.md) as f64,
            line: scale.recip().max(0.5),
        }
    }
    /// Where a list row's label starts: the mark column, then the gap.
    fn label_x(&self) -> f64 {
        self.inset + self.mark + self.gap
    }
}

pub(super) struct State {
    spec: Spec,
    query: String,
    /// The keyboard's row in the middle list (a row index), if any.
    lit: Option<usize>,
    sender: async_channel::Sender<Command>,
    dismiss: Option<String>,
    /// A row that closes the popover was chosen: closing sends no dismissal.
    picked: bool,
    tokens: Tokens,
    popover: Retained<NSPopover>,
    root: Option<Retained<ChooserView>>,
    list: Option<Retained<ChooserView>>,
    scroll: Option<Retained<NSScrollView>>,
    search: Option<Retained<NSTextField>>,
    /// Middle-list rows as drawn: (row index, frame in the list).
    frames: Vec<(usize, NSRect)>,
    highlight: Option<Retained<NSBox>>,
}

define_class!(
    /// A flipped view that is the chooser's target: rows, rail, search and
    /// the access pop-up all send here.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMAgentChooserView"]
    #[ivars = std::rc::Weak<RefCell<State>>]
    pub(super) struct ChooserView;
    impl ChooserView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _: Option<&NSEvent>) -> bool {
            true
        }
        #[unsafe(method(pick:))]
        fn pick(&self, sender: &NSControl) {
            if let Some(state) = self.ivars().upgrade() {
                choose(&state, sender.tag() as usize);
            }
        }
        /// The Reasoning track settled on a level: that level's row.
        #[unsafe(method(reason:))]
        fn reason(&self, sender: &NSSlider) {
            if let Some(state) = self.ivars().upgrade() {
                let level = sender.doubleValue().round().max(0.) as usize;
                let row = state.borrow().spec.of(Part::Effort).nth(level).map(|(i, _)| i);
                if let Some(index) = row {
                    choose(&state, index);
                }
            }
        }
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            for area in self.trackingAreas().to_vec() {
                self.removeTrackingArea(&area);
            }
            use objc2::AnyThread;
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
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
            unsafe { let _: () = msg_send![super(self), updateTrackingAreas]; }
        }
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let at = self.convertPoint_fromView(event.locationInWindow(), None);
            self.hover(Some(at));
        }
        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _: &NSEvent) {
            self.hover(None);
        }
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            self.ivars().upgrade().is_some_and(|state| key(&state, &chord(event)))
        }
    }
    unsafe impl NSObjectProtocol for ChooserView {}
    unsafe impl NSTextFieldDelegate for ChooserView {}
    unsafe impl NSControlTextEditingDelegate for ChooserView {
        #[unsafe(method(controlTextDidChange:))]
        fn text_did_change(&self, _: &NSNotification) {
            if let Some(state) = self.ivars().upgrade() {
                let query = state
                    .borrow()
                    .search
                    .as_ref()
                    .map(|s| s.stringValue().to_string())
                    .unwrap_or_default();
                {
                    let mut s = state.borrow_mut();
                    s.query = query;
                    s.lit = None;
                }
                let _ = build_list(&state);
            }
        }
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _: &NSControl, _: &NSTextView, command: objc2::runtime::Sel) -> Bool {
            let name = match command.name().to_bytes() {
                b"moveUp:" => "up",
                b"moveDown:" => "down",
                b"insertNewline:" => "enter",
                b"cancelOperation:" => "escape",
                _ => return Bool::NO,
            };
            Bool::new(self.ivars().upgrade().is_some_and(|state| key(&state, name)))
        }
    }
    unsafe impl NSPopoverDelegate for ChooserView {
        #[unsafe(method(popoverDidClose:))]
        fn popover_did_close(&self, _: &NSNotification) {
            if let Some(state) = self.ivars().upgrade() {
                let s = state.borrow();
                if !s.picked
                    && let Some(name) = &s.dismiss
                {
                    let _ = s.sender.try_send(Command { name: name.clone(), args: vec![], token: None });
                }
            }
        }
    }
);

impl ChooserView {
    fn new(mtm: MainThreadMarker, state: &Rc<RefCell<State>>, frame: NSRect) -> Retained<Self> {
        unsafe {
            msg_send![super(Self::alloc(mtm).set_ivars(Rc::downgrade(state))), initWithFrame: frame]
        }
    }
    /// The pointer lights the list row under it (the list view only).
    fn hover(&self, at: Option<NSPoint>) {
        let Some(state) = self.ivars().upgrade() else {
            return;
        };
        let is_list = state
            .borrow()
            .list
            .as_ref()
            .is_some_and(|l| std::ptr::eq(&**l, self));
        if !is_list {
            return;
        }
        let row = at.and_then(|at| {
            let s = state.borrow();
            s.frames
                .iter()
                .find(|(i, f)| {
                    choosable(&s.spec.rows[*i])
                        && at.y >= f.origin.y
                        && at.y < f.origin.y + f.size.height
                })
                .map(|(i, _)| *i)
        });
        state.borrow_mut().lit = row;
        light(&state);
    }
}

fn choosable(row: &Row) -> bool {
    row.part == Some(Part::Model) && !row.disabled
}

/// Sends the chosen row; a row that does not stay closes the popover.
fn choose(state: &Rc<RefCell<State>>, index: usize) {
    let (popover, close) = {
        let mut s = state.borrow_mut();
        let Some(row) = s.spec.rows.get(index) else {
            return;
        };
        if row.disabled || row.part == Some(Part::Heading) {
            return;
        }
        let close = !row.stays;
        s.picked |= close;
        let _ = s.sender.try_send(Command {
            name: format!("agentPick:{index}"),
            args: vec![],
            token: None,
        });
        (s.popover.clone(), close)
    };
    if close {
        popover.close();
    }
}

/// A key the popover owns: the arrows and Return walk and choose the list,
/// ⌘1…⌘9 choose a model, Escape closes. True when it was used.
fn key(state: &Rc<RefCell<State>>, key: &str) -> bool {
    if let Some(n) = key
        .strip_prefix("cmd+")
        .and_then(|d| d.parse::<usize>().ok())
        .filter(|n| (1..=9).contains(n))
    {
        let pick = {
            let s = state.borrow();
            s.spec.numbered(&s.query).get(n - 1).copied()
        };
        if let Some(index) = pick {
            choose(state, index);
        }
        return true;
    }
    match key {
        "escape" => {
            let popover = state.borrow().popover.clone();
            popover.close();
            true
        }
        "up" | "down" => {
            {
                let mut s = state.borrow_mut();
                let rows: Vec<usize> = s
                    .spec
                    .listed(&s.query)
                    .into_iter()
                    .filter(|&i| choosable(&s.spec.rows[i]))
                    .collect();
                if rows.is_empty() {
                    return true;
                }
                let at = s.lit.and_then(|l| rows.iter().position(|&i| i == l));
                let next = match (at, key) {
                    (None, "up") => rows.len() - 1,
                    (None, _) => 0,
                    (Some(p), "up") => (p + rows.len() - 1) % rows.len(),
                    (Some(p), _) => (p + 1) % rows.len(),
                };
                s.lit = Some(rows[next]);
            }
            light(state);
            true
        }
        "enter" => {
            let lit = {
                let s = state.borrow();
                s.lit.or_else(|| {
                    s.spec
                        .listed(&s.query)
                        .into_iter()
                        .find(|&i| choosable(&s.spec.rows[i]))
                })
            };
            if let Some(index) = lit {
                choose(state, index);
            }
            true
        }
        _ => false,
    }
}

/// Moves the list's highlight to the lit row (or hides it).
fn light(state: &Rc<RefCell<State>>) {
    let s = state.borrow();
    let Some(highlight) = &s.highlight else {
        return;
    };
    match s
        .lit
        .and_then(|lit| s.frames.iter().find(|(i, _)| *i == lit))
    {
        Some((_, frame)) => {
            highlight.setFrame(*frame);
            highlight.setHidden(false);
            if let Some(list) = &s.list {
                list.scrollRectToVisible(*frame);
            }
        }
        None => highlight.setHidden(true),
    }
}

fn label(text: &str, size: f32, ink: u32, mtm: MainThreadMarker) -> Retained<NSTextField> {
    popup_label(text, size, ink, mtm)
}

fn mark(name: &str, points: f64, ink: u32, mtm: MainThreadMarker) -> Retained<NSImageView> {
    let image = crate::icons::glyph(name)
        .and_then(|g| glyph_image(g, points, ""))
        .unwrap_or_else(|| {
            let image = symbol(name, "");
            let config =
                NSImageSymbolConfiguration::configurationWithPointSize_weight(points, unsafe {
                    NSFontWeightRegular
                });
            image.imageWithSymbolConfiguration(&config).unwrap_or(image)
        });
    let view = NSImageView::imageViewWithImage(&image, mtm);
    view.setContentTintColor(Some(&native_color(ink)));
    view.setAccessibilityElement(false);
    view
}

fn rule(frame: NSRect, t: &Tokens, mtm: MainThreadMarker) -> Retained<NSBox> {
    let line = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
    line.setBoxType(NSBoxType::Custom);
    line.setBorderWidth(0.);
    line.setFillColor(&native_color(t.colors.separator));
    line.setAccessibilityElement(false);
    line
}

fn tile(frame: NSRect, fill: u32, radius: f64, mtm: MainThreadMarker) -> Retained<NSBox> {
    let tile = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
    tile.setBoxType(NSBoxType::Custom);
    tile.setBorderWidth(0.);
    tile.setCornerRadius(radius);
    tile.setFillColor(&native_color(fill));
    tile.setAccessibilityElement(false);
    tile
}

/// A row-wide press target that draws nothing itself.
fn press(
    target: &ChooserView,
    index: usize,
    frame: NSRect,
    name: &str,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(""),
            Some(target),
            Some(sel!(pick:)),
            mtm,
        )
    };
    button.setTag(index as isize);
    button.setBordered(false);
    button.setTransparent(true);
    button.setFrame(frame);
    button.setAccessibilityLabel(Some(&NSString::from_str(name)));
    button
}

/// Text centred in a band `top..top+height` at `x`, `width` wide.
fn place(view: &NSView, x: f64, top: f64, height: f64, width: f64, h: f64) {
    view.setFrame(NSRect::new(
        NSPoint::new(x, top + ((height - h) / 2.).floor()),
        NSSize::new(width.max(0.), h),
    ));
}

fn fitting(view: &NSView) -> NSSize {
    let size = view.fittingSize();
    NSSize::new(size.width.ceil(), size.height.ceil())
}

/// One of the popover's three surfaces: the floating token on the ground,
/// a hairline edge, the nested corner.
fn surface(frame: NSRect, t: &Tokens, shape: &Shape, mtm: MainThreadMarker) -> Retained<NSBox> {
    let surface = tile(frame, t.colors.floating, shape.radius, mtm);
    surface.setBorderWidth(shape.line);
    surface.setBorderColor(&native_color(t.colors.separator));
    surface
}

/// A native button for the one action a surface offers.
fn action(
    target: &ChooserView,
    index: usize,
    title: &str,
    frame: NSRect,
    t: &Tokens,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(sel!(pick:)),
            mtm,
        )
    };
    button.setTag(index as isize);
    button.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    button.setFont(Some(&NSFont::systemFontOfSize(t.typography.reading as f64)));
    button.setFrame(frame);
    button
}

/// Lays the whole popover out from its state (an open or a refresh): the
/// rail, the list (as tall as its rows, up to the chooser's height, then it
/// scrolls) and the card (as tall as its content), top-aligned on the
/// ground with a gutter between. With no models, the list surface is one
/// compact pane with its one action, and there is no card.
fn build(state: &Rc<RefCell<State>>, mtm: MainThreadMarker) -> NSSize {
    let (spec, t) = {
        let s = state.borrow();
        (s.spec.clone(), s.tokens.clone())
    };
    let body_h = fitting(&label("Ag", t.typography.reading, 0, mtm)).height;
    let shape = Shape::new(&t);
    let empty = spec.of(Part::Model).next().is_none();
    let g = shape.gutter;
    let rail_x = g;
    let list_x = rail_x + shape.rail + g;
    let card_x = list_x + shape.list + g;
    let root = ChooserView::new(mtm, state, NSRect::ZERO);
    root.setAccessibilityLabel(Some(&NSString::from_str("Harness and model")));
    // The ground the surfaces stand on, in the token, not the material.
    root.setWantsLayer(true);
    if let Some(layer) = root.layer() {
        layer.setBackgroundColor(Some(&native_color(t.colors.panel_secondary).CGColor()));
    }
    let mut views: Vec<Retained<NSView>> = vec![];
    let push = |views: &mut Vec<Retained<NSView>>, view: &NSView| views.push(view.retain());

    // The list surface: search over the scrolling rows, or the empty pane.
    let list = ChooserView::new(
        mtm,
        state,
        NSRect::new(NSPoint::ZERO, NSSize::new(shape.list, 0.)),
    );
    let mut search = None;
    let list_h;
    if empty {
        let mut y = shape.pad;
        if let Some((_, row)) = spec.of(Part::Status).next() {
            let (status, h) = popup_description(
                &row.label,
                t.typography.reading,
                t.colors.muted,
                shape.list - 2. * shape.inset,
                mtm,
            );
            status.setFrame(NSRect::new(
                NSPoint::new(list_x + shape.inset, g + y),
                NSSize::new(shape.list - 2. * shape.inset, h),
            ));
            push(&mut views, &status);
            y += h + shape.gap;
        }
        if let Some((i, row)) = spec.of(Part::Action).next() {
            let button = action(
                &root,
                i,
                &row.label,
                NSRect::new(
                    NSPoint::new(list_x + shape.pad, g + y),
                    NSSize::new(shape.list - 2. * shape.pad, shape.control),
                ),
                &t,
                mtm,
            );
            push(&mut views, &button);
            y += shape.control;
        }
        list_h = y + shape.pad;
    } else {
        let symbol = t.metrics.symbol_panel as f64;
        let glass = mark("magnifyingglass", symbol, t.colors.muted, mtm);
        place(
            &glass,
            list_x + shape.inset + (shape.mark - symbol) / 2.,
            g,
            shape.search,
            symbol,
            symbol,
        );
        push(&mut views, &glass);
        let field = NSTextField::new(mtm);
        field.setBezeled(false);
        field.setBordered(false);
        field.setDrawsBackground(false);
        field.setFocusRingType(NSFocusRingType::None);
        field.setFont(Some(&NSFont::systemFontOfSize(t.typography.reading as f64)));
        field.setTextColor(Some(&native_color(t.colors.text)));
        field.setPlaceholderString(Some(&NSString::from_str(&spec.placeholder)));
        field.setStringValue(&NSString::from_str(&state.borrow().query));
        let field_x = list_x + shape.label_x();
        place(
            &field,
            field_x,
            g,
            shape.search,
            list_x + shape.list - shape.inset - field_x,
            body_h + 2.,
        );
        unsafe { field.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*root))) };
        field.setAccessibilityLabel(Some(&NSString::from_str("Search models")));
        push(&mut views, &field);
        push(
            &mut views,
            &rule(
                NSRect::new(
                    NSPoint::new(list_x, g + shape.search),
                    NSSize::new(shape.list, shape.line),
                ),
                &t,
                mtm,
            ),
        );
        state.borrow_mut().list = Some(list.clone());
        let rows_h = build_list(state);
        let top = shape.search + shape.line;
        list_h = (top + rows_h).min(shape.most);
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            NSRect::new(
                NSPoint::new(list_x, g + top),
                NSSize::new(shape.list, list_h - top),
            ),
        );
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setBorderType(NSBorderType::NoBorder);
        scroll.setDocumentView(Some(&list));
        list.setFrameSize(NSSize::new(shape.list, rows_h.max(list_h - top)));
        push(&mut views, &scroll);
        state.borrow_mut().scroll = Some(scroll);
        search = Some(field);
    }

    // The card: the chosen model's title, its facts, Reasoning, one action.
    let mut card_h = 0.;
    if let (false, Some((title, icon))) = (empty, &spec.title) {
        let x = card_x + shape.inset;
        let inner = shape.card - 2. * shape.inset;
        let caption = t.text(pm_design::Text::Caption).size;
        let caption_h = fitting(&label("Ag", caption, 0, mtm)).height;
        let mut y = 0.;
        // The title band lines up with the list's search band.
        let mut lead = x;
        if let Some(icon) = icon {
            let image = mark(icon, shape.title_mark, t.colors.text, mtm);
            place(
                &image,
                x,
                g + y,
                shape.search,
                shape.title_mark,
                shape.title_mark,
            );
            push(&mut views, &image);
            lead += shape.title_mark + shape.gap;
        }
        let size = t.typography.heading;
        let name = label(title, size, t.colors.text, mtm);
        name.setFont(Some(&NSFont::systemFontOfSize_weight(
            size as f64,
            unsafe { NSFontWeightMedium },
        )));
        let name_h = fitting(&name).height;
        place(&name, lead, g + y, shape.search, x + inner - lead, name_h);
        push(&mut views, &name);
        y += shape.search;
        // A label and its value on one row, the value right-aligned, with an
        // optional muted caption after it.
        let fact = |views: &mut Vec<Retained<NSView>>,
                    y: f64,
                    key: &str,
                    value: &str,
                    note: Option<&str>| {
            let k = label(key, t.typography.reading, t.colors.muted, mtm);
            let mut right = x + inner;
            if let Some(note) = note {
                let c = label(note, caption, t.colors.muted, mtm);
                let w = fitting(&c).width;
                place(&c, right - w, g + y, shape.fact, w, caption_h);
                views.push(Retained::into_super(Retained::into_super(c)));
                right -= w + shape.gap;
            }
            let v = label(value, t.typography.reading, t.colors.text, mtm);
            let w = fitting(&v).width.min(inner / 2.);
            place(&v, right - w, g + y, shape.fact, w, body_h);
            place(&k, x, g + y, shape.fact, right - w - shape.pad - x, body_h);
            views.push(Retained::into_super(Retained::into_super(k)));
            views.push(Retained::into_super(Retained::into_super(v)));
        };
        for (key, value) in &spec.facts {
            fact(&mut views, y, key, value, None);
            y += shape.fact;
        }
        // Reasoning: the model's own levels on a snapping track.
        let efforts: Vec<_> = spec.of(Part::Effort).collect();
        if !efforts.is_empty() {
            if !spec.facts.is_empty() {
                y += shape.gap;
            }
            let chosen = efforts.iter().position(|(_, r)| r.checked);
            let level = chosen.map(|p| efforts[p].1);
            fact(
                &mut views,
                y,
                "Reasoning",
                level.map_or("", |r| r.label.as_str()),
                level.filter(|r| r.default).map(|_| "Default"),
            );
            y += shape.fact;
            let slider = NSSlider::new(mtm);
            slider.setMinValue(0.);
            slider.setMaxValue((efforts.len().max(2) - 1) as f64);
            slider.setNumberOfTickMarks(efforts.len().max(2) as isize);
            slider.setAllowsTickMarkValuesOnly(true);
            slider.setTickMarkPosition(NSTickMarkPosition::Below);
            slider.setContinuous(false);
            slider.setEnabled(efforts.len() > 1);
            slider.setDoubleValue(chosen.unwrap_or(0) as f64);
            unsafe {
                slider.setTarget(Some(&root));
                slider.setAction(Some(sel!(reason:)));
            }
            slider.setAccessibilityLabel(Some(&NSString::from_str("Reasoning")));
            let h = fitting(&slider).height;
            place(&slider, x, g + y, shape.control, inner, h);
            push(&mut views, &slider);
            y += shape.control;
        }
        // The footer: one existing action, under a hairline.
        if let Some((i, row)) = spec.of(Part::Action).next() {
            y += shape.pad;
            push(
                &mut views,
                &rule(
                    NSRect::new(
                        NSPoint::new(card_x, g + y),
                        NSSize::new(shape.card, shape.line),
                    ),
                    &t,
                    mtm,
                ),
            );
            y += shape.pad;
            let button = action(
                &root,
                i,
                &row.label,
                NSRect::new(
                    NSPoint::new(card_x + shape.pad, g + y),
                    NSSize::new(shape.card - 2. * shape.pad, shape.control),
                ),
                &t,
                mtm,
            );
            push(&mut views, &button);
            y += shape.control;
        }
        card_h = y + shape.pad;
    }

    // The rail: one mark per harness, the chosen one on a soft tile.
    let harnesses = spec.of(Part::Harness).count() as f64;
    let tile_inset = ((shape.rail - shape.tile) / 2.).floor();
    let rail_content = 2. * tile_inset + harnesses * (shape.tile + shape.tile_gap) - shape.tile_gap;
    let rail_h = rail_content.max(list_h);
    for (n, (i, row)) in spec.of(Part::Harness).enumerate() {
        let frame = NSRect::new(
            NSPoint::new(
                rail_x + tile_inset,
                g + tile_inset + n as f64 * (shape.tile + shape.tile_gap),
            ),
            NSSize::new(shape.tile, shape.tile),
        );
        if row.checked {
            push(
                &mut views,
                &tile(
                    frame,
                    t.colors.selected_fill,
                    t.metrics.control_radius as f64,
                    mtm,
                ),
            );
        }
        if let Some(icon) = &row.icon {
            let ink = if row.checked {
                t.colors.text
            } else {
                t.colors.secondary
            };
            let image = mark(icon, shape.rail_icon, ink, mtm);
            place(
                &image,
                frame.origin.x + (shape.tile - shape.rail_icon) / 2.,
                frame.origin.y,
                shape.tile,
                shape.rail_icon,
                shape.rail_icon,
            );
            push(&mut views, &image);
        }
        let button = press(&root, i, frame, &row.label, mtm);
        button.setToolTip(Some(&NSString::from_str(&row.label)));
        if row.checked {
            unsafe { button.setAccessibilityValue(Some(&NSString::from_str("Selected"))) };
        }
        push(&mut views, &button);
    }

    // Each surface keeps its own height; the rail runs as tall as the list.
    let width = card_x + if card_h > 0. { shape.card + g } else { 0. };
    let height = g + rail_h.max(card_h) + g;
    root.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(width, height)));
    root.addSubview(&surface(
        NSRect::new(NSPoint::new(rail_x, g), NSSize::new(shape.rail, rail_h)),
        &t,
        &shape,
        mtm,
    ));
    root.addSubview(&surface(
        NSRect::new(NSPoint::new(list_x, g), NSSize::new(shape.list, list_h)),
        &t,
        &shape,
        mtm,
    ));
    if card_h > 0. {
        root.addSubview(&surface(
            NSRect::new(NSPoint::new(card_x, g), NSSize::new(shape.card, card_h)),
            &t,
            &shape,
            mtm,
        ));
    }
    for view in views {
        root.addSubview(&view);
    }
    {
        let mut s = state.borrow_mut();
        s.root = Some(root);
        s.search = search;
        if empty {
            s.list = None;
            s.scroll = None;
        }
    }
    NSSize::new(width, height)
}

/// Lays the list's rows out for the current query (in the list view's own
/// coordinates); answers their height.
fn build_list(state: &Rc<RefCell<State>>) -> f64 {
    let Some(list) = state.borrow().list.clone() else {
        return 0.;
    };
    let mtm = list.mtm();
    for view in list.subviews().to_vec() {
        view.removeFromSuperview();
    }
    let (spec, t, query) = {
        let s = state.borrow();
        (s.spec.clone(), s.tokens.clone(), s.query.clone())
    };
    let shape = Shape::new(&t);
    let caption = t.text(pm_design::Text::Caption).size;
    let caption_h = fitting(&label("Ag", caption, 0, mtm)).height;
    let body_h = fitting(&label("Ag", t.typography.reading, 0, mtm)).height;
    let width = shape.list;
    let x = shape.inset;
    let label_x = shape.label_x();
    // The highlight sits inside the surface with the content's own inset.
    let lane = t.space.xs as f64;
    let highlight = tile(
        NSRect::ZERO,
        t.colors.hover,
        t.metrics.inner_radius as f64,
        mtm,
    );
    highlight.setHidden(true);
    list.addSubview(&highlight);
    let numbered = spec.numbered(&query);
    let mut frames = vec![];
    let mut y = lane;
    let listed = spec.listed(&query);
    if listed.is_empty() {
        let none = label(
            if query.trim().is_empty() {
                "No models"
            } else {
                "No models match"
            },
            t.typography.reading,
            t.colors.muted,
            mtm,
        );
        place(&none, label_x, y, shape.row, width - label_x - x, body_h);
        list.addSubview(&none);
        y += shape.row;
    }
    let check = t.metrics.symbol_inline as f64;
    for (place_at, i) in listed.into_iter().enumerate() {
        let row = &spec.rows[i];
        if row.part == Some(Part::Heading) {
            // Each account after the first starts after a gap with a hairline.
            if place_at > 0 {
                list.addSubview(&rule(
                    NSRect::new(
                        NSPoint::new(0., y + ((shape.gap - shape.line) / 2.).floor()),
                        NSSize::new(width, shape.line),
                    ),
                    &t,
                    mtm,
                ));
                y += shape.gap;
            }
            let ink = if row.disabled {
                t.colors.faint
            } else {
                t.colors.muted
            };
            if let Some(icon) = &row.icon {
                let s = t.metrics.symbol_inline as f64;
                let image = mark(icon, s, ink, mtm);
                place(&image, x + (shape.mark - s) / 2., y, shape.heading, s, s);
                list.addSubview(&image);
            }
            let text = if row.detail.is_empty() {
                row.label.clone()
            } else {
                format!("{} · {}", row.label, row.detail)
            };
            let heading = label(&text, caption, ink, mtm);
            place(
                &heading,
                label_x,
                y,
                shape.heading,
                width - label_x - x,
                caption_h,
            );
            heading.setAccessibilityElement(true);
            list.addSubview(&heading);
            y += shape.heading;
            continue;
        }
        let frame = NSRect::new(
            NSPoint::new(lane, y),
            NSSize::new(width - 2. * lane, shape.row),
        );
        let ink = if row.disabled {
            t.colors.faint
        } else {
            t.colors.text
        };
        if let Some(icon) = &row.icon {
            let image = mark(icon, shape.mark, ink, mtm);
            place(&image, x, y, shape.row, shape.mark, shape.mark);
            list.addSubview(&image);
        }
        let mut trailing = width - x;
        let mut name = row.label.clone();
        if let Some(n) = numbered.iter().position(|&m| m == i) {
            // T3's chip: a 26 × 20 box, the caption centred in it.
            let (chip_w, chip_h) = (
                (t.metrics.hit + t.space.hair) as f64,
                (t.metrics.hit - t.space.xs) as f64,
            );
            let chip = tile(
                NSRect::new(
                    NSPoint::new(trailing - chip_w, y + ((shape.row - chip_h) / 2.).floor()),
                    NSSize::new(chip_w, chip_h),
                ),
                t.colors.hover,
                t.radius.xs as f64,
                mtm,
            );
            list.addSubview(&chip);
            let text = label(&format!("⌘{}", n + 1), caption, t.colors.muted, mtm);
            text.setAlignment(NSTextAlignment::Center);
            place(&text, trailing - chip_w, y, shape.row, chip_w, caption_h);
            list.addSubview(&text);
            trailing -= chip_w + shape.pad;
            name.push_str(&format!(", Command-{}", n + 1));
        }
        // The name gives way first, keeping room for its check.
        let title = label(&row.label, t.typography.reading, ink, mtm);
        let room = trailing - label_x - if row.checked { check + shape.gap } else { 0. };
        let title_w = fitting(&title).width.min(room.max(0.));
        place(&title, label_x, y, shape.row, title_w, body_h);
        list.addSubview(&title);
        if row.checked {
            let mark = mark("checkmark", check, t.colors.secondary, mtm);
            place(
                &mark,
                label_x + title_w + shape.gap,
                y,
                shape.row,
                check,
                check,
            );
            list.addSubview(&mark);
        }
        let button = press(&list, i, frame, &name, mtm);
        button.setEnabled(!row.disabled);
        if row.checked {
            unsafe { button.setAccessibilityValue(Some(&NSString::from_str("Selected"))) };
        }
        list.addSubview(&button);
        frames.push((i, frame));
        y += shape.row;
    }
    y += lane;
    if let Some(scroll) = list.enclosingScrollView() {
        list.setFrameSize(NSSize::new(width, y.max(scroll.contentSize().height)));
    }
    {
        let mut s = state.borrow_mut();
        s.frames = frames;
        s.highlight = Some(highlight);
    }
    light(state);
    y
}

/// The open chooser, held by the host while it shows.
pub(super) struct Chooser {
    state: Rc<RefCell<State>>,
}
impl Chooser {
    pub(super) fn is_open(&self) -> bool {
        self.state.borrow().popover.isShown()
    }
    pub(super) fn owns_event(&self, event: &NSEvent) -> bool {
        self.is_open()
            && self
                .state
                .borrow()
                .root
                .as_ref()
                .and_then(|r| r.window())
                .is_some_and(|w| w.windowNumber() == event.windowNumber())
    }
    pub(super) fn key(&self, chord: &str) -> bool {
        key(&self.state, chord)
    }
    pub(super) fn close(&self) {
        let popover = self.state.borrow().popover.clone();
        popover.close();
    }
}

impl Host {
    /// Opens the chooser off `anchor` (the pill, in the owner's content
    /// coordinates), replacing one already open.
    pub fn open_chooser(
        &mut self,
        owner: Option<Retained<NSWindow>>,
        anchor: crate::layout::Rect,
        spec: Spec,
        dismiss: Option<String>,
    ) {
        if let Some(open) = self.chooser.take() {
            open.state.borrow_mut().picked = true;
            open.close();
        }
        let owner = owner.unwrap_or_else(|| self.window.clone());
        let Some(content) = owner.contentView() else {
            return;
        };
        let mtm = self.window.mtm();
        let popover = NSPopover::new(mtm);
        popover.setBehavior(NSPopoverBehavior::Transient);
        popover.setAnimates(!self.tokens.accessibility.reduce_motion);
        let appearance = NSAppearance::appearanceNamed(unsafe {
            if self.tokens.scheme == Scheme::Dark {
                NSAppearanceNameDarkAqua
            } else {
                NSAppearanceNameAqua
            }
        });
        popover.setAppearance(appearance.as_deref());
        let state = Rc::new(RefCell::new(State {
            spec,
            query: String::new(),
            lit: None,
            sender: self.sender.clone(),
            dismiss,
            picked: false,
            tokens: self.tokens.clone(),
            popover: popover.clone(),
            root: None,
            list: None,
            scroll: None,
            search: None,
            frames: vec![],
            highlight: None,
        }));
        let size = build(&state, mtm);
        let root = state.borrow().root.clone();
        let Some(root) = root else {
            return;
        };
        popover.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*root)));
        let controller = NSViewController::new(mtm);
        controller.setView(&root);
        popover.setContentViewController(Some(&controller));
        popover.setContentSize(size);
        let h = content.bounds().size.height;
        // Above the pill; AppKit moves it below or beside when the screen
        // has no room above.
        popover.showRelativeToRect_ofView_preferredEdge(
            rect(anchor, h),
            &content,
            objc2_foundation::NSRectEdge::MaxY,
        );
        if let Some(search) = state.borrow().search.clone()
            && let Some(window) = search.window()
        {
            window.makeFirstResponder(Some(&search));
        }
        self.chooser = Some(Chooser { state });
    }
    /// The open chooser's rows after one that stays was chosen, in place.
    /// False when it is not open any more.
    pub fn update_chooser(&mut self, spec: Spec, dismiss: Option<String>) -> bool {
        let Some(open) = self.chooser.as_ref().filter(|c| c.is_open()) else {
            return false;
        };
        let state = open.state.clone();
        // A choice within the same harness keeps the list where it was.
        let kept = {
            let s = state.borrow();
            s.spec
                .keeps_place(&spec)
                .then(|| {
                    s.scroll
                        .as_ref()
                        .map(|scroll| scroll.contentView().bounds().origin)
                })
                .flatten()
        };
        {
            let mut s = state.borrow_mut();
            s.spec = spec;
            s.lit = None;
            if dismiss.is_some() {
                s.dismiss = dismiss;
            }
        }
        let mtm = self.window.mtm();
        let had_focus = state
            .borrow()
            .search
            .as_ref()
            .and_then(|s| s.window())
            .and_then(|w| w.firstResponder())
            .is_some();
        let size = build(&state, mtm);
        let (popover, root, search) = {
            let s = state.borrow();
            (s.popover.clone(), s.root.clone(), s.search.clone())
        };
        if let (Some(root), Some(controller)) = (root, popover.contentViewController()) {
            popover.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*root)));
            controller.setView(&root);
            popover.setContentSize(size);
        }
        if let (Some(origin), Some(scroll)) = (kept, state.borrow().scroll.clone()) {
            let clip = scroll.contentView();
            clip.scrollToPoint(origin);
            scroll.reflectScrolledClipView(&clip);
        }
        if had_focus
            && let Some(search) = search
            && let Some(window) = search.window()
        {
            window.makeFirstResponder(Some(&search));
            if let Some(editor) = search.currentEditor() {
                let end = search.stringValue().length();
                editor.setSelectedRange(objc2_foundation::NSRange::new(end, 0));
            }
        }
        true
    }
    pub fn chooser_open(&self) -> bool {
        self.chooser.as_ref().is_some_and(Chooser::is_open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(part: Part, label: &str) -> Row {
        Row {
            part: Some(part),
            label: label.into(),
            ..Default::default()
        }
    }
    #[test]
    fn the_list_heads_only_matching_rows_and_numbers_the_first_nine_models() {
        let mut spec = Spec {
            rows: vec![
                row(Part::Harness, "Powermove"),
                row(Part::Heading, "ChatGPT"),
                row(Part::Model, "GPT Sol"),
                row(Part::Model, "GPT Luna"),
                row(Part::Heading, "Anthropic API key"),
                row(Part::Model, "Claude Opus"),
                Row {
                    disabled: true,
                    detail: "Coming later".into(),
                    ..row(Part::Heading, "Aerolite")
                },
                row(Part::Effort, "High"),
            ],
            ..Default::default()
        };
        assert_eq!(spec.listed(""), vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(spec.numbered(""), vec![2, 3, 5]);
        assert_eq!(spec.listed("opus"), vec![4, 5]);
        assert_eq!(spec.numbered("luna"), vec![3]);
        assert!(spec.listed("nothing").is_empty());
        spec.rows
            .extend((0..12).map(|n| row(Part::Model, &format!("M{n}"))));
        assert_eq!(spec.numbered("").len(), 9);
    }
    #[test]
    fn a_choice_in_the_same_harness_keeps_the_list_place_and_a_harness_switch_does_not() {
        let spec = |harness: &str, model: usize| Spec {
            rows: ["Powermove", "Codex"]
                .iter()
                .map(|h| Row {
                    checked: *h == harness,
                    ..row(Part::Harness, h)
                })
                .chain((0..20).map(|n| Row {
                    checked: n == model,
                    ..row(Part::Model, &format!("Model {n}"))
                }))
                .collect(),
            ..Default::default()
        };
        // Twenty 28 pt rows overflow the 360 pt list; choosing the 18th
        // (below the fold) keeps the place, switching harness starts over.
        assert!(spec("Powermove", 0).keeps_place(&spec("Powermove", 17)));
        assert!(!spec("Powermove", 0).keeps_place(&spec("Codex", 0)));
        assert!(!Spec::default().keeps_place(&Spec::default()));
        let t = Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default());
        let shape = Shape::new(&t);
        assert!(shape.search + 20. * shape.row > shape.most);
    }
    #[test]
    fn a_request_becomes_rows_and_the_panel_title() {
        let spec = Spec::from_request(&serde_json::json!({
            "placeholder": "Search models…",
            "chooser": {"title": "GPT Sol", "icon": "provider.chatgpt", "facts": [["Context", "272K"]], "note": ""},
            "options": [
                {"kind": "harness", "label": "Codex", "icon": "provider.chatgpt", "checked": true, "stays": true},
                {"kind": "model", "label": "GPT Sol", "checked": true, "stays": true},
                {"kind": "effort", "label": "High", "default": true},
                {"kind": "action", "label": "Add a provider…"},
                {"kind": "unknown", "label": "?"}
            ]
        }));
        assert_eq!(
            spec.title,
            Some(("GPT Sol".into(), Some("provider.chatgpt".into())))
        );
        assert_eq!(spec.facts, vec![("Context".to_owned(), "272K".to_owned())]);
        assert_eq!(spec.rows[0].part, Some(Part::Harness));
        assert!(spec.rows[0].checked && spec.rows[0].stays);
        assert!(spec.rows[2].default);
        assert_eq!(spec.rows[3].part, Some(Part::Action));
        assert_eq!(spec.rows[4].part, None);
        assert_eq!(spec.placeholder, "Search models…");
    }
}
