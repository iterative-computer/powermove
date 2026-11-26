//! The Agent composer's two menus (PM-727, w110), our own floating panels
//! anchored to the composer's pill: the model picker above the model part,
//! the reasoning menu above the effort part.
//! The model picker, on Capy's layout (the owner's reference): a rail of
//! harness marks with a tile on the chosen one, attached to the card of the
//! models that harness runs, searchable and scrolling, under their account
//! headings, with ⌘1…⌘9 local to the picker and Aero's usage monitor at the
//! card's foot. The reasoning menu: the model's levels in order, one short
//! list on the same card, row and type, ✓ on the current level. The panel's
//! TypeScript sends the rows (`kind` says where each stands) and gets the
//! chosen row's index back as `agentPick:i`, as its menus do; a row that
//! `stays` keeps the picker open for the rows that follow it.
use super::*;
use objc2::Message;
use objc2::runtime::Bool;
use objc2_foundation::{NSNotification, NSNotificationCenter};

/// Where a row of the chooser stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Harness,
    Heading,
    Model,
    Effort,
    Action,
    /// One quiet line in the compact card (signed out, couldn't start).
    Status,
    /// Add a provider…: the "+" tile at the foot of the rail.
    Add,
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
            "add" => Self::Add,
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

/// `[[label, value], …]` as the panel sends the usage monitor.
fn pairs(list: &serde_json::Value) -> Vec<(String, String)> {
    list.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|f| Some((f[0].as_str()?.to_owned(), f[1].as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}
/// Which of the pill's two menus the panel shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Menu {
    /// The model picker: the rail and the searchable card of models.
    #[default]
    Models,
    /// The reasoning menu: the model's levels, one short list.
    Effort,
}
/// The menu's rows, from the panel's request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spec {
    pub rows: Vec<Row>,
    pub menu: Menu,
    /// The usage monitor at the model card's foot (Aero's credits, plan and
    /// reset day), as (label, value); none until the backend reports them.
    pub usage: Vec<(String, String)>,
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
        Self {
            rows,
            menu: if input["menu"] == "effort" {
                Menu::Effort
            } else {
                Menu::Models
            },
            usage: pairs(&input["usage"]),
            placeholder: text(&input["placeholder"]),
        }
    }
    /// The list's rows for `query`, in order. The reasoning menu lists its
    /// levels. The model card: a heading stands only over rows that match; a
    /// disabled heading (a source coming later) shows only while nothing is
    /// searched.
    pub fn listed(&self, query: &str) -> Vec<usize> {
        if self.menu == Menu::Effort {
            return self.of(Part::Effort).map(|(i, _)| i).collect();
        }
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
        if self.menu == Menu::Effort {
            return vec![];
        }
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

/// The menus' sizes, all from tokens (the model card measured against
/// Capy's picker in the lane notes).
struct Shape {
    /// The gap between the rail and the card.
    gutter: f64,
    /// Room round the surfaces for their shadows, inside the clear panel.
    shadow: f64,
    rail: f64,
    tile: f64,
    tile_gap: f64,
    rail_icon: f64,
    list: f64,
    /// The most the model list grows to; it scrolls past it.
    most: f64,
    search: f64,
    heading: f64,
    row: f64,
    /// A usage monitor line.
    fact: f64,
    control: f64,
    mark: f64,
    title_mark: f64,
    /// A surface's leading content inset (the mark column starts here).
    inset: f64,
    /// How far a row's highlight stands in from the surface's sides.
    lane: f64,
    /// The gap between two rows' highlights.
    list_gap: f64,
    gap: f64,
    pad: f64,
    radius: f64,
    line: f64,
    /// The list's scroll fades.
    fade: f64,
    /// The menu's distance from the pill.
    offset: f64,
}
impl Shape {
    fn new(t: &Tokens) -> Self {
        let m = &t.metrics;
        // A hairline: half a point on a Retina screen, a point at 1x.
        let scale = MainThreadMarker::new()
            .and_then(NSScreen::mainScreen)
            .map_or(2., |s| s.backingScaleFactor());
        Self {
            gutter: t.space.md as f64,
            rail: m.chooser_rail as f64,
            tile: m.control as f64,
            tile_gap: t.space.hair as f64,
            rail_icon: (m.chooser_mark + t.space.hair) as f64,
            list: m.chooser_list as f64,
            most: m.chooser_height as f64,
            search: (m.control + t.space.md) as f64,
            heading: m.hit as f64,
            row: m.row as f64,
            fact: m.field as f64,
            control: m.control as f64,
            mark: m.chooser_mark as f64,
            title_mark: m.symbol_panel as f64,
            inset: t.space.lg as f64,
            lane: t.space.xs as f64,
            list_gap: m.list_gap as f64,
            gap: t.space.sm as f64,
            pad: t.space.md as f64,
            radius: t.radius.nested(m.popover_radius, t.space.md) as f64,
            shadow: (t.space.xl + t.space.lg) as f64,
            line: scale.recip().max(0.5),
            fade: t.space.xxl as f64,
            offset: t.space.sm as f64,
        }
    }
    /// Where a list row's label starts: the mark column, then the gap; the
    /// reasoning menu's rows have no mark.
    fn label_x(&self, menu: Menu) -> f64 {
        match menu {
            Menu::Models => self.inset + self.mark + self.gap,
            Menu::Effort => self.inset,
        }
    }
}

pub(super) struct State {
    spec: Spec,
    query: String,
    /// The keyboard's row in the middle list (a row index), if any.
    lit: Option<usize>,
    sender: async_channel::Sender<Command>,
    dismiss: Option<String>,
    /// A row that closes the chooser was chosen: closing sends no dismissal.
    picked: bool,
    tokens: Tokens,
    /// One borderless, transparent panel (a child of the editor window)
    /// holding the floating surfaces.
    window: Retained<PopupWindow>,
    parent: Retained<NSWindow>,
    open: bool,
    /// The pill part it was opened off and the room it stays inside (the
    /// editor window on its screen), in screen points: every layout is
    /// placed from them.
    around: (NSRect, NSRect),
    /// The list's width as laid out.
    width: f64,
    /// What dismisses it from outside (the app going inactive, its window
    /// moving or resizing) and the mouse-down monitor, removed when it closes.
    watchers: Vec<Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>>,
    /// The current list's scroll observer (for the fades), one per layout.
    observers:
        Vec<Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>>,
    monitor: Option<Retained<AnyObject>>,
    root: Option<Retained<ChooserView>>,
    list: Option<Retained<ChooserView>>,
    scroll: Option<Retained<NSScrollView>>,
    search: Option<Retained<NSTextField>>,
    /// Middle-list rows as drawn: (row index, frame in the list).
    frames: Vec<(usize, NSRect)>,
    highlight: Option<Retained<NSBox>>,
}

define_class!(
    /// A flipped view that is the chooser's target: rows, rail and search
    /// all send here.
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
        /// The reasoning menu's keys come here (it has no search field).
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }
        /// Keys the menu does not use (the host gave it the arrows, Return
        /// and Escape) do nothing, rather than beep.
        #[unsafe(method(keyDown:))]
        fn key_down(&self, _: &NSEvent) {}
        #[unsafe(method(pick:))]
        fn pick(&self, sender: &NSControl) {
            if let Some(state) = self.ivars().upgrade() {
                choose(&state, sender.tag() as usize);
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
                fades(&state);
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
    matches!(row.part, Some(Part::Model | Part::Effort)) && !row.disabled
}

/// Sends the chosen row; a row that does not stay closes the chooser.
fn choose(state: &Rc<RefCell<State>>, index: usize) {
    let close_after = {
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
        close
    };
    if close_after {
        close(state);
    }
}

/// What closes the chooser from outside it. Everything does but a press
/// inside its own panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outside {
    Press { in_chooser: bool },
    Escape,
    AppResigned,
    WindowMoved,
    WindowResized,
}
pub fn dismisses(event: Outside) -> bool {
    !matches!(event, Outside::Press { in_chooser: true })
}

/// Closes the chooser once: its watchers go, the panel leaves its window,
/// and the panel's caller hears it closed with nothing chosen (unless a row
/// that closes it was).
fn close(state: &Rc<RefCell<State>>) {
    let (window, parent, observers, monitor, dismiss) = {
        let mut s = state.borrow_mut();
        if !s.open {
            return;
        }
        s.open = false;
        let dismiss = (!s.picked).then(|| s.dismiss.clone()).flatten();
        (
            s.window.clone(),
            s.parent.clone(),
            std::mem::take(&mut s.observers)
                .into_iter()
                .chain(std::mem::take(&mut s.watchers))
                .collect::<Vec<_>>(),
            s.monitor.take(),
            dismiss.map(|name| (name, s.sender.clone())),
        )
    };
    let center = NSNotificationCenter::defaultCenter();
    for observer in observers {
        unsafe { center.removeObserver(observer.as_ref()) };
    }
    if let Some(monitor) = monitor {
        unsafe { NSEvent::removeMonitor(&monitor) };
    }
    // Gone at once to the eye and the pointer; ordered out after the event
    // that closed it: ordering out hands key back to the editor window, whose
    // handler must not run inside the host's key monitor (it did: a crash).
    gpui_macos::child_window::forget(&window);
    window.setIgnoresMouseEvents(true);
    window.setAlphaValue(0.);
    let out = block2::RcBlock::new(move || {
        parent.removeChildWindow(&window);
        window.orderOut(None);
    });
    unsafe { objc2_foundation::NSOperationQueue::mainQueue().addOperationWithBlock(&out) };
    if let Some((name, sender)) = dismiss {
        let _ = sender.try_send(Command {
            name,
            args: vec![],
            token: None,
        });
    }
}

/// A key the chooser owns: the arrows and Return walk and choose the list,
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
            if dismisses(Outside::Escape) {
                close(state);
            }
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
                // The reasoning menu's walk starts from the current level.
                let at = s
                    .lit
                    .or_else(|| {
                        (s.spec.menu == Menu::Effort)
                            .then(|| rows.iter().copied().find(|&i| s.spec.rows[i].checked))
                            .flatten()
                    })
                    .and_then(|l| rows.iter().position(|&i| i == l));
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
                let listed = s.spec.listed(&s.query);
                s.lit
                    .or_else(|| {
                        (s.spec.menu == Menu::Effort)
                            .then(|| listed.iter().copied().find(|&i| s.spec.rows[i].checked))
                            .flatten()
                    })
                    .or_else(|| listed.into_iter().find(|&i| choosable(&s.spec.rows[i])))
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
            // A row keeps its pitch; its highlight stands `list_gap` apart
            // from a neighbour's.
            let gap = Shape::new(&s.tokens).list_gap;
            highlight.setFrame(NSRect::new(
                NSPoint::new(frame.origin.x, frame.origin.y + gap / 2.),
                NSSize::new(frame.size.width, frame.size.height - gap),
            ));
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

/// One of the menus' floating surfaces: the floating token, a hairline
/// edge, the nested corner and the menu elevation's drop shadow.
fn surface(frame: NSRect, t: &Tokens, shape: &Shape, mtm: MainThreadMarker) -> Retained<NSBox> {
    let surface = tile(frame, t.colors.floating, shape.radius, mtm);
    surface.setBorderWidth(shape.line);
    surface.setBorderColor(&native_color(t.colors.separator));
    surface.setWantsLayer(true);
    let drop = t
        .elevation(pm_design::Elevation::Menu)
        .into_iter()
        .find(|s| s.blur > 0. && !s.inset);
    if let (Some(layer), Some(drop)) = (surface.layer(), drop) {
        layer.setMasksToBounds(false);
        layer.setShadowColor(Some(&native_color(drop.color).CGColor()));
        layer.setShadowOpacity(drop.alpha);
        // A CSS blur is twice Core Animation's shadow radius.
        layer.setShadowRadius(drop.blur as f64 / 2.);
        layer.setShadowOffset(NSSize::new(0., -drop.y as f64));
    }
    surface
}

/// A native button for the one action a card offers.
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
    button.setFont(Some(&NSFont::systemFontOfSize(t.typography.body as f64)));
    button.setFrame(frame);
    button
}

