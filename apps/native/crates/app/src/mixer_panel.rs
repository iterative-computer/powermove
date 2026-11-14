//! Window ▸ Panels ▸ Mixer (S097) over pm-services `surface_backing::mixer`:
//! a strip per top-level layer that carries sound, Master pinned first. Meters
//! read the engine's level taps of what is heard and run the backing's
//! ballistics; they sample only while the panel is painted. Levels, mute and
//! the level keyframe write the document as today's mixer does; Solo is the
//! engine's monitor solo, a listening aid that is never saved.
use crate::session::Controller;
use gpui::{prelude::*, *};
use pm_kernel::{Call, Caller, Command, Published, Reply, Request, WriteBody};
use pm_services::surface_backing::mixer::{
    self as backing, Kind, Listening, Meter, MixerSource, Silence, Strip,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    time::{Duration, Instant},
};

const TICK: Duration = Duration::from_millis(33);
const HIDDEN: Duration = Duration::from_secs(1);
/// The fader's console law (`FADER_MARKS`): dB and rail position, top down.
const MARKS: [(f64, f64); 9] = [
    (12., 1.),
    (6., 0.86),
    (0., 0.72),
    (-6., 0.58),
    (-12., 0.45),
    (-20., 0.33),
    (-30., 0.22),
    (-40., 0.13),
    (-60., 0.04),
];
const FLOOR_DB: f64 = -100.;
const DETENT_DB: f64 = 0.35;

/// Rail position (0 bottom, 1 top) of a level in dB (`faderPosition`).
pub fn fader_position(db: f64) -> f64 {
    if db.is_nan() || db <= FLOOR_DB {
        return 0.;
    }
    if db >= MARKS[0].0 {
        return 1.;
    }
    let (low_db, low_pos) = MARKS[MARKS.len() - 1];
    if db < low_db {
        return (db - FLOOR_DB) / (low_db - FLOOR_DB) * low_pos;
    }
    for pair in MARKS.windows(2) {
        let ((high_db, high_pos), (low_db, low_pos)) = (pair[0], pair[1]);
        if db >= low_db {
            return low_pos + (db - low_db) / (high_db - low_db) * (high_pos - low_pos);
        }
    }
    0.
}
/// The level at a rail position (`faderDb`).
pub fn fader_db(position: f64) -> f64 {
    if position.is_nan() || position <= 0. {
        return f64::NEG_INFINITY;
    }
    if position >= 1. {
        return MARKS[0].0;
    }
    let (low_db, low_pos) = MARKS[MARKS.len() - 1];
    if position < low_pos {
        return FLOOR_DB + position / low_pos * (low_db - FLOOR_DB);
    }
    for pair in MARKS.windows(2) {
        let ((high_db, high_pos), (low_db, low_pos)) = (pair[0], pair[1]);
        if position >= low_pos {
            return low_db + (position - low_pos) / (high_pos - low_pos) * (high_db - low_db);
        }
    }
    f64::NEG_INFINITY
}
/// Readout text (`formatDb`): `0.0`, `+2.5`, `−3.2`, `−∞`.
pub fn format_db(db: f64) -> String {
    if db.is_nan() || db <= FLOOR_DB {
        return "−∞".into();
    }
    let rounded = (db * 10.).round() / 10.;
    if rounded == 0. {
        return "0.0".into();
    }
    let text = format!("{:.1}", rounded.abs());
    if rounded > 0. {
        format!("+{text}")
    } else {
        format!("−{text}")
    }
}
/// `levelLabel`: the Undo step a level edit is named for.
fn level_label(strip: Option<&Strip>) -> &'static str {
    match strip {
        None => "Master level",
        Some(s) if s.kind == Kind::Audio => "Gain",
        Some(_) => "Audio gain",
    }
}
/// The backing's command objects (today's edit commands) as kernel commands.
fn kernel_command(command: &Value) -> Option<Command> {
    use pm_kernel::commands::{comps::CompPatch, layers::LayerPatch, properties::SetProperty};
    let target = || command["target"].as_str().map(str::to_owned);
    let model = |v: &Value| pm_model::json::parse(&v.to_string()).ok();
    Some(match command["type"].as_str()? {
        "set_composition" => pm_kernel::CompCommand::Set {
            patch: Box::new(CompPatch {
                audio_gain: command["patch"]["audioGain"].as_f64(),
                ..Default::default()
            }),
        }
        .into(),
        "set_property" => {
            let mut set = SetProperty::new(
                target()?,
                command["path"].as_str()?,
                model(&command["value"])?,
                command["time"].as_f64()?,
            );
            set.preserve_hand_edits = command["preserveHandEdits"] != false;
            pm_kernel::PropertyCommand::Set(set).into()
        }
        "set_content" => {
            let pm_model::Value::Object(patch) = model(&command["patch"])? else {
                return None;
            };
            pm_kernel::ContentCommand::Set {
                layer_id: target()?,
                patch,
                time: command["time"].as_f64().unwrap_or(0.),
            }
            .into()
        }
        "set_layer" => pm_kernel::LayerCommand::Set {
            layer_id: target()?,
            patch: LayerPatch {
                visible: command["patch"]["visible"].as_bool(),
                ..Default::default()
            },
        }
        .into(),
        _ => return None,
    })
}

