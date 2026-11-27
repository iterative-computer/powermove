//! Worker-side publication adapters. Scratch stores below only plan/validate a
//! frozen snapshot; the actor remains the sole writer of live project state.
//! N1/N3: guarded Restore fences BOTH document session and revision atomically,
//! unlike Transact's revision-only guard. An open edit refuses; nothing is lost.
use super::*;
use pm_kernel::commands::comps::NewComp;
use pm_kernel::commands::keyframes::{
    EaseSide, HandleValue, Interpolation, KeyframeCommand, TemporalEase,
};
use pm_kernel::{
    Call, Caller, Command, CompCommand, DocumentStore, KernelHandle, Origin, Published, Request,
};
use pm_model::Value;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogError {
    Stale,
    Invalid(&'static str),
    MissingBacking(&'static str),
    Kernel(String),
}
#[derive(Clone, Debug)]
pub struct Context {
    publication: Arc<Published>,
    pub composition: String,
}
impl Context {
    pub fn capture(handle: &KernelHandle, project: &str) -> Result<Self, DialogError> {
        let publication = handle.published(project).ok_or(DialogError::Stale)?;
        let composition = publication.snapshot.open_comp().ok_or(DialogError::Stale)?;
        Ok(Self {
            publication,
            composition,
        })
    }
    pub fn project(&self) -> &str {
        &self.publication.project
    }
    fn root(&self) -> Result<Value, DialogError> {
        self.publication
            .snapshot
            .root()
            .map(|n| n.to_value())
            .ok_or(DialogError::Stale)
    }
    fn comp(&self) -> Result<Value, DialogError> {
        self.publication
            .snapshot
            .root()
            .and_then(|r| r.get("comps"))
            .and_then(|c| c.get(&self.composition))
            .map(|c| c.to_value())
            .ok_or(DialogError::Stale)
    }
    pub fn composition_draft(&self, settings: bool) -> Result<CompositionDraft, DialogError> {
        let c = self.comp()?;
        let names = composition_names(&self.root()?);
        Ok(CompositionDraft {
            name: if settings {
                text(&c, "name").to_owned()
            } else {
                unique_name("Comp", &names)
            },
            width: number(&c, "w", 1920.),
            height: number(&c, "h", 1080.),
            fps: number(&c, "fps", 30.),
            duration: number(&c, "dur", 10.),
            background: if settings {
                text(&c, "bg").to_owned()
            } else {
                "#000000".into()
            },
        })
    }
    /// Layers a shorter duration cuts off: those ending past it. Keeping or
    /// lengthening the duration cuts none, so the sheet opens without a warning.
    pub fn layers_cut_by(&self, duration: f64) -> Result<usize, DialogError> {
        let comp = self.comp()?;
        if duration.partial_cmp(&number(&comp, "dur", f64::NAN)) != Some(std::cmp::Ordering::Less) {
            return Ok(0);
        }
        Ok(array(&comp, "layers")
            .iter()
            .filter(|l| number(l, "from", 0.) + number(l, "dur", 0.) > duration + 1e-9)
            .count())
    }
    pub fn composition_edit(
        &self,
        draft: &CompositionDraft,
        new_id: Option<&str>,
    ) -> Result<PreparedEdit, DialogError> {
        if let Some(error) = draft.errors().values().next() {
            return Err(DialogError::Invalid(error));
        }
        // N3/N4: unchanged controls must not rewrite an authored backgroundFill,
        // work area or opaque data merely because the user changed the name.
        let before = self.composition_draft(true)?;
        let creating = new_id.is_some();
        let settings = NewComp {
            name: (creating || draft.name != before.name).then(|| draft.name.clone()),
            width: (creating || draft.width != before.width).then_some(draft.width),
            height: (creating || draft.height != before.height).then_some(draft.height),
            fps: (creating || draft.fps != before.fps).then_some(draft.fps),
            duration: (creating || draft.duration != before.duration).then_some(draft.duration),
            background: (creating || draft.background != before.background)
                .then(|| draft.background.clone()),
        };
        let cmd = match new_id {
            Some(id) => CompCommand::Create {
                comp_id: id.into(),
                settings,
            },
            None => CompCommand::Settings {
                comp_id: self.composition.clone(),
                settings,
            },
        };
        let edit = self.prepare(
            if new_id.is_some() {
                "New Composition"
            } else {
                "Composition Settings"
            },
            vec![Command::Comp(cmd)],
        )?;
        let comp = edit
            .after
            .get("comps")
            .and_then(|c| c.get(new_id.unwrap_or(&self.composition)))
            .ok_or(DialogError::Stale)?;
        if number(comp, "dur", f64::NAN) != draft.duration {
            return Err(DialogError::MissingBacking(
                "kernel composition duration floor would change the requested duration; preserve the specified 0.1–3600 s range",
            ));
        }
        Ok(edit)
    }
    /// Snapshot-derived key selection excludes locked ancestors, not just the layer.
    pub fn keys(&self, ids: &[String]) -> Result<Vec<Key>, DialogError> {
        let comp = self.comp()?;
        let root = self.publication.snapshot.root().ok_or(DialogError::Stale)?;
        let mut keys = Vec::new();
        for layer in array(&comp, "layers") {
            let layer_id = text(layer, "id");
            let Some(loc) = pm_kernel::model::find_layer(root, layer_id) else {
                continue;
            };
            if pm_kernel::model::is_locked(root, &loc) {
                continue;
            }
            collect_keys(
                layer,
                layer_id,
                number(layer, "from", 0.),
                number(layer, "dur", 0.),
                &mut keys,
            );
        }
        keys.retain(|k| ids.contains(&k.id));
        Ok(keys)
    }
    pub fn values(&self, ids: &[String], value: &MixedNumber) -> Result<PreparedEdit, DialogError> {
        let keys = self.keys(ids)?;
        if keys.is_empty() {
            return Err(DialogError::Stale);
        }
        let Some(value) = value.parsed().map_err(DialogError::Invalid)? else {
            return self.prepare("Keyframe Value", vec![]);
        };
        self.prepare(
            "Keyframe Value",
            keys.iter()
                .map(|k| {
                    Command::Keyframe(KeyframeCommand::Edit {
                        key_id: k.id.clone(),
                        time: None,
                        value: Some(Value::Number(value)),
                    })
                })
                .collect(),
        )
    }
    /// The existing value-edit planner adopts temporal handles. Reapply each
    /// unchanged value on a frozen scratch tree to read exact legacy/native
    /// ease without changing times, values or the live document.
    pub fn velocity_keys(&self, ids: &[String]) -> Result<Vec<Key>, DialogError> {
        let keys = self.keys(ids)?;
        if keys.is_empty() {
            return Ok(keys);
        }
        let normalized = self.prepare(
            "Keyframe Velocity",
            keys.iter()
                .map(|key| {
                    Command::Keyframe(KeyframeCommand::Edit {
                        key_id: key.id.clone(),
                        time: None,
                        value: Some(key.value.clone()),
                    })
                })
                .collect(),
        )?;
        let comp = normalized
            .after
            .get("comps")
            .and_then(|c| c.get(&self.composition))
            .ok_or(DialogError::Stale)?;
        let mut result = Vec::new();
        for layer in array(comp, "layers") {
            collect_keys(
                layer,
                text(layer, "id"),
                number(layer, "from", 0.),
                number(layer, "dur", 0.),
                &mut result,
            );
        }
        result.retain(|k| keys.iter().any(|target| target.id == k.id));
        Ok(result)
    }
    pub fn velocity(
        &self,
        ids: &[String],
        draft: &VelocityDraft,
    ) -> Result<PreparedEdit, DialogError> {
        if !draft.valid() {
            return Err(DialogError::Invalid(VELOCITY_ERROR));
        }
        let keys = self.velocity_keys(ids)?;
        if keys.is_empty() {
            return Err(DialogError::Stale);
        }
        let mut commands = vec![Command::Keyframe(KeyframeCommand::SetInterpolation {
            key_ids: keys.iter().map(|k| k.id.clone()).collect(),
            side: EaseSide::Both,
            interpolation: Interpolation::Bezier,
        })];
        for key in keys {
            for (side, speed, influence, incoming) in [
                (
                    EaseSide::In,
                    &draft.incoming_speed,
                    &draft.incoming_influence,
                    true,
                ),
                (
                    EaseSide::Out,
                    &draft.outgoing_speed,
                    &draft.outgoing_influence,
                    false,
                ),
            ] {
                let old = if incoming { key.incoming } else { key.outgoing };
                let s = speed.parsed().map_err(DialogError::Invalid)?;
                let i = influence.parsed().map_err(DialogError::Invalid)?;
                // Preserve mixed components from each key, never use a shared default.
                {
                    commands.push(Command::Keyframe(KeyframeCommand::SetHandle {
                        key_id: key.id.clone(),
                        side,
                        value: HandleValue::Ease(TemporalEase {
                            speed: s.unwrap_or(old.speed),
                            influence: i.unwrap_or(old.influence),
                        }),
                        split: true,
                    }));
                }
            }
        }
        self.prepare("Keyframe Velocity", commands)
    }
    pub fn scale(&self, ids: &[String], draft: &ScaleDraft) -> Result<PreparedEdit, DialogError> {
        if !draft.valid() {
            return Err(DialogError::Invalid(if draft.collision {
                COLLISION_ERROR
            } else {
                ""
            }));
        }
        let keys = self.keys(ids)?;
        if keys.is_empty() {
            return Err(DialogError::Stale);
        }
        let anchor = keys
            .iter()
            .map(|k| k.time + k.from)
            .min_by(f64::total_cmp)
            .ok_or(DialogError::Stale)?;
        let comp = self.comp()?;
        let fps = number(&comp, "fps", 30.);
        let time_scale = draft.time_percent / 100.;
        let numeric = keys.iter().any(|k| k.value.as_f64().is_some());
        let mut all = Vec::new();
        for l in array(&comp, "layers") {
            collect_keys(
                l,
                text(l, "id"),
                number(l, "from", 0.),
                number(l, "dur", 0.),
                &mut all,
            );
        }
        let frame = |t: f64| (t * fps).round();
        let at = |k: &Key| {
            if time_scale == 1. {
                k.time
            } else {
                frame(anchor - k.from + (k.time + k.from - anchor) * time_scale) / fps
            }
        };
        for k in &keys {
            let t = at(k);
            if !t.is_finite() || t < 0. || t > k.duration {
                return Err(DialogError::Invalid(COLLISION_ERROR));
            }
            for other in &all {
                if other.id == k.id || other.track != k.track || other.layer != k.layer {
                    continue;
                }
                let target = if keys.iter().any(|k| k.id == other.id) {
                    at(other)
                } else {
                    other.time
                };
                if frame(target) == frame(t) {
                    return Err(DialogError::Invalid(COLLISION_ERROR));
                }
            }
        }
        let edit = self.prepare(
            "Scale Keyframes",
            vec![Command::Keyframe(KeyframeCommand::Transform {
                key_ids: keys.iter().map(|k| k.id.clone()).collect(),
                anchor_time: anchor,
                time_scale,
                anchor_value: 0.,
                value_scale: if numeric {
                    draft.value_percent / 100.
                } else {
                    1.
                },
            })],
        )?;
        // Existing graph kernel may additionally clamp to comp bounds/neighbours.
        // Refuse any silent clamping: authored scale must be exact (N3).
        for k in &keys {
            let actual = find_key(&edit.after, &k.id).ok_or(DialogError::Stale)?;
            if (number(actual, "t", f64::NAN) - at(k)).abs() > 1e-8 {
                return Err(DialogError::Invalid(COLLISION_ERROR));
            }
        }
        Ok(edit)
    }
    /// Pure frozen preflight. Call on a worker; never publish the scratch store.
    fn prepare(&self, label: &str, commands: Vec<Command>) -> Result<PreparedEdit, DialogError> {
        let root = self.root()?;
        let mut scratch = DocumentStore::new_untitled(root);
        scratch
            .transact(label, Origin::Interface, &commands)
            .map_err(|e| DialogError::Kernel(e.to_string()))?;
        let after = scratch
            .snapshot()
            .root()
            .ok_or(DialogError::Stale)?
            .to_value();
        Ok(PreparedEdit {
            project: self.project().into(),
            document: self.publication.snapshot.doc_id,
            revision: self.publication.revision(),
            label: label.into(),
            after,
        })
    }
}
#[derive(Clone, Debug)]
pub struct PreparedEdit {
    project: String,
    document: u64,
    revision: u64,
    pub label: String,
    after: Value,
}
impl PreparedEdit {
    pub fn document(&self) -> u64 {
        self.document
    }
    pub fn submit(
        self,
        handle: &KernelHandle,
        txid: u64,
    ) -> pm_kernel::Pending<Result<pm_kernel::Ack, pm_kernel::kernel::KernelFailure>> {
        handle.call(
            &self.project,
            Request::new(
                Caller::native(),
                Call::Restore {
                    project: self.after,
                    label: self.label,
                    expected_revision: self.revision,
                    expected_document: self.document,
                },
            )
            .with_txid(txid),
        )
    }
}
#[derive(Clone, Debug)]
pub struct Key {
    pub id: String,
    pub layer: String,
    pub track: String,
    pub time: f64,
    pub from: f64,
    pub duration: f64,
    pub value: Value,
    pub incoming: TemporalEase,
    pub outgoing: TemporalEase,
}
fn collect_keys(layer: &Value, layer_id: &str, from: f64, duration: f64, out: &mut Vec<Key>) {
    fn walk(v: &Value, path: &str, layer: &str, from: f64, duration: f64, out: &mut Vec<Key>) {
        match v {
            Value::Object(m) => {
                if let Some(keys) = v.get("kf").and_then(Value::as_array) {
                    for k in keys {
                        if text(k, "i").is_empty() {
                            continue;
                        }
                        let ease = |name: &str| {
                            let e = k.get(name);
                            TemporalEase {
                                speed: e.map_or(0., |e| number(e, "speed", 0.)),
                                influence: e
                                    .map_or(100. / 3., |e| number(e, "influence", 100. / 3.)),
                            }
                        };
                        out.push(Key {
                            id: text(k, "i").into(),
                            layer: layer.into(),
                            track: path.into(),
                            time: number(k, "t", 0.),
                            from,
                            duration,
                            value: k.get("v").cloned().unwrap_or(Value::Null),
                            incoming: ease("inEase"),
                            outgoing: ease("outEase"),
                        });
                    }
                }
                for (k, v) in m {
                    if k.as_str() != Some("kf") {
                        walk(
                            v,
                            &format!("{path}.{}", k.to_string_lossy()),
                            layer,
                            from,
                            duration,
                            out,
                        );
                    }
                }
            }
            Value::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    walk(v, &format!("{path}.{i}"), layer, from, duration, out);
                }
            }
            _ => {}
        }
    }
    walk(layer, "", layer_id, from, duration, out)
}
fn find_key<'a>(v: &'a Value, id: &str) -> Option<&'a Value> {
    if text(v, "i") == id {
        return Some(v);
    }
    match v {
        Value::Object(m) => m.iter().find_map(|(_, v)| find_key(v, id)),
        Value::Array(a) => a.iter().find_map(|v| find_key(v, id)),
        _ => None,
    }
}
fn composition_names(root: &Value) -> Vec<String> {
    root.get("comps")
        .and_then(Value::as_object)
        .map_or_else(Vec::new, |m| {
            m.iter().map(|(_, c)| text(c, "name").into()).collect()
        })
}
fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], |a| a.as_slice())
}
fn number(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(default)
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
