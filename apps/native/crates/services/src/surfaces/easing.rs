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
    /// One caller-supplied fresh ID per incoming group/preset, in traversal order.
    pub fn merge(&self, incoming: &Library, ids: &[String]) -> Result<Change, Problem> {
        let count = incoming.raw["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| 1 + g["presets"].as_array().unwrap().len())
            .sum::<usize>();
        if count != ids.len() {
            return Err(Problem::IdCollision);
        }
        let mut ids = ids.iter();
        let mut raw = self.raw.clone();
        for group in incoming.raw["groups"].as_array().unwrap() {
            let mut group = group.clone();
            group["id"] = json!(ids.next().unwrap());
            for preset in group["presets"].as_array_mut().unwrap() {
                preset["id"] = json!(ids.next().unwrap());
            }
            raw["active"] = group["id"].clone();
            raw["groups"].as_array_mut().unwrap().push(group);
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
        store.archive_and_publish(STORAGE_KEY, &self.before, &self.after)?;
        Ok(Library { raw: self.after })
    }
}
pub trait LibraryArchive {
    fn archive_and_publish(
        &mut self,
        key: &str,
        expected: &Value,
        next: &Value,
    ) -> crate::Result<()>;
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
            ) -> crate::Result<()> {
                assert_eq!(key, STORAGE_KEY);
                if self.fail || &self.value != expected {
                    return Err(crate::ServiceError::message(
                        crate::ServiceErrorCode::DiskFull,
                        "fixture archive failure",
                    ));
                }
                self.archive.push(expected.clone());
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
}
