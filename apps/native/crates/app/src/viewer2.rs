//! Viewer 2: output mastering switch (SDR / PQ / HLG), engine-drawn locked
//! overlays (safe areas, guides), a popout viewer window and the View-menu
//! items for them. Everything here is main-thread AppKit glue; pixels,
//! overlays and output transforms are drawn by engine-service in its final
//! display pass, never by GPUI.
use crate::session::Controller;
use foreign_types::ForeignType;
use gpui::{App, Context, WeakEntity};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::AnyObject, sel,
};
use objc2_app_kit::{
    NSBackingStoreType, NSMenu, NSMenuDelegate, NSMenuItem, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use pm_engine_service::{MediaMode, OverlayGeometry, OverlayLine, color::Mastering};
use std::{cell::RefCell, rc::Rc};

/// The viewer's output transform. SDR keeps the default extended-linear P3
/// layer (EDR values preserved); PQ and HLG relabel the layer BT.2100 and
/// apply the engine's explicit mastering transform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputMode {
    #[default]
    Sdr,
    /// An HDR project's light in the EDR layer, up to the display's headroom.
    Edr,
    Pq,
    Hlg,
}
/// Explicit mastering peaks (nits) of an SDR project; an HDR project's come
/// from the project. Dynamic display headroom never edits authored light; it
/// is telemetry for WindowServer's display mapping.
pub const SOURCE_PEAK: f32 = 1000.;
pub const TARGET_PEAK: f32 = 1000.;
impl OutputMode {
    pub const ALL: [OutputMode; 4] = [
        OutputMode::Sdr,
        OutputMode::Edr,
        OutputMode::Pq,
        OutputMode::Hlg,
    ];
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "sdr" | "linear" | "p3" => Some(Self::Sdr),
            "edr" => Some(Self::Edr),
            "pq" | "hdr10" | "st2084" => Some(Self::Pq),
            "hlg" => Some(Self::Hlg),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Sdr => "sdr",
            Self::Edr => "edr",
            Self::Pq => "pq",
            Self::Hlg => "hlg",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Sdr => "SDR (Extended Linear P3)",
            Self::Edr => "HDR EDR (Extended Linear P3)",
            Self::Pq => "HDR PQ (BT.2100)",
            Self::Hlg => "HDR HLG (BT.2100)",
        }
    }
    pub fn mastering(self) -> Mastering {
        match self {
            Self::Sdr => Mastering::LinearP3,
            Self::Edr => Mastering::Edr,
            Self::Pq => Mastering::Pq {
                source_peak: SOURCE_PEAK,
                target_peak: TARGET_PEAK,
            },
            Self::Hlg => Mastering::Hlg {
                source_peak: SOURCE_PEAK,
                target_peak: TARGET_PEAK,
            },
        }
    }
    /// `PM_NATIVE_VIEWER_OUTPUT=sdr|pq|hlg` selects the launch mode (verification).
    pub fn from_env() -> Self {
        std::env::var("PM_NATIVE_VIEWER_OUTPUT")
            .ok()
            .and_then(|s| Self::parse(&s))
            .unwrap_or_default()
    }
}

/// SMPTE ST 2046-1 safe areas: action safe 93 %, title safe 90 % of each axis.
pub const ACTION_SAFE: f32 = 0.93;
pub const TITLE_SAFE: f32 = 0.90;
/// Linear P3 RGBA. Neutral grey safe areas, accent-blue guides; both partly
/// transparent so the picture stays readable under them.
const SAFE_COLOR: [f32; 4] = [0.75, 0.75, 0.75, 0.7];
const GUIDE_COLOR: [f32; 4] = [0.05, 0.35, 1.0, 0.7];

