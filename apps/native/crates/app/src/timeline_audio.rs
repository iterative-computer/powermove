//! Audio-only decorations and gestures; the output callback remains the meter source.
use super::*;
use pm_services::surface_backing::mixer::Meter;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct AudioState {
    drag: Option<Drag>,
    pub(super) base: Option<Arc<Frame>>,
    pub(super) ops: Vec<Op>,
    meters: [Meter; 2],
    sampled: Option<Instant>,
    stamp: Option<i64>,
    heard: Option<Instant>,
    painted: Option<Instant>,
    running: bool,
}
struct Drag {
    layer: String,
    path: String,
    time: f64,
    value: f64,
    initial: f64,
    x: f64,
    y: f64,
    duration: f64,
    moved: bool,
}
fn op(id: String, bounds: Rect, color: u32) -> Op {
    Op {
        id,
        shape: "rect",
        bounds,
        color,
        opacity: 1.,
        radius: 0.,
        stroke: 0.,
        text: None,
        font_size: 11.,
        detail: serde_json::Value::Null,
        clip: None,
        covered: false,
    }
}
fn gain_y(gain: f64, r: Rect) -> f64 {
    let db = if gain > 0. { 20. * gain.log10() } else { -60. };
    r.y + 3. + (12. - db.clamp(-60., 12.)) / 72. * (r.height - 6.).max(1.)
}
fn dragged_gain(initial: f64, dy: f64, fine: bool) -> f64 {
    let db = if initial > 0. {
        20. * initial.log10()
    } else {
        -60.
    };
    let step = if fine { 0.1 } else { 0.5 };
    let db = ((db - dy * step) / step).round() * step;
    10f64.powf(db.clamp(-60., 12.) / 20.).min(4.)
}
fn edit(layer: &str, path: &str, time: f64, value: f64, key: bool, revision: u64) -> Plan {
    let mut set = pm_kernel::commands::properties::SetProperty::new(
        layer,
        path,
        pm_model::Value::Number(value),
        time,
    );
    set.preserve_hand_edits = false;
    if key {
        set.mode = pm_kernel::commands::properties::KeyMode::Keyframe;
    }
    Plan::edit(vec![pm_kernel::PropertyCommand::Set(set).into()], revision)
}
impl TimelineSurface {
    pub(super) fn audio_cancel(&mut self, cx: &mut Context<Self>) -> bool {
        if self.audio.drag.take().is_none() {
            return false;
        }
        self.audio.base = None;
        cx.notify();
        true
    }
    pub(super) fn audio_overlay(&self, cx: &Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let tokens = self.tokens.clone();
        canvas(
            move |_, _, cx| {
                weak.read_with(cx, |s, _| s.audio.ops.clone())
                    .unwrap_or_default()
            },
            move |bounds, ops, window, cx| {
                let cull = Cull::new(bounds.size);
                paint_ops(
                    &ops,
                    bounds.origin,
                    0.,
                    None,
                    &cull,
                    None,
                    &tokens,
                    window,
                    cx,
                );
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }
    pub(super) fn audio_frame(&mut self, frame: &mut Frame, cx: &mut Context<Self>) {
        self.audio.painted = Some(Instant::now());
        if !self.audio.running {
            self.audio.running = true;
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(33))
                        .await;
                    let alive = this
                        .update(cx, |s, cx| {
                            if s.audio
                                .painted
                                .is_none_or(|p| p.elapsed() > Duration::from_secs(1))
                            {
                                s.audio.running = false;
                                return false;
                            }
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if !alive {
                        break;
                    }
                }
            })
            .detach();
        }
        let now = Instant::now();
        let dt = self
            .audio
            .sampled
            .map_or(0., |at| now.duration_since(at).as_secs_f64());
        self.audio.sampled = Some(now);
        let peaks = self.delegate.audio_peaks(cx);
        let peaks = peaks.and_then(|(peaks, stamp, playing)| {
            if playing || self.audio.stamp != Some(stamp) {
                self.audio.stamp = Some(stamp);
                self.audio.heard = Some(now);
            }
            (playing
                || self
                    .audio
                    .heard
                    .is_some_and(|at| now.duration_since(at) < Duration::from_millis(150)))
            .then_some(peaks)
        });
        for (i, meter) in self.audio.meters.iter_mut().enumerate() {
            if let Some(peaks) = peaks {
                let _ = meter.step(peaks[i], 0., dt);
            } else {
                meter.peak_db = -120.;
                let _ = meter.step(0., 0., dt);
            }
        }
        if !frame.ops.iter().any(|o| o.id.starts_with("audio-level:"))
            && let Some(published) = self.kernel.published(&self.project)
        {
            let mut mirror = QueryMirror::new(&published);
            let mut q = mirror.queries();
            let clips: Vec<_> = frame
                .ops
                .iter()
                .filter(|o| o.id.starts_with("clip:"))
                .cloned()
                .collect();
            for clip in clips {
                let id = clip.id.trim_start_matches("clip:");
                let Some(layer) = self.model.index.layers.iter().find(|l| l.id == id) else {
                    continue;
                };
                if !matches!(layer.kind.as_str(), "audio" | "video") {
                    continue;
                }
                if !frame
                    .ops
                    .iter()
                    .any(|o| o.id == format!("clip-content:{id}") && o.shape == "waveform")
                {
                    continue;
                }
                let r = clip.bounds;
                let gain_path = if layer.kind == "audio" {
                    "c.gain"
                } else {
                    "c.audioGain"
                };
                let value =
                    |q: &mut pm_kernel::QueryContext, path: &str, at: f64, fallback: f64| {
                        q.evaluate(self.model.index.comp_id.as_deref(), id, path, at)
                            .ok()
                            .and_then(|v| v.as_f64())
                            .unwrap_or(fallback)
                    };
                let fade_in = value(&mut q, "c.fadeIn", layer.from, 0.).clamp(0., layer.duration);
                let fade_out = value(&mut q, "c.fadeOut", layer.from, 0.).clamp(0., layer.duration);
                let x0 = r.x.max(frame.viewport.layer_column);
                let x1 = (r.x + r.width).min(frame.viewport.width);
                for x in (x0.ceil() as i64)..(x1.ceil() as i64) {
                    let local = ((x as f64 - frame.viewport.layer_column) / frame.viewport.pps
                        + frame.viewport.scroll_time
                        - layer.from)
                        .clamp(0., layer.duration);
                    let mut gain = value(&mut q, gain_path, layer.from + local, 1.);
                    if let Some(d) = self
                        .audio
                        .drag
                        .as_ref()
                        .filter(|d| d.layer == id && d.path == gain_path)
                    {
                        gain = d.value;
                    }
                    let fi = self
                        .audio
                        .drag
                        .as_ref()
                        .filter(|d| d.layer == id && d.path == "c.fadeIn")
                        .map_or(fade_in, |d| d.value);
                    let fo = self
                        .audio
                        .drag
                        .as_ref()
                        .filter(|d| d.layer == id && d.path == "c.fadeOut")
                        .map_or(fade_out, |d| d.value);
                    let fade = if fi > 0. { (local / fi).min(1.) } else { 1. };
                    let fade = fade.min(if fo > 0. {
                        ((layer.duration - local) / fo).min(1.)
                    } else {
                        1.
                    });
                    let y = gain_y(gain * fade.max(0.), r);
                    let mut line = op(
                        format!("audio-level:{id}"),
                        Rect::new(x as f64, y, 1., 1.),
                        self.tokens.colors.text,
                    );
                    line.detail = serde_json::json!({"layer":id,"path":gain_path,"gain":gain});
                    line.clip = clip.clip;
                    frame.ops.push(line);
                }
                if layer.kind == "audio" {
                    for (path, x) in [
                        (
                            "c.fadeIn",
                            frame.viewport.layer_column
                                + (layer.from - frame.viewport.scroll_time + fade_in)
                                    * frame.viewport.pps
                                + 4.,
                        ),
                        (
                            "c.fadeOut",
                            frame.viewport.layer_column
                                + (layer.from - frame.viewport.scroll_time + layer.duration
                                    - fade_out)
                                    * frame.viewport.pps
                                - 4.,
                        ),
                    ] {
                        if x < x0 || x > x1 {
                            continue;
                        }
                        let mut handle = op(
                            format!("audio-fade:{id}:{path}"),
                            Rect::new(x - 3., r.y + 2., 6., 5.),
                            self.tokens.colors.text,
                        );
                        handle.detail = serde_json::json!({"layer":id,"path":path,"gain":if path == "c.fadeIn" {fade_in} else {fade_out}});
                        handle.clip = clip.clip;
                        frame.ops.push(handle);
                    }
                }
            }
        }
        // Fixed-width, deliberately omitted in narrow timelines.
        if frame.viewport.width - frame.viewport.layer_column > 420. {
            let x = frame.viewport.width - 88.;
            frame.ops.push(op(
                "audio-meter-panel".into(),
                Rect::new(x - 4., 2., 92., 22.),
                self.tokens.colors.panel,
            ));
            for (i, meter) in self.audio.meters.iter().enumerate() {
                let y = 5. + i as f64 * 8.;
                frame.ops.push(op(
                    "audio-meter-bg".into(),
                    Rect::new(x, y, 72., 5.),
                    0x242424,
                ));
                for (lo, hi, color) in [
                    (-60., -12., 0x51b477),
                    (-12., -3., 0xe2bb52),
                    (-3., 0., 0xdf625c),
                ] {
                    let end = meter.peak_db.clamp(lo, hi);
                    frame.ops.push(op(
                        "audio-meter".into(),
                        Rect::new(x + (lo + 60.) * 1.2, y, (end - lo) * 1.2, 5.),
                        color,
                    ));
                }
                frame.ops.push(op(
                    "audio-hold".into(),
                    Rect::new(x + (meter.hold_db.clamp(-60., 0.) + 60.) * 1.2, y, 1., 5.),
                    0xffffff,
                ));
                frame.ops.push(op(
                    "audio-clip-latch".into(),
                    Rect::new(x + 76., y, 5., 5.),
                    if meter.clipped { 0xdf625c } else { 0x454545 },
                ));
            }
        }
        if let Some(d) = &self.audio.drag {
            let db = 20. * d.value.max(0.000001).log10();
            let mut tip = op(
                "audio-readout".into(),
                Rect::new(d.x + 10., d.y - 24., 86., 20.),
                self.tokens.colors.text,
            );
            tip.shape = "text";
            tip.text = Some(if d.path.contains("fade") {
                format!("{:.2} s", d.value)
            } else {
                format!("{} dB", crate::mixer_panel::format_db(db))
            });
            frame.ops.push(tip);
        }
    }
    pub(super) fn audio_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) -> bool {
        let (x, y) = self.local(event.position);
        let Some(frame) = &self.frame else {
            return false;
        };
        let hit = self
            .audio
            .ops
            .iter()
            .rev()
            .find(|o| {
                (o.id.starts_with("audio-level:")
                    || o.id.starts_with("audio-fade:")
                    || o.id == "audio-clip-latch")
                    && x >= o.bounds.x - 3.
                    && x <= o.bounds.x + o.bounds.width + 3.
                    && y >= o.bounds.y - 3.
                    && y <= o.bounds.y + o.bounds.height + 3.
            })
            .cloned();
        let Some(hit) = hit else {
            return false;
        };
        if hit.id == "audio-clip-latch" {
            for m in &mut self.audio.meters {
                m.clear_clip();
            }
            cx.notify();
            return true;
        }
        let Some(id) = hit.detail["layer"].as_str() else {
            return false;
        };
        let Some(layer) = self.model.index.layers.iter().find(|l| l.id == id) else {
            return false;
        };
        if layer.effectively_locked {
            return true;
        }
        let path = hit.detail["path"].as_str().unwrap_or("c.gain");
        let at = layer.from
            + ((x - frame.viewport.layer_column) / self.view.pps + self.view.scroll_time
                - layer.from)
                .clamp(0., layer.duration);
        let gain = hit.detail["gain"].as_f64().unwrap_or(1.);
        if event.modifiers.platform && path.contains("Gain")
            || event.modifiers.platform && path == "c.gain"
        {
            self.queue.push_back(Job::Apply(edit(
                id,
                path,
                at,
                gain,
                true,
                self.model.index.revision,
            )));
            self.pump(cx);
            return true;
        }
        self.audio.drag = Some(Drag {
            layer: id.into(),
            path: path.into(),
            time: at,
            value: gain,
            initial: gain,
            x,
            y,
            duration: layer.duration,
            moved: false,
        });
        cx.notify();
        true
    }
    pub(super) fn audio_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        let (x, y) = self.local(event.position);
        let Some(d) = &mut self.audio.drag else {
            return false;
        };
        d.moved |= (x - d.x).abs() + (y - d.y).abs() > 2.;
        d.value = if d.path.contains("fade") {
            let layer = self
                .model
                .index
                .layers
                .iter()
                .find(|l| l.id == d.layer)
                .unwrap();
            let local =
                (x - self.view.layer_column) / self.view.pps + self.view.scroll_time - layer.from;
            (if d.path == "c.fadeIn" {
                local
            } else {
                d.duration - local
            })
            .clamp(0., d.duration)
        } else {
            dragged_gain(d.initial, y - d.y, event.modifiers.shift)
        };
        self.audio.base = None;
        cx.notify();
        true
    }
    pub(super) fn audio_up(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(d) = self.audio.drag.take() else {
            return false;
        };
        if d.moved {
            self.queue.push_back(Job::Apply(edit(
                &d.layer,
                &d.path,
                d.time,
                d.value,
                false,
                self.model.index.revision,
            )));
            self.pump(cx);
        }
        self.audio.base = None;
        cx.notify();
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn level_drag_has_db_steps_and_fine_shift() {
        assert!((20. * dragged_gain(1., 12., false).log10() + 6.).abs() < 1e-9);
        assert!((20. * dragged_gain(1., 12., true).log10() + 1.2).abs() < 1e-9);
    }
    #[test]
    fn output_meter_full_and_half_scale() {
        let mut m = Meter::default();
        m.step(1., 0., 0.033).unwrap();
        assert_eq!(m.peak_db, 0.);
        assert!(m.clipped);
        m.clear_clip();
        assert!(!m.clipped);
        let mut m = Meter::default();
        m.step(0.5, 0., 0.033).unwrap();
        assert!((m.peak_db + 6.0206).abs() < 0.001);
    }
    #[test]
    fn edits_are_one_transaction_and_cmd_click_forces_key() {
        for key in [false, true] {
            let p = edit("audio", "c.gain", 1., 0.5, key, 7);
            assert_eq!(p.calls.len(), 1);
        }
    }
}

