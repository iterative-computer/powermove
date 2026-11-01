//! Shared, strict media timing for the pro commands. No runtime/media IO.
use super::{Cx, channels, values};
use crate::CommandError;
use pm_model::Value;

pub(super) const EPS: f64 = 1e-8;
pub(super) fn invalid(message: &str) -> CommandError {
    CommandError::Invalid(message.into())
}
pub(super) fn number(n: f64) -> Result<f64, CommandError> {
    values::finite(n, "edit time")
}

/// A timeline lane is addressed by its group id and authored track number.
/// The open composition is never inferred from a cached layer index.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TrackTarget {
    pub group_id: Option<String>,
    pub track: i32,
}
impl TrackTarget {
    pub fn new(track: i32) -> Self {
        Self {
            group_id: None,
            track,
        }
    }
    pub(super) fn matches(&self, layer: &Value) -> bool {
        values::text(layer, "group") == self.group_id.as_deref()
            && values::num(layer, "track").unwrap_or(0.0) == f64::from(self.track)
    }
}

#[derive(Clone)]
pub(super) struct Clip {
    pub id: String,
    pub comp: Option<String>,
    pub value: Value,
    pub from: f64,
    pub dur: f64,
    pub trim: f64,
    pub rate: f64,
}
impl Clip {
    pub fn read(cx: &Cx<'_>, id: &str) -> Result<Self, CommandError> {
        let (loc, value) = cx.layer(id)?;
        if !matches!(values::text(&value, "type"), Some("video" | "audio")) {
            return Err(invalid("Pro media edits require video or audio clips."));
        }
        if value.get("d").and_then(Value::as_object).is_none() {
            return Err(invalid("The clip has no source content."));
        }
        let from = required(&value, "from")?;
        let dur = required(&value, "dur")?;
        let trim = content_number(&value, "trim", 0.0)?;
        let speed = content_number(&value, "speed", 1.0)?;
        let rate = if values::text(&value, "type") == Some("video") {
            speed
        } else {
            if speed != 1.0 {
                return Err(invalid("Audio clips require unit playback rate."));
            }
            1.0
        };
        for key in ["sourceTime", "timeRemap"] {
            let n = content_number(&value, key, 0.0)?;
            if key == "timeRemap" && n != 0.0 {
                return Err(invalid(
                    "Edit time-remapped clips with explicit property commands.",
                ));
            }
        }
        if from < 0.0 || dur <= 0.0 || trim < 0.0 || rate <= 0.0 {
            return Err(invalid("The clip has invalid source or timeline timing."));
        }
        Ok(Self {
            id: id.into(),
            comp: loc.comp_id,
            value,
            from,
            dur,
            trim,
            rate,
        })
    }
    pub fn editable(cx: &Cx<'_>, id: &str) -> Result<Self, CommandError> {
        let clip = Self::read(cx, id)?;
        cx.check_lock(&cx.find(id)?, id)?;
        if cx.origin().is_generated()
            && clip
                .value
                .get("locked_intent")
                .and_then(Value::as_object)
                .is_some_and(|m| m.values().any(Value::is_truthy))
        {
            return Err(invalid("The clip contains protected hand edits."));
        }
        Ok(clip)
    }
    pub fn end(&self) -> f64 {
        self.from + self.dur
    }
    pub fn same_track(&self, other: &Self) -> bool {
        self.comp == other.comp
            && values::text(&self.value, "group") == values::text(&other.value, "group")
            && values::num(&self.value, "track").unwrap_or(0.0)
                == values::num(&other.value, "track").unwrap_or(0.0)
    }
    pub fn validate(&self, cx: &Cx<'_>, handles: bool) -> Result<(), CommandError> {
        for n in [
            self.from,
            self.dur,
            self.trim,
            self.end(),
            self.trim + self.dur * self.rate,
        ] {
            number(n)?;
        }
        if self.from < -EPS
            || self.dur + EPS < 1.0 / cx.fps(self.comp.as_deref())
            || self.end() > cx.duration(self.comp.as_deref()) + EPS
            || self.trim < -EPS
        {
            return Err(invalid(
                "The edit must retain one frame inside the composition and source.",
            ));
        }
        if handles {
            let asset = self
                .value
                .get("d")
                .and_then(|d| values::text(d, "asset"))
                .ok_or_else(|| invalid("The clip has no source asset."))?;
            let duration = cx
                .root()
                .get("assets")
                .and_then(|a| a.get(asset))
                .and_then(|a| a.get("dur"))
                .and_then(crate::model::number_of)
                .filter(|n| n.is_finite());
            if !duration.is_some_and(|d| self.trim + self.dur * self.rate <= d + EPS) {
                return Err(invalid("The edit exceeds available source handles."));
            }
        }
        Ok(())
    }
    /// Retain absolute animation time at an In edge (keys are layer-local).
    pub fn trim_in(&mut self, delta: f64) -> Result<(), CommandError> {
        self.from += delta;
        self.dur -= delta;
        self.trim += delta * self.rate;
        rebase(&mut self.value, delta)
    }
    pub fn into_value(mut self, cx: &Cx<'_>, handles: bool) -> Result<Value, CommandError> {
        self.validate(cx, handles)?;
        let m = values::obj(&mut self.value);
        m.insert("from", Value::Number(self.from.max(0.0)));
        m.insert("dur", Value::Number(self.dur));
        set_content(&mut self.value, "trim", self.trim.max(0.0));
        Ok(self.value)
    }
    pub fn write(self, cx: &mut Cx<'_>, handles: bool) -> Result<(), CommandError> {
        let loc = cx.find(&self.id)?;
        let value = self.into_value(cx, handles)?;
        cx.put_layer(&loc, value)
    }
}
fn required(v: &Value, key: &str) -> Result<f64, CommandError> {
    number(values::num(v, key).ok_or_else(|| invalid("The clip has incomplete timing."))?)
}
fn content_number(v: &Value, key: &str, default: f64) -> Result<f64, CommandError> {
    let Some(v) = v.get("d").and_then(|d| d.get(key)) else {
        return Ok(default);
    };
    let v = if values::is_property(v) {
        if v.get("kf")
            .and_then(Value::as_array)
            .is_some_and(|k| !k.is_empty())
            || v.get("expr").is_some_and(Value::is_truthy)
        {
            return Err(invalid(
                "Edit animated source timing with explicit property commands.",
            ));
        }
        v.get("v").ok_or_else(|| invalid("Missing timing value."))?
    } else {
        v
    };
    if let Some(b) = v.as_bool() {
        return Ok(f64::from(u8::from(b)));
    }
    number(
        v.as_f64()
            .ok_or_else(|| invalid("Invalid source timing value."))?,
    )
}
pub(super) fn set_content(layer: &mut Value, key: &str, n: f64) {
    let d = values::obj(layer).get_mut("d").expect("media content");
    let m = values::obj(d);
    if let Some(p) = m.get_mut(key).filter(|p| values::is_property(p)) {
        values::obj(p).insert("v", Value::Number(n));
    } else {
        m.insert(key, Value::Number(n));
    }
}
pub(super) fn rebase(layer: &mut Value, delta: f64) -> Result<(), CommandError> {
    for (_, path) in channels::all(layer) {
        if let Some(keys) = channels::get_mut(layer, &path)
            .and_then(Value::as_object_mut)
            .and_then(|p| p.get_mut("kf"))
            .and_then(Value::as_array_mut)
        {
            for key in keys {
                let t = required(key, "t")? - delta;
                values::obj(key).insert("t", Value::Number(number(t)?));
            }
        }
    }
    Ok(())
}
pub(super) fn same_comp(clips: &[Clip]) -> Result<Option<String>, CommandError> {
    let first = clips
        .first()
        .ok_or_else(|| invalid("Choose at least one clip."))?;
    if clips.iter().any(|c| c.comp != first.comp) {
        return Err(invalid("Edit clips in one composition at a time."));
    }
    Ok(first.comp.clone())
}