fn rect(lines: &mut Vec<OverlayLine>, scale: f32, color: [f32; 4]) {
    let (a, b) = ((1. - scale) / 2., (1. + scale) / 2.);
    for ends in [[a, a, b, a], [b, a, b, b], [b, b, a, b], [a, b, a, a]] {
        lines.push(OverlayLine { ends, color });
    }
}
/// Normalized to the content viewport, so they register with the picture at
/// any zoom, size or display (the engine draws them in the same pass).
pub fn overlay_lines(safe: bool, guides: bool) -> Vec<OverlayLine> {
    let mut lines = vec![];
    if safe {
        rect(&mut lines, ACTION_SAFE, SAFE_COLOR);
        rect(&mut lines, TITLE_SAFE, SAFE_COLOR);
    }
    if guides {
        for third in [1. / 3., 2. / 3.] {
            lines.push(OverlayLine {
                ends: [third, 0., third, 1.],
                color: GUIDE_COLOR,
            });
            lines.push(OverlayLine {
                ends: [0., third, 1., third],
                color: GUIDE_COLOR,
            });
        }
        // Centre cross, 4 % of each axis.
        lines.push(OverlayLine {
            ends: [0.48, 0.5, 0.52, 0.5],
            color: GUIDE_COLOR,
        });
        lines.push(OverlayLine {
            ends: [0.5, 0.48, 0.5, 0.52],
            color: GUIDE_COLOR,
        });
    }
    lines
}
/// The Resolution pop-up (Preview Resolution), read from the same view state
/// the panel writes. The engine renders at it; `Auto` adapts while playing.
pub fn engine_quality(chosen: pm_panel_viewerchrome::state::Quality) -> pm_engine_service::Quality {
    use pm_engine_service::Quality as Q;
    use pm_panel_viewerchrome::state::Quality as Chosen;
    match chosen {
        Chosen::Auto => Q::Auto,
        Chosen::Full => Q::Full,
        Chosen::Half => Q::Half,
        Chosen::Quarter => Q::Quarter,
    }
}

/// Menu tags. Stable: automation names the same actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Output(OutputMode),
    SafeAreas,
    Guides,
    Popout,
    Media(MediaMode),
    ToggleProxies,
    ApplyLut,
    /// An index of [`crate::colour::CAMERA_LOGS`], or the file's own reading.
    Interpret(Option<usize>),
    CleanAperture,
    RenderSelection,
    KeepRendered,
    DeleteRendered,
    Skimming,
    AudioSkimming,
}
/// View › Viewer Media, in the order of the menu.
const MEDIA_MODES: [(MediaMode, &str); 4] = [
    (MediaMode::Automatic, "Automatic"),
    (MediaMode::Originals, "Originals"),
    (MediaMode::ProxyPreferred, "Proxy Preferred"),
    (MediaMode::ProxyOnly, "Proxy Only"),
];
impl Action {
    const ALL: [Action; 26] = [
        Action::Output(OutputMode::Sdr),
        Action::Output(OutputMode::Pq),
        Action::Output(OutputMode::Hlg),
        Action::SafeAreas,
        Action::Guides,
        Action::Popout,
        Action::Media(MediaMode::Automatic),
        Action::Media(MediaMode::Originals),
        Action::Media(MediaMode::ProxyPreferred),
        Action::Media(MediaMode::ProxyOnly),
        Action::ToggleProxies,
        Action::Output(OutputMode::Edr),
        Action::ApplyLut,
        Action::Interpret(None),
        Action::Interpret(Some(0)),
        Action::Interpret(Some(1)),
        Action::Interpret(Some(2)),
        Action::Interpret(Some(3)),
        Action::Interpret(Some(4)),
        Action::Interpret(Some(5)),
        Action::CleanAperture,
        Action::RenderSelection,
        Action::KeepRendered,
        Action::DeleteRendered,
        Action::Skimming,
        Action::AudioSkimming,
    ];
    fn tag(self) -> isize {
        Self::ALL.iter().position(|a| *a == self).unwrap() as isize
    }
    fn from_tag(tag: isize) -> Option<Self> {
        usize::try_from(tag)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }
}

struct MenuIvars {
    actions: async_channel::Sender<Action>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMNativeViewerMenuTarget"]
    #[ivars = MenuIvars]
    struct MenuTarget;
    impl MenuTarget {
        #[unsafe(method(act:))]
        fn act(&self, sender: &NSMenuItem) {
            if let Some(action) = Action::from_tag(sender.tag()) {
                let _ = self.ivars().actions.try_send(action);
            }
        }
    }
);

/// `SEL *` out-parameter of `menuHasKeyEquivalent:forEvent:target:action:`.
#[repr(transparent)]
struct SelOut(*mut std::ffi::c_void);
unsafe impl objc2::encode::Encode for SelOut {
    const ENCODING: objc2::encode::Encoding =
        objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Sel);
}
define_class!(
    /// The Skimming menu shows its shortcuts (⌥S, ⌥⇧S) without matching
    /// them: the shell's chord resolver owns them, so in a text field ⌥S
    /// still types.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMNativeSkimMenuDelegate"]
    struct DisplayOnly;
    unsafe impl NSObjectProtocol for DisplayOnly {}
    unsafe impl NSMenuDelegate for DisplayOnly {
        #[unsafe(method(menuHasKeyEquivalent:forEvent:target:action:))]
        fn has_key_equivalent(
            &self,
            _: &NSMenu,
            _: &objc2_app_kit::NSEvent,
            _: *mut *mut AnyObject,
            _: SelOut,
        ) -> bool {
            false
        }
    }
);