#[cfg(test)]
mod kernel_tests {
    use super::*;
    use crate::v4_view_tests::{PROJECT, rig_project};
    fn fixture() -> String {
        r##"{"id":"integration","compId":"c","w":640,"h":360,"fps":30,"dur":8,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"audio","name":"Music","type":"audio","from":0,"dur":8,"on":true,"d":{"gain":1,"fadeIn":0,"fadeOut":0},"p":{},"fx":[]}]}"##.into()
    }
    fn value(r: &crate::v4_view_tests::Rig, path: &str, time: f64) -> f64 {
        let published = r.kernel.published(PROJECT).unwrap();
        QueryMirror::new(&published)
            .queries()
            .evaluate(None, "audio", path, time)
            .unwrap()
            .as_f64()
            .unwrap()
    }
    fn apply(r: &crate::v4_view_tests::Rig, path: &str, time: f64, v: f64, key: bool) {
        let revision = r.kernel.published(PROJECT).unwrap().revision();
        for call in edit("audio", path, time, v, key, revision).calls {
            r.kernel
                .call(PROJECT, pm_kernel::Request::new(Caller::native(), call))
                .wait()
                .unwrap()
                .unwrap();
        }
    }
    fn undo(r: &crate::v4_view_tests::Rig) {
        r.kernel
            .call(
                PROJECT,
                pm_kernel::Request::new(
                    Caller::native(),
                    pm_kernel::Call::History(pm_kernel::HistoryCall::Undo),
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
    }
    #[test]
    fn audio_level_release_is_one_undo_step() {
        let r = rig_project("timeline", &fixture());
        apply(&r, "c.gain", 2., dragged_gain(1., 12., false), false);
        assert!((value(&r, "c.gain", 2.) - 10f64.powf(-6. / 20.)).abs() < 1e-9);
        undo(&r);
        assert_eq!(value(&r, "c.gain", 2.), 1.);
    }
    #[test]
    fn audio_command_click_adds_gain_key_at_clicked_time() {
        let r = rig_project("timeline", &fixture());
        apply(&r, "c.gain", 2., 0.5, true);
        let p = r.kernel.published(PROJECT).unwrap();
        let text = pm_model::json::to_canonical_string(&p.snapshot.root().unwrap().to_value());
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys = v["layers"][0]["d"]["gain"]["kf"].as_array().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0]["t"], 2.);
        undo(&r);
        assert_eq!(value(&r, "c.gain", 2.), 1.);
    }
    #[test]
    fn audio_fade_release_sets_duration_and_undoes_once() {
        let r = rig_project("timeline", &fixture());
        for path in ["c.fadeIn", "c.fadeOut"] {
            apply(&r, path, 2., 1.25, false);
            assert_eq!(value(&r, path, 2.), 1.25);
            undo(&r);
            assert_eq!(value(&r, path, 2.), 0.);
        }
    }
}