/// The list's fades: `space.xxl` at the bottom while rows lie below, and
/// at the top once it has scrolled, as a mask on the scroll view.
fn fades(state: &Rc<RefCell<State>>) {
    let Some(scroll) = state.borrow().scroll.clone() else {
        return;
    };
    let fade_h = Shape::new(&state.borrow().tokens).fade;
    let shown = scroll.contentSize().height;
    let (top, bottom) = scroll.documentView().map_or((false, false), |doc| {
        let y = scroll.contentView().bounds().origin.y;
        (y > 0.5, y + shown < doc.frame().size.height - 0.5)
    });
    scroll.setWantsLayer(true);
    let Some(layer) = scroll.layer() else {
        return;
    };
    if !top && !bottom {
        unsafe { layer.setMask(None) };
        return;
    }
    let edge = fade_h / shown.max(1.);
    let stop = |on: bool| {
        let color = if on {
            NSColor::clearColor()
        } else {
            NSColor::blackColor()
        };
        mask_stop(&color)
    };
    let fade = CAGradientLayer::new();
    unsafe {
        fade.setColors(Some(&NSArray::from_retained_slice(&[
            stop(top),
            mask_stop(&NSColor::blackColor()),
            mask_stop(&NSColor::blackColor()),
            stop(bottom),
        ])));
    }
    fade.setLocations(Some(&NSArray::from_retained_slice(&[
        NSNumber::new_f64(0.),
        NSNumber::new_f64(edge),
        NSNumber::new_f64(1. - edge),
        NSNumber::new_f64(1.),
    ])));
    // The scroll view's layer is flipped: its top edge is 1.
    fade.setStartPoint(NSPoint::new(0.5, 1.));
    fade.setEndPoint(NSPoint::new(0.5, 0.));
    fade.setFrame(NSRect::new(NSPoint::ZERO, scroll.frame().size));
    unsafe { layer.setMask(Some(&fade)) };
}