struct Menu {
    _target: Retained<MenuTarget>,
    _skim_delegate: Option<Retained<DisplayOnly>>,
    items: Vec<(Action, Retained<NSMenuItem>)>,
    controllers: Vec<WeakEntity<Controller>>,
}
thread_local! { static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) }; }

/// Adds a "Viewer" section to the shell's View menu (AppKit, like the Undo
/// label update). The shell's menu model stays unchanged; when the shell grows
/// these commands, this glue is removed.
pub fn install(entity: &gpui::Entity<Controller>, cx: &mut App) {
    let weak = entity.downgrade();
    let first = MENU.with(|m| m.borrow().is_none());
    if first {
        let mtm = MainThreadMarker::new().unwrap();
        let (tx, rx) = async_channel::unbounded();
        let target: Retained<MenuTarget> = unsafe {
            msg_send![
                super(MenuTarget::alloc(mtm).set_ivars(MenuIvars { actions: tx })),
                init
            ]
        };
        let mut items = vec![];
        let mut skim_delegate = None;
        if let Some(view) = view_menu(mtm) {
            view.addItem(&NSMenuItem::separatorItem(mtm));
            let output =
                NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Viewer Output"));
            for mode in OutputMode::ALL {
                let item = menu_item(mtm, mode.label(), Action::Output(mode), &target, "");
                output.addItem(&item);
                items.push((Action::Output(mode), item));
            }
            let parent = menu_item(
                mtm,
                "Viewer Output",
                Action::Output(OutputMode::Sdr),
                &target,
                "",
            );
            unsafe { parent.setAction(None) };
            parent.setSubmenu(Some(&output));
            view.addItem(&parent);
            let media =
                NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Viewer Media"));
            for (mode, label) in MEDIA_MODES {
                let item = menu_item(mtm, label, Action::Media(mode), &target, "");
                media.addItem(&item);
                items.push((Action::Media(mode), item));
            }
            let parent = menu_item(
                mtm,
                "Viewer Media",
                Action::Media(MediaMode::Automatic),
                &target,
                "",
            );
            unsafe { parent.setAction(None) };
            parent.setSubmenu(Some(&media));
            view.addItem(&parent);
            for (label, action) in [
                ("Toggle Proxies", Action::ToggleProxies),
                ("Show Safe Areas", Action::SafeAreas),
                ("Show Guides", Action::Guides),
                ("Popout Viewer", Action::Popout),
            ] {
                let item = menu_item(mtm, label, action, &target, "");
                view.addItem(&item);
                items.push((action, item));
            }
            view.addItem(&NSMenuItem::separatorItem(mtm));
            for (label, action) in [
                ("Render Selection", Action::RenderSelection),
                ("Keep Rendered", Action::KeepRendered),
                ("Delete Rendered Frames", Action::DeleteRendered),
            ] {
                view.addItem(&menu_item(mtm, label, action, &target, ""));
            }
            view.addItem(&NSMenuItem::separatorItem(mtm));
            let skim = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Skimming"));
            let display_only: Retained<DisplayOnly> =
                unsafe { msg_send![DisplayOnly::alloc(mtm), init] };
            skim.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(
                &*display_only,
            )));
            for (label, action, shift) in [
                ("Skimming", Action::Skimming, false),
                ("Audio Skimming", Action::AudioSkimming, true),
            ] {
                let item = menu_item(mtm, label, action, &target, "s");
                let mut mask = objc2_app_kit::NSEventModifierFlags::Option;
                if shift {
                    mask |= objc2_app_kit::NSEventModifierFlags::Shift;
                }
                item.setKeyEquivalentModifierMask(mask);
                skim.addItem(&item);
                items.push((action, item));
            }
            let parent = menu_item(mtm, "Skimming", Action::Skimming, &target, "");
            unsafe { parent.setAction(None) };
            parent.setSubmenu(Some(&skim));
            view.addItem(&parent);
            skim_delegate = Some(display_only);
            view.addItem(&NSMenuItem::separatorItem(mtm));
            view.addItem(&menu_item(mtm, "Apply LUT…", Action::ApplyLut, &target, ""));
            let interpret = NSMenu::initWithTitle(
                NSMenu::alloc(mtm),
                &NSString::from_str("Interpret Selection As"),
            );
            interpret.addItem(&menu_item(
                mtm,
                "As the File Says",
                Action::Interpret(None),
                &target,
                "",
            ));
            for (index, log) in crate::colour::CAMERA_LOGS.iter().enumerate() {
                let action = Action::Interpret(Some(index));
                interpret.addItem(&menu_item(mtm, log.label, action, &target, ""));
            }
            interpret.addItem(&NSMenuItem::separatorItem(mtm));
            interpret.addItem(&menu_item(
                mtm,
                "Crop to Clean Aperture",
                Action::CleanAperture,
                &target,
                "",
            ));
            let parent = menu_item(
                mtm,
                "Interpret Selection As",
                Action::Interpret(None),
                &target,
                "",
            );
            unsafe { parent.setAction(None) };
            parent.setSubmenu(Some(&interpret));
            view.addItem(&parent);
        }
        MENU.with(|m| {
            *m.borrow_mut() = Some(Menu {
                _target: target,
                _skim_delegate: skim_delegate,
                items,
                controllers: vec![],
            })
        });
        cx.spawn(async move |cx| {
            while let Ok(action) = rx.recv().await {
                cx.update(|cx| {
                    if let Some(c) = focused(cx) {
                        c.update(cx, |v, cx| v.viewer_action(action, cx));
                    }
                });
            }
        })
        .detach();
    }
    MENU.with(|m| {
        if let Some(m) = &mut *m.borrow_mut() {
            m.controllers.push(weak);
        }
    });
}
fn view_menu(_: MainThreadMarker) -> Option<Retained<NSMenu>> {
    let app = objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
    app.mainMenu()?
        .itemArray()
        .iter()
        .find(|i| i.title().to_string() == "View")?
        .submenu()
}
fn menu_item(
    mtm: MainThreadMarker,
    label: &str,
    action: Action,
    target: &MenuTarget,
    key: &str,
) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(label),
            Some(sel!(act:)),
            &NSString::from_str(key),
        )
    };
    item.setTag(action.tag());
    unsafe { item.setTarget(Some(target)) };
    item
}
/// The controller owning the key window (editor or its popout), else the
/// first live one.
fn focused(cx: &mut App) -> Option<gpui::Entity<Controller>> {
    let live: Vec<_> = MENU.with(|m| {
        m.borrow_mut().as_mut().map_or(vec![], |m| {
            m.controllers.retain(|w| w.upgrade().is_some());
            m.controllers.iter().filter_map(|w| w.upgrade()).collect()
        })
    });
    live.iter()
        .find(|c| {
            let v = c.read(cx);
            v.host.borrow().window.isKeyWindow()
                || v.popout.as_ref().is_some_and(|p| p.window.isKeyWindow())
        })
        .or(live.first())
        .cloned()
}
/// Checkmarks follow the focused controller's state.
pub fn update_checks(output: OutputMode, media: MediaMode, safe: bool, guides: bool, popout: bool) {
    MENU.with(|m| {
        if let Some(m) = &*m.borrow() {
            for (action, item) in &m.items {
                let on = match action {
                    Action::Output(mode) => *mode == output,
                    Action::Media(mode) => *mode == media,
                    Action::ToggleProxies => false,
                    Action::SafeAreas => safe,
                    Action::Guides => guides,
                    Action::Popout => popout,
                    Action::ApplyLut
                    | Action::Interpret(_)
                    | Action::CleanAperture
                    | Action::RenderSelection
                    | Action::KeepRendered
                    | Action::DeleteRendered => false,
                    Action::Skimming | Action::AudioSkimming => continue,
                };
                item.setState(if on { 1 } else { 0 });
            }
        }
    });
}