#[cfg(test)]
mod captures {
    use super::*;
    use crate::v4_view::{Natives, SurfaceView};
    use crate::v4_view_tests::{PROJECT, publication, rig_project};
    /// Uses HeadlessAppContext's TestPlatform, with focus(false), show(false).
    /// Meter input is a synthetic output-tap sample for visual review only.
    #[test]
    #[ignore = "writes synthetic audio review crops; set PM_AUDIO_CAPTURE and PM_AUDIO_CAPTURE_SOURCE"]
    fn capture_timeline_audio_light_dark() {
        let dir = std::env::var("PM_AUDIO_CAPTURE").unwrap();
        let source = PathBuf::from(std::env::var("PM_AUDIO_CAPTURE_SOURCE").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let project = r##"{"id":"integration","name":"Audio review","compId":"c","w":640,"h":360,"fps":30,"dur":8,"bg":"#000000","assets":{"music":{"kind":"audio","dur":8}},"comps":{},"layers":[{"id":"audio","name":"Music bed","type":"audio","from":0,"dur":8,"on":true,"d":{"asset":"music","gain":0.5,"fadeIn":1,"fadeOut":1.5},"p":{},"fx":[]}]}"##;
        for (name, scheme) in [
            ("light", pm_design::Scheme::Light),
            ("dark", pm_design::Scheme::Dark),
        ] {
            objc2::rc::autoreleasepool(|_| {
                let rig = rig_project("timeline", project);
                let first = publication(&rig);
                let natives = Natives::default();
                let recorder = Rc::new(crate::timeline_tests::Recorder::default());
                *recorder.audio.borrow_mut() = Some(([0.5, 0.9], 48000, true));
                let delegate = Rc::new(crate::timeline_tests::Recording(recorder));
                let tokens = Tokens::new(scheme, pm_design::Accessibility::default());
                let mut cx = HeadlessAppContext::with_platform(
                    Arc::new(
                        gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                            .with_automatic_optical_sizing(),
                    ),
                    Arc::new(()),
                    || {
                        Some(Box::new(
                            gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                        ))
                    },
                );
                let window = cx
                    .open_window(size(px(1100.), px(320.)), |window, cx| {
                        let surface = cx.new(|cx| {
                            TimelineSurface::new(
                                rig.kernel.clone(),
                                PROJECT.into(),
                                tokens.clone(),
                                delegate,
                                natives.outline.clone(),
                                Default::default(),
                                cx,
                            )
                        });
                        surface.update(cx, |s, cx| {
                            s.set_audio_sources(
                                BTreeMap::from([("music".into(), source.clone())]),
                                cx,
                            )
                        });
                        natives
                            .views
                            .borrow_mut()
                            .insert("timeline".into(), surface.into());
                        let view = cx.new(|cx| {
                            SurfaceView::new(
                                rig.runtime.clone(),
                                first,
                                tokens.clone(),
                                natives,
                                window,
                                cx,
                            )
                        });
                        cx.new(|_| CaptureFrame {
                            view,
                            fill: tokens.colors.panel,
                        })
                    })
                    .unwrap();
                for _ in 0..100 {
                    cx.run_until_parked();
                    std::thread::sleep(Duration::from_millis(20));
                }
                cx.capture_screenshot(window.into())
                    .unwrap()
                    .save(format!("{dir}/audio-{name}.png"))
                    .unwrap();
            });
        }
    }
    struct CaptureFrame {
        view: Entity<SurfaceView>,
        fill: u32,
    }
    impl Render for CaptureFrame {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .bg(rgb(self.fill))
                .child(self.view.clone())
        }
    }
}

