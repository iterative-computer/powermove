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
/// Rows the font picker lists at once; typing narrows the rest.
const FONT_PAGE: usize = 80;
#[derive(Default)]
pub struct Host {
    /// The font picker's open request: the panel's runtime and the request
    /// it answers when a face is chosen (or nothing).
    pub font_pick: Option<(Arc<Builtins>, u64)>,
    /// The Agent panel's open menu: its runtime, the request it answers,
    /// and the values of its rows.
    pub agent_pick: Option<(Arc<Builtins>, u64, Vec<Value>)>,
    /// The Agent panel is in the workspace's docks (pinned), and whether its
    /// toolbar item has parked it (hidden, its place kept).
    pub agent_docked: bool,
    pub agent_parked: bool,
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
    /// The viewer panel's rect (its bar is the top `metrics.header`), for the
    /// bar's native timecode layer while playing.
    pub viewer_place: Option<pm_shell::layout::Rect>,
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
    /// The frame (ticks) Properties last followed while playing.
    followed: Option<i64>,
    headers: BTreeMap<String, Entity<crate::v4_view::HeaderView>>,
    pub(crate) ready: BTreeSet<String>,
    generations: BTreeMap<String, u64>,
    pub(crate) requests: VecDeque<PlatformRequest>,
    pub windows: BTreeMap<String, AnyWindowHandle>,
    /// The size each panel window was opened at (the Agent popover's, for the
    /// MCP panel tools).
    pub window_sizes: BTreeMap<String, (f32, f32)>,
    opening_windows: BTreeSet<String>,
    /// When the Agent popover last gave way to a press in its editor (the
    /// toolbar's Agent item among them), so that same press's toggle does
    /// not open it again.
    agent_dismissed: Option<std::time::Instant>,
    /// The Settings page the next show asks for (Keyboard Shortcuts…).
    pub settings_page: Option<String>,
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
        let mut environment = pm_agents::launch::Environment::current(
            self.mcp.profile.clone(),
            pm_mcp::catalog::names(),
            pm_mcp::catalog::composition_names(),
        );
        environment.compatible = Some(Arc::new(crate::providers::Chat {
            profile: self.mcp.profile.clone(),
        }));
        runtime.set_agent_environment(environment, Arc::new(pm_agents::account::open_in_browser));
        runtime.set_agent_notices(crate::notifications::sink(self.window_key));
        runtime.share_effect_clipboard(crate::extensions::effect_clipboard());
        let mut tickets = vec![];
        for panel in BUILTIN_PANELS.iter() {
            if application && !panel.application {
                continue;
            }
            if panel.parks && runtime.defer_mount(&panel.id) {
                continue;
            }
            tickets.push((panel.id.clone(), runtime.mount(&panel.id)));
        }
        let runtime = Arc::new(runtime);
        self.builtins.runtime = Some(runtime.clone());
        self.wire_effect_desk();
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
                            // A panel window kept open across a restart (Export
                            // asked for before the project arrived) shows its
                            // panel from the new realm, not an empty sheet.
                            if v.builtins.windows.contains_key(&id) {
                                let _ = check.show(&id);
                            }
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
                v.bound_open_wait(&mounting, cx);
                cx.notify();
            });
        })
        .detach();
        // Weak: the runtime owns this loop's sender, so a strong handle kept
        // the realms (their isolates) alive after the editor closed.
        let runtime = Arc::downgrade(&runtime);
        cx.spawn(async move |this, cx| {
            while rx.recv().await.is_ok() {
                if this
                    .update(cx, |v, cx| {
                        if v.builtins
                            .runtime
                            .as_ref()
                            .is_some_and(|r| std::ptr::eq(Arc::as_ptr(r), runtime.as_ptr()))
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
    /// Every panel of `project` has mounted or failed: the open is over,
    /// unless its viewer waits for effects its extensions are still loading.
    pub(crate) fn settled(&mut self, project: &str) {
        if self.settling.as_deref() == Some(project) && self.awaiting_extension_effects() {
            self.extensions.settle_after = Some(project.to_owned());
            return;
        }
        self.finish_open(project);
    }
    /// The open of `project` is over: its panels show it.
    pub(crate) fn finish_open(&mut self, project: &str) {
        if self.settling.as_deref() == Some(project) {
            self.settling = None;
            self.opening_name = None;
            self.open_timer.mark("settled", serde_json::json!({}));
            self.busy = None;
            // Bound to the open that brought `project`: they run only if this
            // window still shows it.
            let shown = self.session.as_ref().is_some_and(|s| s.project == project);
            let (run, dropped) = self.after_open.settle(self.settling_open, shown);
            if let Some(note) = dropped {
                self.message = note;
            }
            for (name, args) in run {
                self.host.borrow().send_command(&name, args);
            }
            if shown && std::mem::take(&mut self.workspace_after_open) {
                self.host.borrow().send_command("bringWorkspace", vec![]);
            }
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
    fn facts(&self, cx: &App) -> Value {
        let status = self.viewer.as_ref().map(|v| v.status());
        let playing = status.as_ref().is_some_and(|s| {
            s.clock
                .is_some_and(|c| c.state == pm_protocol::clock::TransportState::Playing)
        });
        // Auto's measured resolution; 0 until a frame has drawn.
        // Layers left out of the frame on screen, by ID, so Properties can say
        // so on that layer only (3d-studio.md §10).
        let failures: serde_json::Map<String, Value> = status
            .as_ref()
            .map(|s| {
                s.layer_failures
                    .iter()
                    .map(|f| (f.layer.clone(), json!(f.message)))
                    .collect()
            })
            .unwrap_or_default();
        let effective = status
            .as_ref()
            .map(|s| s.effective_quality)
            .filter(|q| *q > 0.)
            .unwrap_or(1.);
        json!({"time":self.time(),"playing":playing,"looping":self.playback.looping,
            "channels":crate::viewer2::CHANNELS.iter().find(|(m, _, _)| *m == self.channels).map_or("viewer.showRGB", |(_, _, c)| c),"effectiveQuality":effective,"layerFailures":failures,"studioRefineProgress":status.as_ref().and_then(|s|s.refine_progress),"studioExportEstimate":status.as_ref().map(|s|&s.export_estimate),"studioDenoiser":if pm_engine_service::studio::OIDN{"oidn"}else{"guided"},"focusPart":self.part_focus.as_ref().map(|(layer,slot,seq)|json!({"layer":layer,"slot":slot,"seq":seq})),"stage":self.view.stage,"timelineShown":self.timeline_shown,"light":self.host.borrow().tokens.scheme==pm_design::Scheme::Light,
            "mediaSettings":self.media_settings_fact(),"storageSettings":self.storage_settings_fact(),"skimming":self.skimming.on,"shortcuts":self.shortcuts_page,"cloud":crate::cloud::fact(cx),"provider":crate::providers::fact(cx),"agentClients":crate::agent_clients::fact(cx),
            // The Agent panel is a popover while its own window is open, a docked panel otherwise.
            "agentPopover":self.builtins.opening_windows.contains("agent")
                || self.builtins.windows.get("agent").is_some_and(|w| cx.windows().contains(w))})
    }
    /// Paused, panels follow the playhead. Playing, only Properties does
    /// (`follow_playback`): the playhead and timecode are CALayers fed from
    /// the display link.
    pub fn tick_builtins(&mut self, cx: &mut Context<Self>) {
        let (playing, time) = (self.playing(), self.time());
        // Extensions follow playback themselves; the host coalesces the updates.
        if let Some(host) = &self.extensions.host {
            host.publish_time(time, playing);
        }
        self.sync_viewer_stage();
        if !self.builtins.clock.due(time, playing) {
            if playing {
                self.follow_playback(time, cx);
            }
            return;
        }
        if let Some(gizmos) = &self.builtins.gizmos {
            gizmos.update(cx, |g, cx| g.refresh(cx));
        }
        self.update_builtins(cx);
    }
    /// Playing, Properties shows the values of each frame the viewer shows
    /// (v1's inspector refreshed at 15 Hz): only that panel publishes, once
    /// per new frame, one update at a time off the main thread, so it keeps
    /// the picture's pace up to the display's and skips a frame while an
    /// update is under way rather than queueing behind playback.
    fn follow_playback(&mut self, time: f64, cx: &mut Context<Self>) {
        const PANEL: &str = "inspector";
        let Some(frame) = crate::session::frame_start_at(self.rate, time) else {
            return;
        };
        if self.builtins.updating
            || self.builtins.followed == Some(frame)
            || !self.builtins.open.contains(PANEL)
            || !self.builtins.ready.contains(PANEL)
        {
            return;
        }
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        self.builtins.followed = Some(frame);
        let time = frame as f64 / pm_protocol::common::TICKS_PER_SECOND as f64;
        self.builtins.updating = true;
        let check = runtime.clone();
        let started = std::time::Instant::now();
        cx.spawn(async move |this, cx| {
            let ticket = cx
                .background_executor()
                .spawn(async move { runtime.follow_time(PANEL, time, true) })
                .await;
            if let Some(Err(error)) = match ticket {
                Some(Ok(ticket)) => Some(ticket.await.map(|_| ())),
                Some(Err(error)) => Some(Err(error)),
                None => None,
            } {
                eprintln!("Panel {PANEL} update: {error}");
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
                {
                    let _span = crate::diag::span("follow.drain");
                    v.drain_builtins(cx);
                }
                crate::diag::line(
                    json!({"follow": PANEL, "ms": started.elapsed().as_secs_f64() * 1000.}),
                );
                if std::mem::take(&mut v.builtins.again) {
                    v.update_builtins(cx);
                }
            });
        })
        .detach();
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
    /// The font picker (v1's): our menu under the field that opened it, a
    /// search over every installed family, each row in its own face, the
    /// current one checked and in view. An agent's pick is claimed by label
    /// as for every panel menu, so no menu is left on screen for it.
    fn font_menu(
        &mut self,
        runtime: &Arc<Builtins>,
        request: &pm_builtins::PlatformRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let families = crate::colour_panel::font_families();
        let current = request.input["current"].as_str().unwrap_or_default();
        if let Some(previous) = self.builtins.font_pick.take() {
            previous.0.platform_reply(previous.1, Ok(Value::Null));
        }
        let options: Vec<Value> = families.iter().map(|f| json!({"label": f})).collect();
        if let Some(choice) = pm_shell::agent_menu::claim(
            &pm_shell::agent_menu_windows(&self.host.borrow().window),
            &options,
        ) {
            let family = choice.and_then(|i| families.get(i)).cloned();
            runtime.platform_reply(request.id, Ok(json!(family)));
            return;
        }
        let anchor = request.input["anchor"].as_u64().and_then(|id| {
            let view = self.builtins.views.get(&request.panel)?;
            let b = view.read(cx).anchor_bounds(id)?;
            Some(pm_shell::layout::Rect {
                x: f32::from(b.origin.x),
                y: f32::from(b.origin.y),
                w: f32::from(b.size.width),
                h: f32::from(b.size.height),
            })
        });
        let choices = families
            .iter()
            .map(|f| {
                pm_shell::transient::Choice::new(f.clone(), format!("fontPick:{f}"))
                    .checked(f == current)
                    .in_font(f.clone())
            })
            .collect();
        self.builtins.font_pick = Some((runtime.clone(), request.id));
        let _ = window;
        self.host.borrow_mut().open_menu(
            None,
            choices,
            anchor,
            Some(FONT_PAGE),
            Some("fontPick:".into()),
            "Search fonts",
        );
    }
    /// The Agent panel's pop-up menus (threads, model, access, slash): the
    /// field menu under the control that asked, standing above the popover
    /// when the panel is shown there.
    fn agent_menu(
        &mut self,
        runtime: &Arc<Builtins>,
        request: &pm_builtins::PlatformRequest,
        cx: &mut Context<Self>,
    ) {
        if let Some((previous, id, _)) = self.builtins.agent_pick.take() {
            previous.platform_reply(id, Ok(Value::Null));
        }
        let options = request.input["options"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let anchor = request.input["anchor"].as_u64().and_then(|id| {
            let view = self.builtins.views.get(&request.panel)?;
            let b = view.read(cx).anchor_bounds(id)?;
            Some(pm_shell::layout::Rect {
                x: f32::from(b.origin.x),
                y: f32::from(b.origin.y),
                w: f32::from(b.size.width),
                h: f32::from(b.size.height),
            })
        });
        let owner = (!self.builtins.agent_docked)
            .then(|| self.builtins.windows.get("agent").copied())
            .flatten()
            .and_then(|w| w.update(cx, |_, w, _| crate::platform::ns_window(w)).ok())
            .flatten();
        let choices = options
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let mut c = pm_shell::transient::Choice::new(
                    o["label"].as_str().unwrap_or_default(),
                    format!("agentPick:{i}"),
                );
                if let Some(category) = o["category"].as_str() {
                    c = c.within(category);
                }
                if let Some(on) = o["checked"].as_bool() {
                    c = c.checked(on);
                }
                c
            })
            .collect();
        let values = options.iter().map(|o| o["value"].clone()).collect();
        self.builtins.agent_pick = Some((runtime.clone(), request.id, values));
        self.host.borrow_mut().open_menu(
            owner,
            choices,
            anchor,
            None,
            Some("agentPick:".into()),
            request.input["placeholder"].as_str().unwrap_or("Search"),
        );
    }
    /// The Agent menu's answer: a row's value, or none when it closed unchosen.
    pub fn agent_picked(&mut self, index: &str) {
        if let Some((runtime, id, values)) = self.builtins.agent_pick.take() {
            let value = index
                .parse::<usize>()
                .ok()
                .and_then(|i| values.get(i).cloned())
                .unwrap_or(Value::Null);
            runtime.platform_reply(id, Ok(value));
        }
    }
    /// The font picker's answer: a family, or none when it closed unchosen.
    pub fn font_picked(&mut self, family: &str) {
        if let Some((runtime, id)) = self.builtins.font_pick.take() {
            let value = if family.is_empty() {
                Value::Null
            } else {
                json!(family)
            };
            runtime.platform_reply(id, Ok(value));
        }
    }
    pub fn update_builtins(&mut self, cx: &mut Context<Self>) {
        self.sync_keymap();
        if crate::first_run::take_workspace_request() {
            self.host.borrow().send_command("firstRunWorkspace", vec![]);
        }
        self.start_builtins(cx);
        self.probe_builtin_media(cx);
        if self.builtins.updating {
            self.builtins.again = true;
            return;
        }
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        let facts = self.facts(cx);
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
        let (working, attention, elapsed, background) = runtime.agent_status();
        self.host
            .borrow_mut()
            .set_agent_status(pm_shell::AgentStatus {
                working,
                attention,
                elapsed: elapsed.map(|ms| format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)),
                background,
            });
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
                let wake = self.playback_wake.clone();
                view.update(cx, |v, _| v.installed = wake);
            }
            if id == "inspector"
                && let Some(layer) = self.extensions.focus_text.take()
            {
                view.update(cx, |v, cx| v.focus_text(layer, cx));
            }
            self.builtins.views.insert(id.into(), view);
        }
        let view = self.builtins.views.get(id)?.clone();
        view.update(cx, |v, cx| {
            v.attach(window, cx);
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
            self.builtins.viewer_place = Some(bounds);
            let body_h = (bounds.h - t.metrics.header).max(1.);
            // With Studio 3D views the main viewer is the first pane.
            let split = self.host.borrow().viewer_split.get();
            let (body_w, body_h) = if split > 1 {
                let r = pm_shell::layout::Rect {
                    x: 0.,
                    y: 0.,
                    w: bounds.w,
                    h: body_h,
                };
                let first = pm_shell::layout::view_tiles(r, split, t.metrics.border * 2.)[0];
                (first.w, first.h)
            } else {
                (bounds.w, body_h)
            };
            let stage = [body_w as f64, body_h as f64];
            if self.view.stage != stage {
                // The chrome's Fit readout and zoom steps measure this slot.
                self.view.stage = stage;
                cx.defer_in(window, |v, _, cx| v.update_builtins(cx));
            }
            // Shown rulers take a band along the top and the leading edge;
            // the composition fits inside them (the first pane with 3D views).
            let inset = t.metrics.inset
                + if self.rulers {
                    t.metrics.viewer_ruler
                } else {
                    0.
                };
            let fit = crate::panels::fit_scale(body_w, body_h, self.dimensions, inset);
            // Free View fills the whole viewer body; the shot keeps its fit.
            let free = self
                .builtins
                .gizmos
                .as_ref()
                .is_some_and(|g| g.read(cx).studio_free());
            self.view.fill.set(free);
            let scale = if free {
                crate::panels::cover_scale(body_w, body_h, self.dimensions)
            } else {
                self.view.zoom.unwrap_or(fit).min(self.zoom_limit())
            };
            if self.view.scale.replace(scale) != scale
                && let Some(gizmos) = &self.builtins.gizmos
            {
                gizmos.update(cx, |g, cx| g.redraw(cx));
            }
            self.host.borrow_mut().viewer_transform(
                self.dimensions,
                Some(scale),
                self.view.shown_pan(),
            );
            // A cropped canvas has its edges outside the pane: no outline.
            let cropped = free || self.host.borrow().viewer_crop.get().is_some();
            self.builtins.frame_outline.show(
                &self.host.borrow(),
                !cropped && crate::panels::needs_frame_outline(self.background, t.viewer_surround()),
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
        tabbed: bool,
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
            .or_insert_with(|| cx.new(|cx| crate::v4_view::HeaderView::new(view, cx)))
            .clone();
        header.update(cx, |h, cx| h.set_tabbed(tabbed, cx));
        Some(div().w_full().child(header).into_any_element())
    }
    /// Settings › General › Bring My Workspace…: Settings gives way to the
    /// Agent panel, where a new thread asks for the After Effects workspace.
    pub(crate) fn bring_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            self.message = "Open a project to bring your workspace into".into();
            cx.notify();
            return;
        }
        if let Some(handle) = self.builtins.windows.remove("settings") {
            let _ = handle.update(cx, |_, w, _| w.remove_window());
        }
        self.show_builtin("agent", window, cx);
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        // The panel's backing opens with its window: wait for it off the
        // main thread, then start the thread there too (it saves).
        let started = cx.background_executor().spawn(async move {
            let mut result = runtime.bring_workspace();
            for _ in 0..50 {
                if result.is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
                result = runtime.bring_workspace();
            }
            result
        });
        cx.spawn(async move |this, cx| {
            let result = started.await;
            let _ = this.update(cx, |v, cx| {
                v.message = match result {
                    Ok(true) => "Bringing your After Effects workspace".into(),
                    Ok(false) => {
                        "Connect an agent, then send the request in the Agent panel".into()
                    }
                    Err(_) => "Could not start the workspace transfer. Try again.".into(),
                };
                v.drain_builtins(cx);
                cx.notify();
            });
        })
        .detach();
    }
    /// Whether the Agent popover is showing (or on its way).
    pub fn agent_open(&self, cx: &App) -> bool {
        self.builtins.opening_windows.contains("agent")
            || self
                .builtins
                .windows
                .get("agent")
                .is_some_and(|w| cx.windows().contains(w))
    }
    /// The Agent popover goes away (Escape, a press outside it, the toolbar
    /// item, the panel's own close). Its conversation, draft and runs stay
    /// with the panel's model, so reopening shows them as they were.
    pub fn close_agent(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.builtins.windows.remove("agent") else {
            return;
        };
        let _ = handle.update(cx, |_, w, _| w.remove_window());
        self.update_builtins(cx);
        gpui_macos::child_window::raise_if_active(&self.host.borrow().window);
        cx.notify();
    }
    /// The toolbar's Agent item and View ▸ Agent toggle the popover, as the
    /// original app's launcher does.
    ///
    /// Pinned, the docked panel is hidden or shown in place
    /// (agent-panel.md §1).
    pub fn toggle_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.builtins.agent_docked {
            self.builtins.agent_parked = true;
            self.host.borrow().send_command("panel:park:agent", vec![]);
            return;
        }
        if std::mem::take(&mut self.builtins.agent_parked) {
            self.host.borrow().send_command("panel:show:agent", vec![]);
            return;
        }
        let dismissed = self.builtins.agent_dismissed.take();
        match agent_toggle(self.agent_open(cx), dismissed.map(|at| at.elapsed())) {
            AgentToggle::Close => self.close_agent(cx),
            AgentToggle::Open => self.show_builtin("agent", window, cx),
            AgentToggle::Keep => {}
        }
    }
    /// The popover stopped being the key window. A press in its editor
    /// (anywhere outside the popover) closes it; another window taking key
    /// (Settings, a file picker, another app) leaves it open.
    fn agent_resigned(&mut self, cx: &mut Context<Self>) {
        let editor = self.close_window;
        if agent_dismisses(self.agent_open(cx), editor.is_some() && cx.active_window() == editor) {
            self.close_agent(cx);
            self.builtins.agent_dismissed = Some(std::time::Instant::now());
        }
    }
    pub fn show_builtin(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.start_builtins(cx);
        if id == "settings" {
            // Settings › Media shows the protected folders as they are now,
            // and Settings › Storage what both folders take now.
            self.refresh_waiting_folders(cx);
            self.refresh_storage(cx);
        }
        if self.builtins.opening_windows.contains(id) {
            return;
        }
        if let Some(existing) = self.builtins.windows.get(id).copied()
            && existing.update(cx, |_, w, _| w.activate()).is_ok()
        {
            if id == "settings"
                && let Some(page) = self.builtins.settings_page.take()
                && let Some(runtime) = self.builtins.runtime.clone()
            {
                cx.background_executor()
                    .spawn(async move {
                        if let Ok(ticket) = runtime.show_section("settings", &page) {
                            let _ = ticket.await;
                        }
                    })
                    .detach();
            }
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
                m.agent_height.min(
                    f32::from(window.viewport_size().height) - m.toolbar - m.status - 2. * m.gap,
                ),
                WindowKind::PopUp,
            ),
            _ => (m.sheet_width_wide, m.settings_height, WindowKind::Dialog),
        };
        self.builtins
            .window_sizes
            .insert(id.into(), (width, height));
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
        // The panel's first publication in this window already knows where it is shown.
        let facts = self.facts(cx);
        let page = (id == "settings")
            .then(|| self.builtins.settings_page.take())
            .flatten();
        cx.spawn(async move |this, cx| {
            let told = runtime.clone();
            cx.background_executor()
                .spawn(async move { told.update_facts(facts) })
                .await;
            let shown = match &page {
                Some(page) => runtime.show_section(&id, page),
                None => runtime.show(&id),
            };
            if let Ok(ticket) = shown {
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
            // Settings is laid out as Home is: a sidebar on the window's
            // glass and a content sheet beside it.
            let sidebar = (id == "settings").then(pm_shell::tokens);
            if popover.is_some() || sidebar.as_ref().is_some_and(|t| t.translucent()) {
                // Clear GPUI content over the native material (language §12).
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
                    if let Some(tokens) = &sidebar {
                        pm_shell::sidebar_frame(window, tokens, tokens.metrics.settings_sidebar);
                    }
                    cx.new(|cx| {
                        cx.observe(&owner, |_, _, cx| cx.notify()).detach();
                        let keys = PopoverKeys::new(cx);
                        if panel == "agent" {
                            // Its own keys reach it from the first moment,
                            // before anything inside is clicked.
                            keys.claim(window, cx);
                            // A press anywhere in the editor closes the
                            // popover, as a popover does (the original app's
                            // outside-press dismissal).
                            cx.observe_window_activation(window, |this: &mut SurfaceWindow, window, cx| {
                                if window.is_window_active() {
                                    // Key again (a reopen, a return from
                                    // another window): Escape still reaches
                                    // it with nothing inside focused.
                                    this.keys.claim(window, cx);
                                    return;
                                }
                                let owner = this.owner.clone();
                                cx.spawn(async move |_, cx| {
                                    cx.background_executor()
                                        .timer(std::time::Duration::from_millis(60))
                                        .await;
                                    let _ = owner.update(cx, |v, cx| v.agent_resigned(cx));
                                })
                                .detach();
                            })
                            .detach();
                        }
                        SurfaceWindow { owner, panel, keys }
                    })
                },
            );
            let _ = this.update(cx, |v, cx| {
                v.builtins.opening_windows.remove(&id);
                match result {
                    Ok(handle) => {
                        v.builtins.windows.insert(id, handle.into());
                        // The panel learns where it is shown now (the
                        // popover's header has Pin, the docked one does not).
                        v.update_builtins(cx);
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
    /// A viewer double-click on an imported model's part: Properties opens
    /// and scrolls to that material slot's section (`None` leaves it).
    pub fn focus_part(&mut self, layer: &str, slot: Option<u64>, cx: &mut Context<Self>) {
        let seq = self.part_focus.as_ref().map_or(0, |f| f.2) + 1;
        self.part_focus = slot.map(|slot| (layer.to_owned(), slot, seq));
        self.update_builtins(cx);
        if let Some(slot) = slot
            && let Some(view) = self.builtins.views.get("inspector").cloned()
        {
            view.update(cx, |v, cx| {
                v.reveal_semantic(format!("section:3d.part.{slot}"), cx)
            });
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
            // The Export sheet asks for its 3D estimate from its first
            // snapshot, before the panel is marked ready. The request only
            // asks the viewer to measure and edits nothing, so it never
            // waits on the panel.
            let measure_only = request.operation == "export.estimate";
            if !measure_only
                && (!self.builtins.ready.contains(&request.panel)
                    || self.builtins.generations.get(&request.panel) != Some(&request.generation))
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
                "storage.settings" => {
                    self.storage_settings(&request.input, cx);
                    Ok(Value::Null)
                }
                // The Export sheet's 3D estimate: the viewer measures the
                // shot on its idle worker; the state comes back in the facts.
                "export.estimate" => {
                    let n = |k: &str| request.input[k].as_u64().unwrap_or(0);
                    // Measure the viewer's current scene (its stamp, not the
                    // last drawn frame's, which may already be superseded).
                    if let Some(viewer) = &self.viewer
                        && let Some(stamp) = viewer.stamp()
                        && n("width") > 0
                        && n("height") > 0
                    {
                        let time = viewer.status().frame.map_or(pm_protocol::common::Ticks::ZERO, |f| f.time);
                        if let Err(error) = viewer.estimate_studio_export_scaled(
                            stamp,
                            time,
                            pm_protocol::common::Size::new(n("width") as u32, n("height") as u32),
                            n("frames").max(1),
                            request.input["renderScale"].as_f64().unwrap_or(1.) as f32,
                        ) {
                            eprintln!("Studio export estimate not started: {error}");
                        }
                    }
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
                "agent.media" => {
                    // A file or address a reply names. The reply chose the path,
                    // so the file is resolved and checked off the main thread
                    // (a real image, video or sound, never an app or a link to
                    // one) and opened as a document; a web address goes to the
                    // browser only when it is http(s).
                    let input = request.input.clone();
                    let path = input["path"].as_str().unwrap_or_default().to_owned();
                    let reveal = input["reveal"] == true;
                    if input["remote"] == true {
                        if !(pm_services::surfaces::agent_reply::safe_link(&path)
                            && crate::platform::open_url(&path))
                        {
                            self.errors
                                .borrow_mut()
                                .tell("Agent", "That address couldn’t be opened.");
                        }
                    } else {
                        cx.spawn(async move |this, cx| {
                            let outcome = cx
                                .background_executor()
                                .spawn(async move {
                                    let file = crate::platform::media_document(
                                        std::path::Path::new(&path),
                                    )?;
                                    if reveal {
                                        crate::platform::reveal(&file);
                                    } else if !crate::platform::open_document(&file) {
                                        return Err("No app on this Mac opens that file.");
                                    }
                                    Ok(())
                                })
                                .await;
                            if let Err(message) = outcome {
                                let _ = this.update(cx, |v, cx| {
                                    v.errors.borrow_mut().tell("Agent", message);
                                    cx.notify();
                                });
                            }
                        })
                        .detach();
                    }
                    Ok(Value::Null)
                }
                "agent.close" => {
                    // Docked, Close takes it out of the workspace: the Agent
                    // item opens the popover again (agent-panel.md §1).
                    if self.builtins.agent_docked {
                        self.host.borrow().send_command("panel:close:agent", vec![]);
                    }
                    self.close_agent(cx);
                    Ok(Value::Null)
                }
                // Pin to Workspace: the popover closes and the panel docks
                // beside Properties, saved with the workspace's layout.
                "agent.pin" => {
                    if let Some(handle) = self.builtins.windows.remove("agent") {
                        let _ = handle.update(cx, |_, w, _| w.remove_window());
                    }
                    self.builtins.agent_parked = false;
                    self.host.borrow().send_command("panel:show:agent", vec![]);
                    self.update_builtins(cx);
                    Ok(Value::Null)
                }
                // Delete Thread asks first, as v1's PM.confirm.
                "agent.delete" => {
                    let thread = request.input["thread"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned();
                    let title = request.input["title"].as_str().unwrap_or("this thread");
                    let detail = if request.input["busy"] == true {
                        "This stops its run and removes the thread’s history. This cannot be undone."
                    } else {
                        "This removes the thread’s history. This cannot be undone."
                    };
                    use crate::alert::{Alert, Button};
                    let rx = Alert::new(
                        format!("Delete “{title}”?"),
                        detail,
                        vec![
                            Button::new("Delete Thread", true).default().destructive(),
                            Button::new("Cancel", false).cancel(),
                        ],
                    )
                    .sheet(&self.host.borrow().window);
                    let runtime = runtime.clone();
                    cx.spawn(async move |this, cx| {
                        if rx.recv().await != Ok(Some(true)) {
                            return;
                        }
                        let outcome = cx
                            .background_executor()
                            .spawn(async move { runtime.agent_delete_thread(&thread) })
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
                // Full access asks first (language §9), as v1's consent.
                "agent.consent" => {
                    let effect = request.input["effect"].as_u64().unwrap_or_default();
                    use crate::alert::{Alert, Button};
                    let rx = Alert::new(
                        "Give the agent full access?",
                        "It can run commands and edit files anywhere on this Mac without asking. Choose this only for work you trust.",
                        vec![
                            Button::new("Allow Full Access", true).default(),
                            Button::new("Cancel", false).cancel(),
                        ],
                    )
                    .critical()
                    .sheet(&self.host.borrow().window);
                    let runtime = runtime.clone();
                    cx.spawn(async move |_, cx| {
                        let accepted = rx.recv().await == Ok(Some(true));
                        let _ = cx
                            .background_executor()
                            .spawn(async move { runtime.agent_consent(effect, accepted) })
                            .await;
                    })
                    .detach();
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
                    self.font_menu(&runtime, &request, window, cx);
                    continue;
                }
                "agentMenu" => {
                    self.agent_menu(&runtime, &request, cx);
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
    /// Zooms from the keyboard or a menu: about the point under the pointer
    /// when it is over the viewer, else about the viewer's middle. With
    /// `to`, to that zoom (100 %, 200 %); else by `factor`.
    pub(crate) fn step_zoom(
        &mut self,
        factor: f64,
        to: Option<f64>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let center = self.viewer_center();
        let over = self
            .builtins
            .gizmos
            .as_ref()
            .is_some_and(|g| g.read(cx).pointer_over(window));
        let at = if over {
            window.mouse_position()
        } else {
            center
        };
        let current = self.view.scale.get();
        let factor = to.map_or(factor, |to| to / current.max(f64::MIN_POSITIVE));
        let limit = self.zoom_limit();
        self.view
            .navigate(crate::gizmo::Navigate::Zoom { factor, at }, center, limit);
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
    fn skim(&self, seconds: Option<f64>, cx: &mut App) {
        let input = crate::skimming::input();
        let _ = self.0.update(cx, |v, _| v.skim(seconds, input));
    }
    fn skimming(&self, cx: &App) -> bool {
        crate::skimming::on(&self.0, cx)
    }
    fn skimmer_shown(&self, cx: &App) -> bool {
        self.0
            .upgrade()
            .is_some_and(|c| c.read(cx).skimming.at.is_some())
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
    fn view_moved(&self, cx: &mut App) {
        if let Some(controller) = self.0.upgrade()
            && let Some(wake) = &controller.read(cx).playback_wake
        {
            let _ = wake.try_send(());
        }
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
        crate::timeline_surface::Delegate::run(self, name, vec![], cx);
    }
    fn run(&self, name: &str, args: Vec<Value>, cx: &mut App) {
        if let Some(controller) = self.0.upgrade() {
            controller.read(cx).host.borrow().send_command(name, args);
        }
    }
    fn layer_menu_rows(&self, layer: &str, cx: &App) -> Vec<Value> {
        let Some(controller) = self.0.upgrade() else {
            return vec![];
        };
        let controller = controller.read(cx);
        controller
            .published
            .as_ref()
            .map(|p| crate::layer_commands::layer_menu(p, layer, controller.time()))
            .unwrap_or_default()
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
                px(slot.x + (slot.w - w as f32) / 2. + v.view.shown_pan()[0] as f32),
                px(slot.y + (slot.h - h as f32) / 2. + v.view.shown_pan()[1] as f32),
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
    fn pick_surface(
        &self,
        at: [f32; 2],
        cx: &App,
    ) -> Option<async_channel::Receiver<Result<pm_engine_service::SurfaceReply, String>>> {
        let c = self.0.upgrade()?;
        let viewer = c.read(cx).viewer.as_ref()?;
        let frame = viewer.status().frame?;
        viewer
            .pick_surface(
                frame.stamp,
                frame.time,
                at,
                pm_engine_service::SurfaceQueryKind::Click,
            )
            .ok()
    }
    fn open_composition(&self, comp: String, cx: &mut App) {
        let _ = self
            .0
            .update(cx, |v, cx| v.open_builtin_composition(&comp, cx));
    }
    fn scene3d_assets(&self, cx: &App) -> Option<Arc<dyn pm_scene3d::geometry::AssetGeometry>> {
        let c = self.0.upgrade()?;
        let originals = c.read(cx).media.originals.clone();
        Some(Arc::new(pm_scene3d::geometry::FileAssets::new(
            originals.into_iter().collect(),
        )))
    }
    fn focus_part(&self, layer: &str, slot: Option<u64>, cx: &mut App) {
        let _ = self.0.update(cx, |v, cx| v.focus_part(layer, slot, cx));
    }
    fn studio_view(&self, view: Option<pm_engine_service::studio::FreeView>, cx: &mut App) {
        if let Some(c) = self.0.upgrade()
            && let Some(viewer) = &c.read(cx).viewer
        {
            let _ = viewer.studio_view_at(view, Controller::event_time());
        }
    }
    fn input(&self, at: f64, cx: &mut App) {
        if let Some(c) = self.0.upgrade()
            && let Some(viewer) = &c.read(cx).viewer
        {
            viewer.input_event(at);
        }
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
        // is a tool here, not a command) and what extensions add to it.
        let Some(id) = layer else {
            controller.update(cx, |v, cx| {
                let entries = vec![
                    pm_shell::MenuEntry::submenu(
                        "New",
                        vec![
                            pm_shell::MenuEntry::item("Solid", "newSolid"),
                            pm_shell::MenuEntry::item("Shape Layer", "newShape"),
                            pm_shell::MenuEntry::item("Adjustment Layer", "newAdjustment"),
                            pm_shell::MenuEntry::item("Null Object", "newNull"),
                            pm_shell::MenuEntry::separator(),
                            pm_shell::MenuEntry::item("Composition…", "newComposition"),
                        ],
                    ),
                    // `new3d:<kind>` runs Layer ▸ Add 3D Layer with that kind.
                    pm_shell::MenuEntry::submenu(
                        "Add 3D Layer",
                        pm_shell::menu::new3d()
                            .into_iter()
                            .map(|i| match i.args.first().and_then(|a| a.as_str()) {
                                Some(kind) => {
                                    pm_shell::MenuEntry::item(i.label, format!("new3d:{kind}"))
                                }
                                None => pm_shell::MenuEntry::separator(),
                            })
                            .collect(),
                    ),
                ];
                let locations = vec![("viewer:context", json!({"layerId":null,"time":time}))];
                if let Some(entries) = v.extension_menu(locations, entries, at, cx) {
                    v.host.borrow().context_menu(&entries, at.0, at.1);
                }
            });
            return;
        };
        // The press selects the layer before the menu opens, but that
        // write may not have published yet: the rows are for this layer.
        let rows = controller
            .read(cx)
            .published
            .as_ref()
            .map(|p| crate::layer_commands::layer_menu(p, &id, time))
            .unwrap_or_default();
        let chosen = rows.clone();
        let choose: crate::timeline_surface::Choose = Box::new(move |index, cx| {
            if let (Some((command, args)), Some(controller)) = (
                index.and_then(|i| crate::layer_commands::layer_menu_choice(&chosen, i)),
                host.upgrade(),
            ) {
                controller
                    .read(cx)
                    .host
                    .borrow()
                    .send_command(&command, args);
            }
        });
        if !controller.read(cx).extensions_listen() {
            crate::v4_native::menu(window, &rows, cx, choose);
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
            v.extension_rows_menu(locations, &rows, at, choose, cx)
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
    keys: PopoverKeys,
}
/// A popover's own keys. With nothing focused GPUI delivers a key to the
/// window's root only, so the popover's root holds focus whenever nothing
/// inside does; a focused control inside still sees keys first, and what it
/// leaves bubbles to the root (Escape closes the popover).
pub(crate) struct PopoverKeys {
    pub(crate) focus: FocusHandle,
}
impl PopoverKeys {
    pub(crate) fn new(cx: &mut App) -> Self {
        Self {
            focus: cx.focus_handle(),
        }
    }
    /// Nothing in the window is focused: the popover's root takes focus.
    pub(crate) fn claim(&self, window: &mut Window, cx: &mut App) {
        if window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }
    }
    /// Plain Escape, the popover's dismissal key.
    pub(crate) fn dismisses(e: &KeyDownEvent) -> bool {
        let m = e.keystroke.modifiers;
        e.keystroke.key == "escape" && !(m.platform || m.control || m.alt || m.shift)
    }
}
impl Render for SurfaceWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let panel = self.panel.clone();
        let popover = self.panel == "agent";
        if popover {
            // A focused control that went away leaves nothing focused.
            self.keys.claim(window, cx);
        }
        // Export is a sheet: no title bar band (its window fits its content).
        let sheet = self.panel == "export";
        // Settings draws its own sheet beside a sidebar on the window's
        // glass: the window paints only the ground around the sheet.
        let sidebar = self.panel == "settings";
        let (mut fill, mut band) = (0, 0.);
        let mut ground = None;
        let (head, body) = self.owner.update(cx, |v, cx| {
            let (header, colour, titlebar) = {
                let t = &v.host.borrow().tokens;
                if sidebar {
                    ground = Some((t.ground(), t.metrics.settings_sidebar));
                }
                // The popover's content, its header too, is on solid
                // `floating` inside the glass frame (agent-panel.md §8).
                let colour = if popover {
                    t.colors.floating
                } else {
                    t.colors.panel
                };
                (t.metrics.header, colour, t.metrics.titlebar)
            };
            fill = colour;
            // Windows with a transparent title bar keep their content below
            // it; Settings keeps only its sidebar below the window buttons.
            band = if popover || sheet || sidebar {
                0.
            } else {
                titlebar
            };
            // A panel's header row is the dock's; the popover has no dock, so
            // it shows the panel's header (Thread, New Thread, Pin) itself.
            let rect = pm_shell::layout::Rect {
                x: 0.,
                y: 0.,
                w: f32::from(size.width),
                h: f32::from(size.height) + if popover { 0. } else { header } - band,
            };
            let head = popover
                .then(|| v.builtin_header(&panel, rect, false, window, cx))
                .flatten();
            (head, v.builtin_panel(&panel, rect, window, cx))
        });
        // The agent popover's frame is native glass with its content on
        // solid `floating`; other panel windows paint the panel colour.
        div()
            .when(popover, |d| {
                // Escape closes the popover from anywhere in it; a control
                // that uses Escape itself (a menu, the composer's shortcuts)
                // handles it first. Deferred: this window is the one going.
                d.track_focus(&self.keys.focus)
                .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                    if PopoverKeys::dismisses(e) {
                        cx.stop_propagation();
                        let owner = this.owner.clone();
                        cx.defer(move |cx| {
                            let _ = owner.update(cx, |v, cx| v.close_agent(cx));
                        });
                    }
                }))
            })
            .size_full()
            .overflow_hidden()
            .pt(px(band))
            .when(!sidebar, |d| d.bg(rgb(fill)))
            .when_some(ground, |d, ((colour, alpha), side)| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .left(px(side))
                        .bg(gpui::rgba((colour << 8) | (alpha * 255.).round() as u32)),
                )
            })
            .children(head)
            .children(body)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AgentToggle {
    Open,
    Close,
    /// The press that toggles already closed it from outside.
    Keep,
}
/// The toolbar's Agent item: an open popover closes; a closed one opens,
/// unless the same press just closed it (the item lies outside the popover).
fn agent_toggle(open: bool, since_dismissed: Option<std::time::Duration>) -> AgentToggle {
    if open {
        AgentToggle::Close
    } else if since_dismissed.is_some_and(|d| d < AGENT_PRESS) {
        AgentToggle::Keep
    } else {
        AgentToggle::Open
    }
}
/// The popover gave up key: it closes when its own editor took it (a press
/// outside the popover), never for another window or app.
fn agent_dismisses(open: bool, editor_took_key: bool) -> bool {
    open && editor_took_key
}
#[cfg(test)]
mod agent_close_tests {
    use super::*;
    use std::time::Duration;
    /// A popover's root as SurfaceWindow draws it: its keys, a control inside.
    struct Popover {
        keys: PopoverKeys,
        inner: FocusHandle,
        dismissed: usize,
    }
    impl Render for Popover {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.keys.claim(window, cx);
            div()
                .track_focus(&self.keys.focus)
                .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                    if PopoverKeys::dismisses(e) {
                        this.dismissed += 1;
                        cx.stop_propagation();
                    }
                }))
                .size_full()
                .child(div().track_focus(&self.inner).size_full())
        }
    }
    #[gpui::test]
    fn escape_dismisses_a_reopened_popover_with_nothing_inside_focused(cx: &mut TestAppContext) {
        // The reported path: reopened, nothing clicked, Escape.
        let (view, vcx) = cx.add_window_view(|_, cx| Popover {
            keys: PopoverKeys::new(cx),
            inner: cx.focus_handle(),
            dismissed: 0,
        });
        let mut vcx = vcx.clone();
        vcx.run_until_parked();
        vcx.simulate_keystrokes("escape");
        assert_eq!(view.read_with(&vcx, |p, _| p.dismissed), 1);
        // A focused control inside: Escape it leaves still bubbles up.
        view.update_in(&mut vcx, |p, window, cx| window.focus(&p.inner, cx));
        vcx.simulate_keystrokes("escape");
        assert_eq!(view.read_with(&vcx, |p, _| p.dismissed), 2);
        // Modified Escape is not the dismissal key.
        vcx.simulate_keystrokes("shift-escape");
        assert_eq!(view.read_with(&vcx, |p, _| p.dismissed), 2);
    }
    #[::core::prelude::v1::test]
    fn the_toolbar_item_toggles_the_agent_popover_closed_and_open() {
        // The reported bug: the item only ever opened (activated) it.
        assert_eq!(agent_toggle(true, None), AgentToggle::Close);
        assert_eq!(agent_toggle(false, None), AgentToggle::Open);
        // The press on the item closed it from outside first: stay closed.
        assert_eq!(agent_toggle(false, Some(Duration::from_millis(80))), AgentToggle::Keep);
        // Long after an outside press, the item opens it again.
        assert_eq!(agent_toggle(false, Some(Duration::from_secs(5))), AgentToggle::Open);
    }
    #[::core::prelude::v1::test]
    fn a_press_in_the_editor_closes_the_popover_but_another_window_does_not() {
        assert!(agent_dismisses(true, true));
        assert!(!agent_dismisses(true, false));
        assert!(!agent_dismisses(false, true));
    }
}
/// How long after a press outside the Agent popover closed it a toolbar
/// toggle from that same press is still the press (mouse down to mouse up).
const AGENT_PRESS: std::time::Duration = std::time::Duration::from_millis(600);
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
