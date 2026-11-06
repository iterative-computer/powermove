//! Window-owned v4 runtime lifecycle and shell integration. The main thread
//! only publishes immutable trees and enqueues input; JavaScript runs on owners.
use crate::{session::Controller, v4_view::SurfaceView};
use gpui::{prelude::*, *};
use pm_builtins::{Builtins, LIVE_PANELS, PlatformRequest, Publication};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};
#[derive(Default)]
pub struct Host {
    project: Option<String>,
    application: bool,
    pub runtime: Option<Arc<Builtins>>,
    pending: BTreeMap<String, Publication>,
    pub views: BTreeMap<String, Entity<SurfaceView>>,
    headers: BTreeMap<String, Entity<crate::v4_view::HeaderView>>,
    ready: BTreeSet<String>,
    generations: BTreeMap<String, u64>,
    requests: VecDeque<PlatformRequest>,
    windows: BTreeMap<String, AnyWindowHandle>,
    opening_windows: BTreeSet<String>,
    pub export_job: Option<crate::export_coordinator::Job>,
    pub export_retry: Option<crate::export_coordinator::Frozen>,
    pub export_tasks: Arc<std::sync::Mutex<pm_promedia::tasks::BackgroundTasks>>,
    pub last_tick: Option<std::time::Instant>,
    pub last_time: f64,
    pub last_playing: bool,
    pub media_runtime: pm_panel_media::readiness::ObservedRuntime,
    media_probe_key: String,
    updating: bool,
    again: bool,
}
impl Controller {
    pub fn start_builtins(&mut self, cx: &mut Context<Self>) {
        let application = self.session.is_none();
        let project = self
            .session
            .as_ref()
            .map(|s| s.project.clone())
            .unwrap_or_else(|| "builtin-application".into());
        if self.builtins.project.as_ref() == Some(&project)
            && self.builtins.application == application
        {
            return;
        }
        if let Some(job) = &self.builtins.export_job {
            job.cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        if let Some(runtime) = self.builtins.runtime.take() {
            runtime.close();
        }
        let windows = std::mem::take(&mut self.builtins.windows);
        self.builtins = Host::default();
        self.builtins.windows = windows;
        self.builtins.project = Some(project.clone());
        self.builtins.application = application;
        self.start_extensions(cx);
        let (tx, rx) = async_channel::bounded(1);
        let wake = move || {
            let _ = tx.try_send(());
        };
        let created = if application {
            Builtins::application(self.kernel.clone(), wake)
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
    fn probe_builtin_media(&mut self, cx: &mut Context<Self>) {
        let Some(published) = self.published.clone() else {
            return;
        };
        let originals = self.media.originals.clone();
        let proxies = self.media.proxies.clone();
        let key = format!(
            "{}:{}:{:?}:{:?}",
            published.snapshot.doc_id,
            published.revision(),
            originals,
            proxies
        );
        if self.builtins.media_probe_key == key {
            return;
        }
        self.builtins.media_probe_key = key.clone();
        cx.spawn(async move |this, cx| {
            let observed = cx
                .background_executor()
                .spawn(async move {
                    pm_builtins::delivery::observe_media(
                        &published.project,
                        &published.snapshot,
                        &originals,
                        &proxies,
                    )
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                if v.builtins.media_probe_key != key {
                    return;
                }
                match observed {
                    Ok(observed) => {
                        v.builtins.media_runtime = observed;
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
        let playing = self.viewer.as_ref().is_some_and(|v| {
            v.status()
                .clock
                .is_some_and(|c| c.state == pm_protocol::clock::TransportState::Playing)
        });
        json!({"time":self.time(),"playing":playing,"stage":[self.dimensions[0],self.dimensions[1]],"light":self.host.borrow().tokens.scheme==pm_design::Scheme::Light})
    }
    pub fn tick_builtins(&mut self, cx: &mut Context<Self>) {
        let playing = self.playing();
        let time = self.time();
        if let Some(host) = &self.extensions.host {
            host.publish_time(time, playing);
        }
        if time == self.builtins.last_time && playing == self.builtins.last_playing {
            return;
        }
        if playing == self.builtins.last_playing
            && playing
            && self
                .builtins
                .last_tick
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(50))
        {
            return;
        }
        self.builtins.last_time = time;
        self.builtins.last_playing = playing;
        self.builtins.last_tick = Some(std::time::Instant::now());
        self.update_builtins(cx);
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
        for p in runtime.take_publications() {
            self.builtins.pending.insert(p.panel.clone(), p);
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
        cx.notify();
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
        if let Some(publication) = self.builtins.pending.remove(id) {
            if let Some(view) = self.builtins.views.get(id) {
                view.update(cx, |v, cx| {
                    v.tokens = self.host.borrow().tokens.clone();
                    v.publish(publication, window, cx);
                });
            } else {
                let tokens = self.host.borrow().tokens.clone();
                let view = cx.new(|cx| SurfaceView::new(runtime, publication, tokens, window, cx));
                self.builtins.views.insert(id.into(), view);
            }
        }
        let view = self.builtins.views.get(id)?.clone();
        let t = self.host.borrow().tokens.clone();
        if id == "timeline" {
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
            self.host
                .borrow_mut()
                .viewer_transform(self.dimensions, Some(self.zoom.unwrap_or(fit)));
            crate::playback::frame_outline(
                &self.host.borrow(),
                crate::panels::needs_frame_outline(self.background, t.colors.viewer),
                t.colors.separator,
                t.metrics.border,
            );
        }
        Some(
            div()
                .w_full()
                .h(px((bounds.h - t.metrics.header).max(0.)))
                .overflow_hidden()
                .child(view)
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
        view.read(cx).header_id()?;
        let header = self
            .builtins
            .headers
            .entry(id.into())
            .or_insert_with(|| cx.new(|_| crate::v4_view::HeaderView(view)));
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
    fn open_builtin_composition(&mut self, id: &str, cx: &mut Context<Self>) {
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
                                pm_panel_media::Effect::CompositionSettings(_) => {
                                    self.show_builtin("settings", window, cx)
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
                            .and_then(|index| selected.get(index))
                            .and_then(|item| item.get("id"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        runtime.platform_reply(request.id, Ok(value));
                    });
                    continue;
                }
                "effectMenu" => {
                    let state = self.kernel.state();
                    let options = state
                        .registries
                        .get("effects")
                        .into_iter()
                        .flatten()
                        .map(|entry| json!({"id":entry.id,"label":entry.id}))
                        .collect::<Vec<_>>();
                    let selected = options.clone();
                    let runtime = runtime.clone();
                    crate::v4_native::menu(window, &options, cx, move |index, _| {
                        runtime.platform_reply(
                            request.id,
                            Ok(index
                                .and_then(|index| selected.get(index))
                                .and_then(|item| item.get("id"))
                                .cloned()
                                .unwrap_or(Value::Null)),
                        );
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