/// The body text's line height, for centring a label in its band.
fn body_height(t: &Tokens, mtm: MainThreadMarker) -> f64 {
    fitting(&label("Ag", t.typography.body, 0, mtm)).height
}

/// Lays the open menu out from its state (an open or a refresh) inside a
/// clear panel with room for the surfaces' shadows; answers the panel's
/// size.
fn build(state: &Rc<RefCell<State>>, mtm: MainThreadMarker) -> NSSize {
    let menu = state.borrow().spec.menu;
    let root = {
        let s = state.borrow();
        let root = ChooserView::new(mtm, state, NSRect::ZERO);
        root.setAccessibilityElement(true);
        root.setAccessibilityRole(Some(unsafe { NSAccessibilityPopoverRole }));
        root.setAccessibilityLabel(Some(&NSString::from_str(match s.spec.menu {
            Menu::Models => "Harness and model",
            Menu::Effort => "Reasoning effort",
        })));
        root
    };
    let size = match menu {
        Menu::Models => build_models(state, &root, mtm),
        Menu::Effort => build_effort(state, &root, mtm),
    };
    root.setFrame(NSRect::new(NSPoint::ZERO, size));
    state.borrow_mut().root = Some(root);
    fades(state);
    size
}

/// The reasoning menu: one surface holding the model's levels, as wide as
/// its longest level (never narrower than the part it hangs from).
fn build_effort(state: &Rc<RefCell<State>>, root: &ChooserView, mtm: MainThreadMarker) -> NSSize {
    let (spec, t, anchor) = {
        let s = state.borrow();
        (s.spec.clone(), s.tokens.clone(), s.around.0)
    };
    let shape = Shape::new(&t);
    let check = t.metrics.symbol_inline as f64;
    let widest = spec
        .of(Part::Effort)
        .map(|(_, r)| fitting(&label(&r.label, t.typography.body, 0, mtm)).width)
        .fold(0., f64::max);
    let width = (shape.inset + widest + shape.gap + check + shape.inset).max(anchor.size.width);
    let pad = shape.shadow;
    let list = ChooserView::new(
        mtm,
        state,
        NSRect::new(NSPoint::new(pad, pad), NSSize::new(width, 0.)),
    );
    {
        let mut s = state.borrow_mut();
        s.width = width;
        s.list = Some(list.clone());
        s.scroll = None;
        s.search = None;
    }
    let h = build_list(state);
    list.setFrameSize(NSSize::new(width, h));
    root.addSubview(&surface(
        NSRect::new(NSPoint::new(pad, pad), NSSize::new(width, h)),
        &t,
        &shape,
        mtm,
    ));
    root.addSubview(&list);
    NSSize::new(pad + width + pad, pad + h + pad)
}

