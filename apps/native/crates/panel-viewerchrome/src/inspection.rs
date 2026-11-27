//! Semantic inspection for v4/MCP hosts; IDs address the same model actions as
//! local input. No MCP tool catalog or v1–3 service contract is replaced here.
use crate::*;
use pm_model::{Map, Value};
pub const ELEMENT_IDS: &[&str] = &[
    "viewer.zoom",
    "viewer.zoom.in",
    "viewer.zoom.out",
    "viewer.fit",
    "viewer.recovery.fit",
    "viewer.quality",
    "viewer.controls",
    "viewer.guides",
    "viewer.safe-areas",
    "viewer.transport.play",
    "viewer.transport.seek",
    "viewer.transport.step",
    "viewer.work.in",
    "viewer.work.out",
    "viewer.work.reset",
    "viewer.source.show",
    "viewer.source.toggle",
    "viewer.source.play",
    "viewer.source.close",
    "viewer.source.seek",
    "viewer.context",
    "viewer.selection",
    "viewer.selection.add",
    "viewer.selection.toggle",
    "viewer.scene.transform",
    "viewer.scene.axes",
    "viewer.scene.view",
    "viewer.scene.navigation",
];
impl ViewModel {
    /// Selection IDs are paged separately. This response never materializes
    /// the layer list or serializes document/media contents.
    pub fn inspect(&self) -> Value {
        let mut map = Map::new();
        map.insert("project", Value::from(self.stamp.project.as_str()));
        // Revisions/identity are decimal strings to survive JS's 53-bit limit.
        map.insert("document", Value::from(self.stamp.document.to_string()));
        map.insert("revision", Value::from(self.stamp.revision.to_string()));
        map.insert(
            "stateRevision",
            Value::from(self.stamp.state_revision.to_string()),
        );
        map.insert(
            "elements",
            Value::Array(ELEMENT_IDS.iter().map(|id| Value::from(*id)).collect()),
        );
        map.insert("fit", Value::Bool(self.chrome.navigation.fit));
        map.insert("zoom", Value::Number(self.chrome.navigation.zoom));
        map.insert(
            "pan",
            Value::Array(vec![
                Value::Number(self.chrome.navigation.pan.x),
                Value::Number(self.chrome.navigation.pan.y),
            ]),
        );
        map.insert("quality", Value::from(self.chrome.quality.id()));
        map.insert(
            "effectiveQuality",
            Value::Number(self.chrome.effective_quality),
        );
        map.insert("controls", Value::Bool(self.chrome.show_controls));
        map.insert("guides", Value::Bool(self.chrome.overlays.guides_visible));
        map.insert(
            "safeAreas",
            Value::Bool(self.chrome.overlays.safe_areas_visible),
        );
        map.insert("time", Value::Number(self.transport.time));
        map.insert("playing", Value::Bool(self.transport.playing));
        map.insert("fps", Value::Number(self.transport.fps));
        map.insert("duration", Value::Number(self.transport.duration));
        map.insert(
            "work",
            Value::Array(vec![
                Value::Number(self.transport.work.0),
                Value::Number(self.transport.work.1),
            ]),
        );
        map.insert(
            "selectionCount",
            Value::Number(self.selection.layers().len() as f64),
        );
        Value::Object(map)
    }
}
/// Evaluated world bounds, active and pickable facts come from the owning
/// viewer-tools evaluator at the same document/revision/time. Conditional
/// evaluator errors must be surfaced before constructing this list.
#[derive(Clone, Debug, PartialEq)]
pub struct MarqueeCandidate {
    pub id: String,
    pub bounds: Rect,
    pub active: bool,
    pub pickable: bool,
}
#[derive(Clone, Debug)]
pub struct Marquee {
    pub start: Point,
    pub before: Vec<String>,
    pub shift: bool,
    pub rectangle: Option<Rect>,
    cancelled: bool,
}
impl Marquee {
    pub fn new(start: Point, before: Vec<String>, shift: bool) -> Result<Self, Error> {
        start.checked()?;
        Ok(Self {
            start,
            before,
            shift,
            rectangle: None,
            cancelled: false,
        })
    }
    /// The host's approved drag recognizer decides when movement starts.
    pub fn update(&mut self, pointer: Point, moved: bool) -> Result<(), Error> {
        pointer.checked()?;
        if self.cancelled {
            return Err(Error::Invalid("cancelled marquee"));
        }
        if moved {
            self.rectangle = Some(Rect::between(self.start, pointer));
        }
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.rectangle = None;
        self.cancelled = true;
    }
    pub fn finish(&self, candidates: &[MarqueeCandidate]) -> Result<Vec<String>, Error> {
        if self.cancelled {
            return Ok(self.before.clone());
        }
        let Some(rect) = self.rectangle else {
            return Ok(if self.shift {
                self.before.clone()
            } else {
                vec![]
            });
        };
        let mut selected: std::collections::HashSet<String> = if self.shift {
            self.before.iter().cloned().collect()
        } else {
            Default::default()
        };
        let mut incoming = vec![];
        let mut seen = std::collections::HashSet::new();
        for c in candidates {
            c.bounds.min.checked()?;
            c.bounds.max.checked()?;
            if c.id.is_empty()
                || c.bounds.min.x > c.bounds.max.x
                || c.bounds.min.y > c.bounds.max.y
                || !seen.insert(c.id.as_str())
            {
                return Err(Error::Invalid("marquee candidate"));
            }
            if c.active && c.pickable && rect.contains(c.bounds.min) && rect.contains(c.bounds.max)
            {
                if self.shift && !selected.insert(c.id.clone()) {
                    selected.remove(&c.id);
                } else {
                    selected.insert(c.id.clone());
                    incoming.push(c.id.clone());
                }
            }
        }
        let mut ids = if self.shift {
            self.before
                .iter()
                .filter(|id| selected.contains(*id))
                .cloned()
                .collect::<Vec<_>>()
        } else {
            vec![]
        };
        ids.extend(incoming);
        Ok(ids)
    }
}
