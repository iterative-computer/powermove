//! Window-owned v4 runtime lifecycle and shell integration. The main thread
//! only publishes immutable trees and enqueues input; JavaScript runs on owners.
use crate::{session::Controller, v4_view::SurfaceView};
use gpui::{prelude::*, *};
use pm_builtins::{Builtins, LIVE_PANELS, PlatformRequest, Publication};
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
    pub gizmos: Option<Entity<crate::gizmo::GizmoSurface>>,
    clock: crate::playback::PanelClock,
    headers: BTreeMap<String, Entity<crate::v4_view::HeaderView>>,
    ready: BTreeSet<String>,
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
            let surface = cx.new(|cx| {
                crate::timeline_surface::TimelineSurface::new(
                    kernel,
                    project.clone(),
                    tokens,
                    delegate,
                    outline,
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
        for id in LIVE_PANELS {
            if application && !matches!(*id, "agent" | "settings") {
                continue;
            }
            tickets.push((id.to_string(), runtime.mount(id)));
        }
        let runtime = Arc::new(runtime);
        self.builtins.runtime = Some(runtime.clone());
        let check = runtime.clone();
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
        json!({"time":self.time(),"playing":playing,"effectiveQuality":effective,"stage":[self.dimensions[0],self.dimensions[1]],"light":self.host.borrow().tokens.scheme==pm_design::Scheme::Light})
    }
    /// Paused, panels follow the playhead. Playing, nothing reaches V8 or GPUI:
    /// the playhead and timecode are CALayers fed from the display link.
    pub fn tick_builtins(&mut self, cx: &mut Context<Self>) {
        let (playing, time) = (self.playing(), self.time());
        // Extensions follow playback themselves; the host coalesces the updates.
        if let Some(host) = &self.extensions.host {
            host.publish_time(time, playing);
        }
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
        let observed = self.builtins.media_runtime.clone();
        self.builtins.updating = true;
        let check = runtime.clone();
        cx.spawn(async move |this, cx| {
            let tickets = cx
                .background_executor()
                .spawn(async move {
                    runtime.update_facts(facts);
                    runtime.update_media(originals, observed);
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
    pub(crate) fn drain_builtins(&mut self, cx: &mut Context<Self>) {
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        let mut mounted = false;
        for p in runtime.take_publications() {
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
        let t = self.host.borrow().tokens.clone();
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
            crate::playback::set_timeline(
                cx.entity_id(),
                bounds,
                t.metrics.layer_column,
                t.metrics.header,
            );
        }
        if id == "viewer" {
            let body_h = (bounds.h - t.metrics.header).max(1.);
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
            crate::playback::frame_outline(
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
        let title: SharedString = match id {
            "agent" => "Agent",
            "export" => "Export",
            _ => "Settings",
        }
        .into();
        let id = id.to_owned();
        // Opening GPUI renders the new root synchronously. Do it after this
        // Controller update so the window can acquire its published panel safely.
        cx.spawn(async move |this, cx| {
            if let Ok(ticket) = runtime.show(&id) {
                let _ = ticket.await;
            }
            let current = this
                .update(cx, |v, _| {
                    v.builtins
                        .runtime
                        .as_ref()
                        .is_some_and(|r| Arc::ptr_eq(r, &runtime))
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            let result = cx.open_window(
                WindowOptions::new()
                    .kind(kind)
                    .titlebar(Some(TitlebarOptions {
                        title: Some(title),
                        ..Default::default()
                    }))
                    .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                        point(px(m.toolbar), px(m.toolbar)),
                        size(px(width), px(height)),
                    )))),
                |_, cx| {
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
                "export.begin" | "export.retry" => {
                    self.begin_builtin_export(
                        request.input.clone(),
                        request.operation == "export.retry",
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
                    }
                    Ok(Value::Null)
                }
                "export.reveal" => {
                    self.show_builtin("export", window, cx);
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
                "agent.open" => {
                    self.show_builtin("agent", window, cx);
                    Ok(Value::Null)
                }
                "agent.close" => {
                    if let Some(handle) = self.builtins.windows.remove("agent") {
                        let _ = handle.update(cx, |_, w, _| w.remove_window());
                    }
                    self.host.borrow().window.makeKeyAndOrderFront(None);
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
                    self.host.borrow().window.makeKeyAndOrderFront(None);
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
            other => v.message = format!("Not available yet: {other:?}"),
        });
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
            _ => Tool::Other,
        }
    }
    fn overlay(&self, lines: Vec<pm_engine_service::OverlayLine>, cx: &mut App) {
        let _ = self.0.update(cx, |v, _| v.set_gizmos(lines));
    }
    fn layer_menu(&self, window: &mut Window, cx: &mut App) {
        let host = self.0.clone();
        crate::v4_native::menu(
            window,
            &crate::layer_commands::layer_menu_rows(),
            cx,
            move |index, cx| {
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
            },
        );
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
}
struct SurfaceWindow {
    owner: Entity<Controller>,
    panel: String,
}
impl Render for SurfaceWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let panel = self.panel.clone();
        let body = self.owner.update(cx, |v, cx| {
            let header = v.host.borrow().tokens.metrics.header;
            v.builtin_panel(
                &panel,
                pm_shell::layout::Rect {
                    x: 0.,
                    y: 0.,
                    w: f32::from(size.width),
                    h: f32::from(size.height) + header,
                },
                window,
                cx,
            )
        });
        div().size_full().overflow_hidden().children(body)
    }
}