/// Checkmarks of View ▸ Skimming.
pub fn update_skim_checks(skimming: bool, audio: bool) {
    MENU.with(|m| {
        if let Some(m) = &*m.borrow() {
            for (action, item) in &m.items {
                match action {
                    Action::Skimming => item.setState(if skimming { 1 } else { 0 }),
                    Action::AudioSkimming => item.setState(if audio { 1 } else { 0 }),
                    _ => {}
                }
            }
        }
    });
}

struct ViewIvars {
    layer: metal::MetalLayer,
    closed: async_channel::Sender<()>,
}
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMNativePopoutViewerView"]
    #[ivars = ViewIvars]
    struct PopoutView;
    impl PopoutView {
        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool { true }
        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.fit();
        }
        #[unsafe(method(viewDidChangeBackingProperties))]
        fn backing_changed(&self) {
            let _: () = unsafe { msg_send![super(self), viewDidChangeBackingProperties] };
            self.fit();
        }
        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, _: Option<&objc2_foundation::NSNotification>) {
            let _ = self.ivars().closed.try_send(());
        }
    }
);
impl PopoutView {
    /// The layer fills the view; its drawable follows bounds × backing scale.
    /// The engine-side OutputBinding observes `drawableSize` and reconfigures.
    fn fit(&self) {
        let bounds = self.bounds();
        let scale = self.window().map_or(2., |w| w.backingScaleFactor());
        let layer = &self.ivars().layer;
        unsafe {
            let p = layer.as_ptr() as *mut AnyObject;
            let _: () = msg_send![objc2::class!(CATransaction), begin];
            let _: () = msg_send![objc2::class!(CATransaction), setDisableActions: true];
            let _: () = msg_send![p, setFrame: bounds];
            let _: () = msg_send![p, setContentsScale: scale];
            // CGSize and NSSize share a layout on 64-bit macOS.
            let drawable = NSSize::new(
                (bounds.size.width * scale).round().max(1.),
                (bounds.size.height * scale).round().max(1.),
            );
            let _: () = msg_send![p, setDrawableSize: drawable];
            let _: () = msg_send![objc2::class!(CATransaction), commit];
        }
    }
}

