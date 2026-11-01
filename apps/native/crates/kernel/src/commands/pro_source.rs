//! W082: source-monitor range resolution and targeted insert/overwrite.
use super::pro_edit::{TrackTarget, in_scope};
use super::pro_links;
use super::pro_support::{Clip, EPS, invalid, number, set_content};
use super::{Cx, layers, values};
use crate::CommandError;
use pm_model::{Map, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EditPoints {
    pub source_in: Option<f64>,
    pub source_out: Option<f64>,
    pub timeline_in: Option<f64>,
    pub timeline_out: Option<f64>,
    pub playhead: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedEdit {
    pub source_in: f64,
    pub source_out: f64,
    pub timeline_in: f64,
    pub timeline_out: f64,
}
impl EditPoints {
    /// Half-open ranges at unit playback rate. Exactly three marks determine
    /// the fourth; four matching marks are accepted, conflicting marks refuse.
    /// With both source marks and no timeline marks, playhead supplies In.
    pub fn resolve(self) -> Result<ResolvedEdit, CommandError> {
        let mut points = [
            self.source_in,
            self.source_out,
            self.timeline_in,
            self.timeline_out,
        ];
        if points[2].is_none() && points[3].is_none() && points[0].is_some() && points[1].is_some()
        {
            points[2] = Some(number(self.playhead)?);
        }
        for n in points.iter().flatten() {
            if number(*n)? < 0.0 {
                return Err(invalid("Edit marks must be nonnegative."));
            }
        }
        if points.iter().flatten().count() < 3 {
            return Err(invalid("Set at least three source/timeline edit points."));
        }
        match points {
            [None, Some(so), Some(ti), Some(to)] => points[0] = Some(so - (to - ti)),
            [Some(si), None, Some(ti), Some(to)] => points[1] = Some(si + (to - ti)),
            [Some(si), Some(so), None, Some(to)] => points[2] = Some(to - (so - si)),
            [Some(si), Some(so), Some(ti), None] => points[3] = Some(ti + (so - si)),
            _ => (),
        }
        let [si, so, ti, to] = points.map(|p| p.expect("three marks resolved"));
        for n in [si, so, ti, to] {
            if number(n)? < 0.0 {
                return Err(invalid("Resolved edit points must be nonnegative."));
            }
        }
        if so <= si || to <= ti || ((so - si) - (to - ti)).abs() > EPS {
            return Err(invalid(
                "Source and timeline ranges must have the same positive duration.",
            ));
        }
        Ok(ResolvedEdit {
            source_in: si,
            source_out: so,
            timeline_in: ti,
            timeline_out: to,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceEditMode {
    Insert,
    Overwrite,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceMedia {
    Video,
    Audio,
}
impl SourceMedia {
    fn name(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct SourceTarget {
    pub media: SourceMedia,
    pub track: TrackTarget,
    pub layer_id: String,
}
/// Needed only when a targeted clip retains both sides of an edit boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceSplit {
    pub layer_id: String,
    pub tail_id: String,
    pub tail_link_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProSourceCommand {
    pub comp_id: Option<String>,
    pub asset_id: String,
    pub points: EditPoints,
    pub mode: SourceEditMode,
    pub targets: Vec<SourceTarget>,
    pub link_id: Option<String>,
    pub splits: Vec<SourceSplit>,
}
impl ProSourceCommand {
    pub fn label(&self) -> String {
        match self.mode {
            SourceEditMode::Insert => "Insert Source",
            SourceEditMode::Overwrite => "Overwrite Source",
        }
        .into()
    }
    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let range = self.points.resolve()?;
        let comp = cx.comp_id(self.comp_id.as_deref());
        cx.comp_node(comp.as_deref())?;
        let duration = range.timeline_out - range.timeline_in;
        if duration + EPS < 1.0 / cx.fps(comp.as_deref())
            || range.timeline_out > cx.duration(comp.as_deref()) + EPS
        {
            return Err(invalid(
                "The source edit must fit the composition and retain one frame.",
            ));
        }
        let asset = cx
            .root()
            .get("assets")
            .and_then(|a| a.get(&self.asset_id))
            .ok_or_else(|| invalid("Choose an imported source asset."))?
            .to_value();
        let kind =
            values::text(&asset, "kind").ok_or_else(|| invalid("The source has no media kind."))?;
        if !matches!(kind, "audio" | "video")
            || !values::num(&asset, "dur")
                .is_some_and(|d| d.is_finite() && d + EPS >= range.source_out)
        {
            return Err(invalid("The edit exceeds available source handles."));
        }
        if self.targets.is_empty() || self.targets.len() > 2 {
            return Err(invalid("Target video, audio, or both."));
        }
        let mut fresh = HashSet::new();
        let mut tracks = Vec::new();
        let mut media = HashSet::new();
        for t in &self.targets {
            fresh_id(cx, &t.layer_id, &mut fresh)?;
            if !media.insert(t.media.name()) || tracks.contains(&t.track) {
                return Err(invalid("Use distinct media and track targets."));
            }
            if t.media == SourceMedia::Video && kind != "video"
                || t.media == SourceMedia::Audio
                    && kind == "video"
                    && !values::flag(&asset, "hasAudio")
            {
                return Err(invalid(
                    "The source does not provide the targeted media stream.",
                ));
            }
            if let Some(group) = &t.track.group_id {
                let (loc, g) = cx.layer_mut_checked(group)?;
                if loc.comp_id != comp || values::text(&g, "type") != Some("group") {
                    return Err(invalid("Target group must belong to the edit composition."));
                }
            }
            tracks.push(t.track.clone());
        }
        if self.targets.len() == 2 {
            fresh_link(
                cx,
                self.link_id
                    .as_deref()
                    .ok_or_else(|| invalid("A/V inserts require a fresh link id."))?,
            )?;
        } else if self.link_id.is_some() {
            return Err(invalid("A single stream does not take a pair link id."));
        }
        let original = cx.layers(comp.as_deref());
        let mut next = Vec::new();
        let mut changes = HashMap::<String, Vec<Value>>::new();
        let mut used_splits = HashSet::new();
        for layer in &original {
            if !in_scope(layer, &tracks) {
                next.push(layer.clone());
                continue;
            }
            let id = values::text(layer, "id").ok_or_else(|| invalid("Missing clip id."))?;
            let from =
                number(values::num(layer, "from").ok_or_else(|| invalid("Missing clip start."))?)?;
            let end = number(
                from + values::num(layer, "dur")
                    .ok_or_else(|| invalid("Missing clip duration."))?,
            )?;
            let affected = match self.mode {
                SourceEditMode::Insert => end > range.timeline_in + EPS,
                SourceEditMode::Overwrite => {
                    from < range.timeline_out - EPS && end > range.timeline_in + EPS
                }
            };
            if !affected {
                next.push(layer.clone());
                continue;
            }
            let clip = Clip::editable(cx, id)?;
            let mut pieces = Vec::new();
            if self.mode == SourceEditMode::Insert && clip.from >= range.timeline_in - EPS {
                let mut c = clip.clone();
                c.from += duration;
                pieces.push(c.into_value(cx, false)?);
            } else {
                let cut_end = if self.mode == SourceEditMode::Insert {
                    range.timeline_in
                } else {
                    range.timeline_out
                };
                let has_head = clip.from < range.timeline_in - EPS;
                let has_tail = clip.end() > cut_end + EPS;
                if has_head {
                    let mut head = clip.clone();
                    head.dur = range.timeline_in - head.from;
                    values::obj(&mut head.value).insert("transitionOut", Value::Null);
                    if values::text(&head.value, "type") == Some("audio") {
                        set_content(&mut head.value, "fadeOut", 0.0);
                    }
                    pieces.push(head.into_value(cx, true)?);
                }
                if has_tail {
                    let mut tail = clip.clone();
                    if has_head {
                        let split =
                            self.splits
                                .iter()
                                .find(|s| s.layer_id == id)
                                .ok_or_else(|| {
                                    invalid(
                                        "Supply a tail id for every clip split by the source edit.",
                                    )
                                })?;
                        if !used_splits.insert(id.to_owned()) {
                            return Err(invalid("Duplicate source split target."));
                        }
                        fresh_id(cx, &split.tail_id, &mut fresh)?;
                        tail.value = layers::clone_layer(&clip.value, &split.tail_id, &original);
                        tail.id = split.tail_id.clone();
                        if tail
                            .value
                            .get("editLink")
                            .is_some_and(|v| *v != Value::Null)
                        {
                            let link_id = split
                                .tail_link_id
                                .as_deref()
                                .ok_or_else(|| invalid("Supply a fresh tail pair link id."))?;
                            fresh_link(cx, link_id)?;
                            if self.link_id.as_deref() == Some(link_id) {
                                return Err(invalid(
                                    "Tail links and inserted pair need distinct ids.",
                                ));
                            }
                            let link = values::obj(&mut tail.value)
                                .get_mut("editLink")
                                .expect("checked");
                            values::obj(link).insert("id", values::s(link_id));
                        } else if split.tail_link_id.is_some() {
                            return Err(invalid("Unlinked tails do not take a link id."));
                        }
                    }
                    tail.trim_in(cut_end - clip.from)?;
                    if self.mode == SourceEditMode::Insert {
                        tail.from += duration;
                    }
                    values::obj(&mut tail.value).insert("transitionIn", Value::Null);
                    if values::text(&tail.value, "type") == Some("audio") {
                        set_content(&mut tail.value, "fadeIn", 0.0);
                    }
                    pieces.push(tail.into_value(cx, true)?);
                }
            }
            next.extend(pieces.iter().cloned());
            changes.insert(id.into(), pieces);
        }
        if used_splits.len() != self.splits.len() {
            return Err(invalid(
                "Split ids must exactly match clips retaining both sides.",
            ));
        }
        // A source edit never silently detaches a peer on an untargeted track.
        // Both members must undergo the same structural edit, including tails.
        for (id, pieces) in &changes {
            let peers = pro_links::expand(cx, std::slice::from_ref(id), true)?;
            for peer in peers.iter().filter(|p| *p != id) {
                let other = changes.get(peer).ok_or_else(|| {
                    invalid("Target both linked tracks or unlink before source editing.")
                })?;
                if pieces.len() != other.len() {
                    return Err(invalid(
                        "Linked source edits must retain matching pair fragments.",
                    ));
                }
                for (a, b) in pieces.iter().zip(other) {
                    if a.get("editLink").and_then(|l| l.get("id"))
                        != b.get("editLink").and_then(|l| l.get("id"))
                    {
                        return Err(invalid(
                            "Matching tail fragments need the same fresh pair link id.",
                        ));
                    }
                }
            }
        }
        // A fresh tail link belongs to precisely one video/audio pair.
        let mut tail_counts = HashMap::<&str, usize>::new();
        for split in &self.splits {
            if let Some(id) = split.tail_link_id.as_deref() {
                *tail_counts.entry(id).or_default() += 1;
            }
        }
        if tail_counts.values().any(|n| *n != 2) {
            return Err(invalid("Each tail link id must identify exactly one pair."));
        }
        let gone: HashSet<&str> = changes
            .iter()
            .filter(|(_, p)| p.is_empty())
            .map(|(id, _)| id.as_str())
            .collect();
        for layer in &mut next {
            if values::text(layer, "parent").is_some_and(|id| gone.contains(id)) {
                values::obj(layer).insert("parent", Value::Null);
            }
        }
        for t in &self.targets {
            let mut d = Map::new();
            d.insert("asset", values::s(&self.asset_id));
            d.insert("trim", values::property(Value::Number(range.source_in)));
            if t.media == SourceMedia::Video {
                d.insert("speed", values::property(Value::Number(1.0)));
                d.insert("embeddedAudio", Value::Bool(false));
                for key in ["w", "h"] {
                    if let Some(n) = values::num(&asset, key).filter(|n| n.is_finite() && *n > 0.0)
                    {
                        d.insert(key, Value::Number(n));
                    }
                }
            }
            let comp_node = cx.comp_node(comp.as_deref())?;
            let dim = |key: &str, default: f64| {
                comp_node
                    .get(key)
                    .and_then(crate::model::number_of)
                    .unwrap_or(default)
            };
            let mut layer = layers::make_layer(
                t.media.name(),
                &t.layer_id,
                values::text(&asset, "name"),
                (
                    dim("w", 1920.0),
                    dim("h", 1080.0),
                    cx.duration(comp.as_deref()),
                ),
                &d,
                &Map::new(),
            )?;
            let m = values::obj(&mut layer);
            m.insert("from", Value::Number(range.timeline_in));
            m.insert("dur", Value::Number(duration));
            m.insert("track", Value::Number(f64::from(t.track.track)));
            m.insert("compat", Value::Number(2.0));
            if let Some(g) = &t.track.group_id {
                m.insert("group", values::s(g));
            }
            if let Some(link_id) = &self.link_id {
                let mut link = Map::new();
                link.insert("id", values::s(link_id));
                link.insert("phase", Value::Number(range.timeline_in - range.source_in));
                m.insert("editLink", Value::Object(link));
            }
            next.insert(0, layer);
        }
        cx.replace(cx.layers_path(comp.as_deref()), Value::Array(next))
    }
}
fn fresh_id(cx: &Cx<'_>, id: &str, seen: &mut HashSet<String>) -> Result<(), CommandError> {
    if id.is_empty() || cx.find(id).is_ok() || !seen.insert(id.into()) {
        Err(invalid("Created layer ids must be fresh and unique."))
    } else {
        Ok(())
    }
}
fn fresh_link(cx: &Cx<'_>, id: &str) -> Result<(), CommandError> {
    if id.is_empty()
        || cx.all_layers().iter().any(|loc| {
            cx.layer_node(loc)
                .ok()
                .and_then(|n| n.get("editLink"))
                .and_then(|n| n.get("id"))
                .and_then(crate::model::str_of)
                == Some(id)
        })
    {
        Err(invalid("Created link ids must be fresh and nonempty."))
    } else {
        Ok(())
    }
}
