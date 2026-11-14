//! Studio 3D views (3d-studio.md §2): 1, 2 or 4 panes in the viewer. The
//! first pane is the editor's own viewer (its Look Through, tools and
//! Preview). Each other pane is an embedded viewer with its own camera and
//! Fast or Quality, chosen from a pull-down in its corner; they follow the
//! document and transport but never edit the shot or Undo.
use crate::session::Controller;
use crate::viewer2::Popout;
use gpui::Context;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    sel,
};
use objc2_app_kit::{NSAccessibility, NSFont, NSMenuItem, NSPopUpButton};
use objc2_foundation::{NSObject, NSObjectProtocol, NSRect, NSString};
use pm_engine_service::studio::{Axis, Pane as EnginePane, Preview, ViewCamera};
use pm_scene3d::navigation::Vec3;

/// What a pane looks through: the shot, or a flat view along an axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Shot,
    Axis(Axis),
}
const LOOKS: [(Look, &str); 7] = [
    (Look::Shot, "Shot Camera"),
    (Look::Axis(Axis::Top), "Top"),
    (Look::Axis(Axis::Front), "Front"),
    (Look::Axis(Axis::Right), "Right"),
    (Look::Axis(Axis::Left), "Left"),
    (Look::Axis(Axis::Back), "Back"),
    (Look::Axis(Axis::Bottom), "Bottom"),
];
/// Default looks for the extra panes: two views add Top; four add Top,
/// Front and Right (the usual product-shot set).
fn default_look(index: usize, count: usize) -> Look {
    match (count, index) {
        (2, _) => Look::Axis(Axis::Top),
        (_, 0) => Look::Axis(Axis::Top),
        (_, 1) => Look::Axis(Axis::Front),
        _ => Look::Axis(Axis::Right),
    }
}

pub struct Pane {
    popout: Popout,
    look: Look,
    tier: Preview,
    /// A flat view's own framing: pan in view units (right, up) and zoom.
    pan: [f64; 2],
    zoom: f64,
    /// The press in progress: where it began, the last point, and whether
    /// it moved (a click selects; a drag pans).
    press: Option<([f64; 2], [f64; 2], bool)>,
    /// What the engine viewer was last given, and for which document.
    applied: Option<(ViewCamera, Preview)>,
    document: Option<String>,
}

struct TargetIvars {
    pane: usize,
    sender: async_channel::Sender<(usize, isize)>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMStudioPaneMenuTarget"]
    #[ivars = TargetIvars]
    struct PaneTarget;
    unsafe impl NSObjectProtocol for PaneTarget {}
    impl PaneTarget {
        #[unsafe(method(choose:))]
        fn choose(&self, sender: &NSPopUpButton) {
            let index = sender.indexOfSelectedItem();
            let _ = self.ivars().sender.try_send((self.ivars().pane, index));
        }
    }
);