/// A second viewer on the same engine device, following the editor's
/// publications and transport. It has no audio (the editor's viewer owns
/// the audio device) and its own display link and output binding, so it
/// reconfigures independently when dragged to another display.
pub struct Popout {
    pub window: Retained<NSWindow>,
    view: Retained<PopoutView>,
    pub layer: metal::MetalLayer,
    pub viewer: Rc<RefCell<Option<pm_engine_service::Viewer>>>,
    pub output: Rc<std::cell::Cell<Option<pm_engine_service::Output>>>,
    binding: Option<pm_engine_service::layers::OutputBinding>,
    link: Option<pm_engine_service::layers::DisplayLink>,
    /// Last transport state mirrored from the editor: (playing, media ticks).
    pub mirrored: Option<(bool, i64)>,
}
impl Popout {
    pub fn open(
        device: &metal::DeviceRef,
        title: &str,
    ) -> anyhow::Result<(Self, async_channel::Receiver<()>)> {
        let mtm = MainThreadMarker::new().unwrap();
        let layer = metal::MetalLayer::new();
        pm_engine_service::layers::configure_layer(mtm, &layer, device)?;
        // Points; the drawable is pixels (640 × 360 at 1×, 1280 × 720 at 2×).
        let content = NSRect::new(NSPoint::new(160., 160.), NSSize::new(640., 360.));
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                content,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Resizable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(title));
        window.setContentAspectRatio(NSSize::new(16., 9.));
        let (closed, rx) = async_channel::bounded(1);
        let view: Retained<PopoutView> = unsafe {
            msg_send![super(PopoutView::alloc(mtm).set_ivars(ViewIvars {
                layer: layer.clone(),
                closed,
            })), initWithFrame: NSRect::new(NSPoint::ZERO, content.size)]
        };
        unsafe {
            let _: () = msg_send![&*view, setLayer: layer.as_ptr() as *mut AnyObject];
        }
        view.setWantsLayer(true);
        window.setContentView(Some(&view));
        unsafe {
            objc2_foundation::NSNotificationCenter::defaultCenter()
                .addObserver_selector_name_object(
                    &view,
                    sel!(windowWillClose:),
                    Some(&NSString::from_str("NSWindowWillCloseNotification")),
                    Some(AsRef::<AnyObject>::as_ref(&*window)),
                );
        }
        // Bottom-right of the editor's screen: a fully covered editor viewer
        // gets its drawables back slowly, which stalls the shared worker.
        if let Some(screen) = window.screen() {
            let v = screen.visibleFrame();
            window.setFrameOrigin(NSPoint::new(
                v.origin.x + v.size.width - window.frame().size.width,
                v.origin.y,
            ));
        }
        gpui_macos::child_window::present(&window, true);
        view.fit();
        Ok((
            Self {
                window,
                view,
                layer,
                viewer: Default::default(),
                output: Default::default(),
                binding: None,
                link: None,
                mirrored: None,
            },
            rx,
        ))
    }
    /// Binds output/display-link feeds to the current viewer (call after
    /// creating or replacing it).
    pub fn bind(&mut self, viewer: pm_engine_service::Viewer, hz: f64) {
        if let Some(old) = self.viewer.borrow_mut().replace(viewer) {
            old.retire();
        }
        self.mirrored = None;
        let placed = self.viewer.clone();
        let output = self.output.clone();
        let binding = pm_engine_service::layers::OutputBinding::new(
            &self.window,
            self.layer.clone(),
            Rc::new(move |o| {
                output.set(Some(o));
                if let Some(v) = &*placed.borrow() {
                    let _ = v.configure(o);
                }
            }),
        );
        binding.sync();
        self.binding = Some(binding);
        if self.link.is_none() {
            let ticks = self.viewer.clone();
            self.link = Some(pm_engine_service::layers::DisplayLink::new(
                &self.view,
                hz,
                Rc::new(move |target, interval| {
                    if let Some(v) = &*ticks.borrow() {
                        let _ = v.display_tick(target, interval);
                    }
                }),
            ));
        }
    }
    pub fn initial_output(&self) -> pm_engine_service::Output {
        let size = self.layer.drawable_size();
        let base = pm_engine_service::Output {
            width: size.width.round().max(1.) as u32,
            height: size.height.round().max(1.) as u32,
            display: 0,
            headroom: 1.,
            crop: None,
        };
        self.window
            .screen()
            .and_then(|s| {
                pm_engine_service::layers::screen_output(
                    MainThreadMarker::new().unwrap(),
                    &s,
                    base.width,
                    base.height,
                )
                .ok()
            })
            .map_or(base, |(o, _)| o)
    }
    pub fn set_link_paused(&self, paused: bool) {
        if let Some(link) = &self.link {
            link.set_paused(paused);
        }
    }
    /// Stops feeds and retires the viewer; the window closes if still open.
    pub fn shutdown(mut self) {
        self.binding.take();
        self.link.take();
        if let Some(v) = self.viewer.borrow_mut().take() {
            v.retire();
        }
        unsafe {
            objc2_foundation::NSNotificationCenter::defaultCenter().removeObserver(&self.view);
        }
        if self.window.isVisible() {
            self.window.close();
        }
    }
}