/// The model picker: the rail attached to the card. The rail: a mark per
/// harness and "+" (Providers…) at its foot. The card: search over the
/// rows, as tall as they are up to the picker's height, then it scrolls
/// between fades; Aero's usage monitor at its foot. Signed out or a harness
/// that couldn't start: the card is compact (the harness, a status line,
/// its one button).
fn build_models(state: &Rc<RefCell<State>>, root: &ChooserView, mtm: MainThreadMarker) -> NSSize {
    let (spec, t) = {
        let s = state.borrow();
        (s.spec.clone(), s.tokens.clone())
    };
    let body_h = body_height(&t, mtm);
    let shape = Shape::new(&t);
    let empty = spec.of(Part::Model).next().is_none();
    let (pad, g) = (shape.shadow, shape.gutter);
    let rail_x = pad;
    let list_x = rail_x + shape.rail + g;
    state.borrow_mut().width = shape.list;
    let mut views: Vec<Retained<NSView>> = vec![];
    let push = |views: &mut Vec<Retained<NSView>>, view: &NSView| views.push(view.retain());
    let row =
        |views: &mut Vec<Retained<NSView>>, x: f64, y: f64, inner: f64, key: &str, value: &str| {
            let k = label(key, t.typography.body, t.colors.muted, mtm);
            let v = label(value, t.typography.body, t.colors.text, mtm);
            let w = fitting(&v).width.min(inner * 0.6);
            place(&v, x + inner - w, pad + y, shape.fact, w, body_h);
            place(&k, x, pad + y, shape.fact, inner - w - shape.pad, body_h);
            views.push(Retained::into_super(Retained::into_super(k)));
            views.push(Retained::into_super(Retained::into_super(v)));
        };

    let list = ChooserView::new(
        mtm,
        state,
        NSRect::new(NSPoint::ZERO, NSSize::new(shape.list, 0.)),
    );
    let mut search = None;
    let mut list_h;
    if empty {
        // The compact card: the harness, one status line, one button.
        let x = list_x + shape.inset;
        let inner = shape.list - 2. * shape.inset;
        let mut y = 0.;
        if let Some((_, harness)) = spec.of(Part::Harness).find(|(_, r)| r.checked) {
            let mut lead = x;
            if let Some(icon) = &harness.icon {
                let image = mark(icon, shape.title_mark, t.colors.text, mtm);
                place(
                    &image,
                    x,
                    pad + y,
                    shape.search,
                    shape.title_mark,
                    shape.title_mark,
                );
                push(&mut views, &image);
                lead += shape.title_mark + shape.gap;
            }
            let title = t.text(pm_design::Text::Title);
            let name = label(&harness.label, title.size, t.colors.text, mtm);
            name.setFont(Some(&NSFont::systemFontOfSize_weight(
                title.size as f64,
                unsafe { NSFontWeightMedium },
            )));
            let h = fitting(&name).height;
            place(&name, lead, pad + y, shape.search, x + inner - lead, h);
            push(&mut views, &name);
            y += shape.search;
        }
        if let Some((_, row)) = spec.of(Part::Status).next() {
            let (status, h) =
                popup_description(&row.label, t.typography.body, t.colors.muted, inner, mtm);
            status.setFrame(NSRect::new(NSPoint::new(x, pad + y), NSSize::new(inner, h)));
            push(&mut views, &status);
            y += h + shape.pad;
        }
        if let Some((i, row)) = spec.of(Part::Action).next() {
            let button = action(
                root,
                i,
                &row.label,
                NSRect::new(NSPoint::new(x, pad + y), NSSize::new(inner, shape.control)),
                &t,
                mtm,
            );
            push(&mut views, &button);
            y += shape.control;
        }
        list_h = y + shape.inset;
    } else {
        let symbol = t.metrics.symbol_panel as f64;
        let glass = mark("magnifyingglass", symbol, t.colors.muted, mtm);
        place(
            &glass,
            list_x + shape.inset + (shape.mark - symbol) / 2.,
            pad,
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
        field.setFont(Some(&NSFont::systemFontOfSize(t.typography.body as f64)));
        field.setTextColor(Some(&native_color(t.colors.text)));
        field.setPlaceholderString(Some(&NSString::from_str(&spec.placeholder)));
        field.setStringValue(&NSString::from_str(&state.borrow().query));
        let field_x = list_x + shape.label_x(Menu::Models);
        place(
            &field,
            field_x,
            pad,
            shape.search,
            list_x + shape.list - shape.inset - field_x,
            body_h + t.space.hair as f64,
        );
        unsafe { field.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(root))) };
        field.setAccessibilityLabel(Some(&NSString::from_str("Search models")));
        push(&mut views, &field);
        push(
            &mut views,
            &rule(
                NSRect::new(
                    NSPoint::new(list_x, pad + shape.search),
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
                NSPoint::new(list_x, pad + top),
                NSSize::new(shape.list, list_h - top),
            ),
        );
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setBorderType(NSBorderType::NoBorder);
        scroll.setDocumentView(Some(&list));
        list.setFrameSize(NSSize::new(
            shape.list,
            rows_h.max(scroll.contentSize().height),
        ));
        // The fades follow the scroll.
        let clip = scroll.contentView();
        clip.setPostsBoundsChangedNotifications(true);
        let weak = Rc::downgrade(state);
        let observer = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(NSViewBoundsDidChangeNotification),
                Some(&clip),
                None,
                &block2::RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
                    if let Some(state) = weak.upgrade() {
                        fades(&state);
                    }
                }),
            )
        };
        push(&mut views, &scroll);
        {
            let mut s = state.borrow_mut();
            s.scroll = Some(scroll);
            s.observers.push(observer);
        }
        search = Some(field);
    }
    // The usage monitor (Aero's credits), under a hairline at the card's foot.
    if !spec.usage.is_empty() {
        push(
            &mut views,
            &rule(
                NSRect::new(
                    NSPoint::new(list_x, pad + list_h),
                    NSSize::new(shape.list, shape.line),
                ),
                &t,
                mtm,
            ),
        );
        let inner = shape.list - 2. * shape.inset;
        list_h += shape.line + shape.lane;
        for (key, value) in &spec.usage {
            row(&mut views, list_x + shape.inset, list_h, inner, key, value);
            list_h += shape.fact;
        }
        list_h += shape.lane;
    }

    // The rail: a mark per harness on its tile, then "+" at its foot.
    let tile_inset = ((shape.rail - shape.tile) / 2.).floor();
    let step = shape.tile + shape.tile_gap;
    let harnesses = spec.of(Part::Harness).count() as f64;
    let add = spec.of(Part::Add).next();
    let rail_content = 2. * tile_inset + (harnesses + if add.is_some() { 1. } else { 0. }) * step
        - shape.tile_gap
        + if add.is_some() { shape.gap } else { 0. };
    // The rail and the card share one height.
    let rail_h = rail_content.max(list_h);
    let rail_tile = |views: &mut Vec<Retained<NSView>>, i: usize, row: &Row, top: f64| {
        let frame = NSRect::new(
            NSPoint::new(rail_x + tile_inset, top),
            NSSize::new(shape.tile, shape.tile),
        );
        if row.checked {
            // The chosen harness's tile, with a hairline so it reads in light too.
            let chosen = tile(
                frame,
                t.colors.selected_fill,
                t.metrics.control_radius as f64,
                mtm,
            );
            chosen.setBorderWidth(shape.line);
            chosen.setBorderColor(&native_color(t.colors.separator));
            views.push(Retained::into_super(chosen));
        }
        if let Some(icon) = &row.icon {
            let ink = if row.checked {
                t.colors.text
            } else {
                t.colors.secondary
            };
            let image = mark(icon, shape.rail_icon, ink, mtm);
            let o = (shape.tile - shape.rail_icon) / 2.;
            image.setFrame(NSRect::new(
                NSPoint::new(frame.origin.x + o, frame.origin.y + o),
                NSSize::new(shape.rail_icon, shape.rail_icon),
            ));
            views.push(Retained::into_super(Retained::into_super(image)));
        }
        let button = press(root, i, frame, &row.label, mtm);
        button.setToolTip(Some(&NSString::from_str(&row.label)));
        if row.checked {
            unsafe { button.setAccessibilityValue(Some(&NSString::from_str("Selected"))) };
        }
        views.push(Retained::into_super(Retained::into_super(button)));
    };
    for (n, (i, row)) in spec.of(Part::Harness).enumerate() {
        rail_tile(&mut views, i, row, pad + tile_inset + n as f64 * step);
    }
    if let Some((i, row)) = add {
        let row = Row {
            icon: Some("plus".into()),
            ..row.clone()
        };
        rail_tile(&mut views, i, &row, pad + rail_h - tile_inset - shape.tile);
    }

    let width = list_x + shape.list + pad;
    let height = pad + rail_h + pad;
    root.addSubview(&surface(
        NSRect::new(NSPoint::new(rail_x, pad), NSSize::new(shape.rail, rail_h)),
        &t,
        &shape,
        mtm,
    ));
    root.addSubview(&surface(
        NSRect::new(NSPoint::new(list_x, pad), NSSize::new(shape.list, rail_h)),
        &t,
        &shape,
        mtm,
    ));
    for view in views {
        root.addSubview(&view);
    }
    {
        let mut s = state.borrow_mut();
        s.search = search;
        if empty {
            s.list = None;
            s.scroll = None;
        }
    }
    NSSize::new(width, height)
}