/// A drag of one strip's fader (`None`: Master). The knob and readout
/// follow the pointer; the release writes the level as one Undo step.
struct Drag {
    strip: Option<Strip>,
    at: f64,
    gain: f64,
}

#[derive(Default)]
pub struct State {
    painted: Option<Instant>,
    running: bool,
    /// The open composition's layers and every composition, as JSON, for
    /// the revision they were read at.
    cache: Option<(u64, u64, Value, BTreeMap<String, Value>)>,
    meters: HashMap<String, [Meter; 2]>,
    master: [Meter; 2],
    sampled: Option<Instant>,
    listening: Listening,
    heard: Vec<String>,
    rails: HashMap<String, Bounds<Pixels>>,
    drag: Option<Drag>,
    /// The document and revision last drawn.
    drawn: Option<(u64, u64)>,
}

pub fn panel(
    v: &mut Controller,
    id: &str,
    bounds: pm_shell::layout::Rect,
    cx: &mut Context<Controller>,
) -> Option<AnyElement> {
    (id == "mixer").then(|| render(v, bounds, cx))
}

/// Values of the open composition at `time` through the kernel's evaluator.
struct Source<'a> {
    published: &'a Published,
    layers: &'a [Value],
    fps: f64,
    mirror: std::cell::RefCell<pm_kernel::QueryMirror>,
}
impl MixerSource for Source<'_> {
    fn evaluate(
        &self,
        layer: &Value,
        raw: &Value,
        time: f64,
        path: &str,
    ) -> pm_services::Result<Value> {
        if !raw.get("kf").is_some_and(Value::is_array) {
            return Ok(raw.clone());
        }
        let index = self.layers.iter().position(|l| l["id"] == layer["id"]);
        let open = self.published.snapshot.open_comp();
        let query = json!({"op":"evP","comp":open,"open":open,"layer":index,"t":time,"key":path,"prop":raw});
        let value = pm_model::json::parse(&query.to_string())
            .ok()
            .and_then(|q| self.mirror.borrow_mut().queries().query(&q).ok())
            .map(|v| pm_model::json::to_canonical_string(&v));
        Ok(value
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_else(|| raw["v"].clone()))
    }
    fn key_at(&self, layer: &Value, raw: &Value, time: f64) -> pm_services::Result<bool> {
        let local = ((time - layer["from"].as_f64().unwrap_or(0.)) * self.fps).round() / self.fps;
        let half = 0.5 / self.fps;
        Ok(raw["kf"].as_array().is_some_and(|keys| {
            keys.iter()
                .any(|k| k["t"].as_f64().is_some_and(|t| (t - local).abs() < half))
        }))
    }
    fn group_ancestors(&self, layer: &Value) -> pm_services::Result<Vec<Value>> {
        let mut out = vec![];
        let mut next = layer["group"].as_str();
        while let Some(id) = next {
            let Some(group) = self.layers.iter().find(|l| l["id"] == id) else {
                break;
            };
            if out.iter().any(|g: &Value| g["id"] == id) {
                break;
            }
            out.push(group.clone());
            next = group["group"].as_str();
        }
        Ok(out)
    }
}