/// Moves an AppKit window onto screen `index` (automation: display moves).
pub fn move_to_screen(window: &NSWindow, index: usize) -> anyhow::Result<()> {
    let mtm = MainThreadMarker::new().unwrap();
    let screens = objc2_app_kit::NSScreen::screens(mtm);
    anyhow::ensure!(
        index < screens.len(),
        "no screen {index} ({} screens)",
        screens.len()
    );
    let target = screens.objectAtIndex(index);
    let visible = target.visibleFrame();
    let frame = window.frame();
    let size = NSSize::new(
        frame.size.width.min(visible.size.width),
        frame.size.height.min(visible.size.height),
    );
    let origin = NSPoint::new(
        visible.origin.x + (visible.size.width - size.width) / 2.,
        visible.origin.y + (visible.size.height - size.height) / 2.,
    );
    window.setFrame_display(NSRect::new(origin, size), true);
    Ok(())
}
/// Screens with their current and potential EDR headroom (automation).
pub fn screens() -> Vec<serde_json::Value> {
    let mtm = MainThreadMarker::new().unwrap();
    objc2_app_kit::NSScreen::screens(mtm)
        .iter()
        .map(|s| {
            let f = s.frame();
            let id = pm_engine_service::layers::screen_output(mtm, &s, 1, 1)
                .map(|(o, _)| o.display)
                .unwrap_or(0);
            serde_json::json!({
                "display": id,
                "frame": [f.origin.x, f.origin.y, f.size.width, f.size.height],
                "headroom": s.maximumExtendedDynamicRangeColorComponentValue(),
                "potential_headroom": s.maximumPotentialExtendedDynamicRangeColorComponentValue(),
                "max_fps": s.maximumFramesPerSecond(),
            })
        })
        .collect()
}

/// Development builds: a View-menu render action by its menu title, for
/// on-screen reviews that must not press a menu item.
#[cfg(debug_assertions)]
pub fn review_action(title: &str) -> Option<Action> {
    match title {
        "Render Selection" => Some(Action::RenderSelection),
        "Keep Rendered" => Some(Action::KeepRendered),
        "Delete Rendered Frames" => Some(Action::DeleteRendered),
        _ => None,
    }
}