/// Where a menu's surfaces go, in screen points (AppKit's: y up): above the
/// pill part it hangs from, `offset` clear of it, the card's leading edge
/// (`lead` into the surfaces: past the model picker's rail) on the part's
/// leading edge; below the part when there is no room above. Always inside
/// `room` (the editor window on its screen), moved in along each axis.
pub fn placement(size: NSSize, part: NSRect, lead: f64, room: NSRect, offset: f64) -> NSRect {
    let (min_x, max_x) = (room.origin.x, room.origin.x + room.size.width);
    let (min_y, max_y) = (room.origin.y, room.origin.y + room.size.height);
    let x = (part.origin.x - lead).min(max_x - size.width).max(min_x);
    let above = part.origin.y + part.size.height + offset;
    let below = part.origin.y - offset - size.height;
    let y = if above + size.height <= max_y || below < min_y {
        above.min(max_y - size.height).max(min_y)
    } else {
        below
    };
    NSRect::new(NSPoint::new(x, y), size)
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
    let (spec, t, query, width) = {
        let s = state.borrow();
        (s.spec.clone(), s.tokens.clone(), s.query.clone(), s.width)
    };
    let shape = Shape::new(&t);
    let caption = t.text(pm_design::Text::Caption).size;
    let caption_h = fitting(&label("Ag", caption, 0, mtm)).height;
    let body_h = body_height(&t, mtm);
    let x = shape.inset;
    let label_x = shape.label_x(spec.menu);
    // The highlight sits inside the surface with the content's own inset.
    let lane = shape.lane;
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
            t.typography.body,
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
        let title = label(&row.label, t.typography.body, ink, mtm);
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
        self.state.borrow().open
    }
    pub(super) fn owns_event(&self, event: &NSEvent) -> bool {
        self.is_open() && self.state.borrow().window.windowNumber() == event.windowNumber()
    }
    pub(super) fn key(&self, chord: &str) -> bool {
        key(&self.state, chord)
    }
    pub(super) fn close(&self) {
        close(&self.state);
    }
}

