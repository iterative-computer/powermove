//! Persistent edit links, distinct from transform parents. Unknown layer fields
//! stay intact; link metadata travels through the ordinary project/history IO.
use super::pro_support::{Clip, invalid, same_comp};
use super::{Cx, Origin, values};
use crate::CommandError;
use pm_model::{Map, Node, Value};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq)]
pub enum ProLinkCommand {
    /// Pair exactly one video and one audio clip at their current sync point.
    Link {
        video_id: String,
        audio_id: String,
        link_id: String,
    },
    /// Remove the entire pair, including when only one member is supplied.
    Unlink { layer_ids: Vec<String> },
}
impl ProLinkCommand {
    pub fn label(&self) -> String {
        match self {
            Self::Link { .. } => "Link Audio and Video",
            Self::Unlink { .. } => "Unlink Audio and Video",
        }
        .into()
    }
    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        match self {
            Self::Link {
                video_id,
                audio_id,
                link_id,
            } => {
                let clips = [Clip::editable(cx, video_id)?, Clip::editable(cx, audio_id)?];
                same_comp(&clips)?;
                if link_id.is_empty()
                    || video_id == audio_id
                    || values::text(&clips[0].value, "type") != Some("video")
                    || values::text(&clips[1].value, "type") != Some("audio")
                {
                    return Err(invalid(
                        "Supply a video, an audio clip and a nonempty link id.",
                    ));
                }
                if clips
                    .iter()
                    .any(|c| c.value.get("editLink").is_some_and(|v| v != &Value::Null))
                    || cx.all_layers().iter().any(|loc| {
                        cx.layer_node(loc)
                            .ok()
                            .and_then(|v| v.get("editLink"))
                            .and_then(|v| v.get("id"))
                            .and_then(crate::model::str_of)
                            == Some(link_id)
                    })
                {
                    return Err(invalid(
                        "Unlink existing pairs first; link ids must be unique.",
                    ));
                }
                for c in clips {
                    let mut link = Map::new();
                    link.insert("id", values::s(link_id));
                    link.insert(
                        "phase",
                        Value::Number(super::pro_support::number(c.from - c.trim / c.rate)?),
                    );
                    let mut path = cx.find(&c.id)?.path();
                    path.push("editLink".into());
                    cx.set(path, Value::Object(link))?;
                }
                Ok(())
            }
            Self::Unlink { layer_ids } => {
                for id in expand(cx, layer_ids, true)? {
                    let (loc, _) = cx.layer_mut_checked(&id)?;
                    let mut path = loc.path();
                    path.push("editLink".into());
                    cx.remove(path)?;
                }
                Ok(())
            }
        }
    }
}
fn link_id(v: &Value) -> Result<Option<&str>, CommandError> {
    match v.get("editLink") {
        None | Some(Value::Null) => Ok(None),
        Some(link) => {
            let id = values::text(link, "id")
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("Malformed edit link; repair its metadata explicitly."))?;
            if !values::num(link, "phase").is_some_and(f64::is_finite) {
                return Err(invalid("Malformed edit-link sync phase."));
            }
            Ok(Some(id))
        }
    }
}
pub(super) fn expand(
    cx: &Cx<'_>,
    ids: &[String],
    linked: bool,
) -> Result<Vec<String>, CommandError> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        let (loc, layer) = cx.layer(id)?;
        if seen.insert(id.clone()) {
            out.push(id.clone());
        }
        if !linked {
            continue;
        }
        let Some(link) = link_id(&layer)? else {
            continue;
        };
        let mut pair = Vec::new();
        for other in cx.layers(loc.comp_id.as_deref()) {
            if link_id(&other)? == Some(link) {
                pair.push(other);
            }
        }
        if pair.len() != 2
            || !pair
                .iter()
                .any(|p| values::text(p, "type") == Some("video"))
            || !pair
                .iter()
                .any(|p| values::text(p, "type") == Some("audio"))
        {
            return Err(invalid(
                "The edit link must contain one video and one audio clip.",
            ));
        }
        for member in pair {
            let id = values::text(&member, "id")
                .ok_or_else(|| invalid("Linked clip has no id."))?
                .to_owned();
            if seen.insert(id.clone()) {
                out.push(id);
            }
        }
    }
    Ok(out)
}
/// Headless selection policy. Caller stores selection through the existing
/// kernel selection transaction, outside document undo. `bypass` is explicit.
pub fn linked_selection(
    root: &Node,
    ids: &[String],
    bypass: bool,
) -> Result<Vec<String>, CommandError> {
    expand(&Cx::new(root, Origin::Interface), ids, !bypass)
}
#[derive(Clone, Debug, PartialEq)]
pub struct SyncBadge {
    pub layer_id: String,
    pub peer_id: String,
    pub offset_seconds: f64,
}
/// Positive offset means this member is later than its recorded sync relation.
/// Return signed seconds; the UI may format at the composition frame rate.
pub fn sync_badges(root: &Node, layer_id: &str) -> Result<Vec<SyncBadge>, CommandError> {
    let cx = Cx::new(root, Origin::Interface);
    let ids = expand(&cx, &[layer_id.into()], true)?;
    if ids.len() < 2 {
        return Ok(Vec::new());
    }
    let clips: Vec<Clip> = ids
        .iter()
        .map(|id| Clip::read(&cx, id))
        .collect::<Result<_, _>>()?;
    let phase = |c: &Clip| {
        c.from
            - c.trim / c.rate
            - values::num(c.value.get("editLink").expect("expanded pair"), "phase")
                .expect("validated phase")
    };
    let offset = phase(&clips[0]) - phase(&clips[1]);
    Ok(vec![
        SyncBadge {
            layer_id: ids[0].clone(),
            peer_id: ids[1].clone(),
            offset_seconds: offset,
        },
        SyncBadge {
            layer_id: ids[1].clone(),
            peer_id: ids[0].clone(),
            offset_seconds: -offset,
        },
    ])
}