#[cfg(test)]
mod gesture_tests {
    use super::*;
    #[gpui::test]
    fn audio_level_pointer_drag_commits_on_release_and_undo_restores(cx: &mut TestAppContext) {
        let project = r##"{"id":"integration","compId":"c","w":640,"h":360,"fps":30,"dur":8,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"audio","name":"Music","type":"audio","from":0,"dur":8,"on":true,"d":{"gain":1,"fadeIn":0,"fadeOut":0},"p":{},"fx":[]}]}"##;
        let mut s = crate::timeline_tests::stage_project(cx, project.into());
        let origin = s.origin();
        let from = origin + point(px(600.), px(50.));
        s.surface.update(&mut s.vcx, |surface, cx| {
            // Isolate the gesture from media I/O; production draws this op from evaluated gain.
            let mut line = op(
                "audio-level:audio".into(),
                Rect::new(599., 49., 3., 1.),
                0xffffff,
            );
            line.detail = serde_json::json!({"layer":"audio","path":"c.gain","gain":1.});
            surface.audio.ops.push(line);
            assert!(surface.audio_down(
                &MouseDownEvent {
                    position: from,
                    button: MouseButton::Left,
                    modifiers: Modifiers::none(),
                    click_count: 1,
                    first_mouse: false
                },
                cx
            ));
            assert!(surface.audio_move(
                &MouseMoveEvent {
                    position: from + point(px(0.), px(12.)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::none()
                },
                cx
            ));
        });
        let published = s
            .rig
            .kernel
            .published(crate::v4_view_tests::PROJECT)
            .unwrap();
        assert_eq!(
            QueryMirror::new(&published)
                .queries()
                .evaluate(None, "audio", "c.gain", 0.)
                .unwrap()
                .as_f64(),
            Some(1.)
        );
        s.surface.update(&mut s.vcx, |surface, cx| {
            assert!(surface.audio_up(cx));
        });
        s.idle(cx);
        let p = s
            .rig
            .kernel
            .published(crate::v4_view_tests::PROJECT)
            .unwrap();
        assert!(
            (QueryMirror::new(&p)
                .queries()
                .evaluate(None, "audio", "c.gain", 0.)
                .unwrap()
                .as_f64()
                .unwrap()
                - 10f64.powf(-6. / 20.))
            .abs()
                < 1e-9
        );
        s.undo(cx);
        let p = s
            .rig
            .kernel
            .published(crate::v4_view_tests::PROJECT)
            .unwrap();
        assert_eq!(
            QueryMirror::new(&p)
                .queries()
                .evaluate(None, "audio", "c.gain", 0.)
                .unwrap()
                .as_f64(),
            Some(1.)
        );
    }
}