/// The part of `a` inside `b` (`a` when they do not meet).
fn intersection(a: NSRect, b: NSRect) -> NSRect {
    let x = a.origin.x.max(b.origin.x);
    let y = a.origin.y.max(b.origin.y);
    let r = (a.origin.x + a.size.width).min(b.origin.x + b.size.width);
    let t = (a.origin.y + a.size.height).min(b.origin.y + b.size.height);
    if r <= x || t <= y {
        return a;
    }
    NSRect::new(NSPoint::new(x, y), NSSize::new(r - x, t - y))
}

/// Places the chooser's panel for its surfaces (`size` holds their shadow
/// room) from the pill and composer it was opened off, in screen points.
fn frame_panel(state: &Rc<RefCell<State>>, size: NSSize) {
    let s = state.borrow();
    let shape = Shape::new(&s.tokens);
    let shadow = shape.shadow;
    let surfaces = NSSize::new(size.width - 2. * shadow, size.height - 2. * shadow);
    let (part, room) = s.around;
    // The model card, not its rail, stands on the part's leading edge.
    let lead = match s.spec.menu {
        Menu::Models => shape.rail + shape.gutter,
        Menu::Effort => 0.,
    };
    let at = placement(surfaces, part, lead, room, shape.offset);
    s.window.setFrame_display(
        NSRect::new(
            NSPoint::new(at.origin.x - shadow, at.origin.y - shadow),
            size,
        ),
        true,
    );
}