impl Controller {
    /// The strips and the Master level at the playhead, from the cached JSON
    /// of the current revision.
    fn mixer_strips(&mut self) -> Option<(Vec<Strip>, f64)> {
        let published = self.published.clone()?;
        let stamp = (published.snapshot.doc_id, published.revision());
        if self
            .mixer
            .cache
            .as_ref()
            .is_none_or(|c| (c.0, c.1) != stamp)
        {
            let root = published.snapshot.root()?;
            let open = published.snapshot.open_comp();
            let comp = pm_kernel::model::comp_node(root, open.as_deref()).unwrap_or(root);
            let wire = |n: &pm_model::Node| -> Option<Value> {
                serde_json::from_str(&pm_model::json::to_canonical_string(&n.to_value())).ok()
            };
            let comp = wire(comp)?;
            let comps = root
                .get("comps")
                .and_then(wire)
                .and_then(|c| serde_json::from_value(c).ok())
                .unwrap_or_default();
            self.mixer.cache = Some((stamp.0, stamp.1, comp, comps));
        }
        let (_, _, comp, comps) = self.mixer.cache.as_ref()?;
        let time = self.time();
        let source = Source {
            published: &published,
            layers: comp["layers"].as_array().map(Vec::as_slice).unwrap_or(&[]),
            fps: comp["fps"].as_f64().filter(|f| *f > 0.).unwrap_or(30.),
            mirror: std::cell::RefCell::new(pm_kernel::QueryMirror::new(&published)),
        };
        let strips = backing::strips(comp, comps, &source, time).ok()?;
        let master = backing::clamp_gain(comp["audioGain"].as_f64().unwrap_or(1.));
        Some((strips, master))
    }

