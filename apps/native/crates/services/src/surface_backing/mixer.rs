//! S097. Tag: mixer/{strips,edits,levels,level-control}.ts, MixerPanel.svelte.
use super::{Session, Ticket, invalid, stale, ticket};
use crate::Result;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Audio,
    Video,
    Precomp,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Silence {
    Off,
    Remap,
    TimelineSolo,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    pub id: String,
    /// None is a neutral presentation placeholder; the model invents no names.
    pub name: Option<String>,
    pub kind: Kind,
    pub gain: f64,
    pub animated: bool,
    pub key_at_time: bool,
    pub muted: bool,
    pub mute_animated: bool,
    pub locked: bool,
    pub silence: Option<Silence>,
}
impl Strip {
    pub fn gain_key(&self) -> &'static str {
        if self.kind == Kind::Audio {
            "gain"
        } else {
            "audioGain"
        }
    }
}
pub trait MixerSource {
    fn evaluate(&self, layer: &Value, raw: &Value, time: f64, path: &str) -> Result<Value>;
    fn key_at(&self, layer: &Value, raw: &Value, time: f64) -> Result<bool>;
    fn group_ancestors(&self, layer: &Value) -> Result<Vec<Value>>;
}
fn property(value: &Value) -> bool {
    value.get("kf").is_some_and(Value::is_array)
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|v| v != 0.),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}
/// Graph walk has no arbitrary depth ceiling. Cycles/missing precomps are quiet;
/// a cycle with an audible branch still contributes sound (N3/N4).
pub fn carries_audio(layer: &Value, comps: &BTreeMap<String, Value>) -> bool {
    let mut pending = vec![layer];
    let mut seen = BTreeSet::new();
    while let Some(layer) = pending.pop() {
        let d = &layer["d"];
        match layer["type"].as_str() {
            Some("audio") if d["asset"].as_str().is_some_and(|s| !s.is_empty()) => return true,
            Some("video") if d["embeddedAudio"] == true && truthy(&d["asset"]) => return true,
            Some("precomp") => {
                if let Some(comp_id) = d["comp"].as_str()
                    && seen.insert(comp_id)
                    && let Some(layers) = comps.get(comp_id).and_then(|c| c["layers"].as_array())
                {
                    pending.extend(layers);
                }
            }
            _ => {}
        }
    }
    false
}
pub fn strips(
    project: &Value,
    comps: &BTreeMap<String, Value>,
    source: &impl MixerSource,
    time: f64,
) -> Result<Vec<Strip>> {
    if !time.is_finite() {
        return Err(invalid("invalid mixer time"));
    }
    let layers = project["layers"]
        .as_array()
        .ok_or_else(|| invalid("missing composition layers"))?;
    let soloing = layers.iter().any(|l| truthy(&l["solo"]));
    let mut ids = BTreeSet::new();
    let mut out = Vec::new();
    for layer in layers {
        if !carries_audio(layer, comps) {
            continue;
        }
        let id = super::id(layer)?.to_owned();
        if !ids.insert(id.clone()) {
            return Err(invalid("duplicate mixer layer id"));
        }
        let kind = match layer["type"].as_str() {
            Some("audio") => Kind::Audio,
            Some("video") => Kind::Video,
            _ => Kind::Precomp,
        };
        let key = if kind == Kind::Audio {
            "gain"
        } else {
            "audioGain"
        };
        let raw = &layer["d"][key];
        let gain = source
            .evaluate(layer, raw, time, &format!("c.{key}"))?
            .as_f64()
            .unwrap_or(1.);
        let gain = clamp_gain(gain);
        let animated = raw["kf"].as_array().is_some_and(|v| !v.is_empty());
        let groups = source.group_ancestors(layer)?;
        let muted = if kind == Kind::Audio {
            source.evaluate(layer, &layer["on"], time, "l.on")? == false
        } else {
            layer["d"]["audioMuted"] == true
        };
        let mut off =
            kind != Kind::Audio && source.evaluate(layer, &layer["on"], time, "l.on")? == false;
        for group in &groups {
            off |= source.evaluate(group, &group["on"], time, "l.on")? == false;
        }
        let remap = kind != Kind::Audio
            && truthy(&source.evaluate(layer, &layer["d"]["timeRemap"], time, "c.timeRemap")?);
        let timeline_silenced =
            soloing && !truthy(&layer["solo"]) && !groups.iter().any(|g| truthy(&g["solo"]));
        out.push(Strip {
            id,
            name: layer["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            kind,
            gain,
            animated,
            key_at_time: animated && source.key_at(layer, raw, time)?,
            muted,
            mute_animated: kind == Kind::Audio && property(&layer["on"]),
            locked: truthy(&layer["lock"]) || groups.iter().any(|g| truthy(&g["lock"])),
            silence: if off {
                Some(Silence::Off)
            } else if remap {
                Some(Silence::Remap)
            } else if timeline_silenced {
                Some(Silence::TimelineSolo)
            } else {
                None
            },
        });
    }
    Ok(out)
}
pub fn clamp_gain(gain: f64) -> f64 {
    if gain.is_nan() {
        0.
    } else {
        gain.clamp(0., 4.)
    }
}
pub fn gain_to_db(gain: f64) -> f64 {
    if gain > 0. {
        20. * gain.log10()
    } else {
        f64::NEG_INFINITY
    }
}
pub fn db_to_gain(db: f64) -> f64 {
    if db.is_nan() || db <= -100. {
        0.
    } else {
        clamp_gain(10f64.powf(db / 20.))
    }
}

pub fn level_command(strip: Option<&Strip>, gain: f64, at: f64) -> Result<Value> {
    if !gain.is_finite() || !(0. ..=4.).contains(&gain) || !at.is_finite() {
        return Err(invalid("invalid level edit"));
    }
    let Some(strip) = strip else {
        return Ok(json!({"type":"set_composition","patch":{"audioGain":gain}}));
    };
    if strip.locked {
        return Err(invalid("locked mixer strip"));
    }
    let key = strip.gain_key();
    Ok(if strip.animated {
        json!({"type":"set_property","target":strip.id,"path":format!("c.{key}"),"value":gain,"time":at,"mode":"auto","preserveHandEdits":false})
    } else {
        json!({"type":"set_content","target":strip.id,"patch":{key:gain}})
    })
}
pub fn mute_command(strip: &Strip, time: f64) -> Result<Value> {
    if !time.is_finite() {
        return Err(invalid("invalid mixer time"));
    }
    if strip.mute_animated {
        // S097 deliberately enables the animated switch at the playhead.
        return Ok(
            json!({"type":"set_property","target":strip.id,"path":"l.on","value":strip.muted,"time":time,"mode":"auto","overrideLock":true,"preserveHandEdits":false}),
        );
    }
    // Like the timeline's audio switch, mute intentionally overrides locks.
    Ok(if strip.kind == Kind::Audio {
        json!({"type":"set_layer","target":strip.id,"patch":{"visible":strip.muted},"overrideLock":true})
    } else {
        json!({"type":"set_content","target":strip.id,"patch":{"audioMuted":!strip.muted},"overrideLock":true})
    })
}
#[derive(Clone, Debug, PartialEq)]
pub struct LevelGesture {
    pub ticket: Ticket,
    pub session: Session,
    pub revision: u64,
    pub at: f64,
    pub strip: Option<Strip>,
}
impl LevelGesture {
    pub fn new(session: Session, revision: u64, at: f64, strip: Option<Strip>) -> Result<Self> {
        level_command(strip.as_ref(), 1., at)?;
        Ok(Self {
            ticket: ticket(),
            session,
            revision,
            at,
            strip,
        })
    }
    /// `at` and target are captured once; playback/selection cannot smear a drag.
    pub fn command(&self, session: &Session, gain: f64) -> Result<Value> {
        if session != &self.session {
            return Err(stale());
        }
        level_command(self.strip.as_ref(), gain, self.at)
    }
}
pub trait MixerHost {
    /// Real kernel gesture lease; dispatch retunes live voices; commit is one
    /// Undo entry, cancel rolls back selectively. Validate the session inside actor.
    fn begin(&mut self, gesture: &LevelGesture) -> Result<()>;
    fn dispatch(&mut self, gesture: &LevelGesture, command: Value) -> Result<()>;
    fn finish(&mut self, gesture: &LevelGesture, commit: bool) -> Result<()>;
    fn monitor_solo(&mut self, ids: &BTreeSet<String>) -> Result<()>;
}
#[derive(Default)]
pub struct Listening {
    pub solo: BTreeSet<String>,
    pub panel_shown: bool,
    pub window_visible: bool,
    pub playing: bool,
    pub audio_running: bool,
}
impl Listening {
    pub fn toggle(&mut self, id: &str, exclusive: bool) {
        if exclusive {
            self.solo = if self.solo.len() == 1 && self.solo.contains(id) {
                BTreeSet::new()
            } else {
                BTreeSet::from([id.into()])
            };
        } else if !self.solo.remove(id) {
            self.solo.insert(id.into());
        }
    }
    pub fn reconcile(&mut self, ids: &BTreeSet<String>) {
        if !self.panel_shown {
            self.solo.clear();
        } else {
            self.solo.retain(|id| ids.contains(id));
        }
    }
    /// Minimized/covered windows stop meter sampling but keep listening solo;
    /// closing/hiding the actual panel clears solo via reconcile (tag intent).
    pub fn sample_meters(&self) -> bool {
        self.panel_shown && self.window_visible && self.playing && self.audio_running
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Meter {
    pub peak_db: f64,
    pub power: f64,
    pub hold_db: f64,
    pub hold_age: f64,
    pub clipped: bool,
}
impl Default for Meter {
    fn default() -> Self {
        Self {
            peak_db: -120.,
            power: 0.,
            hold_db: -120.,
            hold_age: 0.,
            clipped: false,
        }
    }
}
impl Meter {
    pub fn step(&mut self, peak: f64, rms: f64, dt: f64) -> Result<()> {
        if !peak.is_finite() || !rms.is_finite() || rms < 0. || !dt.is_finite() {
            return Err(invalid("invalid meter sample"));
        }
        let elapsed = dt.clamp(0., 0.25);
        let peak_db = gain_to_db(peak.abs()).max(-120.);
        let fall = 20. * elapsed;
        let power = rms * rms;
        if !power.is_finite() {
            return Err(invalid("meter power overflow"));
        }
        self.peak_db = peak_db.max(self.peak_db - fall).max(-120.);
        let tau = if power > self.power { 0.06 } else { 0.3 };
        self.power += (power - self.power) * (1. - (-elapsed / tau).exp());
        if self.power < 1e-12 {
            self.power = 0.;
        }
        if peak_db >= self.hold_db {
            self.hold_db = peak_db;
            self.hold_age = 0.;
        } else {
            self.hold_age += elapsed;
            if self.hold_age > 1.5 {
                self.hold_db = (-120f64).max(self.hold_db - fall).max(self.peak_db);
            }
        }
        self.clipped |= peak.abs() >= 0.999;
        Ok(())
    }
    pub fn clear_clip(&mut self) {
        self.clipped = false;
    }
    pub fn playback_started(&mut self) {
        self.clear_clip();
    }
}