impl Host {
    /// Opens one of the pill's menus above `anchor` (the pill part, in the
    /// owner's content coordinates), replacing one already open: one
    /// borderless panel, a child of the editor window, holding its floating
    /// surfaces. It closes on a press outside it, Escape, the app going
    /// inactive, and its window moving or resizing.
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
        let mtm = self.window.mtm();
        let window: Retained<PopupWindow> = unsafe {
            msg_send![PopupWindow::alloc(mtm),initWithContentRect:NSRect::new(NSPoint::ZERO, NSSize::new(1., 1.)),styleMask:NSWindowStyleMask::Borderless,backing:NSBackingStoreType::Buffered,defer:false]
        };
        unsafe { window.setReleasedWhenClosed(false) };
        gpui_macos::child_window::apply(&window);
        window.setTitle(&NSString::from_str("Agent Chooser"));
        window.setOpaque(false);
        window.setHasShadow(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        appearance(&window, &self.tokens);
        let around = {
            let h = owner.contentView().map_or(0., |c| c.bounds().size.height);
            let part = owner.convertRectToScreen(rect(anchor, h));
            // The editor window it belongs to (the Agent popover is its
            // child), on its screen.
            let editor = owner.parentWindow().unwrap_or_else(|| owner.clone());
            let visible = editor.screen().map_or(editor.frame(), |s| s.visibleFrame());
            (part, intersection(editor.frame(), visible))
        };
        let state = Rc::new(RefCell::new(State {
            spec,
            query: String::new(),
            lit: None,
            sender: self.sender.clone(),
            dismiss,
            picked: false,
            tokens: self.tokens.clone(),
            window: window.clone(),
            parent: owner.clone(),
            open: true,
            around,
            width: 0.,
            watchers: vec![],
            observers: vec![],
            monitor: None,
            root: None,
            list: None,
            scroll: None,
            search: None,
            frames: vec![],
            highlight: None,
        }));
        let size = build(&state, mtm);
        let Some(root) = state.borrow().root.clone() else {
            return;
        };
        window.setContentView(Some(&root));
        frame_panel(&state, size);
        // What closes it from outside.
        let center = NSNotificationCenter::defaultCenter();
        let closer = |state: &Rc<RefCell<State>>, why: Outside| {
            let weak = Rc::downgrade(state);
            block2::RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
                if dismisses(why)
                    && let Some(state) = weak.upgrade()
                {
                    close(&state);
                }
            })
        };
        let app = NSApplication::sharedApplication(mtm);
        let watched: [(&NSString, Option<&AnyObject>, Outside); 3] = unsafe {
            [
                (
                    NSApplicationDidResignActiveNotification,
                    Some(&*app as &AnyObject),
                    Outside::AppResigned,
                ),
                (
                    NSWindowDidMoveNotification,
                    Some(&*owner as &AnyObject),
                    Outside::WindowMoved,
                ),
                (
                    NSWindowDidResizeNotification,
                    Some(&*owner as &AnyObject),
                    Outside::WindowResized,
                ),
            ]
        };
        for (name, object, why) in watched {
            let observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(name),
                    object,
                    None,
                    &closer(&state, why),
                )
            };
            state.borrow_mut().watchers.push(observer);
        }
        let weak = Rc::downgrade(&state);
        let panel = window.windowNumber();
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::LeftMouseDown
                    | NSEventMask::RightMouseDown
                    | NSEventMask::OtherMouseDown,
                &block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
                    let e = event.as_ref();
                    if dismisses(Outside::Press {
                        in_chooser: e.windowNumber() == panel,
                    }) && let Some(state) = weak.upgrade()
                    {
                        close(&state);
                    }
                    event.as_ptr()
                }),
            )
        };
        state.borrow_mut().monitor = monitor;
        gpui_macos::child_window::attach(&window, &owner);
        gpui_macos::child_window::show(&window, true);
        // The fades from the first layout, again once the panel is on
        // screen (its layers are the window's now) and after that pass.
        fades(&state);
        let weak = Rc::downgrade(&state);
        let later = block2::RcBlock::new(move || {
            if let Some(state) = weak.upgrade() {
                fades(&state);
            }
        });
        unsafe { objc2_foundation::NSOperationQueue::mainQueue().addOperationWithBlock(&later) };
        if let Some(search) = state.borrow().search.clone() {
            window.makeFirstResponder(Some(&search));
        } else if let Some(root) = state.borrow().root.clone() {
            // The reasoning menu takes the keys itself.
            window.makeFirstResponder(Some(&root));
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
        let had_focus = {
            let s = state.borrow();
            s.search.is_some() && s.window.firstResponder().is_some()
        };
        let window = {
            let mut s = state.borrow_mut();
            s.spec = spec;
            s.lit = None;
            if dismiss.is_some() {
                s.dismiss = dismiss;
            }
            s.window.clone()
        };
        let mtm = self.window.mtm();
        // The old list's scroll observer goes with it.
        let center = NSNotificationCenter::defaultCenter();
        for observer in std::mem::take(&mut state.borrow_mut().observers) {
            unsafe { center.removeObserver(observer.as_ref()) };
        }
        let size = build(&state, mtm);
        let Some(root) = state.borrow().root.clone() else {
            return false;
        };
        window.setContentView(Some(&root));
        // Placed again for its new size: still clear of the composer.
        frame_panel(&state, size);
        if let (Some(origin), Some(scroll)) = (kept, state.borrow().scroll.clone()) {
            let clip = scroll.contentView();
            clip.scrollToPoint(origin);
            scroll.reflectScrolledClipView(&clip);
        }
        fades(&state);
        if had_focus && let Some(search) = state.borrow().search.clone() {
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
                row(Part::Harness, "Aero"),
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
            rows: ["Aero", "Codex"]
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
        assert!(spec("Aero", 0).keeps_place(&spec("Aero", 17)));
        assert!(!spec("Aero", 0).keeps_place(&spec("Codex", 0)));
        assert!(!Spec::default().keeps_place(&Spec::default()));
        let t = Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default());
        let shape = Shape::new(&t);
        assert!(shape.search + 20. * shape.row > shape.most);
    }
    #[test]
    fn everything_outside_dismisses_it_but_a_press_inside() {
        assert!(!dismisses(Outside::Press { in_chooser: true }));
        for outside in [
            Outside::Press { in_chooser: false },
            Outside::Escape,
            Outside::AppResigned,
            Outside::WindowMoved,
            Outside::WindowResized,
        ] {
            assert!(dismisses(outside), "{outside:?}");
        }
    }
    #[test]
    fn a_menu_hangs_above_its_part_on_the_parts_leading_edge_inside_the_window() {
        let r = |x: f64, y: f64, w: f64, h: f64| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        let window = r(100., 50., 1200., 800.);
        let part = r(900., 80., 140., 24.);
        let size = NSSize::new(364., 300.);
        // Above the part, `offset` clear of it; the card (past a 44 pt rail)
        // on the part's leading edge.
        let at = placement(size, part, 44., window, 6.);
        assert_eq!(at.origin.y, 80. + 24. + 6.);
        assert_eq!(at.origin.x, 900. - 44.);
        // The reasoning menu: its own leading edge on the part's.
        assert_eq!(placement(size, part, 0., window, 6.).origin.x, 900.);
        // Near the window's trailing edge it moves in, never out of it.
        let edge = r(1250., 80., 40., 24.);
        let at = placement(size, edge, 0., window, 6.);
        assert_eq!(at.origin.x + at.size.width, 1300.);
        // Near its leading edge, the rail stays inside too.
        let lead = r(110., 80., 40., 24.);
        assert_eq!(placement(size, lead, 44., window, 6.).origin.x, 100.);
        // No room above (a part at the window's top): below it.
        let high = r(900., 760., 140., 24.);
        let at = placement(size, high, 0., window, 6.);
        assert_eq!(at.origin.y + at.size.height, 760. - 6.);
        // Room neither above nor below: as high as the window allows.
        let short = r(100., 50., 1200., 320.);
        let at = placement(size, r(900., 200., 140., 24.), 0., short, 6.);
        assert_eq!(at.origin.y + at.size.height, 370.);
        assert!(at.origin.y >= 50.);
    }
    #[test]
    fn a_request_becomes_rows_and_the_usage_monitor() {
        let spec = Spec::from_request(&serde_json::json!({
            "placeholder": "Search models…",
            "usage": [["Credits", "812 of 1,000 left"], ["Plan", "Aerolite"]],
            "options": [
                {"kind": "harness", "label": "Codex", "icon": "provider.chatgpt", "checked": true, "stays": true},
                {"kind": "model", "label": "GPT Sol", "checked": true},
                {"kind": "add", "label": "Providers…"},
                {"kind": "unknown", "label": "?"}
            ]
        }));
        assert_eq!(spec.menu, Menu::Models);
        assert_eq!(
            spec.usage,
            vec![
                ("Credits".to_owned(), "812 of 1,000 left".to_owned()),
                ("Plan".to_owned(), "Aerolite".to_owned())
            ]
        );
        assert_eq!(spec.rows[0].part, Some(Part::Harness));
        assert!(spec.rows[0].checked && spec.rows[0].stays);
        assert!(!spec.rows[1].stays);
        assert_eq!(spec.rows[2].part, Some(Part::Add));
        assert_eq!(spec.rows[3].part, None);
        assert_eq!(spec.placeholder, "Search models…");
    }
    #[test]
    fn the_reasoning_menu_lists_every_level_in_order_with_no_shortcuts() {
        let options: Vec<_> = ["Low", "Medium", "High", "Extra high", "Max"]
            .iter()
            .enumerate()
            .map(|(n, l)| serde_json::json!({"kind": "effort", "label": l, "checked": n == 2}))
            .collect();
        let spec = Spec::from_request(&serde_json::json!({"menu": "effort", "options": options}));
        assert_eq!(spec.menu, Menu::Effort);
        assert_eq!(spec.listed(""), vec![0, 1, 2, 3, 4]);
        // A query never hides a level (the menu has no search).
        assert_eq!(spec.listed("max"), vec![0, 1, 2, 3, 4]);
        assert!(spec.numbered("").is_empty());
        assert!(spec.listed("").iter().all(|&i| choosable(&spec.rows[i])));
        assert!(spec.rows[2].checked);
    }
}