    /// One edit as one Undo step.
    fn mixer_apply(&mut self, label: &str, commands: Vec<Command>, cx: &mut Context<Self>) {
        let Some(session) = &self.session else { return };
        let revision = self.published.as_ref().map(|p| p.revision());
        let call = Call::Transact {
            label: Some(label.into()),
            expected_revision: revision,
            body: WriteBody::Commands(commands),
        };
        let pending = self
            .kernel
            .call(&session.project, Request::new(Caller::native(), call));
        cx.spawn(async move |this, cx| {
            let result = pending.await;
            let _ = this.update(cx, |v, cx| match result {
                Some(Ok(ack)) => {
                    if let Reply::Throw(e) = ack.reply {
                        v.message = e.to_string();
                    }
                    cx.notify();
                }
                _ => {
                    v.message = "Couldn’t change the level".into();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn mixer_drag_to(&mut self, key: &str, y: Pixels, snap: bool, cx: &mut Context<Self>) {
        let Some(rail) = self.mixer.rails.get(key).copied() else {
            return;
        };
        let height = f32::from(rail.size.height).max(1.);
        let position = (f32::from(rail.bottom() - y) / height).clamp(0., 1.) as f64;
        let mut db = fader_db(position);
        if snap && db.abs() < DETENT_DB {
            db = 0.;
        }
        let Some(drag) = self.mixer.drag.as_mut() else {
            return;
        };
        drag.gain = backing::db_to_gain(db);
        cx.notify();
    }

    fn mixer_release(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.mixer.drag.take() else {
            return;
        };
        if let Ok(command) = backing::level_command(drag.strip.as_ref(), drag.gain, drag.at)
            && let Some(command) = kernel_command(&command)
        {
            self.mixer_apply(level_label(drag.strip.as_ref()), vec![command], cx);
        }
        cx.notify();
    }

    /// The engine hears exactly the soloed strips (or every strip).
    fn mixer_monitor(&mut self) {
        let heard: Vec<String> = self.mixer.listening.solo.iter().cloned().collect();
        if heard != self.mixer.heard {
            if let Some(viewer) = &self.viewer {
                let _ = viewer.monitor_solo(heard.clone());
            }
            self.mixer.heard = heard;
        }
    }

    /// Meter ballistics from the engine's latest levels.
    fn mixer_sample(&mut self, strips: &[Strip]) {
        let now = Instant::now();
        let dt = self
            .mixer
            .sampled
            .map_or(0., |at| now.duration_since(at).as_secs_f64());
        self.mixer.sampled = Some(now);
        let status = self.viewer.as_ref().map(|viewer| viewer.status());
        let levels = status.as_ref().and_then(|s| s.levels.as_ref());
        let playing = status.as_ref().is_some_and(|s| {
            s.clock
                .is_some_and(|c| c.state == pm_protocol::clock::TransportState::Playing)
        });
        let step = |meters: &mut [Meter; 2], peak: &[f32], rms: &[f32]| {
            for (i, meter) in meters.iter_mut().enumerate() {
                // Mono feeds both sides, as the taps upmix it.
                let pick = |v: &[f32]| v.get(i).or(v.first()).copied().unwrap_or(0.) as f64;
                let (p, r) = if playing {
                    (pick(peak), pick(rms))
                } else {
                    (0., 0.)
                };
                let _ = meter.step(p, r, dt);
            }
        };
        let master = levels.map(|l| (l.master.peak.clone(), l.master.rms.clone()));
        let (peak, rms) = master.unwrap_or_default();
        step(&mut self.mixer.master, &peak, &rms);
        let ids: BTreeSet<String> = strips.iter().map(|s| s.id.clone()).collect();
        self.mixer.meters.retain(|id, _| ids.contains(id));
        for strip in strips {
            let tap = levels.and_then(|l| l.strips.iter().find(|s| s.id == strip.id));
            let (peak, rms) = tap.map_or((vec![], vec![]), |t| {
                (t.levels.peak.clone(), t.levels.rms.clone())
            });
            step(
                self.mixer.meters.entry(strip.id.clone()).or_default(),
                &peak,
                &rms,
            );
        }
    }
}

/// Ticks while the panel is painted; ends itself (and the monitor solo,
/// which belongs to an open Mixer) once it isn't.
fn start(cx: &mut Context<Controller>) {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            let alive = this.update(cx, |v, cx| {
                if v.mixer.painted.is_none_or(|p| p.elapsed() > HIDDEN) {
                    v.mixer.running = false;
                    v.mixer.listening.panel_shown = false;
                    v.mixer.listening.solo.clear();
                    v.mixer_monitor();
                    return false;
                }
                let moving = v
                    .mixer
                    .meters
                    .values()
                    .chain(std::iter::once(&v.mixer.master))
                    .flatten()
                    .any(|m| m.peak_db > -119. || m.hold_db > -119.);
                let playing = v.viewer.as_ref().is_some_and(|viewer| {
                    viewer
                        .status()
                        .clock
                        .is_some_and(|c| c.state == pm_protocol::clock::TransportState::Playing)
                });
                let stamp = v
                    .published
                    .as_ref()
                    .map(|p| (p.snapshot.doc_id, p.revision()));
                // An edit, undo or another panel's change redraws the strips.
                if playing || moving || stamp != v.mixer.drawn {
                    cx.notify();
                }
                true
            });
            if !matches!(alive, Ok(true)) {
                break;
            }
        }
    })
    .detach();
}

fn render(
    v: &mut Controller,
    bounds: pm_shell::layout::Rect,
    cx: &mut Context<Controller>,
) -> AnyElement {
    let t = v.host.borrow().tokens.clone();
    v.mixer.painted = Some(Instant::now());
    v.mixer.drawn = v
        .published
        .as_ref()
        .map(|p| (p.snapshot.doc_id, p.revision()));
    v.mixer.listening.panel_shown = true;
    if !v.mixer.running {
        v.mixer.running = true;
        start(cx);
    }
    let height = (bounds.h - t.metrics.header).max(0.);
    let Some((strips, master)) = v.mixer_strips().filter(|(s, _)| !s.is_empty()) else {
        return div()
            .w_full()
            .h(px(height))
            .child(pm_shell::view::empty_state(
                &t,
                "No Audio",
                "No audio in this composition.",
            ))
            .into_any_element();
    };
    let ids: BTreeSet<String> = strips.iter().map(|s| s.id.clone()).collect();
    v.mixer.listening.reconcile(&ids);
    v.mixer_monitor();
    v.mixer_sample(&strips);
    let selected: BTreeSet<String> = v
        .published
        .as_ref()
        .map(|p| p.selection.layers().iter().cloned().collect())
        .unwrap_or_default();
    let m = &t.metrics;
    let this = cx.entity();
    // The strips fill the panel's height and scroll sideways; Master stands
    // at the trailing edge, apart by whitespace, and never scrolls (as v1).
    let mut row = div()
        .id("mixer-strips")
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_row()
        .gap(px(m.border * 2.))
        .py(px(m.gap))
        .pl(px(m.gap))
        .overflow_x_scroll();
    // A live drag follows the pointer anywhere in the window and always
    // sees its release, as the gizmos do.
    if v.mixer.drag.is_some() {
        let this = this.clone();
        row = row.child(
            canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    let moved = this.clone();
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            moved.update(cx, |v, cx| {
                                let key = v.mixer.drag.as_ref().map(|d| {
                                    d.strip
                                        .as_ref()
                                        .map_or_else(|| "master".to_owned(), |s| s.id.clone())
                                });
                                if let Some(key) = key {
                                    v.mixer_drag_to(&key, e.position.y, !e.modifiers.alt, cx);
                                }
                            });
                        }
                    });
                    let released = this.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            released.update(cx, |v, cx| v.mixer_release(cx));
                        }
                    });
                },
            )
            .absolute()
            .size_0(),
        );
    }
    for strip in strips {
        let on = selected.contains(&strip.id);
        let gain = strip.gain;
        row = row.child(strip_view(v, Some(strip), gain, &t, on, this.clone(), cx));
    }
    let master = div()
        .flex_none()
        .h_full()
        .py(px(m.gap))
        .pl(px(m.inset))
        .pr(px(m.gap))
        .child(strip_view(v, None, master, &t, false, this.clone(), cx));
    div()
        .id("mixer")
        .w_full()
        .h(px(height))
        .flex()
        .flex_row()
        .child(row)
        .child(master)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn strip_view(
    v: &mut Controller,
    strip: Option<Strip>,
    gain: f64,
    t: &pm_design::Tokens,
    selected: bool,
    this: Entity<Controller>,
    cx: &mut Context<Controller>,
) -> AnyElement {
    let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
    let key = strip
        .as_ref()
        .map_or_else(|| "master".to_owned(), |s| s.id.clone());
    let name = strip
        .as_ref()
        .map_or("Master".to_owned(), |s| s.name.clone().unwrap_or_default());
    // A drag shows its own level until the kernel publishes it.
    let gain = match &v.mixer.drag {
        Some(d) if d.strip.as_ref().map(|s| &s.id) == strip.as_ref().map(|s| &s.id) => d.gain,
        _ => gain,
    };
    let db = backing::gain_to_db(gain);
    let meters = match &strip {
        Some(s) => v.mixer.meters.get(&s.id).cloned().unwrap_or_default(),
        None => v.mixer.master.clone(),
    };
    let clipped = meters.iter().any(|m| m.clipped);
    let quiet = strip.as_ref().and_then(|s| {
        if s.muted {
            Some("Muted")
        } else {
            match s.silence {
                Some(Silence::Off) => Some("Switched off on the timeline"),
                Some(Silence::Remap) => Some("Time remapping mutes the sound"),
                Some(Silence::TimelineSolo) => Some("Silenced by a soloed layer"),
                None => (!v.mixer.listening.solo.is_empty()
                    && !v.mixer.listening.solo.contains(&s.id))
                .then_some("Silenced by solo"),
            }
        }
    });
    let locked = strip.as_ref().is_some_and(|s| s.locked);
    let ink = if quiet.is_some() { c.muted } else { c.text };
    let tabular = || FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]));

    // Level keyframe diamond (none on Master).
    let diamond = match &strip {
        Some(s) => {
            let s2 = s.clone();
            let label = if s.key_at_time {
                format!("Remove {name} level keyframe")
            } else {
                format!("Add {name} level keyframe")
            };
            div()
                .id(SharedString::from(format!("{key}-key")))
                .role(Role::Button)
                .aria_label(label)
                .size(px(m.keyframe))
                // As v1: the level's diamond shows under the pointer, or
                // when the level is animated.
                .when(!s.animated, |d| {
                    d.opacity(0.)
                        .group_hover_style(SharedString::from(format!("{key}-strip-group")), |s| {
                            s.opacity(1.)
                        })
                })
                .child({
                    let (edge, fill) = (
                        rgb(if s.animated { c.text } else { c.muted }),
                        (!s.key_at_time).then(|| rgb(c.panel)),
                    );
                    let inset = px(m.border * 1.5);
                    canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| {
                            diamond(bounds, edge, window);
                            if let Some(fill) = fill {
                                let inner = Bounds {
                                    origin: bounds.origin + point(inset, inset),
                                    size: size(
                                        bounds.size.width - inset * 2.,
                                        bounds.size.height - inset * 2.,
                                    ),
                                };
                                diamond(inner, fill, window);
                            }
                        },
                    )
                    .size_full()
                })
                .when(!locked, |d| {
                    d.on_click(cx.listener(move |v, _, _, cx| {
                        let command = pm_kernel::KeyframeCommand::Toggle {
                            layer_id: s2.id.clone(),
                            channel: format!("c.{}", s2.gain_key()),
                            time: v.time(),
                            key_id: pm_kernel::new_id("k"),
                            value: None,
                        };
                        v.mixer_apply(level_label(Some(&s2)), vec![command.into()], cx);
                    }))
                })
                .into_any_element()
        }
        None => div().size(px(m.keyframe)).into_any_element(),
    };

    let readout = div()
        .id(SharedString::from(format!("{key}-readout")))
        .aria_label(format!("{name} level in decibels"))
        .flex()
        .gap(px(2.))
        .text_size(px(ty.body))
        .font_features(tabular())
        .text_color(rgb(ink))
        .child(format_db(db))
        .child(div().text_color(rgb(c.muted)).child("dB"));

    // Meters: a stereo pair on the fader's own scale.
    let bar = |meter: &Meter| {
        let level = fader_position(meter.peak_db) as f32;
        let hold = fader_position(meter.hold_db) as f32;
        let colour = if meter.peak_db > 0. {
            c.danger
        } else if meter.peak_db > -12. {
            c.warning
        } else {
            c.success
        };
        div()
            .relative()
            .w(px(m.gap))
            .h_full()
            .bg(rgb(c.field))
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .w_full()
                    .h(relative(level))
                    .bg(rgb(if quiet.is_some() { c.muted } else { colour })),
            )
            .child(
                div()
                    .absolute()
                    .w_full()
                    .h(px(1.))
                    .bottom(relative(hold))
                    .bg(rgb(c.text)),
            )
    };
    let scale = MARKS
        .iter()
        .fold(div().relative().h_full().w(px(m.hit)), |d, (mark, pos)| {
            d.child(
                div()
                    .absolute()
                    .bottom(relative(*pos as f32))
                    // Centred on its mark, as the knob is.
                    .mb(px(-ty.minimum * 0.65))
                    .right_0()
                    .text_size(px(ty.minimum))
                    .font_features(tabular())
                    .text_color(rgb(if *mark == 0. { c.text } else { c.muted }))
                    .child(if *mark > 0. {
                        format!("+{mark}")
                    } else {
                        format!("{}", mark.abs())
                    }),
            )
        });
    // Fader: the whole column is the pointer target.
    let knob_at = fader_position(db) as f32;
    let rail_key = key.clone();
    let drag_strip = strip.clone();
    let fader = div()
        .id(SharedString::from(format!("{key}-fader")))
        .role(Role::Slider)
        .aria_label(format!("{name} level"))
        .relative()
        .w(px(m.hit))
        .h_full()
        .flex()
        .justify_center()
        .child(div().w(px(2.)).h_full().bg(rgb(c.separator)))
        .child(
            div()
                .absolute()
                .bottom(relative(knob_at))
                .mb(px(-m.fader_knob / 2.))
                .w(px(m.hit))
                .h(px(m.fader_knob))
                .rounded(px(m.inner_radius))
                .bg(rgb(c.floating))
                .border(px(m.border))
                .border_color(rgb(c.separator))
                .flex()
                .items_center()
                .child(div().w_full().h(px(1.)).bg(rgb(c.text))),
        )
        .child(
            canvas(
                {
                    let this = this.clone();
                    let key = rail_key.clone();
                    move |bounds, _, cx| {
                        this.update(cx, |v, _| {
                            v.mixer.rails.insert(key, bounds);
                        })
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .when(!locked, |d| {
            let key = rail_key.clone();
            d.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, e: &MouseDownEvent, _, cx| {
                    if e.click_count >= 2 {
                        // Double-click returns to 0 dB as one step.
                        v.mixer.drag = None;
                        if let Ok(command) =
                            backing::level_command(drag_strip.as_ref(), 1., v.time())
                            && let Some(command) = kernel_command(&command)
                        {
                            v.mixer_apply(level_label(drag_strip.as_ref()), vec![command], cx);
                        }
                        return;
                    }
                    let at = v.time();
                    v.mixer.drag = Some(Drag {
                        strip: drag_strip.clone(),
                        at,
                        gain: drag_strip.as_ref().map_or(1., |s| s.gain),
                    });
                    v.mixer_drag_to(&key, e.position.y, !e.modifiers.alt, cx);
                }),
            )
        });
    let middle = div()
        .flex_1()
        .min_h_0()
        .my(px(ty.minimum * 0.65))
        .w_full()
        .flex()
        .flex_row()
        .justify_center()
        .gap(px(2.))
        .child(bar(&meters[0]))
        .child(bar(&meters[1]))
        .child(scale)
        .child(fader);

    let clip_key = key.clone();
    let clip = div()
        .id(SharedString::from(format!("{key}-clip")))
        .role(Role::Button)
        .aria_label(if clipped {
            format!("{name} clipped. Clear")
        } else {
            format!("{name} has not clipped")
        })
        .size(px(m.status_dot))
        .rounded_full()
        .bg(rgb(if clipped { c.danger } else { c.field }))
        .tooltip(pm_shell::view::tooltip(
            t,
            if clipped {
                "Clipped · Click to Clear"
            } else {
                "Clip Indicator"
            },
        ))
        .on_click(cx.listener(move |v, _, _, cx| {
            let meters = if clip_key == "master" {
                Some(&mut v.mixer.master)
            } else {
                v.mixer.meters.get_mut(&clip_key)
            };
            for meter in meters.into_iter().flatten() {
                meter.clear_clip();
            }
            cx.notify();
        }));

    let toggle = |id: &str, letter: &'static str, label: String, on: bool, warn: bool| {
        div()
            .id(SharedString::from(format!("{key}-{id}")))
            .role(Role::Button)
            .aria_label(label)
            .size(px(m.hit))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(m.inner_radius))
            .text_size(px(ty.body))
            .font_weight(FontWeight(ty.semibold_weight))
            .when(on, |d| d.bg(rgb(c.selected_fill)))
            .text_color(rgb(if on && warn { c.warning } else { c.text }))
            .child(letter)
    };
    // Master has no Mute or Solo, but keeps the row so faders line up.
    let mut buttons = div().flex().gap(px(2.)).h(px(m.hit));
    if let Some(s) = &strip {
        let mute = s.clone();
        let tip: &'static str = if s.mute_animated {
            "Mute is animated on the timeline"
        } else {
            "Mute"
        };
        buttons = buttons.child(
            toggle("mute", "M", format!("Mute {name}"), s.muted, false)
                .tooltip(pm_shell::view::tooltip(t, tip))
                .on_click(cx.listener(move |v, _, _, cx| {
                    if let Ok(command) = backing::mute_command(&mute, v.time())
                        && let Some(command) = kernel_command(&command)
                    {
                        v.mixer_apply(
                            if mute.muted { "Unmute" } else { "Mute" },
                            vec![command],
                            cx,
                        );
                    }
                })),
        );
        let solo = s.id.clone();
        let soloed = v.mixer.listening.solo.contains(&s.id);
        buttons = buttons.child(
            toggle("solo", "S", format!("Solo {name}"), soloed, true)
                .tooltip(pm_shell::view::tooltip(t, "Option-click to hear only this"))
                .on_click(cx.listener(move |v, e: &ClickEvent, _, cx| {
                    v.mixer.listening.toggle(&solo, e.modifiers().alt);
                    v.mixer_monitor();
                    cx.notify();
                })),
        );
    }
    let select = strip.as_ref().map(|s| s.id.clone());
    let label = div()
        .id(SharedString::from(format!("{key}-name")))
        .w_full()
        .px(px(2.))
        .rounded(px(m.inner_radius))
        .when(selected, |d| d.bg(rgb(c.selected_fill)))
        .text_size(px(ty.minimum))
        .text_color(rgb(ink))
        .overflow_hidden()
        .text_ellipsis()
        .whitespace_nowrap()
        .child(name.clone())
        .when_some(quiet, |d, why| d.tooltip(pm_shell::view::tooltip(t, why)))
        .when_some(select, |d, id| {
            d.on_click(cx.listener(move |v, _, _, cx| {
                v.call(
                    pm_kernel::SelectionCall::Layers {
                        ids: vec![id.clone()],
                        add: false,
                    }
                    .into(),
                    cx,
                );
            }))
        });
    div()
        .id(SharedString::from(format!("{key}-strip")))
        .role(Role::Group)
        .aria_label(name)
        .flex_none()
        .w(px(m.channel_strip))
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(m.gap))
        .p(px(m.gap))
        .group(SharedString::from(format!("{key}-strip-group")))
        .child(diamond)
        .child(readout)
        .child(middle)
        .child(clip)
        .child(buttons)
        .child(label)
        .into_any_element()
}