/// The pull-down: a title row naming the look, the looks, then the previews.
fn menu(pane: usize, sender: async_channel::Sender<(usize, isize)>) -> (Retained<NSPopUpButton>, Retained<PaneTarget>) {
    let mtm = MainThreadMarker::new().unwrap();
    let button = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, true);
    button.addItemWithTitle(&NSString::from_str(""));
    for (_, label) in LOOKS {
        button.addItemWithTitle(&NSString::from_str(label));
    }
    button.menu().map(|m| m.addItem(&NSMenuItem::separatorItem(mtm)));
    for label in ["Fast", "Quality"] {
        button.addItemWithTitle(&NSString::from_str(label));
    }
    let target: Retained<PaneTarget> = unsafe {
        msg_send![super(PaneTarget::alloc(mtm).set_ivars(TargetIvars { pane, sender })), init]
    };
    unsafe {
        button.setTarget(Some(&target));
        button.setAction(Some(sel!(choose:)));
    }
    button.setFont(Some(&NSFont::systemFontOfSize(11.)));
    unsafe {
        let _: () = msg_send![&*button, setControlSize: 1isize]; // small
    }
    button.setAccessibilityLabel(Some(&NSString::from_str("View")));
    (button, target)
}
fn title(button: &NSPopUpButton, look: Look, tier: Preview) {
    let name = LOOKS.iter().find(|(l, _)| *l == look).map_or("", |(_, n)| n);
    let speed = if tier == Preview::Live { "Fast" } else { "Quality" };
    if let Some(item) = button.itemAtIndex(0) {
        item.setTitle(&NSString::from_str(&format!("{name} · {speed}")));
    }
    for (i, (l, _)) in LOOKS.iter().enumerate() {
        if let Some(item) = button.itemAtIndex(i as isize + 1) {
            item.setState(if *l == look { 1 } else { 0 });
        }
    }
    let n = button.numberOfItems();
    for (k, t) in [(n - 2, Preview::Live), (n - 1, Preview::Refine)] {
        if let Some(item) = button.itemAtIndex(k) {
            item.setState(if (t == Preview::Live) == (tier == Preview::Live) { 1 } else { 0 });
        }
    }
    button.sizeToFit();
}

/// Where a flat view looks and how much it shows: the scene's 3D layers,
/// with a margin, along the axis.
fn ortho(axis: Axis, scene: Option<&pm_panel_studio3d::viewport::Scene>) -> ViewCamera {
    let mut lo: Vec3 = [f64::MAX; 3];
    let mut hi: Vec3 = [f64::MIN; 3];
    for item in scene.into_iter().flat_map(|s| s.items.iter()) {
        // Skies, terrains and lights don't set the frame.
        if item.role != pm_scene3d::schema::Role::Object {
            continue;
        }
        for a in 0..3 {
            lo[a] = lo[a].min(item.bounds.min[a]);
            hi[a] = hi[a].max(item.bounds.max[a]);
        }
    }
    if lo[0] > hi[0] {
        (lo, hi) = ([-1.; 3], [1.; 3]);
    }
    let center = std::array::from_fn(|a| (lo[a] + hi[a]) / 2.);
    let size = |a: usize| (hi[a] - lo[a]).abs();
    // Vertical extent on screen: Top and Bottom look down Y, so Z is up.
    let (up, across) = match axis {
        Axis::Top | Axis::Bottom => (size(2), size(0)),
        Axis::Front | Axis::Back => (size(1), size(0)),
        Axis::Left | Axis::Right => (size(1), size(2)),
    };
    // A margin around what's there, and never closer than a few units: text
    // and shapes the bounds don't list still read in the frame.
    let span = (up.max(across * 9. / 16.) * 1.6).clamp(4., 1e5);
    ViewCamera::Orthographic { axis, center, span }
}