impl Controller {
    pub fn viewer_action(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::Output(mode) => self.set_output_mode(mode, cx),
            Action::Skimming => self.toggle_skimming(cx),
            Action::AudioSkimming => self.toggle_audio_skimming(cx),
            Action::SafeAreas => self.set_overlays(!self.safe_areas, self.guides, cx),
            Action::Guides => self.set_overlays(self.safe_areas, !self.guides, cx),
            Action::Popout => self.toggle_popout(cx),
            Action::Media(mode) => self.set_media_mode(mode, cx),
            // Premiere's toggle: proxies where they exist, or originals.
            Action::ToggleProxies => self.set_media_mode(
                match self.media_mode {
                    MediaMode::ProxyPreferred | MediaMode::ProxyOnly => MediaMode::Originals,
                    MediaMode::Automatic | MediaMode::Originals => MediaMode::ProxyPreferred,
                },
                cx,
            ),
            Action::ApplyLut => self.apply_lut(cx),
            Action::Interpret(log) => {
                let log = log.map(|index| crate::colour::CAMERA_LOGS[index]);
                self.interpret_selection(log, cx);
            }
            Action::CleanAperture => self.crop_selection(cx),
            Action::RenderSelection => self.render_selection(cx),
            Action::KeepRendered => self.keep_rendered(cx),
            Action::DeleteRendered => self.delete_rendered(cx),
        }
    }
    pub fn set_media_mode(&mut self, mode: MediaMode, cx: &mut Context<Self>) {
        self.media_mode = mode;
        self.apply_viewer_options();
        self.publish_offline_media(cx);
        self.update_builtins(cx);
        let label = MEDIA_MODES
            .iter()
            .find(|(m, _)| *m == mode)
            .map_or("", |(_, l)| l);
        self.message = format!("Viewer media: {label}");
        cx.notify();
    }
    pub fn set_output_mode(&mut self, mode: OutputMode, cx: &mut Context<Self>) {
        self.output_mode = mode;
        self.apply_viewer_options();
        self.message = format!("Viewer output: {}", mode.label());
        cx.notify();
    }
    pub fn set_overlays(&mut self, safe: bool, guides: bool, cx: &mut Context<Self>) {
        self.safe_areas = safe;
        self.guides = guides;
        self.apply_viewer_options();
        cx.notify();
    }
    /// Selection gizmos draw in the engine's own pass, with the safe areas.
    pub fn set_gizmos(&mut self, lines: Vec<OverlayLine>) {
        self.gizmo_lines = lines;
        if let Some(viewer) = &self.viewer
            && let Err(error) = viewer.safe_areas(self.overlay())
        {
            self.message = error.to_string();
        }
    }
    /// Guides first; a huge selection loses its last boxes, never the guides.
    fn overlay(&self) -> OverlayGeometry {
        let mut lines = overlay_lines(self.safe_areas, self.guides);
        let room = OverlayGeometry::MAX_LINES.saturating_sub(lines.len());
        lines.extend(self.gizmo_lines.iter().take(room).copied());
        OverlayGeometry::new(lines).expect("bounded overlay geometry")
    }
    /// Re-applies output mode, media, Preview Resolution and overlays to
    /// every viewer of this editor (after any viewer is created or an option
    /// changes).
    pub fn apply_viewer_options(&mut self) {
        let mtm = MainThreadMarker::new().unwrap();
        let popout = self.popout.as_ref().and_then(|p| p.viewer.borrow().clone());
        // Selection gizmos belong to the editing viewer alone.
        let geometries = [
            (self.viewer.as_ref(), self.overlay()),
            (
                popout.as_ref(),
                OverlayGeometry::new(overlay_lines(self.safe_areas, self.guides))
                    .expect("bounded overlay geometry"),
            ),
        ];
        for (viewer, geometry) in geometries {
            let Some(viewer) = viewer else { continue };
            if let Err(e) = viewer
                .mastering(mtm, self.output_mode.mastering())
                .and_then(|_| viewer.media_mode(self.media_mode))
                .and_then(|_| viewer.quality(self.quality))
                .and_then(|_| viewer.safe_areas(geometry))
            {
                self.message = format!("Viewer: {e}");
            }
        }
        update_checks(
            self.output_mode,
            self.media_mode,
            self.safe_areas,
            self.guides,
            self.popout.is_some(),
        );
    }
    pub fn toggle_popout(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.popout.take() {
            p.shutdown();
            self.apply_viewer_options();
            cx.notify();
            return;
        }
        let title = format!("{} — Viewer", self.host.borrow().window.title());
        match Popout::open(&self.engine.metal_device(), &title) {
            Ok((popout, closed)) => {
                self.popout = Some(popout);
                self.rebuild_popout_viewer();
                cx.spawn(async move |this, cx| {
                    if closed.recv().await.is_ok() {
                        let _ = this.update(cx, |v, cx| {
                            if let Some(p) = v.popout.take() {
                                p.shutdown();
                            }
                            v.apply_viewer_options();
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
            Err(e) => self.message = format!("Popout viewer failed: {e}"),
        }
        self.apply_viewer_options();
        cx.notify();
    }
    /// A new document (or a first popout) needs a popout viewer for it.
    pub fn rebuild_popout_viewer(&mut self) {
        let (Some(popout), Some(p)) = (&mut self.popout, &self.published) else {
            return;
        };
        let comp = p.snapshot.render_comp();
        let hz = std::env::var("PM_NATIVE_VERIFY_HZ")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120.);
        match self.engine.viewer(pm_engine_service::ViewerConfig {
            snapshot: p.snapshot.clone(),
            composition: comp,
            media: self.media.clone(),
            output: popout.initial_output(),
            layer: Some(popout.layer.clone()),
            rate: self.rate,
            end: Some((self.duration * pm_protocol::common::TICKS_PER_SECOND as f64) as i64),
            audio: false,
        }) {
            Ok(viewer) => {
                if let Err(e) = viewer.resources(self.extensions.resources.clone()) {
                    self.message = e.to_string();
                }
                popout.bind(viewer, hz)
            }
            Err(e) => self.message = format!("Popout viewer failed: {e}"),
        }
        self.apply_viewer_options();
    }
    /// Mirrors the editor transport: play/pause state and the paused frame.
    pub fn sync_popout(&mut self, clock: &pm_protocol::clock::ClockSample) {
        use pm_protocol::clock::{SeekMode, TransportCommand, TransportState};
        let Some(popout) = &mut self.popout else {
            return;
        };
        let Some(viewer) = popout.viewer.borrow().clone() else {
            return;
        };
        let playing = clock.state != TransportState::Stopped && clock.rate.num != 0;
        let time = clock.media_time.get();
        let want = (playing, if playing { 0 } else { time });
        if popout.mirrored == Some(want) {
            return;
        }
        let result = if playing {
            viewer
                .command(TransportCommand::Seek {
                    time: clock.media_time,
                    mode: SeekMode::Exact,
                })
                .and_then(|_| viewer.command(TransportCommand::Play { rate: clock.rate }))
        } else {
            viewer.scrub(clock.media_time)
        };
        if result.is_ok() {
            popout.mirrored = Some(want);
        }
        popout.set_link_paused(!playing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_modes_parse_and_map_to_valid_engine_mastering() {
        for mode in OutputMode::ALL {
            assert_eq!(OutputMode::parse(mode.name()), Some(mode));
            mode.mastering().params().unwrap();
        }
        assert_eq!(OutputMode::parse("HDR10"), Some(OutputMode::Pq));
        assert_eq!(OutputMode::parse("rec709"), None);
        assert_eq!(OutputMode::Sdr.mastering(), Mastering::LinearP3);
        assert_eq!(OutputMode::Pq.mastering().params().unwrap()[0], 1.);
        assert_eq!(OutputMode::Hlg.mastering().params().unwrap()[0], 2.);
    }
    #[test]
    fn overlays_are_normalized_closed_rectangles_and_fit_the_engine_limit() {
        assert!(overlay_lines(false, false).is_empty());
        let safe = overlay_lines(true, false);
        assert_eq!(safe.len(), 8);
        // Action safe inset 3.5 %, title safe 5 % on every edge.
        assert!((safe[0].ends[0] - 0.035).abs() < 1e-6);
        assert!((safe[4].ends[0] - 0.05).abs() < 1e-6);
        for rect in safe.chunks(4) {
            for i in 0..4 {
                let (end, next) = (&rect[i].ends[2..], &rect[(i + 1) % 4].ends[..2]);
                assert_eq!(end, next, "edges join into a closed rectangle");
            }
        }
        let all = overlay_lines(true, true);
        assert_eq!(all.len(), 8 + 4 + 2);
        assert!(all.len() <= OverlayGeometry::MAX_LINES);
        assert!(
            all.iter()
                .all(|l| l.ends.iter().all(|v| (0. ..=1.).contains(v)))
        );
        OverlayGeometry::new(all).unwrap();
    }
    #[test]
    fn menu_tags_round_trip() {
        for action in Action::ALL {
            assert_eq!(Action::from_tag(action.tag()), Some(action));
        }
        assert_eq!(Action::from_tag(-1), None);
        assert_eq!(Action::from_tag(99), None);
    }
}