/// A keyframe diamond filling `area` (the timeline's shape).
fn diamond(area: Bounds<Pixels>, color: Rgba, window: &mut Window) {
    let c = area.center();
    let (w, h) = (area.size.width / 2., area.size.height / 2.);
    let mut path = PathBuilder::fill();
    path.move_to(point(c.x, c.y - h));
    path.line_to(point(c.x + w, c.y));
    path.line_to(point(c.x, c.y + h));
    path.line_to(point(c.x - w, c.y));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Command, Kind, MARKS, Strip, backing, fader_db, fader_position, format_db, kernel_command,
        level_label,
    };

    #[test]
    fn the_fader_law_and_readout_match_todays_mixer() {
        for (db, position) in MARKS {
            assert!((fader_position(db) - position).abs() < 1e-12);
            assert!((fader_db(position) - db).abs() < 1e-9);
        }
        assert_eq!(fader_position(f64::NEG_INFINITY), 0.);
        assert_eq!(fader_position(20.), 1.);
        assert!((fader_db(fader_position(-80.)) + 80.).abs() < 1e-9);
        assert_eq!(format_db(0.04), "0.0");
        assert_eq!(format_db(2.5), "+2.5");
        assert_eq!(format_db(-3.21), "−3.2");
        assert_eq!(format_db(f64::NEG_INFINITY), "−∞");
    }

    #[test]
    fn the_backings_edits_become_kernel_commands() {
        let strip = Strip {
            id: "a".into(),
            name: Some("Music".into()),
            kind: Kind::Audio,
            gain: 1.,
            animated: false,
            key_at_time: false,
            muted: false,
            mute_animated: false,
            locked: false,
            silence: None,
        };
        let level = kernel_command(&backing::level_command(Some(&strip), 0.5, 2.).unwrap());
        let Some(Command::Content(pm_kernel::ContentCommand::Set { layer_id, .. })) = level else {
            panic!("{level:?}");
        };
        assert_eq!(layer_id, "a");
        let animated = Strip {
            animated: true,
            ..strip.clone()
        };
        let keyed = kernel_command(&backing::level_command(Some(&animated), 0.5, 2.).unwrap());
        let Some(Command::Property(pm_kernel::PropertyCommand::Set(set))) = keyed else {
            panic!("{keyed:?}");
        };
        assert_eq!((set.channel.as_str(), set.time), ("c.gain", 2.));
        let master = kernel_command(&backing::level_command(None, 2., 0.).unwrap());
        let Some(Command::Comp(pm_kernel::CompCommand::Set { patch })) = master else {
            panic!("{master:?}");
        };
        assert_eq!(patch.audio_gain, Some(2.));
        let mute = kernel_command(&backing::mute_command(&strip, 0.).unwrap());
        let Some(Command::Layer(pm_kernel::LayerCommand::Set { patch, .. })) = mute else {
            panic!("{mute:?}");
        };
        assert_eq!(patch.visible, Some(false));
        assert_eq!(level_label(None), "Master level");
        assert_eq!(level_label(Some(&strip)), "Gain");
    }
}