impl Controller {
    /// Per display tick: panes follow `studioViews`, the shell's tiles, the
    /// document and their own choices.
    pub(crate) fn sync_studio_panes(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.published.clone() else {
            self.close_studio_panes();
            return;
        };
        // The scene's 3D layers, read once per revision (this runs per tick).
        let revision = p.revision();
        if self.studio_pane_scene.as_ref().map(|(r, _)| *r) != Some(revision) {
            let scene = pm_panel_studio3d::viewport::scene(&p, 0.)
                .ok()
                .filter(|s| !s.items.is_empty())
                .map(std::sync::Arc::new);
            self.studio_pane_scene = Some((revision, scene));
        }
        let studio = self.studio_pane_scene.as_ref().and_then(|(_, s)| s.clone());
        // View ▸ 3D checkmarks follow the same settings as the bar.
        {
            let ui = |k: &str, d: &str| {
                p.ui_state
                    .viewer
                    .get(k)
                    .and_then(pm_model::Value::as_str)
                    .unwrap_or(d)
                    .to_owned()
            };
            let current = [
                ("look", ui("studioLook", "camera")),
                ("tool", ui("studioTool", "move")),
                ("axes", ui("studioAxes", "scene")),
                ("views", ui("studioViews", "1")),
                ("tier", ui("studioTier", "live")),
            ];
            pm_shell::check_view3d(&|group, value| {
                current.iter().any(|(g, v)| *g == group && v == value)
            });
        }
        let count = match p.ui_state.viewer.get("studioViews").and_then(pm_model::Value::as_str) {
            Some("2") if studio.is_some() => 2usize,
            Some("4") if studio.is_some() => 4,
            _ => 1,
        };
        {
            let host = self.host.borrow();
            if host.viewer_split.get() as usize != count {
                host.viewer_split.set(count as u8);
                drop(host);
                if let Some(shell) = self.shell.as_ref().and_then(|s| s.upgrade()) {
                    shell.update(cx, |_, cx| cx.notify());
                }
            }
        }
        if count == 1 {
            self.close_studio_panes();
            return;
        }
        // Grow or shrink to count - 1 extra panes.
        while self.studio_panes.len() > count - 1 {
            if let Some(pane) = self.studio_panes.pop() {
                pane.popout.shutdown();
            }
        }
        while self.studio_panes.len() < count - 1 {
            let index = self.studio_panes.len();
            let window = self.host.borrow().window.clone();
            let Ok(mut popout) = Popout::embed(&self.engine.metal_device(), &window) else {
                break;
            };
            let sender = self.studio_pane_events(cx);
            let (button, target) = menu(index, sender);
            popout.view().addSubview(&button);
            popout.menu = Some(button);
            self.studio_pane_targets.push(Retained::into_super(target));
            let (input, received) = async_channel::unbounded();
            popout.listen(input);
            cx.spawn(async move |this, cx| {
                while let Ok(event) = received.recv().await {
                    if this
                        .update(cx, |v, cx| v.studio_pane_input(index, event, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
            self.studio_panes.push(Pane {
                popout,
                look: default_look(index, count),
                tier: Preview::Live,
                pan: [0.; 2],
                zoom: 1.,
                press: None,
                applied: None,
                document: None,
            });
        }
        let tiles = self.host.borrow().viewer_tiles.borrow().clone();
        let document = format!("{}:{:?}", p.snapshot.doc_id, p.snapshot.render_comp());
        let scene = studio.as_deref();
        let dims = self.dimensions;
        let inset = self.host.borrow().tokens.metrics.inset;
        for (i, pane) in self.studio_panes.iter_mut().enumerate() {
            if let Some(tile) = tiles.get(i + 1) {
                // The tile in the viewer's ground; the picture inset and
                // fitted in it, as the main viewer frames its composition.
                let t = pm_shell::layout::Rect {
                    x: tile.x + inset,
                    y: tile.y + inset,
                    w: (tile.w - inset * 2.).max(1.),
                    h: (tile.h - inset * 2.).max(1.),
                };
                let (tw, th) = (t.w as f64, t.h as f64);
                let aspect = (dims[0] / dims[1]).max(1e-3);
                let (w, h) = if tw / th > aspect {
                    (th * aspect, th)
                } else {
                    (tw, tw / aspect)
                };
                pane.popout.place(
                    [
                        t.x as f64 + (tw - w) / 2.,
                        t.y as f64 + (th - h) / 2.,
                        w,
                        h,
                    ],
                    [tile.x as f64, tile.y as f64, tile.w as f64, tile.h as f64],
                );
            } else {
                // No tile (Home is over the editor): nothing of it shows.
                pane.popout.hide();
            }
            if pane.document.as_deref() != Some(document.as_str()) {
                // A new document or composition: a new viewer for it.
                let viewer = self.engine.viewer(pm_engine_service::ViewerConfig {
                    snapshot: if pane.look == Look::Shot {
                        p.snapshot.clone()
                    } else {
                        flat_view_snapshot(&p)
                    },
                    composition: p.snapshot.render_comp(),
                    media: self.media.clone(),
                    output: pane.popout.initial_output(),
                    layer: Some(pane.popout.layer.clone()),
                    rate: self.rate,
                    end: Some((crate::session::composition_duration(&p) * pm_protocol::common::TICKS_PER_SECOND as f64) as i64),
                    audio: false,
                });
                match viewer {
                    Ok(viewer) => {
                        let _ = viewer.resources(self.extensions.resources.clone());
                        pane.popout.bind(viewer, 120.);
                        pane.document = Some(document.clone());
                        pane.applied = None;
                    }
                    Err(e) => {
                        self.message = format!("3D view: {e}");
                        continue;
                    }
                }
            }
            let camera = match pane.look {
                Look::Shot => ViewCamera::Active,
                Look::Axis(axis) => framed(ortho(axis, scene), axis, pane.pan, pane.zoom),
            };
            if pane.applied.as_ref() != Some(&(camera.clone(), pane.tier)) {
                if let Some(viewer) = &*pane.popout.viewer.borrow() {
                    let _ = viewer.studio_pane(EnginePane {
                        camera: camera.clone(),
                        tier: pane.tier,
                    });
                }
                pane.applied = Some((camera, pane.tier));
            }
            if let Some(button) = &pane.popout.menu {
                title(button, pane.look, pane.tier);
            }
        }
    }
    /// The panes see each new publication (without the main viewer's tier).
    pub(crate) fn publish_studio_panes(&self, p: &pm_kernel::Published) {
        let mut flat = None;
        for pane in &self.studio_panes {
            if let Some(viewer) = &*pane.popout.viewer.borrow() {
                let snapshot = if pane.look == Look::Shot {
                    p.snapshot.clone()
                } else {
                    flat.get_or_insert_with(|| flat_view_snapshot(p)).clone()
                };
                let _ = viewer.snapshot(snapshot);
            }
        }
    }
    /// The panes mirror the editor's transport, as the popout does.
    pub(crate) fn sync_studio_pane_clock(&mut self, clock: &pm_protocol::clock::ClockSample) {
        use pm_protocol::clock::{SeekMode, TransportCommand, TransportState};
        let playing = clock.state != TransportState::Stopped && clock.rate.num != 0;
        let time = clock.media_time.get();
        let want = (playing, if playing { 0 } else { time });
        for pane in &mut self.studio_panes {
            if pane.popout.mirrored == Some(want) {
                continue;
            }
            let Some(viewer) = pane.popout.viewer.borrow().clone() else {
                continue;
            };
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
                pane.popout.mirrored = Some(want);
            }
            pane.popout.set_link_paused(!playing);
        }
    }
    /// A pane's pointer: drag pans a flat view, scroll zooms it, and a click
    /// selects what's under it (the engine's pick in that pane).
    fn studio_pane_input(
        &mut self,
        index: usize,
        event: crate::viewer2::PaneInput,
        cx: &mut Context<Self>,
    ) {
        use crate::viewer2::PaneInput;
        let aspect = (self.dimensions[0] / self.dimensions[1]).max(1e-3);
        let span = |pane: &Pane, scene: Option<&pm_panel_studio3d::viewport::Scene>| match pane.look {
            Look::Axis(axis) => match ortho(axis, scene) {
                ViewCamera::Orthographic { span, .. } => span * pane.zoom,
                _ => 1.,
            },
            Look::Shot => 1.,
        };
        let scene = self.studio_pane_scene.as_ref().and_then(|(_, s)| s.clone());
        let Some(pane) = self.studio_panes.get_mut(index) else {
            return;
        };
        match event {
            PaneInput::Down(uv) => pane.press = Some((uv, uv, false)),
            PaneInput::Drag(uv) => {
                let flat = pane.look != Look::Shot;
                let s = span(pane, scene.as_deref());
                if let Some((_, last, moved)) = pane.press.as_mut() {
                    let (du, dv) = (uv[0] - last[0], uv[1] - last[1]);
                    if flat {
                        // The picture follows the pointer.
                        pane.pan[0] -= du * s * aspect;
                        pane.pan[1] += dv * s;
                    }
                    *last = uv;
                    *moved = true;
                }
            }
            PaneInput::Scroll(dy) => {
                if pane.look != Look::Shot {
                    pane.zoom = (pane.zoom * (1. - dy * 0.01).clamp(0.5, 1.5)).clamp(0.02, 50.);
                }
            }
            PaneInput::Up(uv) => {
                let click = pane.press.take().is_some_and(|(_, _, moved)| !moved);
                let viewer = pane.popout.viewer.borrow().clone();
                if click
                    && let Some(viewer) = viewer
                    && let Some(frame) = viewer.status().frame
                    && let Ok(rx) = viewer.pick_surface(
                        frame.stamp,
                        frame.time,
                        uv.map(|v| v as f32),
                        pm_engine_service::SurfaceQueryKind::Click,
                    )
                {
                    cx.spawn(async move |this, cx| {
                        if let Ok(Ok(reply)) = rx.recv().await {
                            let ids = reply.hit.map(|h| vec![h.layer]).unwrap_or_default();
                            let _ = this.update(cx, |v, cx| {
                                v.call(
                                    pm_kernel::SelectionCall::Layers { ids, add: false }.into(),
                                    cx,
                                )
                            });
                        }
                    })
                    .detach();
                }
            }
        }
        self.sync_studio_panes(cx);
    }
    pub(crate) fn close_studio_panes(&mut self) {
        for pane in self.studio_panes.drain(..) {
            pane.popout.shutdown();
        }
        self.studio_pane_targets.clear();
        let host = self.host.borrow();
        if host.viewer_split.get() != 1 {
            host.viewer_split.set(1);
        }
    }
    /// One channel per window for the panes' pull-downs.
    fn studio_pane_events(&mut self, cx: &mut Context<Self>) -> async_channel::Sender<(usize, isize)> {
        if let Some(tx) = &self.studio_pane_sender {
            return tx.clone();
        }
        let (tx, rx) = async_channel::unbounded::<(usize, isize)>();
        self.studio_pane_sender = Some(tx.clone());
        cx.spawn(async move |this, cx| {
            while let Ok((pane, index)) = rx.recv().await {
                let _ = this.update(cx, |v, cx| {
                    let Some(p) = v.studio_panes.get_mut(pane) else {
                        return;
                    };
                    let looks = LOOKS.len() as isize;
                    match index {
                        i if (1..=looks).contains(&i) => {
                            let look = LOOKS[(i - 1) as usize].0;
                            // The shot draws 2D layers and flat views don't:
                            // switching between them needs the other document.
                            if (look == Look::Shot) != (p.look == Look::Shot) {
                                p.document = None;
                            }
                            p.look = look;
                        }
                        // After the looks and the separator: Fast, Quality.
                        i if i == looks + 2 => p.tier = Preview::Live,
                        i if i == looks + 3 => p.tier = Preview::Refine,
                        _ => {}
                    }
                    v.sync_studio_panes(cx);
                });
            }
        })
        .detach();
        tx
    }
}

/// What a flat (Top, Front, ...) view draws: the composition with its 2D
/// layers off, as After Effects shows 2D layers only through the shot. A
/// layer or group a 3D layer wears as a material stays, so the material
/// still shows; groups stay so 3D layers inside them do.
pub(crate) fn flat_view_snapshot(p: &pm_kernel::Published) -> pm_kernel::document::Snapshot {
    use pm_model::{PathPart, Patch};
    let snapshot = p.snapshot.clone();
    let Some(root) = snapshot.root() else {
        return snapshot;
    };
    let comp = snapshot.render_comp();
    let main = comp.is_empty()
        || root
            .get("compId")
            .map(|n| n.to_value())
            .is_none_or(|v| v.as_str().is_none_or(|id| id == comp));
    let path: Vec<PathPart> = if main {
        vec![PathPart::Key("layers".into())]
    } else {
        vec![
            PathPart::Key("comps".into()),
            PathPart::Key(comp.as_str().into()),
            PathPart::Key("layers".into()),
        ]
    };
    let Some(layers) = root.lookup(&path).map(|n| n.to_value()) else {
        return snapshot;
    };
    let Ok(layers) = pm_scene3d::from_model(&layers) else {
        return snapshot;
    };
    let Some(layers) = layers.as_array() else {
        return snapshot;
    };
    // A layer's group (or parent) chain, to find what sits inside what.
    let up = |id: &str| {
        layers
            .iter()
            .find(|l| l["id"] == id)
            .and_then(|l| l["group"].as_str())
            .map(str::to_owned)
    };
    let is_3d = |l: &serde_json::Value| {
        pm_scene3d::schema::role(l).is_some() || pm_scene3d::extrusion::enabled(l)
    };
    // Groups holding 3D layers stay on, so those layers do.
    let mut holds_3d: std::collections::HashSet<String> = Default::default();
    for l in layers.iter().filter(|l| is_3d(l)) {
        let mut at = l["group"].as_str().map(str::to_owned);
        let mut guard = 0;
        while let Some(g) = at {
            if !holds_3d.insert(g.clone()) || guard > 64 {
                break;
            }
            at = up(&g);
            guard += 1;
        }
    }
    let hidden = |l: &serde_json::Value| {
        !is_3d(l)
            && l["on"] != false
            && !(l["type"] == "group" && holds_3d.contains(l["id"].as_str().unwrap_or("")))
    };
    // Turn off the outermost 2D layer or group only: a group's children keep
    // their own switches, so a group worn as a material still captures (a
    // capture draws its hidden root).
    let mut patches = vec![];
    for (i, l) in layers.iter().enumerate() {
        if !hidden(l) {
            continue;
        }
        let mut inside_hidden = false;
        let mut at = l["group"].as_str().map(str::to_owned);
        let mut guard = 0;
        while let Some(g) = at {
            if layers.iter().any(|x| x["id"] == g.as_str() && hidden(x)) || guard > 64 {
                inside_hidden = true;
                break;
            }
            at = up(&g);
            guard += 1;
        }
        if !inside_hidden {
            let mut at = path.clone();
            at.push(PathPart::Index(i as f64));
            at.push(PathPart::Key("on".into()));
            patches.push(Patch::set(at, pm_model::Value::Bool(false)));
        }
    }
    if patches.is_empty() {
        return snapshot;
    }
    let mut document = (*snapshot.document).clone();
    if document.apply(&patches).is_err() {
        return snapshot;
    }
    pm_kernel::document::Snapshot {
        document: std::sync::Arc::new(document),
        ..snapshot
    }
}

/// A flat view panned and zoomed by its pane: `pan` is along the view's
/// right and up, in scene units.
fn framed(camera: ViewCamera, axis: Axis, pan: [f64; 2], zoom: f64) -> ViewCamera {
    let ViewCamera::Orthographic { center, span, .. } = camera else {
        return camera;
    };
    // Screen right and up for each flat view (Front looks down -Z).
    let (right, up): (Vec3, Vec3) = match axis {
        Axis::Front => ([1., 0., 0.], [0., 1., 0.]),
        Axis::Back => ([-1., 0., 0.], [0., 1., 0.]),
        Axis::Right => ([0., 0., -1.], [0., 1., 0.]),
        Axis::Left => ([0., 0., 1.], [0., 1., 0.]),
        Axis::Top => ([1., 0., 0.], [0., 0., -1.]),
        Axis::Bottom => ([1., 0., 0.], [0., 0., 1.]),
    };
    let center = std::array::from_fn(|a| center[a] + right[a] * pan[0] + up[a] * pan[1]);
    ViewCamera::Orthographic {
        axis,
        center,
        span: (span * zoom).clamp(1e-3, 1e5),
    }
}
