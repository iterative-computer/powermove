//! S094/S095: v1.2.0 panels/easing/{library,curve,operations}.ts.
//! Authored JSON is retained, including future fields and unsupported versions.
//! No persistence fallback silently replaces a rejected library with defaults.
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const STORAGE_KEY: &str = "pm.easing.library.v1";
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    LibraryShape,
    UnsupportedVersion,
    Group,
    DuplicateId,
    Preset,
    CurvePoints,
    Coordinates,
    Handles,
    Endpoints,
    MissingTarget,
    LastGroup,
    Name,
    IdCollision,
    NonConsecutive,
    FlatSpan,
}
#[derive(Debug, Clone)]
pub struct Library {
    raw: Value,
}
/// Always retains the complete rejected source for migration/recovery/export.
#[derive(Debug, Clone)]
pub struct Rejected {
    pub source: Value,
    pub problem: Problem,
}
fn text(value: &Value, max: usize) -> bool {
    value
        .as_str()
        .is_some_and(|s| !s.is_empty() && s.encode_utf16().count() <= max)
}
fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite())
}
pub fn validate_curve(curve: &Value) -> Result<(), Problem> {
    let points = curve["points"].as_array().ok_or(Problem::CurvePoints)?;
    if !(2..=256).contains(&points.len()) {
        return Err(Problem::CurvePoints);
    }
    let mut previous = None;
    for p in points {
        let t = number(&p["t"]).ok_or(Problem::Coordinates)?;
        let v = number(&p["v"]).ok_or(Problem::Coordinates)?;
        if !(0. ..=1.).contains(&t) || v.abs() > 100. || previous.is_some_and(|old| t <= old) {
            return Err(Problem::Coordinates);
        }
        previous = Some(t);
        for side in ["in", "out"] {
            if !matches!(
                p[format!("{side}Interp")].as_str(),
                Some("linear" | "bezier" | "hold")
            ) {
                return Err(Problem::Handles);
            }
            let ease = &p[format!("{side}Ease")];
            let speed = number(&ease["speed"]).ok_or(Problem::Handles)?;
            let influence = number(&ease["influence"]).ok_or(Problem::Handles)?;
            if speed.abs() > 1e8 || !(0.1..=100.).contains(&influence) {
                return Err(Problem::Handles);
            }
        }
    }
    if points[0]["t"] != json!(0) && points[0]["t"] != json!(0.) {
        return Err(Problem::Endpoints);
    }
    if number(&points[0]["v"]) != Some(0.)
        || number(&points.last().unwrap()["t"]) != Some(1.)
        || number(&points.last().unwrap()["v"]) != Some(1.)
    {
        return Err(Problem::Endpoints);
    }
    Ok(())
}
fn validate(raw: &Value) -> Result<(), Problem> {
    if raw["version"] != json!(1) {
        return Err(Problem::UnsupportedVersion);
    }
    let groups = raw["groups"].as_array().ok_or(Problem::LibraryShape)?;
    if groups.is_empty() || groups.len() > 100 {
        return Err(Problem::LibraryShape);
    }
    let mut ids = BTreeSet::new();
    let mut total = 0;
    for g in groups {
        if !text(&g["id"], 200) || !text(&g["name"], 100) {
            return Err(Problem::Group);
        }
        if !ids.insert(g["id"].as_str().unwrap()) {
            return Err(Problem::DuplicateId);
        }
        for p in g["presets"].as_array().ok_or(Problem::Group)? {
            total += 1;
            if total > 5000 || !text(&p["id"], 200) || !text(&p["name"], 100) {
                return Err(Problem::Preset);
            }
            if !ids.insert(p["id"].as_str().unwrap()) {
                return Err(Problem::DuplicateId);
            }
            validate_curve(&p["curve"])?;
        }
    }
    Ok(())
}
impl Library {
    pub fn read(raw: Value) -> Result<Self, Rejected> {
        match validate(&raw) {
            Ok(()) => Ok(Self { raw }),
            Err(problem) => Err(Rejected {
                source: raw,
                problem,
            }),
        }
    }
    pub fn source(&self) -> &Value {
        &self.raw
    }
    pub fn add_group(&self, id: &str, name: &str) -> Result<Change, Problem> {
        let mut raw = self.raw.clone();
        raw["groups"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":id,"name":name,"presets":[]}));
        raw["active"] = json!(id);
        self.change(raw)
    }
    pub fn save_preset(
        &self,
        group_id: &str,
        id: &str,
        name: &str,
        curve: Value,
    ) -> Result<Change, Problem> {
        validate_curve(&curve)?;
        let mut raw = self.raw.clone();
        let group = raw["groups"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|g| g["id"] == group_id)
            .ok_or(Problem::MissingTarget)?;
        group["presets"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":id,"name":name,"color":"","curve":curve}));
        self.change(raw)
    }
    /// Caller supplies the already specified/entered duplicate name; this model
    /// does not invent a suffix or other visible copy.
    pub fn duplicate_preset(&self, id: &str, new_id: &str, name: &str) -> Result<Change, Problem> {
        let mut raw = self.raw.clone();
        let group = raw["groups"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|g| {
                g["presets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|p| p["id"] == id)
            })
            .ok_or(Problem::MissingTarget)?;
        let mut preset = group["presets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .unwrap()
            .clone();
        preset["id"] = json!(new_id);
        preset["name"] = json!(name);
        group["presets"].as_array_mut().unwrap().push(preset);
        self.change(raw)
    }
    pub fn set_color(&self, id: &str, color: &str) -> Result<Change, Problem> {
        if !(color.is_empty()
            || color.len() == 7
                && color.starts_with('#')
                && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit))
        {
            return Err(Problem::Preset);
        }
        let mut raw = self.raw.clone();
        let mut found = false;
        for group in raw["groups"].as_array_mut().unwrap() {
            for preset in group["presets"].as_array_mut().unwrap() {
                if preset["id"] == id {
                    preset["color"] = json!(color);
                    found = true;
                }
            }
        }
        if !found {
            return Err(Problem::MissingTarget);
        }
        self.change(raw)
    }
    pub fn active(&self) -> &str {
        let groups = self.raw["groups"].as_array().unwrap();
        groups
            .iter()
            .find(|g| g["id"] == self.raw["active"])
            .unwrap_or(&groups[0])["id"]
            .as_str()
            .unwrap()
    }
    pub fn move_preset(
        &self,
        id: &str,
        group_id: &str,
        before: Option<&str>,
    ) -> Result<Change, Problem> {
        let mut raw = self.raw.clone();
        let groups = raw["groups"].as_array_mut().unwrap();
        let destination = groups
            .iter()
            .position(|g| g["id"] == group_id)
            .ok_or(Problem::MissingTarget)?;
        let preset = groups
            .iter()
            .flat_map(|g| g["presets"].as_array().unwrap())
            .find(|p| p["id"] == id)
            .cloned()
            .ok_or(Problem::MissingTarget)?;
        if before == Some(id) {
            return self.change(raw);
        }
        for group in groups.iter_mut() {
            group["presets"]
                .as_array_mut()
                .unwrap()
                .retain(|p| p["id"] != id);
        }
        let presets = groups[destination]["presets"].as_array_mut().unwrap();
        let at = before
            .and_then(|id| presets.iter().position(|p| p["id"] == id))
            .unwrap_or(presets.len());
        presets.insert(at, preset);
        self.change(raw)
    }
    pub fn rename(&self, id: &str, name: &str) -> Result<Change, Problem> {
        if name.is_empty() || name.encode_utf16().count() > 100 {
            return Err(Problem::Name);
        }
        let mut raw = self.raw.clone();
        let mut found = false;
        for g in raw["groups"].as_array_mut().unwrap() {
            if g["id"] == id {
                g["name"] = json!(name);
                found = true;
                break;
            }
            for p in g["presets"].as_array_mut().unwrap() {
                if p["id"] == id {
                    p["name"] = json!(name);
                    found = true;
                    break;
                }
            }
        }
        if !found {
            return Err(Problem::MissingTarget);
        }
        self.change(raw)
    }
    pub fn remove(&self, id: &str) -> Result<Change, Problem> {
        let mut raw = self.raw.clone();
        let groups = raw["groups"].as_array_mut().unwrap();
        if let Some(index) = groups.iter().position(|g| g["id"] == id) {
            if groups.len() == 1 {
                return Err(Problem::LastGroup);
            }
            groups.remove(index);
            // Match tagged group deletion's active-first behavior.
            raw["active"] = raw["groups"][0]["id"].clone();
        } else {
            let mut found = false;
            for g in groups {
                let presets = g["presets"].as_array_mut().unwrap();
                let n = presets.len();
                presets.retain(|p| p["id"] != id);
                found |= n != presets.len();
            }
            if !found {
                return Err(Problem::MissingTarget);
            }
        }
        self.change(raw)
    }
    /// S095: retain non-conflicting IDs, use one supplied fresh ID per conflict.
    pub fn merge(&self, incoming: &Library, ids: &[String]) -> Result<Change, Problem> {
        let mut used: BTreeSet<String> = self.raw["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| {
                std::iter::once(g["id"].as_str().unwrap().to_owned()).chain(
                    g["presets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| p["id"].as_str().unwrap().to_owned()),
                )
            })
            .collect();
        let mut ids = ids.iter();
        let mut raw = self.raw.clone();
        let mut assign = |value: &mut Value| -> Result<(), Problem> {
            let old = value.as_str().unwrap();
            if used.contains(old) {
                let id = ids.next().ok_or(Problem::IdCollision)?;
                if !used.insert(id.clone()) {
                    return Err(Problem::IdCollision);
                }
                *value = json!(id);
            } else {
                used.insert(old.into());
            }
            Ok(())
        };
        for group in incoming.raw["groups"].as_array().unwrap() {
            let mut group = group.clone();
            assign(&mut group["id"])?;
            for preset in group["presets"].as_array_mut().unwrap() {
                assign(&mut preset["id"])?;
            }
            raw["active"] = group["id"].clone();
            raw["groups"].as_array_mut().unwrap().push(group);
        }
        if ids.next().is_some() {
            return Err(Problem::IdCollision);
        }
        self.change(raw)
    }
    fn change(&self, after: Value) -> Result<Change, Problem> {
        validate(&after)?;
        Ok(Change {
            before: self.raw.clone(),
            after,
        })
    }
}
#[derive(Debug, Clone)]
pub struct Change {
    before: Value,
    after: Value,
}
impl Change {
    pub fn preview(&self) -> &Value {
        &self.after
    }
    /// The full before-image is required for rename, deletion and replacement,
    /// including unknown fields. Host must commit with a CAS and durable archive
    /// in one transaction; failed archival never publishes the edited library.
    pub fn commit(self, store: &mut impl LibraryArchive) -> crate::Result<Library> {
        store.archive_and_publish(
            STORAGE_KEY,
            &self.before,
            &self.after,
            std::slice::from_ref(&self.before),
        )?;
        Ok(Library { raw: self.after })
    }
}
pub trait LibraryArchive {
    fn archive_and_publish(
        &mut self,
        key: &str,
        expected: &Value,
        next: &Value,
        archives: &[Value],
    ) -> crate::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveColor {
    Accent,
    Mono,
}
pub fn curve_color(library: &Library) -> CurveColor {
    if library.source()["preferences"]["palette"] == "mono" {
        CurveColor::Mono
    } else {
        CurveColor::Accent
    }
}
/// S094/S095: edits and immediate Undo keep working in memory on IO failure.
/// All intermediate before-images remain queued for durable archival on Retry.
pub struct LibrarySession {
    pub current: Library,
    durable: Value,
    pending_archives: Vec<Value>,
    undo: Vec<Value>,
    pub error: Option<crate::ServiceErrorCode>,
}
impl LibrarySession {
    pub fn new(library: Library) -> Self {
        Self {
            durable: library.raw.clone(),
            current: library,
            pending_archives: vec![],
            undo: vec![],
            error: None,
        }
    }
    pub fn edit(&mut self, change: Change, store: &mut impl LibraryArchive) -> crate::Result<()> {
        if change.before != self.current.raw {
            return Err(crate::ServiceError::message(
                crate::ServiceErrorCode::InvalidInput,
                "stale easing library edit",
            ));
        }
        self.undo.push(change.before.clone());
        self.pending_archives.push(change.before);
        self.current.raw = change.after;
        self.retry(store)
    }
    pub fn undo(&mut self, store: &mut impl LibraryArchive) -> crate::Result<bool> {
        let Some(before) = self.undo.pop() else {
            return Ok(false);
        };
        self.pending_archives.push(self.current.raw.clone());
        self.current.raw = before;
        self.retry(store)?;
        Ok(true)
    }
    pub fn retry(&mut self, store: &mut impl LibraryArchive) -> crate::Result<()> {
        if self.pending_archives.is_empty() {
            return Ok(());
        }
        match store.archive_and_publish(
            STORAGE_KEY,
            &self.durable,
            &self.current.raw,
            &self.pending_archives,
        ) {
            Ok(()) => {
                self.durable = self.current.raw.clone();
                self.pending_archives.clear();
                self.error = None;
                Ok(())
            }
            Err(error) => {
                self.error = Some(error.code);
                Err(error)
            }
        }
    }
    pub fn has_unpersisted_work(&self) -> bool {
        !self.pending_archives.is_empty()
    }
}

pub fn mirror(curve: &Value) -> Result<Value, Problem> {
    validate_curve(curve)?;
    let mut next = curve.clone();
    let points = next["points"].as_array_mut().unwrap();
    points.reverse();
    for p in points {
        p["t"] = json!(1. - number(&p["t"]).unwrap());
        p["v"] = json!(1. - number(&p["v"]).unwrap());
        for (a, b) in [("inInterp", "outInterp"), ("inEase", "outEase")] {
            let old = p[a].clone();
            p[a] = p[b].clone();
            p[b] = old;
        }
    }
    validate_curve(&next)?;
    Ok(next)
}

/// Tagged loadCurve/normalize: one selected key includes its next neighbour,
/// or previous at the end. Disjoint selections never become an invented span.
/// Exact normalized speeds use dt/dv; arbitrary key/handle metadata survives.
pub fn load_curve(keys: &[Value], selected: &[String]) -> Result<Value, Problem> {
    let mut indices: Vec<_> = keys
        .iter()
        .enumerate()
        .filter(|(_, key)| {
            key["i"]
                .as_str()
                .is_some_and(|id| selected.iter().any(|s| s == id))
        })
        .map(|(i, _)| i)
        .collect();
    if indices.len() == 1 {
        let i = indices[0];
        indices = if i + 1 < keys.len() {
            vec![i, i + 1]
        } else if i > 0 {
            vec![i - 1, i]
        } else {
            vec![]
        };
    }
    if indices.len() < 2 || indices.windows(2).any(|pair| pair[1] != pair[0] + 1) {
        return Err(Problem::NonConsecutive);
    }
    let first = &keys[indices[0]];
    let last = &keys[*indices.last().unwrap()];
    let from = number(&first["t"]).ok_or(Problem::Coordinates)?;
    let value = number(&first["v"]).ok_or(Problem::Coordinates)?;
    let dt = number(&last["t"]).ok_or(Problem::Coordinates)? - from;
    let dv = number(&last["v"]).ok_or(Problem::Coordinates)? - value;
    if dt <= 0. || !dt.is_finite() || !dv.is_finite() || dv.abs() < 1e-9 {
        return Err(Problem::FlatSpan);
    }
    let mut points = Vec::new();
    for index in indices {
        let mut point = keys[index].clone();
        let t = number(&point["t"]).ok_or(Problem::Coordinates)?;
        let v = number(&point["v"]).ok_or(Problem::Coordinates)?;
        point["t"] = json!((t - from) / dt);
        point["v"] = json!((v - value) / dv);
        for side in ["inEase", "outEase"] {
            let speed = number(&point[side]["speed"]).ok_or(Problem::Handles)? * dt / dv;
            if !speed.is_finite() {
                return Err(Problem::Handles);
            }
            point[side]["speed"] = json!(speed);
        }
        points.push(point);
    }
    Ok(json!({"points":points}))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sections {
    pub incoming: bool,
    pub middle: bool,
    pub outgoing: bool,
}
impl Sections {
    /// macOS-only native app: Shift/Option/Command select the three sections.
    pub fn modifiers(shift: bool, option: bool, command: bool, control: bool) -> Self {
        if !(shift || option || command || control) {
            Self {
                incoming: true,
                middle: true,
                outgoing: true,
            }
        } else {
            Self {
                incoming: shift,
                middle: option,
                outgoing: command,
            }
        }
    }
    pub fn eligible(self, selected: &[bool], index: usize, outgoing: bool) -> bool {
        if selected.get(index) != Some(&true) {
            return false;
        }
        let neighbour = if outgoing {
            index.checked_add(1)
        } else {
            index.checked_sub(1)
        };
        match neighbour.and_then(|i| selected.get(i)) {
            None => false,
            Some(true) => self.middle,
            Some(false) => {
                if outgoing {
                    self.outgoing
                } else {
                    self.incoming
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn point(t: f64, v: f64) -> Value {
        json!({"t":t,"v":v,"inInterp":"bezier","outInterp":"linear","inEase":{"speed":2.,"influence":33.,"future":1},"outEase":{"speed":3.,"influence":66.},"autoBezier":true,"continuous":true,"futurePoint":1})
    }
    fn curve() -> Value {
        json!({"points":[point(0.,0.),point(1.,1.)],"futureCurve":1})
    }
    fn raw() -> Value {
        json!({"version":1,"groups":[{"id":"g","name":"Group","presets":[{"id":"p","name":"Preset","color":"#abcdef","curve":curve(),"futurePreset":1}],"futureGroup":1},{"id":"other","name":"Other","presets":[]}],"active":"g","preferences":{"columns":0,"size":44,"future":1},"futureLibrary":1})
    }
    #[test]
    fn exact_authored_json_and_unknown_fields_survive() {
        let input = raw();
        let l = Library::read(input.clone()).unwrap();
        assert_eq!(l.source(), &input);
        assert_eq!(
            l.source()["groups"][0]["presets"][0]["curve"]["points"][0]["autoBezier"],
            true
        );
    }
    #[test]
    fn invalid_or_future_library_is_retained_not_reset() {
        let mut source = raw();
        source["version"] = json!(2);
        let rejected = Library::read(source.clone()).unwrap_err();
        assert_eq!(rejected.source, source);
        assert_eq!(rejected.problem, Problem::UnsupportedVersion);
    }
    #[test]
    fn global_ids_and_curve_limits_match_tag() {
        let mut source = raw();
        source["groups"][1]["id"] = json!("p");
        assert_eq!(
            Library::read(source).unwrap_err().problem,
            Problem::DuplicateId
        );
        let mut c = curve();
        c["points"][1]["t"] = json!(0.);
        assert_eq!(validate_curve(&c), Err(Problem::Coordinates));
        let mut c = curve();
        c["points"][0]["outEase"]["influence"] = json!(0.);
        assert_eq!(validate_curve(&c), Err(Problem::Handles));
    }
    #[test]
    fn overshoot_values_and_full_precision_are_not_clamped() {
        let mut c = curve();
        c["points"]
            .as_array_mut()
            .unwrap()
            .insert(1, point(0.3333333333333333, 1.25));
        assert!(validate_curve(&c).is_ok());
        assert_eq!(mirror(&c).unwrap()["points"][1]["v"], json!(-0.25));
    }
    #[test]
    fn mirror_swaps_handles_without_negating_speed() {
        let c = mirror(&curve()).unwrap();
        assert_eq!(c["points"][0]["inEase"]["speed"], 3.);
        assert_eq!(c["points"][0]["outEase"]["speed"], 2.);
        assert_eq!(c["futureCurve"], 1);
        assert_eq!(c["points"][0]["futurePoint"], 1);
    }
    #[test]
    fn move_preserves_payload_and_missing_before_appends() {
        let l = Library::read(raw()).unwrap();
        let c = l.move_preset("p", "other", Some("missing")).unwrap();
        assert_eq!(
            c.preview()["groups"][1]["presets"][0],
            raw()["groups"][0]["presets"][0]
        );
        assert!(
            c.preview()["groups"][0]["presets"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(l.source(), &raw());
    }
    #[test]
    fn merge_remaps_every_id_and_is_atomic_on_collision() {
        let l = Library::read(raw()).unwrap();
        let ids = vec!["new-group".into(), "new-preset".into(), "last-group".into()];
        let change = l.merge(&l, &ids).unwrap();
        assert_eq!(change.preview()["active"], "last-group");
        assert_eq!(
            change.preview()["groups"][2]["presets"][0]["futurePreset"],
            1
        );
        assert!(
            l.merge(&l, &["g".into(), "p".into(), "other".into()])
                .is_err()
        );
        assert_eq!(l.source(), &raw());
    }
    #[test]
    fn archive_failure_never_publishes_and_success_retains_full_before() {
        struct Store {
            value: Value,
            archive: Vec<Value>,
            fail: bool,
        }
        impl LibraryArchive for Store {
            fn archive_and_publish(
                &mut self,
                key: &str,
                expected: &Value,
                next: &Value,
                archives: &[Value],
            ) -> crate::Result<()> {
                assert_eq!(key, STORAGE_KEY);
                if self.fail || &self.value != expected {
                    return Err(crate::ServiceError::message(
                        crate::ServiceErrorCode::DiskFull,
                        "fixture archive failure",
                    ));
                }
                self.archive.extend_from_slice(archives);
                self.value = next.clone();
                Ok(())
            }
        }
        let l = Library::read(raw()).unwrap();
        let mut store = Store {
            value: raw(),
            archive: vec![],
            fail: true,
        };
        assert!(l.remove("p").unwrap().commit(&mut store).is_err());
        assert_eq!(store.value, raw());
        store.fail = false;
        let next = l.remove("g").unwrap().commit(&mut store).unwrap();
        assert_eq!(store.archive, vec![raw()]);
        assert_eq!(next.active(), "other");
        assert!(matches!(next.remove("other"), Err(Problem::LastGroup)));
    }
    #[test]
    fn fallback_active_is_projection_only_and_rename_preserves_extras() {
        let mut input = raw();
        input["active"] = json!("missing");
        let l = Library::read(input).unwrap();
        assert_eq!(l.active(), "g");
        assert_eq!(l.source()["active"], "missing");
        let change = l.rename("p", "New").unwrap();
        assert_eq!(
            change.preview()["groups"][0]["presets"][0]["futurePreset"],
            1
        );
        assert!(matches!(
            l.rename("p", &"😀".repeat(51)),
            Err(Problem::Name)
        ));
    }
    #[test]
    fn section_modifiers_match_mac_and_edges_require_neighbours() {
        assert_eq!(
            Sections::modifiers(false, false, false, false),
            Sections {
                incoming: true,
                middle: true,
                outgoing: true
            }
        );
        let middle = Sections::modifiers(false, true, false, false);
        assert!(middle.eligible(&[false, true, true], 1, true));
        assert!(!middle.eligible(&[false, true, true], 1, false));
        assert!(!middle.eligible(&[true], 0, false));
        assert_eq!(
            Sections::modifiers(false, false, false, true),
            Sections {
                incoming: false,
                middle: false,
                outgoing: false
            }
        );
    }
    #[test]
    fn save_duplicate_and_colour_preserve_curve_and_all_future_payloads() {
        let library = Library::read(raw()).unwrap();
        let change = library
            .duplicate_preset("p", "copy", "Entered name")
            .unwrap();
        let copy = &change.preview()["groups"][0]["presets"][1];
        assert_eq!(copy["curve"], curve());
        assert_eq!(copy["futurePreset"], 1);
        assert_eq!(copy["name"], "Entered name");
        let change = library
            .save_preset("other", "saved", "Custom", curve())
            .unwrap();
        assert_eq!(
            change.preview()["groups"][1]["presets"][0]["curve"],
            curve()
        );
        let change = library.set_color("p", "#AbC123").unwrap();
        assert_eq!(
            change.preview()["groups"][0]["presets"][0]["color"],
            "#AbC123"
        );
        assert!(library.set_color("p", "red").is_err());
        let change = library.add_group("new", "Entered group").unwrap();
        assert_eq!(change.preview()["active"], "new");
        assert!(library.add_group("g", "collision").is_err());
    }
    #[test]
    fn load_curve_uses_single_key_neighbour_and_scales_signed_speed_exactly() {
        let mut a = point(4., 20.);
        a["i"] = json!("a");
        let mut b = point(6., 10.);
        b["i"] = json!("b");
        let curve = load_curve(&[a.clone(), b.clone()], &["b".into()]).unwrap();
        assert_eq!(curve["points"][0]["t"], 0.);
        assert_eq!(curve["points"][1]["v"], 1.);
        assert_eq!(curve["points"][0]["inEase"]["speed"], -0.4);
        assert_eq!(curve["points"][0]["futurePoint"], 1);
        let mut c = point(7., 10.);
        c["i"] = json!("c");
        assert_eq!(
            load_curve(&[a, b.clone(), c.clone()], &["a".into(), "c".into()]),
            Err(Problem::NonConsecutive)
        );
        assert_eq!(
            load_curve(&[b, c], &["b".into(), "c".into()]),
            Err(Problem::FlatSpan)
        );
    }
    #[test]
    fn stored_graph_hues_project_to_accent_without_rewriting_preferences() {
        for palette in ["blue", "green", "violet", "native", "mono"] {
            let mut value = raw();
            value["preferences"]["palette"] = json!(palette);
            let library = Library::read(value.clone()).unwrap();
            assert_eq!(
                curve_color(&library),
                if palette == "mono" {
                    CurveColor::Mono
                } else {
                    CurveColor::Accent
                }
            );
            assert_eq!(library.source(), &value);
        }
    }
    #[test]
    fn import_keeps_nonconflicting_ids() {
        let library = Library::read(raw()).unwrap();
        let mut incoming = raw();
        incoming["groups"][0]["id"] = json!("unique");
        incoming["groups"][0]["presets"][0]["id"] = json!("unique-preset");
        incoming["groups"].as_array_mut().unwrap().pop();
        let change = library
            .merge(&Library::read(incoming).unwrap(), &[])
            .unwrap();
        assert_eq!(change.preview()["groups"][2]["id"], "unique");
        assert_eq!(
            change.preview()["groups"][2]["presets"][0]["id"],
            "unique-preset"
        );
    }
    #[test]
    fn memory_edits_and_undo_survive_disk_failure_and_retry_archives_every_step() {
        struct Store {
            value: Value,
            archives: Vec<Value>,
            fail: bool,
        }
        impl LibraryArchive for Store {
            fn archive_and_publish(
                &mut self,
                _: &str,
                expected: &Value,
                next: &Value,
                archives: &[Value],
            ) -> crate::Result<()> {
                if self.fail {
                    return Err(crate::ServiceError::message(
                        crate::ServiceErrorCode::DiskFull,
                        "fixture disk full",
                    ));
                }
                assert_eq!(&self.value, expected);
                self.archives.extend_from_slice(archives);
                self.value = next.clone();
                Ok(())
            }
        }
        let mut session = LibrarySession::new(Library::read(raw()).unwrap());
        let mut store = Store {
            value: raw(),
            archives: vec![],
            fail: true,
        };
        let change = session.current.remove("p").unwrap();
        assert!(session.edit(change, &mut store).is_err());
        assert!(
            session.current.source()["groups"][0]["presets"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(session.has_unpersisted_work());
        assert_eq!(store.value, raw());
        assert!(session.undo(&mut store).is_err());
        assert_eq!(session.current.source(), &raw());
        store.fail = false;
        session.retry(&mut store).unwrap();
        assert!(!session.has_unpersisted_work());
        assert_eq!(store.archives.len(), 2);
        assert_eq!(store.value, raw());
    }
}
