//! W080/W081/W083: atomic media edits, explicit track scope, no UI dependency.
use super::pro_links;
pub use super::pro_support::TrackTarget;
use super::pro_support::{Clip, EPS, invalid, number, same_comp, set_content};
use super::{Cx, LayerCommand, layers, values};
use crate::CommandError;
use pm_model::Value;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    In,
    Out,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SplitTail {
    pub layer_id: String,
    pub tail_id: String,
}

/// Relative deltas are composition seconds (slip converts through each rate).
/// For pointer drags use `update_gesture_from_start` with the total delta.
#[derive(Clone, Debug, PartialEq)]
pub enum ProEditCommand {
    Move {
        layer_ids: Vec<String>,
        delta: f64,
        linked: bool,
    },
    Trim {
        layer_ids: Vec<String>,
        edge: Edge,
        delta: f64,
        linked: bool,
    },
    Slip {
        layer_ids: Vec<String>,
        delta: f64,
        linked: bool,
    },
    Delete {
        layer_ids: Vec<String>,
        linked: bool,
    },
    /// Caller supplies a tail id for every expanded target, and a fresh link
    /// id for the tail pair when the source is linked. At most one pair.
    Split {
        layer_id: String,
        time: f64,
        tails: Vec<SplitTail>,
        linked: bool,
        tail_link_id: Option<String>,
    },
    /// Selected intervals must form one contiguous range on the target tracks.
    RippleDelete {
        layer_ids: Vec<String>,
        tracks: Vec<TrackTarget>,
        linked: bool,
    },
    /// In trims keep the edit's timeline In fixed; source In and duration
    /// change, and downstream clips close/open by the duration delta.
    RippleTrim {
        layer_id: String,
        edge: Edge,
        delta: f64,
        tracks: Vec<TrackTarget>,
        linked: bool,
    },
    Roll {
        left_id: String,
        right_id: String,
        delta: f64,
        linked: bool,
    },
    Slide {
        previous_id: String,
        layer_id: String,
        next_id: String,
        delta: f64,
        linked: bool,
    },
}
impl ProEditCommand {
    pub fn label(&self) -> String {
        match self {
            Self::Move { .. } => "Move Linked Clips",
            Self::Trim { .. } => "Trim Clips",
            Self::Slip { .. } => "Slip Edit",
            Self::Delete { .. } => "Delete Clips",
            Self::Split { .. } => "Split Clips",
            Self::RippleDelete { .. } => "Ripple Delete",
            Self::RippleTrim { .. } => "Ripple Trim",
            Self::Roll { .. } => "Roll Edit",
            Self::Slide { .. } => "Slide Edit",
        }
        .into()
    }
    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        match self {
            Self::Move {
                layer_ids,
                delta,
                linked,
            }
            | Self::Trim {
                layer_ids,
                delta,
                linked,
                ..
            }
            | Self::Slip {
                layer_ids,
                delta,
                linked,
            } => {
                let delta = number(*delta)?;
                for mut clip in targets(cx, layer_ids, *linked)? {
                    match self {
                        Self::Move { .. } => clip.from += delta,
                        Self::Trim { edge: Edge::In, .. } => clip.trim_in(delta)?,
                        Self::Trim {
                            edge: Edge::Out, ..
                        } => clip.dur += delta,
                        Self::Slip { .. } => clip.trim += delta * clip.rate,
                        _ => unreachable!(),
                    }
                    clip.write(cx, !matches!(self, Self::Move { .. }))?;
                }
                Ok(())
            }
            Self::Delete { layer_ids, linked } => {
                let clips = targets(cx, layer_ids, *linked)?;
                delete(cx, &clips)
            }
            Self::Split {
                layer_id,
                time,
                tails,
                linked,
                tail_link_id,
            } => split(cx, layer_id, *time, tails, *linked, tail_link_id.as_deref()),
            Self::RippleDelete {
                layer_ids,
                tracks,
                linked,
            } => {
                let clips = targets(cx, layer_ids, *linked)?;
                let comp = same_comp(&clips)?;
                require_scope(&clips, tracks)?;
                let mut spans: Vec<(f64, f64)> = clips.iter().map(|c| (c.from, c.end())).collect();
                spans.sort_by(|a, b| a.0.total_cmp(&b.0));
                let start = spans[0].0;
                let mut end = spans[0].1;
                for (s, e) in spans.into_iter().skip(1) {
                    if s > end + EPS {
                        return Err(invalid(
                            "Ripple delete requires a contiguous selected interval.",
                        ));
                    }
                    end = end.max(e);
                }
                for layer in cx.layers(comp.as_deref()) {
                    if in_scope(&layer, tracks)
                        && !clips
                            .iter()
                            .any(|c| values::text(&layer, "id") == Some(&c.id))
                    {
                        let from = number(
                            values::num(&layer, "from")
                                .ok_or_else(|| invalid("Missing clip start."))?,
                        )?;
                        let stop = from
                            + number(
                                values::num(&layer, "dur")
                                    .ok_or_else(|| invalid("Missing clip duration."))?,
                            )?;
                        if from < end - EPS && stop > start + EPS {
                            return Err(invalid(
                                "A surviving clip intersects the ripple-deleted interval.",
                            ));
                        }
                    }
                }
                ripple(cx, comp.as_deref(), tracks, end, start - end, &clips)?;
                delete(cx, &clips)
            }
            Self::RippleTrim {
                layer_id,
                edge,
                delta,
                tracks,
                linked,
            } => {
                let delta = number(*delta)?;
                let clips = targets(cx, std::slice::from_ref(layer_id), *linked)?;
                let comp = same_comp(&clips)?;
                require_scope(&clips, tracks)?;
                let first = &clips[0];
                if clips.iter().any(|c| {
                    (c.from - first.from).abs() > EPS || (c.end() - first.end()).abs() > EPS
                }) {
                    return Err(invalid(
                        "Ripple trim linked clips with matching timeline bounds.",
                    ));
                }
                let duration_delta = if *edge == Edge::In { -delta } else { delta };
                ripple(
                    cx,
                    comp.as_deref(),
                    tracks,
                    first.end(),
                    duration_delta,
                    &clips,
                )?;
                for mut c in clips {
                    if *edge == Edge::In {
                        c.trim_in(delta)?;
                        c.from -= delta;
                    } else {
                        c.dur += delta;
                    }
                    c.write(cx, true)?;
                }
                Ok(())
            }
            Self::Roll {
                left_id,
                right_id,
                delta,
                linked,
            } => {
                let delta = number(*delta)?;
                let left = targets(cx, std::slice::from_ref(left_id), *linked)?;
                let right = targets(cx, std::slice::from_ref(right_id), *linked)?;
                for (mut a, mut b) in adjacent_pairs(cx, &left, &right)? {
                    a.dur += delta;
                    b.trim_in(delta)?;
                    a.write(cx, true)?;
                    b.write(cx, true)?;
                }
                Ok(())
            }
            Self::Slide {
                previous_id,
                layer_id,
                next_id,
                delta,
                linked,
            } => {
                let delta = number(*delta)?;
                let left = targets(cx, std::slice::from_ref(previous_id), *linked)?;
                let middle = targets(cx, std::slice::from_ref(layer_id), *linked)?;
                let right = targets(cx, std::slice::from_ref(next_id), *linked)?;
                let lm = adjacent_pairs(cx, &left, &middle)?;
                let mr = adjacent_pairs(cx, &middle, &right)?;
                for (mut a, mut b) in lm {
                    let mut c = mr
                        .iter()
                        .find(|(m, _)| m.id == b.id)
                        .ok_or_else(|| invalid("Slide needs matching neighboring tracks."))?
                        .1
                        .clone();
                    a.dur += delta;
                    b.from += delta;
                    c.trim_in(delta)?;
                    a.write(cx, true)?;
                    b.write(cx, false)?;
                    c.write(cx, true)?;
                }
                Ok(())
            }
        }
    }
}
pub(super) fn targets(
    cx: &Cx<'_>,
    ids: &[String],
    linked: bool,
) -> Result<Vec<Clip>, CommandError> {
    let ids = pro_links::expand(cx, ids, linked)?;
    let clips: Vec<Clip> = ids
        .iter()
        .map(|id| Clip::editable(cx, id))
        .collect::<Result<_, _>>()?;
    same_comp(&clips)?;
    Ok(clips)
}
pub(super) fn in_scope(layer: &Value, tracks: &[TrackTarget]) -> bool {
    values::text(layer, "type") != Some("group") && tracks.iter().any(|t| t.matches(layer))
}
fn require_scope(clips: &[Clip], tracks: &[TrackTarget]) -> Result<(), CommandError> {
    if tracks.is_empty() || clips.iter().any(|c| !in_scope(&c.value, tracks)) {
        return Err(invalid(
            "Every edited clip must be on an explicitly targeted track.",
        ));
    }
    Ok(())
}
/// Shift a suffix, rejecting crossing intervals, non-media rows, locks and
/// linked peers outside the intended suffix. No automatic widening of scope.
pub(super) fn ripple(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    tracks: &[TrackTarget],
    pivot: f64,
    delta: f64,
    exclude: &[Clip],
) -> Result<(), CommandError> {
    let mut clips = Vec::new();
    for layer in cx.layers(comp) {
        if !in_scope(&layer, tracks)
            || exclude
                .iter()
                .any(|c| values::text(&layer, "id") == Some(&c.id))
        {
            continue;
        }
        let from = values::num(&layer, "from").ok_or_else(|| invalid("Missing layer start."))?;
        let end =
            from + values::num(&layer, "dur").ok_or_else(|| invalid("Missing layer duration."))?;
        if from < pivot - EPS && end > pivot + EPS {
            return Err(invalid("Split clips crossing the ripple boundary first."));
        }
        if from >= pivot - EPS {
            clips.push(Clip::editable(
                cx,
                values::text(&layer, "id").ok_or_else(|| invalid("Missing clip id."))?,
            )?);
        }
    }
    let ids: HashSet<&str> = clips.iter().map(|c| c.id.as_str()).collect();
    for c in &clips {
        for member in pro_links::expand(cx, std::slice::from_ref(&c.id), true)? {
            if !ids.contains(member.as_str()) {
                return Err(invalid(
                    "Target the linked peer's downstream track as well, or unlink first.",
                ));
            }
        }
    }
    for mut c in clips {
        c.from += delta;
        c.write(cx, false)?;
    }
    Ok(())
}
fn adjacent_pairs(
    cx: &Cx<'_>,
    left: &[Clip],
    right: &[Clip],
) -> Result<Vec<(Clip, Clip)>, CommandError> {
    if left.len() != right.len() {
        return Err(invalid("Linked neighbors must have matching track pairs."));
    }
    let mut pairs = Vec::new();
    let mut seen = HashSet::new();
    for a in left {
        let candidates: Vec<&Clip> = right
            .iter()
            .filter(|b| a.same_track(b) && a.id != b.id && (a.end() - b.from).abs() <= EPS)
            .collect();
        if candidates.len() != 1 {
            return Err(invalid("Choose exactly adjacent clips on the same track."));
        }
        let b = candidates[0];
        if !seen.insert(&b.id) {
            return Err(invalid("Ambiguous adjacent clips."));
        }
        for layer in cx.layers(a.comp.as_deref()) {
            let id = values::text(&layer, "id").unwrap_or_default();
            if id == a.id || id == b.id || values::text(&layer, "type") == Some("group") {
                continue;
            }
            if layer.get("group") == a.value.get("group")
                && values::num(&layer, "track").unwrap_or(0.0)
                    == values::num(&a.value, "track").unwrap_or(0.0)
            {
                let from = values::num(&layer, "from").unwrap_or(f64::NAN);
                let end = from + values::num(&layer, "dur").unwrap_or(f64::NAN);
                if from < b.end() - EPS && end > a.from + EPS {
                    return Err(invalid(
                        "The track has overlapping clips; choose an unambiguous edit.",
                    ));
                }
            }
        }
        pairs.push((a.clone(), b.clone()));
    }
    Ok(pairs)
}
pub(super) fn delete(cx: &mut Cx<'_>, clips: &[Clip]) -> Result<(), CommandError> {
    let gone: HashSet<&str> = clips.iter().map(|c| c.id.as_str()).collect();
    for c in clips {
        for peer in pro_links::expand(cx, std::slice::from_ref(&c.id), true)? {
            if !gone.contains(peer.as_str()) {
                let (loc, _) = cx.layer_mut_checked(&peer)?;
                let mut p = loc.path();
                p.push("editLink".into());
                cx.remove(p)?;
            }
        }
    }
    LayerCommand::Delete {
        layer_ids: clips.iter().map(|c| c.id.clone()).collect(),
    }
    .plan(cx)
}
fn split(
    cx: &mut Cx<'_>,
    id: &str,
    time: f64,
    tails: &[SplitTail],
    linked: bool,
    tail_link_id: Option<&str>,
) -> Result<(), CommandError> {
    let time = number(time)?;
    let clips = targets(cx, &[id.into()], linked)?;
    if !linked
        && clips[0]
            .value
            .get("editLink")
            .is_some_and(|v| v != &Value::Null)
    {
        return Err(invalid(
            "Unlink before splitting a single member of a pair.",
        ));
    }
    if tails.len() != clips.len()
        || clips
            .iter()
            .any(|c| tails.iter().filter(|t| t.layer_id == c.id).count() != 1)
    {
        return Err(invalid("Supply one tail id for every linked split target."));
    }
    let mut seen = HashSet::new();
    for tail in tails {
        if tail.tail_id.is_empty() || cx.find(&tail.tail_id).is_ok() || !seen.insert(&tail.tail_id)
        {
            return Err(invalid("Tail ids must be fresh and unique."));
        }
    }
    if clips.len() == 2 {
        let link = tail_link_id
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid("Supply a fresh link id for the tail pair."))?;
        if cx.all_layers().iter().any(|loc| {
            cx.layer_node(loc)
                .ok()
                .and_then(|n| n.get("editLink"))
                .and_then(|n| n.get("id"))
                .and_then(crate::model::str_of)
                == Some(link)
        }) {
            return Err(invalid("Tail link id is already in use."));
        }
    } else if tail_link_id.is_some() {
        return Err(invalid("Only a linked pair takes a tail link id."));
    }
    for mut head in clips {
        let offset = time - head.from;
        let tail_id = &tails
            .iter()
            .find(|t| t.layer_id == head.id)
            .expect("validated")
            .tail_id;
        let mut tail = head.clone();
        tail.value = layers::clone_layer(&head.value, tail_id, &cx.layers(head.comp.as_deref()));
        tail.id = tail_id.clone();
        tail.trim_in(offset)?;
        if let Some(link) = tail_link_id {
            let metadata = values::obj(&mut tail.value)
                .get_mut("editLink")
                .expect("linked pair");
            values::obj(metadata).insert("id", values::s(link));
        }
        head.dur = offset;
        values::obj(&mut head.value).insert("transitionOut", Value::Null);
        values::obj(&mut tail.value).insert("transitionIn", Value::Null);
        if values::text(&head.value, "type") == Some("audio") {
            set_content(&mut head.value, "fadeOut", 0.0);
            set_content(&mut tail.value, "fadeIn", 0.0);
        }
        head.validate(cx, true)?;
        tail.validate(cx, true)?;
        // Insert the whole tail then write its calculated timing by id.
        cx.insert_layer(head.comp.as_deref(), 0, tail.value.clone())?;
        head.write(cx, true)?;
        tail.write(cx, true)?;
    }
    Ok(())
}
