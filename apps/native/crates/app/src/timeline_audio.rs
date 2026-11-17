//! Audio-only decorations and gestures; the output callback remains the meter source.
use super::*;
use pm_services::surface_backing::mixer::Meter;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct AudioState {
    drag: Option<Drag>,
    base: Option<Arc<Frame>>,
    ops: Vec<Op>,
    meters: [Meter; 2],
    sampled: Option<Instant>,
    stamp: Option<i64>,
    heard: Option<Instant>,
    painted: Option<Instant>,
    running: bool,
}
struct AudioFrame {
    viewport: Viewport,
    ops: Vec<Op>,
}
fn audio_geometry(base: &Frame) -> AudioFrame {
    AudioFrame {
        viewport: base.viewport,
        ops: base
            .ops
            .iter()
            .filter(|o| o.id.starts_with("clip:") || o.shape == "waveform")
            .map(|o| {
                let mut geometry = op(o.id.clone(), o.bounds, o.color);
                geometry.shape = o.shape;
                geometry.clip = o.clip;
                geometry
            })
            .collect(),
    }
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
        self.rows = None;
        cx.notify();
        true
    }
    pub(super) fn audio_overlay(&self, cx: &Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let tokens = self.tokens.clone();
        let region = self.graph_region();
        canvas(
            move |_, _, cx| {
                weak.read_with(cx, |s, _| {
                    s.audio
                        .ops
                        .iter()
                        .filter(|o| {
                            !o.id.starts_with("audio-level:") && !o.id.starts_with("audio-fade:")
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
            },
            move |bounds, ops, window, cx| {
                let cull = Cull::new(bounds.size);
                let above = region.map(|r| {
                    Bounds::from_corners(
                        bounds.origin,
                        point(bounds.right(), bounds.origin.y + px(r.y as f32)),
                    )
                });
                paint_ops(
                    &ops,
                    bounds.origin,
                    0.,
                    above,
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
    pub(super) fn update_audio(&mut self, base: Arc<Frame>, cx: &mut Context<Self>) {
        // Copy geometry only: never clone waveform JSON or key arrays for meter ticks.
        let mut frame = audio_geometry(&base);
        if self
            .audio
            .base
            .as_ref()
            .is_some_and(|previous| Arc::ptr_eq(previous, &base))
        {
            frame.ops.extend(
                self.audio
                    .ops
                    .iter()
                    .filter(|o| o.id.starts_with("audio-level:") || o.id.starts_with("audio-fade:"))
                    .cloned(),
            );
        }
        self.audio_frame(&mut frame, cx);
        self.audio.ops = frame
            .ops
            .into_iter()
            .filter(|o| o.id.starts_with("audio-"))
            .collect();
        self.audio.base = Some(base);
    }
    fn audio_controls(&self, frame: &mut AudioFrame) {
        if !frame.ops.iter().any(|o| o.id.starts_with("audio-level:"))
            && frame.ops.iter().any(|o| o.shape == "waveform")
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
                            .filter(|v| v.is_finite())
                            .unwrap_or(fallback)
                    };
                let animated_gain = layer.channels.iter().any(|channel| {
                    channel.path == gain_path
                        && (!channel.keys.is_empty() || channel.expression.is_some())
                });
                let base_gain = value(&mut q, gain_path, layer.from, 1.);
                let dragged = self
                    .audio
                    .drag
                    .as_ref()
                    .filter(|d| d.layer == id && d.path == gain_path)
                    .map(|d| d.value);
                let fade_in = value(&mut q, "c.fadeIn", layer.from, 0.).clamp(0., layer.duration);
                let fade_out = value(&mut q, "c.fadeOut", layer.from, 0.).clamp(0., layer.duration);
                let fade_in = self
                    .audio
                    .drag
                    .as_ref()
                    .filter(|d| d.layer == id && d.path == "c.fadeIn")
                    .map_or(fade_in, |d| d.value);
                let fade_out = self
                    .audio
                    .drag
                    .as_ref()
                    .filter(|d| d.layer == id && d.path == "c.fadeOut")
                    .map_or(fade_out, |d| d.value);
                let x0 = r.x.max(frame.viewport.layer_column);
                let x1 = (r.x + r.width).min(frame.viewport.width);
                for x in (x0.ceil() as i64)..(x1.ceil() as i64) {
                    let local = ((x as f64 - frame.viewport.layer_column) / frame.viewport.pps
                        + frame.viewport.scroll_time
                        - layer.from)
                        .clamp(0., layer.duration);
                    let gain = dragged.unwrap_or_else(|| {
                        if animated_gain {
                            value(&mut q, gain_path, layer.from + local, base_gain)
                        } else {
                            base_gain
                        }
                    });
                    let fade = if fade_in > 0. {
                        (local / fade_in).min(1.)
                    } else {
                        1.
                    };
                    let fade = fade.min(if fade_out > 0. {
                        ((layer.duration - local) / fade_out).min(1.)
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
    }
    pub(super) fn audio_rows(&self, frame: &mut Frame) {
        let mut audio = audio_geometry(frame);
        self.audio_controls(&mut audio);
        frame.ops.extend(
            audio
                .ops
                .into_iter()
                .filter(|o| o.id.starts_with("audio-level:") || o.id.starts_with("audio-fade:")),
        );
    }
    fn audio_frame(&mut self, frame: &mut AudioFrame, cx: &mut Context<Self>) {
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
        self.audio_controls(frame);
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
            let mut background = op(
                "audio-readout-bg".into(),
                Rect::new(d.x + 6., d.y - 24., 94., 20.),
                self.tokens.colors.floating,
            );
            background.radius = 3.;
            frame.ops.push(background);
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
        if self.graph_region().is_some_and(|r| y >= r.y) {
            return false;
        }
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
                    && o.clip.is_none_or(|c| {
                        x >= c.x && x <= c.x + c.width && y >= c.y && y <= c.y + c.height
                    })
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
        if event.modifiers.platform && matches!(path, "c.gain" | "c.audioGain") {
            let at = layer.from + frame_time(&self.model, at - layer.from);
            let gain = self
                .kernel
                .published(&self.project)
                .and_then(|published| {
                    QueryMirror::new(&published)
                        .queries()
                        .evaluate(self.model.index.comp_id.as_deref(), id, path, at)
                        .ok()
                        .and_then(|v| v.as_f64())
                })
                .unwrap_or(gain);
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
            let seconds = (x - d.x) / self.view.pps;
            (d.initial
                + if d.path == "c.fadeIn" {
                    seconds
                } else {
                    -seconds
                })
            .clamp(0., d.duration)
        } else {
            dragged_gain(d.initial, y - d.y, event.modifiers.shift)
        };
        self.audio.base = None;
        self.rows = None;
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
        self.rows = None;
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
    /// A real Playback on a simulated device supplies output-mix meter taps.
    #[test]
    #[ignore = "writes headless audio playback review crops; set PM_AUDIO_CAPTURE and PM_AUDIO_CAPTURE_SOURCE"]
    fn capture_timeline_audio_light_dark() {
        let dir = std::env::var("PM_AUDIO_CAPTURE").unwrap();
        let source = PathBuf::from(std::env::var("PM_AUDIO_CAPTURE_SOURCE").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let project = r##"{"id":"integration","name":"Audio review","compId":"c","w":640,"h":360,"fps":30,"dur":8,"bg":"#000000","assets":{"music":{"kind":"audio","dur":8}},"comps":{},"layers":[{"id":"audio","name":"Music bed","type":"audio","from":0,"dur":8,"on":true,"d":{"asset":"music","gain":{"v":0.5,"kf":[{"i":"gain-0","t":0,"v":0.5},{"i":"gain-4","t":4,"v":0.8},{"i":"gain-8","t":8,"v":0.5}],"expr":null},"fadeIn":1,"fadeOut":1.5},"p":{},"fx":[]}]}"##;
        for (name, scheme) in [
            ("light", pm_design::Scheme::Light),
            ("dark", pm_design::Scheme::Dark),
        ] {
            objc2::rc::autoreleasepool(|_| {
                let rig = rig_project("timeline", project);
                let first = publication(&rig);
                let natives = Natives::default();
                let recorder = Rc::new(crate::timeline_tests::Recorder::default());
                let delegate = Rc::new(crate::timeline_tests::Recording(recorder.clone()));
                let cache = pm_audio::SourceCache::new(8 << 20, 2);
                let (playback, mut device) = pm_audio::playback::Playback::start_headless(
                    pm_audio::MixFormat::stereo(48000),
                    cache.clone(),
                    pm_audio::playback::PlaybackConfig::default(),
                    48000,
                    2,
                    0,
                )
                .unwrap();
                let doc = pm_model::Document::new(Some(pm_model::json::parse(project).unwrap()), 1);
                let resolver = |_: &str| Some(source.clone());
                assert!(
                    playback
                        .set_project(Arc::new(doc), "c", &resolver)
                        .unwrap()
                        .is_empty()
                );
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
                let mut captured_surface = None;
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
                        captured_surface = Some(surface.clone());
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
                        let outline = natives.outline.clone();
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
                        outline.bind(view.downgrade());
                        cx.new(|_| CaptureFrame {
                            view,
                            fill: tokens.colors.panel,
                        })
                    })
                    .unwrap();
                playback
                    .command(pm_protocol::clock::TransportCommand::Play {
                        rate: pm_protocol::common::PlaybackRate::NORMAL,
                    })
                    .unwrap();
                let surface = captured_surface.unwrap();
                let mut output = vec![0f32; 960 * 2];
                for _ in 0..100 {
                    device.callback(&mut output, 2, pm_audio::clock::host_now_us() + 20_000);
                    if let Some(tap) = playback.meters() {
                        let levels = tap.to_levels(48000);
                        *recorder.audio.borrow_mut() = Some((
                            std::array::from_fn(|i| {
                                levels.master.peak.get(i).copied().unwrap_or(0.) as f64
                            }),
                            levels.media_time.get(),
                            true,
                        ));
                    }
                    cx.update(|cx| surface.update(cx, |_, cx| cx.notify()));
                    cx.run_until_parked();
                    std::thread::sleep(Duration::from_millis(20));
                }
                cx.update(|cx| {
                    let s = surface.read(cx);
                    assert!(
                        s.frame
                            .as_ref()
                            .unwrap()
                            .ops
                            .iter()
                            .any(|o| o.shape == "waveform"
                                && o.detail["bars"].as_array().is_some_and(|bars| bars
                                    .iter()
                                    .any(|v| v.as_f64().unwrap_or(0.) > 0.)))
                    );
                    assert!(s.audio.ops.iter().any(|o| o.id == "audio-level:audio"));
                    let gains: Vec<f64> = s
                        .audio
                        .ops
                        .iter()
                        .filter(|o| o.id == "audio-level:audio")
                        .filter_map(|o| o.detail["gain"].as_f64())
                        .collect();
                    assert!(
                        gains.iter().any(|g| *g > 0.75) && gains.iter().any(|g| *g < 0.55),
                        "gain curve did not follow its keys"
                    );
                    assert_eq!(
                        s.audio
                            .ops
                            .iter()
                            .filter(|o| o.id.starts_with("audio-fade:"))
                            .count(),
                        2
                    );
                    assert!(
                        s.audio
                            .ops
                            .iter()
                            .any(|o| o.id == "audio-meter" && o.bounds.width > 0.)
                    );
                });
                cx.capture_screenshot(window.into())
                    .unwrap()
                    .save(format!("{dir}/audio-{name}.png"))
                    .unwrap();
                drop(device);
                drop(playback);
                cache.shutdown();
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
    fn stage_audio(cx: &mut TestAppContext) -> crate::timeline_tests::Stage {
        let mut project: serde_json::Value =
            serde_json::from_str(&crate::v4_view_tests::project_json(1)).unwrap();
        project["layers"][0]["id"] = serde_json::json!("audio");
        project["layers"][0]["type"] = serde_json::json!("audio");
        project["layers"][0]["dur"] = serde_json::json!(8);
        project["layers"][0]["d"] = serde_json::json!({"gain":1,"fadeIn":0,"fadeOut":0});
        crate::timeline_tests::stage_project(cx, project.to_string())
    }
    fn gain_value(s: &crate::timeline_tests::Stage, path: &str) -> f64 {
        let p = s
            .rig
            .kernel
            .published(crate::v4_view_tests::PROJECT)
            .unwrap();
        QueryMirror::new(&p)
            .queries()
            .evaluate(None, "audio", path, 0.)
            .unwrap()
            .as_f64()
            .unwrap()
    }
    fn gain_key_count(s: &crate::timeline_tests::Stage) -> usize {
        let p = s
            .rig
            .kernel
            .published(crate::v4_view_tests::PROJECT)
            .unwrap();
        let root: serde_json::Value = serde_json::from_str(&pm_model::json::to_canonical_string(
            &p.snapshot.root().unwrap().to_value(),
        ))
        .unwrap();
        root["layers"][0]["d"]["gain"]["kf"]
            .as_array()
            .map_or(0, Vec::len)
    }
    #[gpui::test]
    fn command_click_on_level_line_adds_one_gain_key(cx: &mut TestAppContext) {
        let mut s = stage_audio(cx);
        let from = s.origin() + point(px(600.), px(50.));
        s.surface.update(&mut s.vcx, |surface, cx| {
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
                    modifiers: Modifiers {
                        platform: true,
                        ..Modifiers::none()
                    },
                    click_count: 1,
                    first_mouse: false
                },
                cx
            ));
        });
        s.idle(cx);
        assert_eq!(gain_key_count(&s), 1);
        assert_eq!(gain_value(&s, "c.gain"), 1.);
        s.undo(cx);
        assert_eq!(gain_value(&s, "c.gain"), 1.);
        assert_eq!(gain_key_count(&s), 0);
    }
    #[gpui::test]
    fn fade_handle_drag_sets_duration_on_release_and_undoes_once(cx: &mut TestAppContext) {
        for (path, dx) in [("c.fadeIn", 100.), ("c.fadeOut", -100.)] {
            let mut s = stage_audio(cx);
            let from = s.origin() + point(px(600.), px(50.));
            let expected = s
                .surface
                .read_with(&s.vcx, |surface, _| 100. / surface.view.pps);
            s.surface.update(&mut s.vcx, |surface, cx| {
                let mut handle = op(
                    "audio-fade:audio".into(),
                    Rect::new(599., 49., 3., 3.),
                    0xffffff,
                );
                handle.detail = serde_json::json!({"layer":"audio","path":path,"gain":0.});
                surface.audio.ops.push(handle);
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
                        position: from + point(px(dx), px(0.)),
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Modifiers::none()
                    },
                    cx
                ));
            });
            assert_eq!(gain_value(&s, path), 0.);
            s.surface.update(&mut s.vcx, |surface, cx| {
                assert!(surface.audio_up(cx));
            });
            s.idle(cx);
            assert!((gain_value(&s, path) - expected).abs() < 1e-9);
            s.undo(cx);
            assert_eq!(gain_value(&s, path), 0.);
        }
    }
}

#[cfg(test)]
mod mix_tap_tests {
    use super::*;
    #[test]
    fn output_mix_tap_reads_full_scale_and_half_scale_test_tone() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("tone.wav");
        let mut writer = pm_audio::offline::WavWriter::new(
            std::io::BufWriter::new(std::fs::File::create(&source).unwrap()),
            pm_audio::offline::PcmFormat::F32,
            2,
            48000,
        )
        .unwrap();
        let mut pcm = Vec::new();
        for n in 0..48000 {
            let sample = (std::f32::consts::TAU * (n % 48) as f32 / 48.).sin();
            pcm.extend([sample, sample]);
        }
        writer.write(0, &pcm).unwrap();
        writer.finish().unwrap();
        for (gain, expected) in [(1., 0.), (0.5, -6.020599913)] {
            let text = format!(
                r##"{{"id":"P","compId":"C","w":640,"h":360,"fps":30,"dur":1,"audioGain":{gain},"assets":{{}},"comps":{{}},"layers":[{{"id":"a","type":"audio","from":0,"dur":1,"on":true,"d":{{"asset":"tone","gain":1}},"p":{{}},"fx":[]}}]}}"##
            );
            let doc = pm_model::Document::new(Some(pm_model::json::parse(&text).unwrap()), 1);
            let cache = pm_audio::SourceCache::new(8 << 20, 0);
            let resolver = |_: &str| Some(source.clone());
            let mut evaluator = pm_eval::Evaluator::new();
            let graph =
                pm_audio::MixGraph::build(&doc, "C", &mut evaluator, &cache, &resolver).unwrap();
            let mut mixer = pm_audio::Mixer::new(
                pm_audio::MixFormat::stereo(48000),
                Arc::new(doc),
                Arc::new(graph),
                evaluator,
                cache.clone(),
            );
            let mut out = vec![Vec::new(), Vec::new()];
            mixer
                .render(
                    4800,
                    4800,
                    &mut out,
                    &pm_audio::RenderOptions {
                        wait: true,
                        meters: true,
                        ..Default::default()
                    },
                )
                .unwrap();
            let levels = mixer.meters(9600).to_levels(48000);
            for peak in levels.master.peak {
                let mut meter = Meter::default();
                meter.step(peak as f64, 0., 0.033).unwrap();
                assert!((meter.peak_db - expected).abs() < 0.001);
            }
            drop(mixer);
            cache.shutdown();
        }
    }
}
