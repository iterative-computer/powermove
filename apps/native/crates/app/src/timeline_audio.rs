//! Audio-only decorations and gestures; the output callback remains the meter source.
use super::*;
use pm_services::surface_backing::mixer::Meter;
use std::time::{Duration, Instant};

#[derive(Default)]
struct TapAge {
    stamp: Option<i64>,
    raw: Option<[f64; 2]>,
    heard: Option<Instant>,
}
impl TapAge {
    fn changed(&self, incoming: Option<([f64; 2], i64, bool)>) -> bool {
        match incoming {
            Some((peaks, stamp, playing)) => {
                playing || self.stamp != Some(stamp) || self.raw != Some(peaks)
            }
            None => self.stamp.is_some() || self.raw.is_some(),
        }
    }
    fn observe(
        &mut self,
        incoming: Option<([f64; 2], i64, bool)>,
        now: Instant,
    ) -> Option<[f64; 2]> {
        let Some((peaks, stamp, playing)) = incoming else {
            *self = Self::default();
            return None;
        };
        // A scrub may repeat at the same media tick. Silence -> sound is fresh too.
        if self.changed(incoming) {
            self.stamp = Some(stamp);
            self.heard = Some(now);
        }
        self.raw = Some(peaks);
        (playing
            || (peaks.iter().any(|v| v.abs() > 0.)
                && self.heard.is_some_and(|at| {
                    now.saturating_duration_since(at) < Duration::from_millis(150)
                })))
        .then_some(peaks)
    }
}
#[derive(Default)]
pub(super) struct AudioState {
    #[cfg(test)]
    baseline_capture: bool,
    drag: Option<Drag>,
    pub(super) pending_gain: Option<u64>,
    base: Option<Arc<Frame>>,
    ops: Vec<Op>,
    meters: [Meter; 2],
    // Font size and measured text width of the clock in the same header.
    clock_width: Option<(f32, f32)>,
    sampled: Option<Instant>,
    tap: TapAge,
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
    gesture: Option<u64>,
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
fn clip_control(op: &Op) -> bool {
    ["audio-level:", "audio-fade:", "audio-key:"]
        .iter()
        .any(|prefix| op.id.starts_with(prefix))
}

fn gain_opacity(gain: f64, animated: bool, interactive: bool) -> Option<f64> {
    if interactive {
        Some(1.)
    } else if animated || (gain - 1.).abs() > 1e-6 {
        Some(0.38)
    } else {
        None
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
/// Content channels use evP; evaluate() addresses only transform properties.
fn content_value(q: &mut pm_kernel::QueryContext, id: &str, path: &str, time: f64) -> Option<f64> {
    let root = q.document().root()?;
    let at = pm_kernel::model::find_layer(root, id)?;
    let layer = pm_kernel::model::layer_at(root, &at)?;
    let raw = layer.get("d")?.get(path.strip_prefix("c.")?)?.to_value();
    if raw.as_f64().is_some() {
        return raw.as_f64();
    }
    let open = pm_kernel::model::open_comp_id(root);
    let mut request = pm_model::Map::new();
    request.insert("op", pm_model::Value::from("evP"));
    request.insert(
        "comp",
        at.comp_id
            .or(open.clone())
            .map_or(pm_model::Value::Null, pm_model::Value::from),
    );
    request.insert(
        "open",
        open.map_or(pm_model::Value::Null, pm_model::Value::from),
    );
    request.insert("layer", pm_model::Value::Number(at.index as f64));
    request.insert("t", pm_model::Value::Number(time));
    request.insert("key", pm_model::Value::from(path));
    request.insert("prop", raw);
    q.query(&pm_model::Value::Object(request)).ok()?.as_f64()
}
fn level_command(layer: &str, path: &str, time: f64, value: f64, key: bool) -> pm_kernel::Command {
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
    pm_kernel::PropertyCommand::Set(set).into()
}
fn edit(layer: &str, path: &str, time: f64, value: f64, key: bool, revision: u64) -> Plan {
    Plan::edit(vec![level_command(layer, path, time, value, key)], revision)
}
impl TimelineSurface {
    pub(super) fn invalidate_audio(&mut self) {
        self.audio.base = None;
        self.rows = None;
    }

    pub(super) fn audio_cancel(&mut self, cx: &mut Context<Self>) -> bool {
        if self.audio.drag.is_none() {
            return false;
        }
        self.release_gain_preview(cx);
        self.audio.drag = None;
        self.audio.base = None;
        self.rows = None;
        cx.notify();
        true
    }
    pub(super) fn release_gain_preview(&mut self, cx: &mut App) {
        if let Some(gesture) = self
            .audio
            .drag
            .as_mut()
            .and_then(|drag| drag.gesture.take())
        {
            let delegate = self.delegate.clone();
            cx.defer(move |cx| {
                delegate.audio_gain_preview(
                    GainPreview::Finish {
                        gesture,
                        revision: None,
                    },
                    cx,
                )
            });
        }
    }
    pub(super) fn audio_overlay(&self, cx: &Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let tokens = self.tokens.clone();
        let region = self.graph_region();
        canvas(
            move |_, _, cx| {
                weak.read_with(cx, |s, _| {
                    #[cfg(test)]
                    if s.audio.baseline_capture {
                        return Vec::new();
                    }
                    s.audio
                        .ops
                        .iter()
                        .filter(|o| !clip_control(o))
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
            frame
                .ops
                .extend(self.audio.ops.iter().filter(|o| clip_control(o)).cloned());
        } else {
            self.audio_controls(&mut frame);
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
        if let Some(mut published) = self.kernel.published(&self.project) {
            if let Some(snapshot) = &self.gain_picture {
                Arc::make_mut(&mut published).snapshot = snapshot.clone();
            }
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
                if layer.kind == "video"
                    && !frame
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
                        content_value(q, id, path, at)
                            .filter(|v| v.is_finite())
                            .unwrap_or(fallback)
                    };
                let gain_channel = layer
                    .channels
                    .iter()
                    .find(|channel| channel.path == gain_path);
                let animated_gain = gain_channel.is_some_and(|channel| {
                    !channel.keys.is_empty() || channel.expression.is_some()
                });
                let base_gain = value(&mut q, gain_path, layer.from, 1.);
                let interactive = self.hover_clip.as_ref().is_some_and(|(clip, _)| clip == id)
                    || self
                        .model
                        .selection
                        .layers()
                        .iter()
                        .any(|selected| selected == id)
                    || self
                        .audio
                        .drag
                        .as_ref()
                        .is_some_and(|drag| drag.layer == id);
                let resting = gain_opacity(base_gain, animated_gain, interactive);
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
                    let Some(opacity) = resting.or_else(|| (fade < 1.).then_some(0.38)) else {
                        continue;
                    };
                    let y = gain_y(gain * fade.max(0.), r);
                    let mut line = op(
                        format!("audio-level:{id}"),
                        Rect::new(x as f64, y, 1., 1.),
                        self.tokens.colors.text,
                    );
                    line.opacity = opacity;
                    line.detail = serde_json::json!({"layer":id,"path":gain_path,"gain":gain,"interactive":interactive});
                    line.clip = clip.clip;
                    frame.ops.push(line);
                }
                if interactive && let Some(channel) = gain_channel {
                    for key in &channel.keys {
                        let t = layer.from + key.time;
                        let x = frame.viewport.layer_column
                            + (t - frame.viewport.scroll_time) * frame.viewport.pps;
                        if x < x0 || x > x1 {
                            continue;
                        }
                        let fade = if fade_in > 0. {
                            (key.time / fade_in).min(1.)
                        } else {
                            1.
                        };
                        let fade = if fade_out > 0. {
                            fade.min((layer.duration - key.time) / fade_out)
                        } else {
                            fade
                        };
                        let gain =
                            dragged.unwrap_or_else(|| value(&mut q, gain_path, t, base_gain));
                        let y = gain_y(gain * fade.max(0.), r);
                        let mut dot = op(
                            format!("audio-key:{id}:{}", key.id),
                            Rect::new(x - 2., y - 2., 4., 4.),
                            self.tokens.colors.text,
                        );
                        dot.shape = "circle";
                        dot.clip = clip.clip;
                        frame.ops.push(dot);
                    }
                }
                if interactive && layer.kind == "audio" {
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
                        handle.detail = serde_json::json!({"layer":id,"path":path,"gain":if path == "c.fadeIn" {fade_in} else {fade_out},"interactive":true});
                        handle.clip = clip.clip;
                        frame.ops.push(handle);
                    }
                }
            }
        }
    }
    pub(super) fn audio_rows(&self, frame: &mut Frame) {
        #[cfg(test)]
        if self.audio.baseline_capture {
            self.baseline_waveform_ops(frame);
            return;
        }
        let mut audio = audio_geometry(frame);
        self.audio_controls(&mut audio);
        frame
            .ops
            .extend(audio.ops.into_iter().filter(|o| clip_control(o)));
    }
    /// Before-crop replay of cea9fdb46's waveform ops, not a running release screenshot.
    /// The native panel/fixture stay identical; only the original sampler/opacity
    /// and absence of the new controls/meters are replayed for visual comparison.
    #[cfg(test)]
    fn baseline_waveform_ops(&self, frame: &mut Frame) {
        let Some(published) = self.kernel.published(&self.project) else {
            return;
        };
        let view = frame.viewport;
        for op in frame.ops.iter_mut().filter(|o| o.shape == "waveform") {
            let Some(id) = op.id.strip_prefix("clip-content:") else {
                continue;
            };
            let Some(layer) = self.model.index.layers.iter().find(|l| l.id == id) else {
                continue;
            };
            let Some(asset) = &layer.asset else {
                continue;
            };
            let Some(path) = self.sources.get(asset) else {
                continue;
            };
            let record = published
                .snapshot
                .root()
                .and_then(|root| root.get("assets")?.get(asset));
            let key = crate::waveforms::Key::of(path.clone(), record);
            let Some(Some(levels)) = crate::waveforms::lookup(&key) else {
                continue;
            };
            let rate = levels.rate;
            let peaks = &levels.combined.peak[0];
            let left = view.layer_column + (layer.from - view.scroll_time) * view.pps;
            let bars: Vec<f32> = (0..op.bounds.width.floor().max(0.) as usize)
                .map(|i| {
                    let at = layer.trim + (op.bounds.x + i as f64 - left) / view.pps;
                    let from = (at * rate).floor().max(0.) as usize;
                    let to = (((at + 1. / view.pps) * rate).floor() as usize)
                        .max(from.saturating_add(1));
                    peaks
                        .get(from.min(peaks.len())..to.min(peaks.len()))
                        .map_or(0., |s| s.iter().copied().fold(0., f32::max))
                })
                .collect();
            op.shape = "waveform-baseline";
            op.detail["bars"] = serde_json::json!(bars);
            op.opacity = 0.72;
        }
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
                            let active = s.audio.tap.changed(s.delegate.audio_peaks(cx))
                                || s.audio.drag.is_some()
                                || s.audio
                                    .meters
                                    .iter()
                                    .any(|m| m.peak_db > -60. || m.hold_db > -60.);
                            // Keep live/hold ballistics at 30 Hz. A quiet heartbeat
                            // discovers a hidden panel without repainting idle graphs.
                            if active
                                || s.audio
                                    .painted
                                    .is_some_and(|p| p.elapsed() >= Duration::from_millis(500))
                            {
                                cx.notify();
                            }
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
        let incoming = self.delegate.audio_peaks(cx);
        let peaks = self.audio.tap.observe(incoming, now);
        for (i, meter) in self.audio.meters.iter_mut().enumerate() {
            if let Some(peaks) = peaks {
                let _ = meter.step(peaks[i], 0., dt);
            } else {
                meter.peak_db = -120.;
                let _ = meter.step(0., 0., dt);
            }
        }
        // The meter stays between the transport controls and the column's
        // routing button. Preserve the column width at which the meter
        // appears, even though Play and Time now sit farther to the left.
        let m = &self.tokens.metrics;
        let size = self.tokens.typography.timecode;
        if self.audio.clock_width.is_none_or(|(previous, _)| previous != size) {
            let slot = crate::playback::timecode_slot(&self.tokens);
            self.audio.clock_width = Some((size, slot.w - m.border));
        }
        let clock_width = self.audio.clock_width.unwrap().1 + m.border;
        let minimum_meter_left = m.inset + 3. * m.control + m.hit + 3. * m.gap + clock_width;
        let x = frame.viewport.layer_column - 120.;
        if x - 4. >= f64::from(minimum_meter_left) {
            let top = (self.tokens.metrics.ruler as f64 - 8.) / 2.;
            frame.ops.push(op(
                "audio-meter-panel".into(),
                Rect::new(x - 4., top - 3., 92., 14.),
                self.tokens.colors.panel,
            ));
            for (i, meter) in self.audio.meters.iter().enumerate() {
                let y = top + i as f64 * 5.;
                frame.ops.push(op(
                    "audio-meter-bg".into(),
                    Rect::new(x, y, 72., 3.),
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
                        Rect::new(x + (lo + 60.) * 1.2, y, (end - lo) * 1.2, 3.),
                        color,
                    ));
                }
                frame.ops.push(op(
                    "audio-hold".into(),
                    Rect::new(x + (meter.hold_db.clamp(-60., 0.) + 60.) * 1.2, y, 1., 3.),
                    0xffffff,
                ));
                frame.ops.push(op(
                    "audio-clip-latch".into(),
                    Rect::new(x + 76., y, 5., 3.),
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
    pub(super) fn audio_hit(&self, x: f64, y: f64) -> Option<&Op> {
        self.audio.ops.iter().rev().find(|o| {
            ((clip_control(o) && o.detail["interactive"] == true) || o.id == "audio-clip-latch")
                && o.clip.is_none_or(|c| {
                    x >= c.x && x <= c.x + c.width && y >= c.y && y <= c.y + c.height
                })
                && x >= o.bounds.x - 3.
                && x <= o.bounds.x + o.bounds.width + 3.
                && y >= o.bounds.y - 3.
                && y <= o.bounds.y + o.bounds.height + 3.
        })
    }
    pub(super) fn audio_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) -> bool {
        let (x, y) = self.local(event.position);
        if self.graph_region().is_some_and(|r| y >= r.y) {
            return false;
        }
        let Some(frame) = &self.frame else {
            return false;
        };
        let hit = self.audio_hit(x, y).cloned();
        let Some(hit) = hit else {
            return false;
        };
        // Existing key glyphs keep ordinary key selection/dragging. Cmd-click
        // on the gain line still inserts (or updates) a gain key as requested.
        if hit.id.starts_with("audio-level:")
            && !event.modifiers.platform
            && frame
                .hit_key_across(x, y, KEY_REACH, self.tokens.metrics.row as f64)
                .is_some()
        {
            return false;
        }
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
                    content_value(QueryMirror::new(&published).queries(), id, path, at)
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
            gesture: None,
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
        let previous = d.value;
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
        if d.moved
            && matches!(d.path.as_str(), "c.gain" | "c.audioGain")
            && (d.gesture.is_none() || d.value != previous)
        {
            let gesture = *d.gesture.get_or_insert_with(GainPreview::gesture);
            let command = level_command(&d.layer, &d.path, d.time, d.value, false);
            self.delegate.audio_gain_preview(
                GainPreview::Sample {
                    gesture,
                    base_revision: None,
                    commands: vec![command],
                },
                cx,
            );
        }
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
            let plan = edit(
                &d.layer,
                &d.path,
                d.time,
                d.value,
                false,
                self.model.index.revision,
            );
            self.queue.push_back(match d.gesture {
                Some(gesture) => {
                    self.audio.pending_gain = Some(gesture);
                    Job::CommitGain(plan, gesture)
                }
                None => Job::Apply(plan),
            });
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
    #[::core::prelude::v1::test]
    fn level_drag_has_db_steps_and_fine_shift() {
        assert!((20. * dragged_gain(1., 12., false).log10() + 6.).abs() < 1e-9);
        assert!((20. * dragged_gain(1., 12., true).log10() + 1.2).abs() < 1e-9);
    }
    #[::core::prelude::v1::test]
    fn resting_gain_line_is_quiet_until_the_clip_is_interactive() {
        let minus_six = 10f64.powf(-6. / 20.);
        assert_eq!(gain_opacity(1., false, false), None);
        assert_eq!(gain_opacity(minus_six, false, false), Some(0.38));
        assert_eq!(gain_opacity(1., true, false), Some(0.38));
        assert_eq!(gain_opacity(minus_six, true, true), Some(1.));
        assert_eq!(gain_opacity(1., false, true), Some(1.));
    }

    #[::core::prelude::v1::test]
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
    #[::core::prelude::v1::test]
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
        content_value(QueryMirror::new(&published).queries(), "audio", path, time).unwrap()
    }
    fn apply(r: &crate::v4_view_tests::Rig, path: &str, time: f64, v: f64, key: bool) {
        let revision = r.kernel.published(PROJECT).unwrap().revision();
        for call in edit("audio", path, time, v, key, revision).calls {
            let ack = r
                .kernel
                .call(PROJECT, pm_kernel::Request::new(Caller::native(), call))
                .wait()
                .unwrap()
                .unwrap();
            assert!(
                !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
                "{:?}",
                ack.reply
            );
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
    #[::core::prelude::v1::test]
    fn audio_level_release_is_one_undo_step() {
        let r = rig_project("timeline", &fixture());
        apply(&r, "c.gain", 2., dragged_gain(1., 12., false), false);
        assert!((value(&r, "c.gain", 2.) - 10f64.powf(-6. / 20.)).abs() < 1e-9);
        undo(&r);
        assert_eq!(value(&r, "c.gain", 2.), 1.);
    }
    #[::core::prelude::v1::test]
    fn audio_command_click_adds_gain_key_at_clicked_time() {
        let r = rig_project("timeline", &fixture());
        apply(&r, "c.gain", 2., 0.5, true);
        let p = r.kernel.published(PROJECT).unwrap();
        let root = p.snapshot.root().unwrap();
        let at = pm_kernel::model::find_layer(root, "audio").unwrap();
        let text = pm_model::json::to_canonical_string(
            &pm_kernel::model::layer_at(root, &at).unwrap().to_value(),
        );
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys = v["d"]["gain"]["kf"].as_array().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0]["t"], 2.);
        undo(&r);
        assert_eq!(value(&r, "c.gain", 2.), 1.);
    }
    #[::core::prelude::v1::test]
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
    #[::core::prelude::v1::test]
    #[ignore = "writes headless audio playback review crops; set PM_AUDIO_CAPTURE and PM_AUDIO_CAPTURE_SOURCE"]
    fn capture_timeline_audio_light_dark() {
        let dir = std::env::var("PM_AUDIO_CAPTURE").unwrap();
        let source = PathBuf::from(std::env::var("PM_AUDIO_CAPTURE_SOURCE").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let project = r##"{"id":"integration","name":"Audio review","compId":"c","w":640,"h":360,"fps":30,"dur":8,"bg":"#000000","assets":{"music":{"kind":"audio","dur":8}},"comps":{},"layers":[{"id":"audio","name":"Music bed","type":"audio","from":0,"dur":8,"on":true,"d":{"asset":"music","gain":{"v":0.5,"kf":[{"i":"gain-0","t":0,"v":0.5},{"i":"gain-4","t":4,"v":0.8},{"i":"gain-8","t":8,"v":0.5}],"expr":null},"fadeIn":1,"fadeOut":1.5},"p":{},"fx":[]}]}"##;
        let review = std::env::var_os("PM_AUDIO_REST_REVIEW").is_some();
        let mut review_project: serde_json::Value = serde_json::from_str(project).unwrap();
        review_project["layers"][0]["d"] = serde_json::json!({
            "asset":"music", "gain":10f64.powf(-6. / 20.), "fadeIn":0, "fadeOut":0
        });
        let review_project = review_project.to_string();
        let project = if review {
            review_project.as_str()
        } else {
            project
        };
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
                let mut captured_root = None;
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
                        let root = cx.new(|_| CaptureFrame {
                            view,
                            fill: tokens.colors.panel,
                            caption_fill: tokens.colors.floating,
                            caption_ink: tokens.colors.text,
                            baseline: false,
                        });
                        captured_root = Some(root.clone());
                        root
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
                        *recorder.time.borrow_mut() = levels.media_time.get() as f64
                            / pm_protocol::common::TICKS_PER_SECOND as f64;
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
                if review {
                    cx.update(|cx| {
                        let s = surface.read(cx);
                        assert!(
                            s.audio
                                .ops
                                .iter()
                                .any(|o| o.id == "audio-level:audio" && o.opacity == 0.38)
                        );
                        assert!(!s.audio.ops.iter().any(|o| o.id.starts_with("audio-fade:")));
                    });
                    cx.capture_screenshot(window.into())
                        .unwrap()
                        .save(format!("{dir}/audio-resting-{name}.png"))
                        .unwrap();
                }
                cx.update(|cx| {
                    surface.update(cx, |s, cx| {
                        let clip = s
                            .frame
                            .as_ref()
                            .unwrap()
                            .ops
                            .iter()
                            .find(|o| o.id == "clip:audio")
                            .unwrap()
                            .bounds;
                        let position = s.bounds.origin
                            + point(
                                px((clip.x + clip.width / 2.) as f32),
                                px((clip.y + clip.height / 2.) as f32),
                            );
                        s.hover(position, cx);
                    })
                });
                cx.run_until_parked();
                if review {
                    cx.update(|cx| {
                        let s = surface.read(cx);
                        assert!(
                            s.audio
                                .ops
                                .iter()
                                .any(|o| o.id == "audio-level:audio" && o.opacity == 1.)
                        );
                        assert_eq!(
                            s.audio
                                .ops
                                .iter()
                                .filter(|o| o.id.starts_with("audio-fade:"))
                                .count(),
                            2
                        );
                    });
                    cx.capture_screenshot(window.into())
                        .unwrap()
                        .save(format!("{dir}/audio-hover-{name}.png"))
                        .unwrap();
                    drop(device);
                    drop(playback);
                    cache.shutdown();
                    return;
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
                                && o.detail["peak"].as_str().is_some_and(|peak| {
                                    pm_panel_timeline::waveform::decode(peak)
                                        .iter()
                                        .any(|v| *v > 0.)
                                }))
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
                    // Meters cannot obscure ruler labels, work-area handles or clips.
                    let column = s.frame.as_ref().unwrap().viewport.layer_column;
                    for o in s.audio.ops.iter().filter(|o| {
                        o.id.starts_with("audio-meter")
                            || matches!(o.id.as_str(), "audio-hold" | "audio-clip-latch")
                    }) {
                        assert!(o.bounds.x >= 32. && o.bounds.x + o.bounds.width <= column - 32.);
                        assert!(
                            o.bounds.y >= 0.
                                && o.bounds.y + o.bounds.height <= tokens.metrics.ruler as f64
                        );
                    }
                    let rails: Vec<_> = s
                        .audio
                        .ops
                        .iter()
                        .filter(|o| o.id == "audio-meter-bg")
                        .collect();
                    assert_eq!(rails.len(), 2);
                    assert!(
                        rails
                            .iter()
                            .all(|o| o.bounds.width == 72. && o.bounds.height == 3.)
                    );
                    assert_eq!(rails[1].bounds.y - rails[0].bounds.y, 5.);
                });
                cx.capture_screenshot(window.into())
                    .unwrap()
                    .save(format!("{dir}/audio-after-{name}.png"))
                    .unwrap();
                let root = captured_root.unwrap();
                cx.update(|cx| {
                    surface.update(cx, |s, cx| {
                        s.audio.baseline_capture = true;
                        s.rows = None;
                        cx.notify();
                    });
                    root.update(cx, |r, cx| {
                        r.baseline = true;
                        cx.notify();
                    });
                });
                for _ in 0..3 {
                    cx.run_until_parked();
                    std::thread::sleep(Duration::from_millis(20));
                }
                cx.capture_screenshot(window.into())
                    .unwrap()
                    .save(format!("{dir}/audio-before-baseline-replay-{name}.png"))
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
        caption_fill: u32,
        caption_ink: u32,
        baseline: bool,
    }
    impl Render for CaptureFrame {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .relative()
                .size_full()
                .bg(rgb(self.fill))
                .child(self.view.clone())
                .children(self.baseline.then(|| div().absolute().left_0().right_0().bottom_0()
                    .h(px(22.)).px(px(8.)).flex().items_center()
                    .bg(rgb(self.caption_fill)).text_color(rgb(self.caption_ink)).text_size(px(10.))
                    .child("BEFORE — baseline-op replay of cea9fdb46 · Not a running release screenshot")
                    .into_any_element()))
        }
    }
}

#[cfg(test)]
mod gesture_tests {
    use super::*;
    // GPUI's attribute expands to #[test]; keep that name bound to Rust's
    // built-in attribute rather than the glob-imported GPUI macro.
    use ::core::prelude::v1::test;
    #[gpui::test]
    fn audio_level_pointer_drag_commits_on_release_and_undo_restores(cx: &mut TestAppContext) {
        let project = r##"{"id":"integration","compId":"c","w":640,"h":360,"fps":30,"dur":8,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"audio","name":"Music","type":"audio","from":0,"dur":8,"on":true,"d":{"gain":1,"fadeIn":0,"fadeOut":0},"p":{},"fx":[]}]}"##;
        let mut s = crate::timeline_tests::stage_project(cx, project.into());
        let before = s
            .rig
            .kernel
            .published(crate::v4_view_tests::PROJECT)
            .unwrap();
        let origin = s.origin();
        let from = origin + point(px(600.), px(50.));
        s.surface.update(&mut s.vcx, |surface, cx| {
            // Isolate the gesture from media I/O; production draws this op from evaluated gain.
            let mut line = op(
                "audio-level:audio".into(),
                Rect::new(599., 49., 3., 1.),
                0xffffff,
            );
            line.detail =
                serde_json::json!({"layer":"audio","path":"c.gain","gain":1.,"interactive":true});
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
        let previews = s.recorder.audio_gain_previews.borrow();
        assert!(
            matches!(
                previews.last().and_then(|preview| match preview {
                    GainPreview::Sample { commands, .. } => Some(commands.as_slice()),
                    _ => None,
                }),
                Some([pm_kernel::Command::Property(pm_kernel::PropertyCommand::Set(set))])
                    if set.layer_id == "audio"
                        && set.channel == "c.gain"
                        && (set.value.as_f64().unwrap() - 10f64.powf(-6. / 20.)).abs() < 1e-9
            ),
            "the live mix must receive the drag before release: {previews:?}"
        );
        drop(previews);
        assert!(s.recorder.timing_previews.borrow().is_empty());
        assert!(s.recorder.seeks.borrow().is_empty());
        let published = s
            .rig
            .kernel
            .published(crate::v4_view_tests::PROJECT)
            .unwrap();
        assert_eq!(published.revision(), before.revision());
        assert_eq!(published.history.entries, before.history.entries);
        assert_eq!(
            content_value(
                QueryMirror::new(&published).queries(),
                "audio",
                "c.gain",
                0.
            ),
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
            (content_value(QueryMirror::new(&p).queries(), "audio", "c.gain", 0.).unwrap()
                - 10f64.powf(-6. / 20.))
            .abs()
                < 1e-9
        );
        assert_eq!(p.history.entries, before.history.entries + 1);
        assert_eq!(p.history.applied, before.history.applied + 1);
        assert!(matches!(s.recorder.audio_gain_previews.borrow().last(),
            Some(GainPreview::Finish { revision: Some(revision), .. }) if *revision == p.revision()));
        s.undo(cx);
        let p = s
            .rig
            .kernel
            .published(crate::v4_view_tests::PROJECT)
            .unwrap();
        assert_eq!(
            content_value(QueryMirror::new(&p).queries(), "audio", "c.gain", 0.),
            Some(1.)
        );
    }
    #[gpui::test]
    fn audio_level_escape_discards_live_gain_without_an_edit(cx: &mut TestAppContext) {
        let mut s = stage_audio(cx);
        let before = s
            .rig
            .kernel
            .published(crate::v4_view_tests::PROJECT)
            .unwrap();
        let from = s.origin() + point(px(600.), px(50.));
        s.surface.update(&mut s.vcx, |surface, cx| {
            let mut line = op(
                "audio-level:audio".into(),
                Rect::new(599., 49., 3., 1.),
                0xffffff,
            );
            line.detail =
                serde_json::json!({"layer":"audio","path":"c.gain","gain":1.,"interactive":true});
            surface.audio.ops.push(line);
            assert!(surface.audio_down(
                &MouseDownEvent {
                    position: from,
                    button: MouseButton::Left,
                    modifiers: Modifiers::none(),
                    click_count: 1,
                    first_mouse: false,
                },
                cx
            ));
            assert!(surface.audio_move(
                &MouseMoveEvent {
                    position: from + point(px(0.), px(12.)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::none(),
                },
                cx
            ));
            assert!(surface.cancel(cx));
        });
        s.idle(cx);
        assert!(matches!(
            s.recorder.audio_gain_previews.borrow().as_slice(),
            [GainPreview::Sample { gesture: began, .. }, GainPreview::Finish { gesture: ended, revision: None }]
                if began == ended
        ));
        let after = s
            .rig
            .kernel
            .published(crate::v4_view_tests::PROJECT)
            .unwrap();
        assert_eq!(after.revision(), before.revision());
        assert_eq!(after.history.entries, before.history.entries);
        assert!(s.recorder.timing_previews.borrow().is_empty());
        assert!(s.recorder.seeks.borrow().is_empty());
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
    #[gpui::test]
    fn meters_fit_the_layer_header_hide_when_narrow_and_clear_on_click(cx: &mut TestAppContext) {
        let mut s = stage_audio(cx);
        let origin = s.origin();
        s.surface.update(&mut s.vcx, |surface, cx| {
            for column in [199., 200., 320., 400.] {
                let mut frame = AudioFrame {
                    viewport: surface.frame.as_ref().unwrap().viewport,
                    ops: Vec::new(),
                };
                frame.viewport.layer_column = column;
                surface.audio_frame(&mut frame, cx);
                let rails: Vec<_> = frame
                    .ops
                    .iter()
                    .filter(|o| o.id == "audio-meter-bg")
                    .collect();
                if column <= 320. {
                    assert!(rails.is_empty(), "the meter would cover the clock at {column}");
                    continue;
                }
                assert_eq!(rails.len(), 2);
                assert!(frame.ops.iter().all(|o| o.bounds.x >= 32.
                    && o.bounds.x + o.bounds.width <= column - 32.
                    && o.bounds.y + o.bounds.height <= surface.tokens.metrics.ruler as f64));
                let latch = frame
                    .ops
                    .iter()
                    .find(|o| o.id == "audio-clip-latch")
                    .unwrap()
                    .bounds;
                surface.audio.ops = frame.ops;
                for meter in &mut surface.audio.meters {
                    meter.clipped = true;
                }
                assert!(surface.audio_down(
                    &MouseDownEvent {
                        position: origin
                            + point(px((latch.x + 2.) as f32), px((latch.y + 1.) as f32)),
                        button: MouseButton::Left,
                        modifiers: Modifiers::none(),
                        click_count: 1,
                        first_mouse: false,
                    },
                    cx
                ));
                assert!(surface.audio.meters.iter().all(|m| !m.clipped));
            }
        });
    }
    fn gain_value(s: &crate::timeline_tests::Stage, path: &str) -> f64 {
        let p = s
            .rig
            .kernel
            .published(crate::v4_view_tests::PROJECT)
            .unwrap();
        content_value(QueryMirror::new(&p).queries(), "audio", path, 0.).unwrap()
    }
    fn gain_key_count(s: &crate::timeline_tests::Stage) -> usize {
        let p = s
            .rig
            .kernel
            .published(crate::v4_view_tests::PROJECT)
            .unwrap();
        let node = p.snapshot.root().unwrap();
        let at = pm_kernel::model::find_layer(node, "audio").unwrap();
        let root: serde_json::Value = serde_json::from_str(&pm_model::json::to_canonical_string(
            &pm_kernel::model::layer_at(node, &at).unwrap().to_value(),
        ))
        .unwrap();
        root["d"]["gain"]["kf"].as_array().map_or(0, Vec::len)
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
            line.detail =
                serde_json::json!({"layer":"audio","path":"c.gain","gain":1.,"interactive":true});
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
                handle.detail =
                    serde_json::json!({"layer":"audio","path":path,"gain":0.,"interactive":true});
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
    #[::core::prelude::v1::test]
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

#[cfg(test)]
mod tap_tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn repeated_paused_scrub_at_same_time_is_fresh_after_silence() {
        let start = Instant::now();
        let half = Some(([0.5, 0.5], 10, false));
        let mut tap = TapAge::default();
        assert_eq!(tap.observe(half, start), Some([0.5, 0.5]));
        assert_eq!(
            tap.observe(Some(([0., 0.], 10, false)), start + Duration::from_secs(1)),
            None
        );
        assert_eq!(
            tap.observe(half, start + Duration::from_millis(1010)),
            Some([0.5, 0.5])
        );
    }
    #[::core::prelude::v1::test]
    fn paused_stale_tap_expires_and_silence_goes_to_floor() {
        let start = Instant::now();
        let mut tap = TapAge::default();
        let half = Some(([0.5, 0.5], 10, false));
        assert_eq!(tap.observe(half, start), Some([0.5, 0.5]));
        assert_eq!(tap.observe(half, start + Duration::from_millis(151)), None);
        assert_eq!(
            tap.observe(Some(([0.5, 0.5], 10, true)), start + Duration::from_secs(1)),
            Some([0.5, 0.5])
        );
        assert_eq!(
            tap.observe(
                Some(([0., 0.], 10, false)),
                start + Duration::from_millis(1010)
            ),
            None
        );
    }
}
