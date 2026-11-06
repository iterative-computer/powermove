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
    Call, Caller, Command, CompCommand, DocumentStore, KernelHandle, Origin, PropertyCommand,
    Published, Request,
};
use pm_model::{Map, Value};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogError {
    Stale,
    Invalid(&'static str),
    MissingBacking(&'static str),
    Kernel(String),
    InvalidPayload,
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
    pub fn current(&self, handle: &KernelHandle) -> bool {
        handle.published(self.project()).is_some_and(|p| {
            p.snapshot.doc_id == self.publication.snapshot.doc_id
                && p.snapshot.open_comp().as_deref() == Some(&self.composition)
        })
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
    pub fn extending_layers(&self, duration: f64) -> Result<usize, DialogError> {
        Ok(array(&self.comp()?, "layers")
            .iter()
            .filter(|l| number(l, "from", 0.) + number(l, "dur", 0.) > duration)
            .count())
    }
    pub fn composition_users(&self, id: &str) -> Result<usize, DialogError> {
        let root = self.root()?;
        Ok(root
            .get("comps")
            .and_then(Value::as_object)
            .ok_or(DialogError::Stale)?
            .iter()
            .filter(|(key, c)| {
                key.as_str() != Some(id)
                    && array(c, "layers").iter().any(|l| {
                        text(l, "type") == "precomp"
                            && l.get("d").is_some_and(|d| text(d, "comp") == id)
                    })
            })
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
    pub fn delete_composition(&self, id: &str) -> Result<PreparedEdit, DialogError> {
        self.prepare(
            "Delete Composition",
            vec![Command::Comp(CompCommand::Delete {
                comp_id: id.into(),
                fallback: None,
            })],
        )
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
    pub fn keyframe_menu(&self, ids: &[String]) -> Result<Vec<KeyframeMenuItem>, DialogError> {
        Ok(super::keyframe_menu(self.keys(ids)?.len()))
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
    /// Two-part properties call this once with each component's key-id set.
    /// Both components are staged in the same transaction; blank = unchanged.
    pub fn component_values(
        &self,
        components: &[(Vec<String>, MixedNumber)],
    ) -> Result<PreparedEdit, DialogError> {
        let mut commands = Vec::new();
        for (ids, draft) in components {
            let Some(value) = draft.parsed().map_err(DialogError::Invalid)? else {
                continue;
            };
            for key in self.keys(ids)? {
                commands.push(Command::Keyframe(KeyframeCommand::Edit {
                    key_id: key.id,
                    time: None,
                    value: Some(Value::Number(value)),
                }));
            }
        }
        self.prepare("Keyframe Value", commands)
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
    pub fn expression(
        &self,
        layer: &str,
        channel: &str,
        source: Option<String>,
    ) -> Result<PreparedEdit, DialogError> {
        self.prepare(
            "Expression",
            vec![Command::Property(PropertyCommand::SetExpression {
                layer_id: layer.into(),
                channel: channel.into(),
                expression: source.filter(|source| !source.trim().is_empty()),
            })],
        )
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
    /// Engine-only worker preview. This never changes a publication, history or
    /// the source text. Units are supplied by the existing inspector metadata.
    pub fn evaluate_expression(
        &self,
        job: &EvaluationJob,
        layer: &str,
        channel: &str,
        time: f64,
        unit: &str,
    ) -> Preview {
        let mut scratch = match self.root() {
            Ok(root) => DocumentStore::new_untitled(root),
            Err(_) => return Preview::Unavailable,
        };
        if !job.source.trim().is_empty() {
            let diagnostic = object([
                ("op", Value::from("expressionDiagnostic")),
                ("source", Value::from(job.source.as_str())),
            ]);
            match scratch.queries().query(&diagnostic) {
                Ok(Value::String(message)) => {
                    let message = message.to_string_lossy();
                    let Some(offset) = message
                        .rsplit_once(" at ")
                        .and_then(|(_, n)| n.parse::<usize>().ok())
                    else {
                        return Preview::SyntaxUnlocated { message };
                    };
                    let line = 1 + job
                        .source
                        .encode_utf16()
                        .take(offset)
                        .filter(|u| *u == 10)
                        .count();
                    return Preview::Syntax { line, message };
                }
                Ok(Value::Null) => {}
                _ => return Preview::Unavailable,
            }
        }
        let command = Command::Property(PropertyCommand::SetExpression {
            layer_id: layer.into(),
            channel: channel.into(),
            expression: if job.source.trim().is_empty() {
                None
            } else {
                Some(job.source.clone())
            },
        });
        if scratch.execute(command, Origin::Interface).is_err() {
            return Preview::Unavailable;
        }
        let value = match scratch
            .queries()
            .evaluate(Some(&self.composition), layer, channel, time)
        {
            Ok(value) => value,
            Err(_) => return Preview::Unavailable,
        };
        let error = object([
            ("op", Value::from("expressionError")),
            ("comp", Value::from(self.composition.as_str())),
            ("layerId", Value::from(layer)),
            ("key", Value::from(channel)),
        ]);
        match scratch.queries().query(&error) {
            Ok(Value::String(message)) => {
                if job.source.contains('\n') {
                    Preview::RuntimeUnlocated {
                        message: message.to_string_lossy(),
                    }
                } else {
                    Preview::Runtime {
                        line: 1,
                        message: message.to_string_lossy(),
                    }
                }
            }
            Ok(Value::Null) => Preview::Value(format!(
                "{}{}{}",
                pm_model::json::to_canonical_string(&value),
                if unit.is_empty() { "" } else { " " },
                unit
            )),
            _ => Preview::Unavailable,
        }
    }
    /// Frozen v1.2.0 EXPRESSION_NAMES order; no native-only names are inserted.
    pub fn expression_names(&self) -> &'static [&'static str] {
        SHIPPED_EXPRESSION_NAMES
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
fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    let mut m = Map::new();
    for (k, v) in entries {
        m.insert(k, v);
    }
    Value::Object(m)
}

/// Presets retain the exact legacy array and every unknown field. A backing
/// service must durably archive complete replaced/deleted entries first (N4).
/// No archive service exists yet: do not turn this into an in-memory production
/// fallback or place archive metadata in the v1–3 renderPresets array (N6).
pub trait PresetArchive {
    fn archive(&mut self, entry: &Value) -> Result<(), ServiceCode>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceCode {
    Offline,
    NotFound,
    TooLarge,
    InvalidProject,
    PermissionDenied,
    DiskFull,
    InvalidInput,
    InvalidData,
    Unavailable,
    Unknown,
}
impl ServiceCode {
    pub fn informative(self) -> &'static str {
        match self {
            Self::Offline => "Check your internet connection and try again.",
            Self::NotFound => "It may have been moved or deleted.",
            Self::TooLarge | Self::InvalidProject => "",
            Self::PermissionDenied => "Check its sharing and permissions in Finder.",
            Self::DiskFull => "Free some space and try again.",
            Self::InvalidInput | Self::InvalidData => "The file may be damaged.",
            Self::Unavailable | Self::Unknown => {
                "Try again. If it keeps happening, choose Window ▸ Error Log for details."
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct Presets {
    entries: Vec<Value>,
    revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresetError {
    Service(ServiceCode),
    Missing,
    NeedsReplace,
    DesignTodo,
    EmptyName,
    DuplicateName,
    Malformed,
}
impl PresetError {
    pub fn inline(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Service(c) => Some((PRESET_FAILURE, c.informative())),
            _ => None,
        }
    }
}
impl Presets {
    pub fn entries(&self) -> &[Value] {
        &self.entries
    }
    pub fn capture(handle: &KernelHandle) -> Result<Self, PresetError> {
        let state = handle.state();
        let entries = match state.settings.get("renderPresets") {
            None => vec![],
            Some(Value::Array(a)) => a.clone(),
            _ => return Err(PresetError::Malformed),
        };
        Ok(Self {
            entries,
            revision: state.revision,
        })
    }
    pub fn select(&self, id: &str) -> Option<Value> {
        self.entries
            .iter()
            .find(|e| text(e, "id") == id)
            .and_then(|e| e.get("options"))
            .cloned()
    }
    pub fn save(
        &self,
        id: &str,
        name: &str,
        mut options: Value,
        replace: bool,
        archive: &mut impl PresetArchive,
    ) -> Result<Self, PresetError> {
        if id.is_empty() || name.trim().is_empty() || options.as_object().is_none() {
            return Err(PresetError::DesignTodo);
        }
        let name = name.trim();
        let old: Vec<_> = self
            .entries
            .iter()
            .filter(|e| text(e, "name") == name || text(e, "id") == id)
            .collect();
        if !old.is_empty() && !replace {
            return Err(PresetError::NeedsReplace);
        }
        for e in &old {
            archive.archive(e).map_err(PresetError::Service)?;
        }
        options.as_object_mut().expect("checked").remove("name");
        let mut next = self.clone();
        next.entries
            .retain(|e| text(e, "name") != name && text(e, "id") != id);
        next.entries.push(object([
            ("id", Value::from(id)),
            ("name", Value::from(name)),
            ("options", options),
        ]));
        Ok(next)
    }
    pub fn rename(&self, id: &str, name: &str) -> Result<Self, PresetError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(PresetError::EmptyName);
        }
        if self
            .entries
            .iter()
            .any(|e| text(e, "id") != id && text(e, "name") == name)
        {
            return Err(PresetError::DuplicateName);
        }
        let original = self
            .entries
            .iter()
            .find(|e| text(e, "id") == id)
            .ok_or(PresetError::Missing)?;
        if text(original, "name") == name {
            return Ok(self.clone());
        }
        let mut next = self.clone();
        let entry = next
            .entries
            .iter_mut()
            .find(|e| text(e, "id") == id)
            .ok_or(PresetError::Missing)?;
        entry
            .as_object_mut()
            .ok_or(PresetError::Malformed)?
            .insert("name", Value::from(name));
        Ok(next)
    }
    pub fn delete(&self, id: &str, archive: &mut impl PresetArchive) -> Result<Self, PresetError> {
        let entry = self
            .entries
            .iter()
            .find(|e| text(e, "id") == id)
            .ok_or(PresetError::Missing)?;
        archive.archive(entry).map_err(PresetError::Service)?;
        let mut next = self.clone();
        next.entries.retain(|e| text(e, "id") != id);
        Ok(next)
    }
    pub fn submit(
        self,
        handle: &KernelHandle,
    ) -> pm_kernel::Pending<Result<pm_kernel::state::StateAck, pm_kernel::kernel::KernelFailure>>
    {
        handle.state_transaction(pm_kernel::state::StateTransaction {
            expected_revision: Some(self.revision),
            ops: vec![pm_kernel::state::StateOp::Setting {
                key: "renderPresets".into(),
                value: Some(Value::Array(self.entries)),
            }],
        })
    }
}

/// Vendored from v1.2.0 legacy/core/expression.ts (EXPRESSION_NAMES,
/// HELPERS and Object.keys(MATH_FUNCTIONS)); no runtime source-file dependency.
pub const SHIPPED_EXPRESSION_NAMES: &[&str] = &[
    "time",
    "t",
    "T",
    "fps",
    "value",
    "layer",
    "comp",
    "ch",
    "idx",
    "PI",
    "Math",
    "clamp",
    "lerp",
    "linear",
    "ease",
    "random",
    "wiggle",
    "bounce",
    "loop",
    "pingpong",
    "param",
    "sample",
    "motionPath",
    "abs",
    "acos",
    "asin",
    "atan",
    "atan2",
    "ceil",
    "cos",
    "exp",
    "floor",
    "log",
    "max",
    "min",
    "pow",
    "round",
    "sign",
    "sin",
    "sqrt",
    "tan",
];
