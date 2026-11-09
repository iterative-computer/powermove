//! Window-owned v4 runtime lifecycle and shell integration. The main thread
//! only publishes immutable trees and enqueues input; JavaScript runs on owners.
use crate::{session::Controller, v4_view::SurfaceView};
use gpui::{prelude::*, *};
use pm_builtins::{BUILTIN_PANELS, Builtins, PlatformRequest, Publication};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    rc::Rc,
    sync::Arc,
};
type MediaProbeInputs = (
    crate::media_refresh::Inputs,
    BTreeMap<String, std::path::PathBuf>,
    BTreeMap<String, std::path::PathBuf>,
);
#[derive(Default)]
pub struct Host {
    project: Option<String>,
    document: Option<u64>,
    application: bool,
    pub runtime: Option<Arc<Builtins>>,
    pending: BTreeMap<String, Publication>,
    pub views: BTreeMap<String, Entity<SurfaceView>>,
    pub natives: crate::v4_view::Natives,
    pub timeline: Option<Entity<crate::timeline_surface::TimelineSurface>>,
    /// Where the shell last placed the timeline panel, and its header height;
    /// `None` while it isn't drawn. The native playhead and timecode follow it.
    pub timeline_place: Option<(pm_shell::layout::Rect, f32)>,
    frame_outline: crate::playback::FrameOutline,
    pub gizmos: Option<Entity<crate::gizmo::GizmoSurface>>,
    /// The window's open panels, and closed built-ins whose isolates are going.
    open: BTreeSet<String>,
    parking: BTreeSet<String>,
    /// The clip painters' revision the timeline last asked for.
    painters: u64,
    /// v4 extensions' viewer overlays (`overlays.viewer`).
    pub overlays: Option<Entity<crate::viewer_overlays::ViewerOverlays>>,
    clock: crate::playback::PanelClock,
    headers: BTreeMap<String, Entity<crate::v4_view::HeaderView>>,
    pub(crate) ready: BTreeSet<String>,
    generations: BTreeMap<String, u64>,
    requests: VecDeque<PlatformRequest>,
    windows: BTreeMap<String, AnyWindowHandle>,
    opening_windows: BTreeSet<String>,
    pub export_job: Option<crate::export_coordinator::Job>,
    pub export_retry: Option<crate::export_coordinator::Frozen>,
    pub export_tasks: Arc<std::sync::Mutex<pm_promedia::tasks::BackgroundTasks>>,
    pub media_runtime: pm_panel_media::readiness::ObservedRuntime,
    media_probes: Arc<pm_builtins::delivery::MediaProbes>,
    /// The media inputs behind `media_runtime`, and the worker observing them.
    media_probed: Option<MediaProbeInputs>,
    media_probe_serial: u64,
    updating: bool,
    again: bool,
    /// The status line asked for the missing files; the next update hands it
    /// to the panels' workers (never their locks on the main thread).
    pub(crate) show_missing: bool,
    /// The application Agent panel's runs while a project is open.
    agent_carry: Option<pm_builtins::AgentCarry>,
}
fn media_stamp(published: &pm_kernel::Published) -> pm_panel_media::Stamp {
    pm_panel_media::Stamp {
        project: published.project.clone(),
        document: published.snapshot.doc_id,
        revision: published.revision(),
    }
}
impl Controller {
    pub fn start_builtins(&mut self, cx: &mut Context<Self>) {
        let application = self.session.is_none();
        let project = self
            .session
            .as_ref()
            .map(|s| s.project.clone())
            .unwrap_or_else(|| "builtin-application".into());
        let document = self.published.as_ref().map(|p| p.snapshot.doc_id);
        if self.builtins.project.as_ref() == Some(&project)
            && self.builtins.document == document
            && self.builtins.application == application
        {
            return;
        }
        if let Some(job) = &self.builtins.export_job {
            job.cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        let mut carry = self.builtins.agent_carry.take();
        if let Some(runtime) = self.builtins.runtime.take() {
            carry = carry.or_else(|| runtime.carry_agent());
            runtime.close();
        }
        let windows = std::mem::take(&mut self.builtins.windows);
        self.builtins = Host::default();
        self.builtins.windows = windows;
        self.builtins.agent_carry = carry;
        self.builtins.project = Some(project.clone());
        self.builtins.document = document;
        self.builtins.application = application;
        if !application {
            let delegate = Rc::new(ControllerDelegate(cx.entity().downgrade()));
            let tokens = self.host.borrow().tokens.clone();
            let (kernel, outline) = (self.kernel.clone(), self.builtins.natives.outline.clone());
            let offline = self.offline_media();
            let surface = cx.new(|cx| {
                crate::timeline_surface::TimelineSurface::new(
                    kernel,
                    project.clone(),
                    tokens,
                    delegate,
                    outline,
                    offline,
                    cx,
                )
            });
            self.builtins
                .natives
                .views
                .borrow_mut()
                .insert("timeline".into(), surface.clone().into());
            self.builtins.timeline = Some(surface);
            let host = Rc::new(ControllerDelegate(cx.entity().downgrade()));
            let (kernel, tokens) = (self.kernel.clone(), self.host.borrow().tokens.clone());
            let gizmos =
                cx.new(|_| crate::gizmo::GizmoSurface::new(kernel, project.clone(), tokens, host));
            self.builtins
                .natives
                .views
                .borrow_mut()
                .insert("viewer.gizmos".into(), gizmos.clone().into());
            self.builtins.gizmos = Some(gizmos);
            let host: Rc<dyn crate::gizmo::Host> =
                Rc::new(ControllerDelegate(cx.entity().downgrade()));
            let tokens = self.host.borrow().tokens.clone();
            let overlays = cx.new(|_| crate::viewer_overlays::ViewerOverlays::new(host, tokens));
            self.builtins
                .natives
                .views
                .borrow_mut()
                .insert("viewer.overlays".into(), overlays.clone().into());
            self.builtins.overlays = Some(overlays);
        }
        self.start_extensions(cx);
        let (tx, rx) = async_channel::bounded(1);
        let wake = move || {
            let _ = tx.try_send(());
        };
        let created = if application {
            Builtins::application(self.kernel.clone(), wake, self.builtins.agent_carry.take())
        } else {
            Builtins::new(self.kernel.clone(), &project, wake)
        };
        let mut runtime = match created {
            Ok(runtime) => runtime,
            Err(e) => {
                self.message = format!("Panels could not start: {e}");
                self.settled(&project);
                return;
            }
        };
        runtime.set_agent_environment(
            pm_agents::launch::Environment::current(
                self.mcp.profile.clone(),
                pm_mcp::catalog::names(),
                pm_mcp::catalog::composition_names(),
            ),
            Arc::new(pm_agents::account::open_in_browser),
        );
        let mut tickets = vec![];
        for panel in BUILTIN_PANELS.iter() {
            if application && !panel.application {
                continue;
            }
            tickets.push((panel.id.clone(), runtime.mount(&panel.id)));
        }
        let runtime = Arc::new(runtime);
        self.builtins.runtime = Some(runtime.clone());
        let check = runtime.clone();
        let mounting = project.clone();
        cx.spawn(async move |this, cx| {
            for (id, ticket) in tickets {
                let result = match ticket {
                    Ok(ticket) => ticket.await.map(|evaluation| evaluation.generation.number),
                    Err(error) => Err(error),
                };
                let _ = this.update(cx, |v, cx| {
                    if !v
                        .builtins
                        .runtime
                        .as_ref()
                        .is_some_and(|r| Arc::ptr_eq(r, &check))
                    {
                        return;
                    }
                    match result {
                        Ok(generation) => {
                            v.builtins.generations.insert(id.clone(), generation);
                            v.builtins.ready.insert(id);
                        }
                        Err(error) => {
                            check.close_panel(&id);
                            v.message = format!("Panel {id} could not start: {error}");
                            eprintln!("{}", v.message);
                        }
                    }
                    v.drain_builtins(cx);
                });
            }
            let _ = this.update(cx, |v, cx| {
                v.settled(&mounting);
                cx.notify();
            });
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while rx.recv().await.is_ok() {
                if this
                    .update(cx, |v, cx| {
                        if v.builtins
                            .runtime
                            .as_ref()
                            .is_some_and(|r| Arc::ptr_eq(r, &runtime))
                        {
                            v.drain_builtins(cx);
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
    /// Every panel of `project` has mounted or failed: the open is over.
    fn settled(&mut self, project: &str) {
        if self.settling.as_deref() == Some(project) {
            self.settling = None;
            self.opening_name = None;
            crate::diag::line(serde_json::json!({"open":"settled"}));
            self.busy = None;
        }
    }
    /// A surface extension's first commit landed; its panel can render and its
    /// platform requests are accepted.
    pub fn surface_ready(&mut self, id: String, generation: u64) {
        self.builtins.generations.insert(id.clone(), generation);
        self.builtins.ready.insert(id);
    }
}
impl Host {
    /// An unmounted surface: its view and anything it published go with it, so a
    /// rebuilt graph starts from its own first commit.
    /// A panel whose realm is being replaced (a reload or a fork swap) keeps
    /// its view: the old tree stays on screen until the new generation's first
    /// publication installs there. A view built mid-swap missed GPUI's input
    /// until the pane next re-rendered.
    pub fn keep_surface(&mut self, id: &str) {
        self.pending.remove(id);
        self.generations.remove(id);
    }
    pub fn forget_surface(&mut self, id: &str) {
        self.ready.remove(id);
        self.pending.remove(id);
        self.views.remove(id);
        self.headers.remove(id);
        self.generations.remove(id);
    }
}
impl Controller {
    /// The observed media runtime with each video's proxy and optimized
    /// media and Always Play Original, as the pro media service has them.
    fn rendition_runtime(&self) -> pm_panel_media::readiness::ObservedRuntime {
        use crate::pro_media::Status;
        use pm_panel_media::ProxyStatus;
        let Some(project) = self.session.as_ref().map(|s| s.project.clone()) else {
            return self.builtins.media_runtime.clone();
        };
        let status = |s: Status, observed: &ProxyStatus| match s {
            Status::Absent => observed.clone(),
            Status::Queued => ProxyStatus::Queued,
            Status::Creating { percent } => ProxyStatus::Generating { percent },
            Status::Ready => ProxyStatus::Ready {
                version: "generated".into(),
            },
            Status::Failed => ProxyStatus::Failed("The task couldn’t be completed.".into()),
        };
        self.builtins
            .media_runtime
            .clone()
            .with_states(|id, state| {
                let (proxy, optimized) = self.pro_media.status(
                    &project,
                    id,
                    self.media_base
                        .originals
                        .get(id)
                        .map(std::path::PathBuf::as_path),
                );
                state.proxy = status(proxy, &state.proxy);
                state.optimized = status(optimized, &state.optimized);
                state.play_original = self.pro_media.plays_original(&project, id);
            })
    }
    /// Readiness depends only on the media inputs, so an edit that left them
    /// alone moves the observation to its revision without any worker.
    fn probe_builtin_media(&mut self, cx: &mut Context<Self>) {
        let Some(published) = self.published.clone() else {
            return;
        };
        let inputs = crate::media_refresh::Inputs::published(&published);
        let originals = self.media.originals.clone();
        let proxies = self.media.proxies.clone();
        let stamp = media_stamp(&published);
        if self
            .builtins
            .media_probed
            .as_ref()
            .is_some_and(|(i, o, p)| i.same(&inputs) && *o == originals && *p == proxies)
        {
            self.builtins.media_runtime = self.builtins.media_runtime.clone().rebound(stamp);
            return;
        }
        self.builtins.media_probed = Some((inputs, originals.clone(), proxies.clone()));
        self.builtins.media_probe_serial += 1;
        let serial = self.builtins.media_probe_serial;
        let probes = self.builtins.media_probes.clone();
        cx.spawn(async move |this, cx| {
            let observed = cx
                .background_executor()
                .spawn(async move {
                    pm_builtins::delivery::observe_media(
                        &published.project,
                        &published.snapshot,
                        &originals,
                        &proxies,
                        &probes,
                    )
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                if v.builtins.media_probe_serial != serial {
                    return;
                }
                match observed {
                    Ok(observed) => {
                        v.builtins.media_runtime = match &v.published {
                            Some(p) => observed.rebound(media_stamp(p)),
                            None => observed,
                        };
                        v.update_builtins(cx);
                    }
                    Err(error) => {
                        v.message = error;
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    fn facts(&self) -> Value {
        let status = self.viewer.as_ref().map(|v| v.status());
        let playing = status.as_ref().is_some_and(|s| {
            s.clock
                .is_some_and(|c| c.state == pm_protocol::clock::TransportState::Playing)
        });
        // Auto's measured resolution; 0 until a frame has drawn.
        let effective = status
            .map(|s| s.effective_quality)
            .filter(|q| *q > 0.)
            .unwrap_or(1.);
        json!({"time":self.time(),"playing":playing,"effectiveQuality":effective,"stage":self.view.stage,"light":self.host.borrow().tokens.scheme==pm_design::Scheme::Light,
            "mediaSettings":self.media_settings_fact()})
    }
    /// Paused, panels follow the playhead. Playing, nothing reaches V8 or GPUI:
    /// the playhead and timecode are CALayers fed from the display link.
    pub fn tick_builtins(&mut self, cx: &mut Context<Self>) {
        let (playing, time) = (self.playing(), self.time());
        // Extensions follow playback themselves; the host coalesces the updates.
        if let Some(host) = &self.extensions.host {
            host.publish_time(time, playing);
        }
        self.sync_viewer_stage();
        if !self.builtins.clock.due(time, playing) {
            return;
        }
        if let Some(gizmos) = &self.builtins.gizmos {
            gizmos.update(cx, |g, cx| g.refresh(cx));
        }
        self.update_builtins(cx);
    }
    /// Escape during a drag restores through the kernel gesture that drag owns.
    pub fn cancel_gesture(&mut self, cx: &mut Context<Self>) -> bool {
        let timeline = self.builtins.timeline.clone();
        let gizmos = self.builtins.gizmos.clone();
        let views: Vec<_> = self.builtins.views.values().cloned().collect();
        timeline.is_some_and(|t| t.update(cx, |t, cx| t.cancel(cx)))
            || gizmos.is_some_and(|g| g.update(cx, |g, cx| g.cancel(cx)))
            || views
                .into_iter()
                .any(|view| view.update(cx, |view, cx| view.cancel_gesture(cx)))
    }
    pub fn update_builtins(&mut self, cx: &mut Context<Self>) {
        self.start_builtins(cx);
        self.probe_builtin_media(cx);
        if self.builtins.updating {
            self.builtins.again = true;
            return;
        }
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        let facts = self.facts();
        let originals = self.media.originals.clone();
        let observed = self.rendition_runtime();
        let show_missing = std::mem::take(&mut self.builtins.show_missing);
        self.builtins.updating = true;
        let check = runtime.clone();
        cx.spawn(async move |this, cx| {
            let tickets = cx
                .background_executor()
                .spawn(async move {
                    runtime.update_facts(facts);
                    runtime.update_media(originals, observed);
                    if show_missing {
                        runtime.show_missing_media();
                    }
                    runtime.publish()
                })
                .await;
            for (id, ticket) in tickets {
                let result = match ticket {
                    Ok(ticket) => ticket.await.map(|_| ()),
                    Err(e) => Err(e),
                };
                if let Err(error) = result {
                    eprintln!("Panel {id} update: {error}");
                }
            }
            let _ = this.update(cx, |v, cx| {
                if !v
                    .builtins
                    .runtime
                    .as_ref()
                    .is_some_and(|r| Arc::ptr_eq(r, &check))
                {
                    return;
                }
                v.builtins.updating = false;
                v.drain_builtins(cx);
                if std::mem::take(&mut v.builtins.again) {
                    v.update_builtins(cx);
                }
            });
        })
        .detach();
    }
    /// Closed built-in panels park (their isolates leave the cage account);
    /// reopened ones start again, the view keeping its tree until the new
    /// generation publishes. Parking waits for the isolate, off the main thread.
    pub fn sync_parking(&mut self, open: &[&str], cx: &mut Context<Self>) {
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        self.builtins.open = open.iter().map(|id| (*id).to_owned()).collect();
        for id in BUILTIN_PANELS
            .iter()
            .filter(|p| p.parks)
            .map(|p| p.id.as_str())
        {
            let shown = open.contains(&id);
            if shown && runtime.is_parked(id) {
                self.unpark(id, cx);
            } else if !shown
                && self.builtins.ready.contains(id)
                && !runtime.is_parked(id)
                && self.builtins.parking.insert(id.to_owned())
            {
                self.builtins.ready.remove(id);
                self.builtins.keep_surface(id);
                let (runtime, panel) = (runtime.clone(), id.to_owned());
                cx.spawn(async move |this, cx| {
                    let parked = {
                        let runtime = runtime.clone();
                        let panel = panel.clone();
                        cx.background_executor()
                            .spawn(async move { runtime.park(&panel) })
                            .await
                    };
                    let _ = this.update(cx, |v, cx| {
                        v.builtins.parking.remove(&panel);
                        if !parked {
                            v.builtins.ready.insert(panel.clone());
                        } else if v.builtins.open.contains(&panel) {
                            // Opened again while it was going.
                            v.unpark(&panel, cx);
                        }
                    });
                })
                .detach();
            }
        }
    }
    fn unpark(&mut self, panel: &str, cx: &mut Context<Self>) {
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        let Some(ticket) = runtime.unpark(panel) else {
            return;
        };
        let id = panel.to_owned();
        cx.spawn(async move |this, cx| {
            let result = match ticket {
                Ok(ticket) => ticket.await.map(|evaluation| evaluation.generation.number),
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |v, cx| match result {
                Ok(generation) => {
                    v.builtins.generations.insert(id.clone(), generation);
                    v.builtins.ready.insert(id);
                    v.drain_builtins(cx);
                    cx.notify();
                }
                Err(error) => v.message = format!("Panel {id} could not start: {error}"),
            });
        })
        .detach();
    }
    pub(crate) fn drain_builtins(&mut self, cx: &mut Context<Self>) {
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        let mut mounted = false;
        for id in runtime.take_cleared_overlays() {
            if let Some(overlays) = &self.builtins.overlays {
                overlays.update(cx, |o, cx| o.clear(&id, cx));
            }
        }
        for p in runtime.take_publications() {
            if p.panel.ends_with(pm_builtins::VIEWER_OVERLAY) {
                if let Some(overlays) = &self.builtins.overlays {
                    let runtime = runtime.clone();
                    overlays.update(cx, |o, cx| o.publish(runtime, p, cx));
                }
                continue;
            }
            if let Some(view) = self.builtins.views.get(&p.panel) {
                view.update(cx, |v, cx| v.publish(p, cx));
            } else {
                self.builtins.pending.insert(p.panel.clone(), p);
                mounted = true;
            }
        }
        for error in runtime.take_errors() {
            eprintln!("v4: {error}");
        }
        self.sync_v4_commands(&runtime);
        let painters = runtime.painters_revision();
        if painters != self.builtins.painters
            && let Some(timeline) = self.builtins.timeline.clone()
        {
            self.builtins.painters = painters;
            timeline.update(cx, |t, cx| t.repaint_decorations(cx));
        }
        self.sync_v4_themes(&runtime, cx);
        self.builtins
            .requests
            .extend(runtime.take_platform_requests());
        if !self.builtins.requests.is_empty() {
            self.host.borrow().send_command("v4.platform", vec![]);
        }
        // Existing panels notify themselves; only a first mount needs the shell.
        if mounted {
            cx.notify();
        }
    }
    pub fn builtin_panel(
        &mut self,
        id: &str,
        bounds: pm_shell::layout::Rect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.builtins.ready.contains(id) {
            return None;
        }
        let runtime = self.builtins.runtime.clone()?;
        let mut t = self.host.borrow().tokens.clone();
        if id == "timeline"
            && let Some(p) = &self.published
        {
            let scheme = if t.scheme == pm_design::Scheme::Dark {
                "dark"
            } else {
                "light"
            };
            let surface = self.extensions.v1_timeline[scheme].clone();
            t = timeline_chrome(t, &p.workspace, &surface);
        }
        if let Some(publication) = self.builtins.pending.remove(id) {
            let (tokens, natives) = (t.clone(), self.builtins.natives.clone());
            let outline = natives.outline.clone();
            let view =
                cx.new(|cx| SurfaceView::new(runtime, publication, tokens, natives, window, cx));
            if id == "timeline" {
                outline.bind(view.downgrade());
            }
            self.builtins.views.insert(id.into(), view);
        }
        let view = self.builtins.views.get(id)?.clone();
        let handle = window.window_handle();
        view.update(cx, |v, cx| {
            v.attach(handle);
            if *v.tokens() != t {
                v.retheme(t.clone(), cx);
            }
        });
        if id == "timeline" {
            if let Some(timeline) = self.builtins.timeline.clone() {
                timeline.update(cx, |timeline, cx| timeline.set_tokens(t.clone(), cx));
            }
            self.builtins.timeline_place = Some((bounds, t.metrics.header));
        }
        if id == "viewer" {
            let body_h = (bounds.h - t.metrics.header).max(1.);
            let stage = [bounds.w as f64, body_h as f64];
            if self.view.stage != stage {
                // The chrome's Fit readout and zoom steps measure this slot.
                self.view.stage = stage;
                cx.defer_in(window, |v, _, cx| v.update_builtins(cx));
            }
            let fit = crate::panels::fit_scale(bounds.w, body_h, self.dimensions, t.metrics.inset);
            let scale = self.view.zoom.unwrap_or(fit).min(self.zoom_limit());
            if self.view.scale.replace(scale) != scale
                && let Some(gizmos) = &self.builtins.gizmos
            {
                gizmos.update(cx, |g, cx| g.redraw(cx));
            }
            self.host
                .borrow_mut()
                .viewer_transform(self.dimensions, Some(scale), self.view.pan);
            // A cropped canvas has its edges outside the pane: no outline.
            let cropped = self.host.borrow().viewer_crop.get().is_some();
            self.builtins.frame_outline.show(
                &self.host.borrow(),
                !cropped && crate::panels::needs_frame_outline(self.background, t.colors.viewer),
                t.colors.separator,
                t.metrics.border,
            );
        }
        Some(
            div()
                .w_full()
                .h(px((bounds.h - t.metrics.header).max(0.)))
                .overflow_hidden()
                .child(view.clone().cached(StyleRefinement::default().size_full()))
                .child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, _, cx| view.read(cx).keep_native_slots(cx),
                    )
                    .absolute()
                    .size_0(),
                )
                .into_any_element(),
        )
    }
    pub fn builtin_header(
        &mut self,
        id: &str,
        bounds: pm_shell::layout::Rect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        // Ensure the accepted header and body refer to the same native publication.
        let _ = self.builtin_panel(id, bounds, window, cx);
        let view = self.builtins.views.get(id)?.clone();
        if !view.read(cx).has_header() {
            return None;
        }
        let header = self
            .builtins
            .headers
            .entry(id.into())
            .or_insert_with(|| cx.new(|cx| crate::v4_view::HeaderView::new(view, cx)));
        Some(div().w_full().child(header.clone()).into_any_element())
    }
    pub fn show_builtin(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.start_builtins(cx);
        if id == "settings" {
            // Settings › Media shows the protected folders as they are now.
            self.refresh_waiting_folders(cx);
        }
        if self.builtins.opening_windows.contains(id) {
            return;
        }
        if let Some(existing) = self.builtins.windows.get(id).copied()
            && existing.update(cx, |_, w, _| w.activate()).is_ok()
        {
            return;
        }
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        self.builtins.opening_windows.insert(id.into());
        let owner = cx.entity();
        let panel = id.to_string();
        let m = self.host.borrow().tokens.metrics;
        let (width, height, kind) = match id {
            "settings" => (m.settings_width, m.settings_height, WindowKind::Normal),
            "agent" => (
                m.agent_width,
                (2. * m.agent_width).min(
                    f32::from(window.viewport_size().height) - m.toolbar - m.status - 2. * m.gap,
                ),
                WindowKind::PopUp,
            ),
            _ => (m.sheet_width_wide, m.settings_height, WindowKind::Dialog),
        };
        // The agent popover hangs from the toolbar's trailing Agent button;
        // other panel windows cascade from the editor's top-left corner.
        let editor = window.bounds();
        let origin = if id == "agent" {
            point(
                editor.right() - px(width + m.gap),
                editor.top() + px(m.toolbar + m.gap),
            )
        } else {
            editor.origin + point(px(m.toolbar), px(m.toolbar))
        };
        let title: SharedString = match id {
            "agent" => "Agent",
            "export" => "Export",
            _ => "Settings",
        }
        .into();
        let id = id.to_owned();
        let popover = (id == "agent").then(|| self.host.borrow().tokens.clone());
        // Export is a sheet on the window that asked for it; the Agent popover
        // sits above it as its child window (P-CHILD).
        let parent = (kind != WindowKind::Normal).then(|| window.window_handle());
        // Opening GPUI renders the new root synchronously. Do it after this
        // Controller update so the window can acquire its published panel safely.
        cx.spawn(async move |this, cx| {
            if let Ok(ticket) = runtime.show(&id) {
                let _ = ticket.await;
            }
            let current = this
                .update(cx, |v, _| {
                    let current = v
                        .builtins
                        .runtime
                        .as_ref()
                        .is_some_and(|r| Arc::ptr_eq(r, &runtime));
                    // A stale open must not leave the panel marked as opening:
                    // every later Settings/Export/Agent request would be ignored.
                    if !current {
                        v.builtins.opening_windows.remove(&id);
                    }
                    current
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            let mut options = WindowOptions::new().kind(kind);
            options.parent = parent;
            if popover.is_some() {
                // Clear GPUI content over the popover material (language §12).
                options = options.macos_window_background(gpui::MacosWindowBackground::Transparent);
            }
            let result = cx.open_window(
                options
                    // A popover has no title bar; other panel windows get a
                    // seamless transparent one over their own material.
                    .titlebar(popover.is_none().then(|| TitlebarOptions {
                        title: Some(title),
                        appears_transparent: true,
                        ..Default::default()
                    }))
                    .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                        origin,
                        size(px(width), px(height)),
                    )))),
                |window, cx| {
                    if let Some(tokens) = &popover {
                        pm_shell::popover_frame(window, tokens);
                    }
                    cx.new(|cx| {
                        cx.observe(&owner, |_, _, cx| cx.notify()).detach();
                        SurfaceWindow { owner, panel }
                    })
                },
            );
            let _ = this.update(cx, |v, cx| {
                v.builtins.opening_windows.remove(&id);
                match result {
                    Ok(handle) => {
                        v.builtins.windows.insert(id, handle.into());
                    }
                    Err(error) => v.message = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// The Export sheet ends (Cancel, or the export finished or was cancelled);
    /// a failure keeps it, showing why. A sheet has no close button.
    pub(crate) fn close_export_sheet(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.builtins.windows.remove("export") {
            let _ = handle.update(cx, |_, w, _| w.remove_window());
        }
    }
    pub fn open_builtin_composition(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(p) = &self.published else {
            return;
        };
        if p.snapshot.open_comp().as_deref() == Some(id) {
            return;
        }
        let Some(root) = p.snapshot.root() else {
            return;
        };
        if pm_kernel::model::comp_node(root, Some(id)).is_none() {
            return;
        }
        let mut value = root.to_value();
        if let Some(object) = value.as_object_mut() {
            object.insert("compId", pm_model::Value::from(id));
        }
        self.call(
            pm_kernel::Call::Restore {
                project: value,
                label: "Open Composition".into(),
                expected_revision: p.revision(),
                expected_document: p.snapshot.doc_id,
            },
            cx,
        );
    }
    pub fn builtin_platform(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        while let Some(request) = self.builtins.requests.pop_front() {
            if !self.builtins.ready.contains(&request.panel)
                || self.builtins.generations.get(&request.panel) != Some(&request.generation)
            {
                runtime.platform_reply(request.id, Err("Panel is not ready".into()));
                continue;
            }
            let result = match request.operation.as_str() {
                "timeline.effect" | "viewer.action" => {
                    match request.input["kind"].as_str() {
                        Some("seek") => {
                            if let Some(time) = request.input["time"].as_f64() {
                                self.seek(time);
                            }
                        }
                        Some("pause") => {
                            if let Some(viewer) = &self.viewer {
                                let _ = viewer.command(pm_protocol::clock::TransportCommand::Pause);
                            }
                        }
                        Some("command") => {
                            let name = request.input["name"].as_str().unwrap_or("");
                            let args = request.input["args"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default();
                            self.host.borrow().send_command(name, args);
                        }
                        Some("openComposition") => {
                            self.open_builtin_composition(
                                request.input["id"].as_str().unwrap_or(""),
                                cx,
                            );
                        }
                        _ => {
                            self.message =
                                format!("Native action unavailable: {}", request.input["action"])
                        }
                    }
                    Ok(Value::Null)
                }
                "project.effect" => {
                    match serde_json::from_value::<pm_panel_media::HostRequest>(
                        request.input.clone(),
                    ) {
                        Ok(effect)
                            if self.published.as_ref().is_some_and(|p| {
                                p.project == effect.owner.project
                                    && p.snapshot.doc_id == effect.owner.document
                                    && p.revision() == effect.owner.revision
                            }) =>
                        {
                            match effect.effect {
                                pm_panel_media::Effect::OpenComposition(id) => {
                                    self.open_builtin_composition(&id, cx)
                                }
                                pm_panel_media::Effect::CompositionSettings(id) => {
                                    self.composition_sheet(true, Some(id), cx)
                                }
                                pm_panel_media::Effect::NewComposition => {
                                    self.composition_sheet(false, None, cx)
                                }
                                pm_panel_media::Effect::Reveal { path, .. } => {
                                    crate::platform::reveal(std::path::Path::new(&path))
                                }
                                pm_panel_media::Effect::PickFiles { replace } => {
                                    let place = replace.is_none();
                                    self.pick_media(replace, place, cx)
                                }
                                pm_panel_media::Effect::PickFolder { folder } => {
                                    self.pick_media_folder(folder, cx)
                                }
                                // A drop on the panel: assets only, no layers. Option
                                // separates a folder from a sequence, and no sequence
                                // imports yet, so it changes nothing here.
                                pm_panel_media::Effect::Import { files, folder, .. } => self
                                    .import_to_panel(
                                        files.into_iter().map(Into::into).collect(),
                                        folder,
                                        cx,
                                    ),
                                pm_panel_media::Effect::Renditions { action, assets } => {
                                    self.renditions(action, assets, cx)
                                }
                                pm_panel_media::Effect::Focus(_)
                                | pm_panel_media::Effect::ClearPreview => {}
                                other => {
                                    self.message =
                                        format!("Native media action unavailable: {other:?}")
                                }
                            }
                            Ok(Value::Null)
                        }
                        _ => Err("Media action belongs to a retired publication".into()),
                    }
                }
                "media.settings" => {
                    self.media_settings(&request.input, cx);
                    Ok(Value::Null)
                }
                "export.begin" | "export.retry" | "export.without" => {
                    self.begin_builtin_export(
                        request.input.clone(),
                        request.operation != "export.begin",
                        request.operation == "export.without",
                        cx,
                    );
                    Ok(Value::Null)
                }
                "export.cancel" => {
                    if let Some(job) = &self.builtins.export_job {
                        // Worker observes the flag and publishes terminal state
                        // only after the child is reaped and cleanup completes.
                        job.cancel.store(true, std::sync::atomic::Ordering::Release);
                    } else {
                        let _ = runtime.observe_export(json!({"phase":"cancelled"}));
                        self.close_export_sheet(cx);
                    }
                    Ok(Value::Null)
                }
                "export.reveal" => {
                    self.show_builtin("export", window, cx);
                    Ok(Value::Null)
                }
                "export.dismiss" => {
                    self.close_export_sheet(cx);
                    Ok(Value::Null)
                }
                "activatePanel" => {
                    if let Some(id) = request.input.as_str() {
                        self.host
                            .borrow()
                            .send_command(&format!("panel:show:{id}"), vec![]);
                    }
                    Ok(Value::Null)
                }
                "command" => {
                    let name = request.input["name"].as_str().unwrap_or("");
                    let args = request.input["args"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    self.host.borrow().send_command(name, args);
                    Ok(Value::Null)
                }
                "focus" => {
                    if let Some(id) = request.input.as_str()
                        && let Some(view) = self.builtins.views.get(&request.panel)
                    {
                        view.update(cx, |v, cx| v.focus_semantic(id, window, cx));
                    }
                    Ok(Value::Null)
                }
                "announce" => {
                    if let Some(message) = request.input.as_str() {
                        crate::v4_native::announce(&self.host.borrow().window, message);
                    }
                    Ok(Value::Null)
                }
                "agent.settings" => {
                    self.show_builtin("settings", window, cx);
                    Ok(Value::Null)
                }
                "agent.attach" => {
                    let target = crate::platform::ns_window(window)
                        .unwrap_or_else(|| self.host.borrow().window.clone());
                    let chosen = crate::platform::attach_paths(&target);
                    let runtime = self.builtins.runtime.clone();
                    cx.spawn(async move |this, cx| {
                        let Ok(paths) = chosen.recv().await else {
                            return;
                        };
                        if paths.is_empty() {
                            return;
                        }
                        // Reading and encoding the files stays off the main thread.
                        let outcome = cx
                            .background_executor()
                            .spawn(async move {
                                let records = crate::media::attachment_records(&paths)?;
                                match runtime {
                                    Some(runtime) => runtime.agent_attach(records),
                                    None => Err("The Agent panel closed.".into()),
                                }
                            })
                            .await;
                        if let Err(message) = outcome {
                            let _ = this.update(cx, |v, cx| {
                                v.errors.borrow_mut().tell("Agent", &message);
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                    Ok(Value::Null)
                }
                "agent.open" => {
                    self.show_builtin("agent", window, cx);
                    Ok(Value::Null)
                }
                "agent.visual" => {
                    let input = &request.input;
                    let dark = self.host.borrow().tokens.scheme == pm_design::Scheme::Dark;
                    let document = pm_agents::visual::document(
                        input["html"].as_str().unwrap_or_default(),
                        dark,
                    );
                    let parent = crate::platform::ns_window(window)
                        .unwrap_or_else(|| self.host.borrow().window.clone());
                    let viewer = pm_compat_host::visual::show(
                        Some(&parent),
                        input["id"].as_str().unwrap_or_default(),
                        input["title"].as_str().unwrap_or("Visualization"),
                        &document,
                        input["height"].as_f64().unwrap_or(360.),
                    );
                    // An ordinary window: forward only while Powermove is
                    // active, otherwise behind the other apps (P-CHILD).
                    gpui_macos::child_window::present(&viewer, true);
                    Ok(Value::Null)
                }
                "agent.close" => {
                    if let Some(handle) = self.builtins.windows.remove("agent") {
                        let _ = handle.update(cx, |_, w, _| w.remove_window());
                    }
                    gpui_macos::child_window::raise_if_active(&self.host.borrow().window);
                    Ok(Value::Null)
                }
                "settings.present" => {
                    self.show_builtin("settings", window, cx);
                    Ok(Value::Null)
                }
                "settings.close" => {
                    if let Some(handle) = self.builtins.windows.remove("settings") {
                        let _ = handle.update(cx, |_, w, _| w.remove_window());
                    }
                    gpui_macos::child_window::raise_if_active(&self.host.borrow().window);
                    Ok(Value::Null)
                }
                "settings.title" => {
                    if let Some(handle) = self.builtins.windows.get("settings") {
                        let title = match request.input.as_str() {
                            Some("project") => "Project",
                            Some("general") => "General",
                            Some("memory") => "Memory",
                            Some("accounts") => "Accounts",
                            Some("transcription") => "Transcription",
                            Some("extensions") => "Extensions",
                            _ => "Settings",
                        };
                        let _ = handle.update(cx, |_, w, _| w.set_window_title(title));
                    }
                    Ok(Value::Null)
                }
                "settings.scroll" => {
                    if let Some(section) = request.input["section"].as_str()
                        && let Some(view) = self.builtins.views.get("settings")
                    {
                        view.update(cx, |v, cx| {
                            v.scroll_semantic(&format!("settings.section.{section}"), cx)
                        });
                    }
                    Ok(Value::Null)
                }
                "contextMenu" => {
                    let options = request.input.as_array().cloned().unwrap_or_default();
                    let selected = options.clone();
                    let runtime = runtime.clone();
                    crate::v4_native::menu(window, &options, cx, move |index, _| {
                        let value = index
                            .and_then(|index| crate::v4_native::menu_item(&selected, index))
                            .and_then(|item| item.get("id"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        runtime.platform_reply(request.id, Ok(value));
                    });
                    continue;
                }
                "effectMenu" => {
                    let options = crate::v4_native::effect_options(&self.kernel.state());
                    let selected = options.clone();
                    let runtime = runtime.clone();
                    crate::v4_native::menu(window, &options, cx, move |index, _| {
                        runtime.platform_reply(
                            request.id,
                            Ok(index
                                .and_then(|index| crate::v4_native::menu_item(&selected, index))
                                .and_then(|item| item.get("id"))
                                .cloned()
                                .unwrap_or(Value::Null)),
                        );
                    });
                    continue;
                }
                "fontMenu" => {
                    let families = crate::colour_panel::font_families();
                    let options: Vec<_> = families.iter().map(|f| json!({"label": f})).collect();
                    let runtime = runtime.clone();
                    crate::v4_native::menu(window, &options, cx, move |index, _| {
                        let family = index.and_then(|i| families.get(i)).cloned();
                        runtime.platform_reply(request.id, Ok(json!(family)));
                    });
                    continue;
                }
                "project.menu" => {
                    let options = request.input["entries"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    let runtime = runtime.clone();
                    crate::v4_native::menu(window, &options, cx, move |index, _| {
                        runtime.platform_reply(request.id, Ok(json!(index)));
                    });
                    continue;
                }
                _ => Err(format!("Native service unavailable: {}", request.operation)),
            };
            runtime.platform_reply(request.id, result);
        }
    }
}
impl Controller {
    /// The highest zoom the display can show this composition at.
    pub(crate) fn zoom_limit(&self) -> f64 {
        self.host.borrow().viewer_zoom_limit(self.dimensions)
    }
    /// One Zoom In or Zoom Out step from what is on screen.
    pub(crate) fn step_zoom(&mut self, factor: f64, cx: &mut Context<Self>) {
        use pm_panel_viewerchrome::{ZOOM_MAX, ZOOM_MIN};
        let top = ZOOM_MAX.min(self.zoom_limit()).max(ZOOM_MIN);
        let zoom = (self.view.scale.get() * factor).clamp(ZOOM_MIN, top);
        self.view.place(Some(zoom), self.view.pan);
        self.write_view(None, cx);
    }
    /// Writes the placement now or after `after`, replacing any write not yet
    /// made. It stays local until the kernel has taken it.
    pub(crate) fn write_view(
        &mut self,
        after: Option<std::time::Duration>,
        cx: &mut Context<Self>,
    ) {
        if let Some(host) = &self.extensions.host {
            host.shell_state(serde_json::json!({
                "zoom": self.view.zoom.unwrap_or(1.),
                "fit": self.view.zoom.is_none(),
                "pan": self.view.pan,
            }));
        }
        self.sync_viewer_stage();
        self.view_write = Some(cx.spawn(async move |this, cx| {
            if let Some(after) = after {
                cx.background_executor().timer(after).await;
            }
            let Ok(pending) = this.update(cx, |v, _| v.write_ui()) else {
                return;
            };
            if let Some(pending) = pending {
                let _ = pending.await;
            }
            let _ = this.update(cx, |v, cx| {
                v.view.written();
                v.refresh(cx);
            });
        }));
    }
    /// The viewer slot's centre, window points.
    fn viewer_center(&self) -> Point<Pixels> {
        self.host
            .borrow()
            .viewer_rect
            .get()
            .map_or(Point::default(), |r| {
                point(px(r.x + r.w / 2.), px(r.y + r.h / 2.))
            })
    }
}
struct ControllerDelegate(WeakEntity<Controller>);
impl crate::timeline_surface::Delegate for ControllerDelegate {
    fn time(&self, cx: &App) -> f64 {
        self.0.upgrade().map_or(0., |c| c.read(cx).time())
    }
    fn seek(&self, seconds: f64, cx: &mut App) {
        let _ = self.0.update(cx, |v, cx| {
            v.seek(seconds);
            v.tick_builtins(cx);
        });
    }
    fn effect(&self, effect: pm_panel_timeline::HostEffect, cx: &mut App) {
        use pm_panel_timeline::HostEffect as E;
        let _ = self.0.update(cx, |v, cx| match effect {
            E::Seek(time) => v.seek(time),
            E::TogglePlayback => v.host.borrow().send_command("play", vec![]),
            E::FrameComposition => v.host.borrow().send_command("viewer.fit", vec![]),
            E::OpenComposition(id) => v.open_builtin_composition(&id, cx),
            E::CopyKeys(tracks) => {
                v.clipboard = crate::layer_commands::Clipboard::Keys(tracks);
                v.update_disabled();
            }
            // The keyframe sheets on the keys the right-click was on.
            E::EditKeyValues(keys) => {
                v.keyframe_sheet_for(crate::keyframe_sheets::Kind::Value, keys, cx)
            }
            E::EditVelocity(keys) => {
                v.keyframe_sheet_for(crate::keyframe_sheets::Kind::Velocity, keys, cx)
            }
            E::ScaleKeys(keys) => {
                v.keyframe_sheet_for(crate::keyframe_sheets::Kind::Scale, keys, cx)
            }
            other => v.message = format!("Not available yet: {other:?}"),
        });
    }
    fn view_facts(&self, facts: serde_json::Value, cx: &mut App) {
        if let Some(controller) = self.0.upgrade()
            && let Some(host) = &controller.read(cx).extensions.host
        {
            host.shell_state(serde_json::json!({ "timeline": facts }));
        }
    }
    fn report(&self, message: String, cx: &mut App) {
        let _ = self.0.update(cx, |v, cx| {
            v.message = message;
            cx.notify();
        });
    }
    fn command(&self, name: &str, cx: &mut App) {
        if let Some(controller) = self.0.upgrade() {
            controller.read(cx).host.borrow().send_command(name, vec![]);
        }
    }
    fn menu(
        &self,
        items: Vec<Value>,
        window: &mut Window,
        cx: &mut App,
        choose: Box<dyn FnOnce(Option<usize>, &mut App)>,
    ) {
        crate::v4_native::menu(window, &items, cx, choose);
    }
    fn painters_revision(&self, cx: &App) -> u64 {
        self.0
            .upgrade()
            .and_then(|c| c.read(cx).builtins.runtime.clone())
            .map_or(0, |runtime| runtime.painters_revision())
    }
    fn paint_clips(&self, clips: Value, done: crate::timeline_surface::PaintDone, cx: &mut App) {
        let Some(runtime) = self
            .0
            .upgrade()
            .and_then(|c| c.read(cx).builtins.runtime.clone())
        else {
            return;
        };
        cx.spawn(async move |cx| {
            // One 100 ms deadline for the whole request: a realm that hasn't
            // answered by then is left out of this paint. Answers are bounded
            // here, on the worker, before they reach the main thread.
            let shapes = cx
                .background_executor()
                .spawn(async move {
                    let visible: std::collections::HashSet<String> = clips
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|c| c["id"].as_str().map(str::to_owned))
                        .collect();
                    let tickets = runtime.paint_clips(&clips);
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_millis(100);
                    let mut shapes = pm_panel_timeline::render::Decorations::new();
                    let mut complete = true;
                    for (_, ticket) in tickets {
                        let Ok(ticket) = ticket else { continue };
                        // A painter that threw has answered; one still running
                        // past the deadline makes this paint incomplete.
                        let answer = crate::ticket::answer(ticket, deadline);
                        let Some(Ok(evaluation)) = answer else {
                            complete &= answer.is_some();
                            continue;
                        };
                        if let Some(answer) = painter_answer(&evaluation.value) {
                            pm_panel_timeline::render::add_decorations(
                                &mut shapes,
                                &answer,
                                &visible,
                            );
                        }
                    }
                    (shapes, complete)
                })
                .await;
            cx.update(|cx| done(shapes.0, shapes.1, cx));
        })
        .detach();
    }
    fn extension_menu(
        &self,
        locations: Vec<(&'static str, Value)>,
        items: Vec<Value>,
        window: &mut Window,
        cx: &mut App,
        choose: Box<dyn FnOnce(Option<usize>, &mut App)>,
    ) {
        let Some(controller) = self.0.upgrade() else {
            return;
        };
        if !controller.read(cx).extensions_listen() {
            crate::v4_native::menu(window, &items, cx, choose);
            return;
        }
        let at = window.mouse_position();
        controller.update(cx, |v, cx| {
            v.extension_rows_menu(
                locations,
                &items,
                (f32::from(at.x), f32::from(at.y)),
                choose,
                cx,
            )
        });
    }
}
impl crate::gizmo::Host for ControllerDelegate {
    fn time(&self, cx: &App) -> f64 {
        self.0.upgrade().map_or(0., |c| c.read(cx).time())
    }
    fn playing(&self, cx: &App) -> bool {
        self.0.upgrade().is_some_and(|c| c.read(cx).playing())
    }
    fn frame(&self, _: Bounds<Pixels>, cx: &App) -> Option<crate::gizmo::ViewFrame> {
        let controller = self.0.upgrade()?;
        let v = controller.read(cx);
        let slot = v.host.borrow().viewer_rect.get()?;
        let scale = v.view.scale.get();
        let (w, h) = (v.dimensions[0] * scale, v.dimensions[1] * scale);
        Some(crate::gizmo::ViewFrame {
            origin: point(
                px(slot.x + (slot.w - w as f32) / 2. + v.view.pan[0] as f32),
                px(slot.y + (slot.h - h as f32) / 2. + v.view.pan[1] as f32),
            ),
            scale,
            size: v.dimensions,
        })
    }
    fn tool(&self, cx: &App) -> crate::gizmo::Tool {
        use crate::gizmo::Tool;
        let Some(controller) = self.0.upgrade() else {
            return Tool::Other;
        };
        match controller.read(cx).tool.as_str() {
            "toolSelect" => Tool::Select,
            "toolHand" => Tool::Hand,
            "toolZoom" => Tool::Zoom,
            "toolRotate" => Tool::Rotate,
            "toolAnchor" => Tool::Anchor,
            "toolShape" => Tool::Shape,
            "toolText" => Tool::Text,
            "toolPen" => Tool::Pen,
            _ => Tool::Other,
        }
    }
    fn overlay(&self, lines: Vec<pm_engine_service::OverlayLine>, cx: &mut App) {
        let _ = self.0.update(cx, |v, _| v.set_gizmos(lines));
    }
    fn layer_menu(&self, layer: Option<String>, window: &mut Window, cx: &mut App) {
        let host = self.0.clone();
        let Some(controller) = self.0.upgrade() else {
            return;
        };
        let time = controller.read(cx).time();
        let at = window.mouse_position();
        let at = (f32::from(at.x), f32::from(at.y));
        // Over nothing: the canvas's own menu (New, as the shipped app; Text
        // and Null have no native command yet) and what extensions add to it.
        let Some(id) = layer else {
            controller.update(cx, |v, cx| {
                let entries = vec![pm_shell::MenuEntry::submenu(
                    "New",
                    vec![
                        pm_shell::MenuEntry::item("Solid", "newSolid"),
                        pm_shell::MenuEntry::item("Shape Layer", "newShape"),
                        pm_shell::MenuEntry::item("Adjustment Layer", "newAdjustment"),
                        pm_shell::MenuEntry::separator(),
                        pm_shell::MenuEntry::item("Composition…", "newComposition"),
                    ],
                )];
                let locations = vec![("viewer:context", json!({"layerId":null,"time":time}))];
                if let Some(entries) = v.extension_menu(locations, entries, at, cx) {
                    v.host.borrow().context_menu(&entries, at.0, at.1);
                }
            });
            return;
        };
        let choose: crate::timeline_surface::Choose = Box::new(move |index, cx| {
            let rows = &crate::layer_commands::LAYER_MENU;
            if let (Some((command, _)), Some(controller)) =
                (index.and_then(|i| rows.get(i)), host.upgrade())
            {
                controller
                    .read(cx)
                    .host
                    .borrow()
                    .send_command(command, vec![]);
            }
        });
        if !controller.read(cx).extensions_listen() {
            crate::v4_native::menu(
                window,
                &crate::layer_commands::layer_menu_rows(),
                cx,
                choose,
            );
            return;
        }
        // The layer's menu with what extensions add for it, as the shipped
        // viewer's layer menu (`layer:context`, then `viewer:context`).
        let layers: Vec<String> = controller
            .read(cx)
            .published
            .as_ref()
            .map(|p| p.selection.layers().to_vec())
            .filter(|layers| layers.contains(&id))
            .unwrap_or_else(|| vec![id.clone()]);
        let locations = vec![
            ("layer:context", json!({"layerId": id, "layerIds": layers})),
            (
                "viewer:context",
                json!({"kind":"layer","layerId":id,"time":time}),
            ),
        ];
        controller.update(cx, |v, cx| {
            v.extension_rows_menu(
                locations,
                &crate::layer_commands::layer_menu_rows(),
                at,
                choose,
                cx,
            )
        });
    }
    fn navigate(&self, change: crate::gizmo::Navigate, cx: &mut App) {
        let _ = self.0.update(cx, |v, cx| {
            let limit = v.zoom_limit();
            let center = v.viewer_center();
            v.view.navigate(change, center, limit);
            v.write_view(Some(crate::gizmo::NAVIGATION_IDLE), cx);
            cx.notify();
        });
    }
    fn report(&self, message: String, cx: &mut App) {
        crate::timeline_surface::Delegate::report(self, message, cx);
    }
    fn shape_variant(&self, cx: &App) -> String {
        self.0
            .upgrade()
            .map_or_else(|| "rect".into(), |c| c.read(cx).shape_variant.clone())
    }
    fn pause(&self, cx: &mut App) {
        if let Some(controller) = self.0.upgrade()
            && let Some(viewer) = &controller.read(cx).viewer
        {
            let _ = viewer.command(pm_protocol::clock::TransportCommand::Pause);
        }
    }
    fn command(&self, name: &str, cx: &mut App) {
        crate::timeline_surface::Delegate::command(self, name, cx);
    }
    fn text_input(
        &self,
        initial: &str,
        window: &mut Window,
        _: &mut App,
    ) -> Option<Rc<dyn crate::gizmo::text::TextInput>> {
        crate::gizmo::text::native_input(initial, window)
    }
}
struct SurfaceWindow {
    owner: Entity<Controller>,
    panel: String,
}
impl Render for SurfaceWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let panel = self.panel.clone();
        let popover = self.panel == "agent";
        let (mut fill, mut band) = (0, 0.);
        let body = self.owner.update(cx, |v, cx| {
            let (header, colour, titlebar) = {
                let t = &v.host.borrow().tokens;
                (t.metrics.header, t.colors.panel, t.metrics.titlebar)
            };
            fill = colour;
            // Windows with a transparent title bar keep their content below it.
            band = if popover { 0. } else { titlebar };
            v.builtin_panel(
                &panel,
                pm_shell::layout::Rect {
                    x: 0.,
                    y: 0.,
                    w: f32::from(size.width),
                    h: f32::from(size.height) + header - band,
                },
                window,
                cx,
            )
        });
        // The agent popover's material is native (glass); other panel
        // windows paint the solid panel colour.
        div()
            .size_full()
            .overflow_hidden()
            .pt(px(band))
            .when(!popover, |d| d.bg(rgb(fill)))
            .children(body)
    }
}

/// v1's timeline `gutterWidth` default (`api-helpers.ts`).
const V1_DEFAULT_GUTTER: f64 = 224.;
/// A painter's answer: JSON text (the realm returns `JSON.stringify(…)`),
/// refused past 4 MiB before parsing.
fn painter_answer(text: &str) -> Option<Value> {
    const LIMIT: usize = 4 * 1024 * 1024;
    if text.len() > LIMIT {
        return None;
    }
    match serde_json::from_str::<Value>(text).ok()? {
        Value::String(inner) => serde_json::from_str(&inner).ok(),
        other => Some(other),
    }
}

/// v1–3's timeline chrome (`workspace.chrome.timeline`, which extensions set
/// through `workspace.mutate`) as the timeline's metrics, within v1's ranges:
/// row height, layer column (gutter) width, ruler height and keyframe size.
pub fn timeline_chrome(
    mut t: pm_design::Tokens,
    workspace: &pm_model::Value,
    surface: &Value,
) -> pm_design::Tokens {
    // The theme's timeline surface (track, gutter, accent, text, line).
    let colour = |key: &str| {
        surface[key]
            .as_u64()
            .filter(|c| *c <= 0xFF_FFFF)
            .map(|c| c as u32)
    };
    for (target, key) in [
        (&mut t.colors.panel, "track"),
        (&mut t.colors.panel_secondary, "gutter"),
        (&mut t.colors.accent, "accent"),
        (&mut t.colors.text, "text"),
        (&mut t.colors.separator, "line"),
    ] {
        if let Some(c) = colour(key) {
            *target = c;
        }
    }
    let chrome = workspace
        .get("chrome")
        .and_then(|c| c.get("timeline"))
        .map(|c| {
            serde_json::from_str::<Value>(&pm_model::json::to_canonical_string(c))
                .unwrap_or(Value::Null)
        })
        .unwrap_or(Value::Null);
    // Main rounds the three lengths and keeps fractional key and corner sizes.
    let field = |key: &str, low: f64, high: f64, round: bool| {
        chrome[key]
            .as_f64()
            .filter(|v| v.is_finite())
            .map(|v| if round { v.round() } else { v }.clamp(low, high))
    };
    let m = &mut t.metrics;
    for (target, value) in [
        (&mut m.track_row, field("rowHeight", 22., 48., true)),
        // v1 wrote its default (224) into every workspace; only another value
        // is a person's choice. The default takes the native, wider column.
        (
            &mut m.layer_column,
            field("gutterWidth", 160., 640., true).filter(|w| *w != V1_DEFAULT_GUTTER),
        ),
        (&mut m.ruler, field("rulerHeight", 20., 42., true)),
        (&mut m.keyframe, field("keyframeSize", 4., 12., false)),
        // Main draws clip bars with at most a 5-point corner.
        (
            &mut m.clip_radius,
            field("clipRadius", 0., 12., false).map(|r| r.min(5.)),
        ),
    ] {
        if let Some(value) = value {
            *target = value as f32;
        }
    }
    // A reversed surface order swaps the track and gutter fills (T1).
    let reversed = workspace
        .get("chrome")
        .and_then(|c| c.get("timelineSurfaceOrder"))
        .and_then(|o| {
            pm_model::json::to_canonical_string(o)
                .parse::<String>()
                .ok()
        })
        .is_some_and(|o| o == "\"reversed\"");
    if reversed {
        std::mem::swap(&mut t.colors.panel, &mut t.colors.panel_secondary);
    }
    t
}
