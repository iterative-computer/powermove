//! Explicit reciprocal edge pairs. Unflagged v1 edges never enter this path.
use crate::node;
pub use crate::transition_fit::{Fitted, RETIMING_REASON, SHORTENED_REASON, fit};
use pm_model::Node;
#[derive(Clone, Copy)]
pub struct Overlap<'a> {
    pub peer: &'a Node,
    pub incoming: bool,
    pub start: f64,
    pub end: f64,
}
/// Validates the reciprocal pair and derives its current effective span.
/// Callers keep a zero-width disabled marker when this returns no span.
pub fn fitted_pair(root: &Node, layer: &Node, peer: &Node, incoming: bool) -> Option<Fitted> {
    let (field, other) = if incoming {
        ("transitionIn", "transitionOut")
    } else {
        ("transitionOut", "transitionIn")
    };
    let t = layer.get(field)?;
    let back = peer.get(other)?;
    let id = node::string(layer.get("id"))?;
    let peer_id = node::string(peer.get("id"))?;
    let duration = node::number(t.get("dur"))?;
    let offset = node::number(t.get("offset"))?;
    let cut = node::number(layer.get("from"))?
        + if incoming {
            0.
        } else {
            node::number(layer.get("dur"))?
        };
    let peer_cut = node::number(peer.get("from"))?
        + if incoming {
            node::number(peer.get("dur"))?
        } else {
            0.
        };
    if node::string(t.get("paired")) != Some(peer_id)
        || node::string(back.get("paired")) != Some(id)
        || node::string(back.get("type")) != node::string(t.get("type"))
        || node::number(back.get("dur")) != Some(duration)
        || node::number(back.get("offset")) != Some(offset)
        || node::number(peer.get("track")).unwrap_or(0.)
            != node::number(layer.get("track")).unwrap_or(0.)
        || node::string(peer.get("group")) != node::string(layer.get("group"))
        || !offset.is_finite()
        || offset > 0.
        || offset < -duration
        || !cut.is_finite()
        || !peer_cut.is_finite()
        || (cut - peer_cut).abs() > 1e-6
    {
        return None;
    }
    let (a, b) = if incoming {
        (peer, layer)
    } else {
        (layer, peer)
    };
    fit(root, a, b, duration, offset).ok()
}
/// Existing pairs stop extending source time if retiming is enabled later.
/// Unflagged v1 fades do not use this admission rule.
pub fn needs_signed_retiming(layer: &Node) -> bool {
    let Some(d) = layer.get("d") else {
        return false;
    };
    let animated = |value: &Node| {
        node::array(value.get("kf")).is_some_and(|keys| !keys.is_empty())
            || node::truthy(value.get("expr"))
    };
    let remapped = d.get("timeRemap").is_some_and(|value| {
        if matches!(value.kind(), pm_model::NodeKind::Object(_)) {
            animated(value) || node::truthy(value.get("v"))
        } else {
            node::truthy(Some(value))
        }
    });
    remapped || d.get("speed").is_some_and(animated)
}
pub fn overlap<'a>(
    root: &Node,
    layer: &'a Node,
    layers: &'a [Node],
    time: f64,
) -> Option<Overlap<'a>> {
    if !time.is_finite() || needs_signed_retiming(layer) {
        return None;
    }
    let id = node::string(layer.get("id"))?;
    for (field, other, incoming) in [
        ("transitionIn", "transitionOut", true),
        ("transitionOut", "transitionIn", false),
    ] {
        let Some(t) = layer.get(field) else { continue };
        let Some(peer_id) = node::string(t.get("paired")) else {
            continue;
        };
        let (Some(dur), Some(offset), Some(from), Some(length)) = (
            node::number(t.get("dur")),
            node::number(t.get("offset")),
            node::number(layer.get("from")),
            node::number(layer.get("dur")),
        ) else {
            continue;
        };
        let cut = if incoming { from } else { from + length };
        // Most clips are outside their overlap at any frame. Reject them
        // before scanning for a peer, keeping long timelines linear to walk.
        if !dur.is_finite()
            || dur <= 0.
            || !offset.is_finite()
            || offset > 0.
            || offset < -dur
            || !cut.is_finite()
            || time < cut - dur
            || time > cut + dur
        {
            continue;
        }
        let Some(peer) = layers
            .iter()
            .find(|p| node::string(p.get("id")) == Some(peer_id))
        else {
            continue;
        };
        if needs_signed_retiming(peer) {
            continue;
        }
        let Some(back) = peer.get(other) else {
            continue;
        };
        if node::string(back.get("paired")) != Some(id)
            || node::string(back.get("type")) != node::string(t.get("type"))
            || node::number(back.get("dur")) != Some(dur)
            || node::number(back.get("offset")) != Some(offset)
            || node::number(peer.get("track")).unwrap_or(0.)
                != node::number(layer.get("track")).unwrap_or(0.)
            || node::string(peer.get("group")) != node::string(layer.get("group"))
        {
            continue;
        }
        let peer_cut = node::number(peer.get("from"))?
            + if incoming {
                node::number(peer.get("dur"))?
            } else {
                0.
            };
        if !peer_cut.is_finite() || (cut - peer_cut).abs() > 1e-6 {
            continue;
        }
        let Some(fitted) = fitted_pair(root, layer, peer, incoming) else {
            continue;
        };
        let start = cut + fitted.offset;
        let end = start + fitted.duration;
        if fitted.duration <= 0. || time < start || time > end {
            continue;
        }
        return Some(Overlap {
            peer,
            incoming,
            start,
            end,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layers(flagged: bool) -> Vec<Node> {
        let pair_a = if flagged {
            r#", "paired":"b", "offset":-0.25"#
        } else {
            ""
        };
        let pair_b = if flagged {
            r#", "paired":"a", "offset":-0.25"#
        } else {
            ""
        };
        [format!(r#"{{"id":"a","from":0,"dur":2,"transitionOut":{{"type":"crossfade","dur":0.5{pair_a}}}}}"#),
         format!(r#"{{"id":"b","from":2,"dur":2,"transitionIn":{{"type":"crossfade","dur":0.5{pair_b}}}}}"#)]
            .iter().map(|json| Node::from_value(pm_model::json::parse(json).unwrap(), 0)).collect()
    }
    #[test]
    fn reciprocal_pair_extends_both_sides_without_changing_the_cut() {
        let clips = layers(true);
        for time in [1.75, 2., 2.25] {
            let a = overlap(
                &Node::from_value(pm_model::Value::Null, 0),
                &clips[0],
                &clips,
                time,
            )
            .unwrap();
            let b = overlap(
                &Node::from_value(pm_model::Value::Null, 0),
                &clips[1],
                &clips,
                time,
            )
            .unwrap();
            assert!(!a.incoming);
            assert!(b.incoming);
            assert_eq!((a.start, a.end), (1.75, 2.25));
            assert_eq!((b.start, b.end), (1.75, 2.25));
        }
        assert!(
            overlap(
                &Node::from_value(pm_model::Value::Null, 0),
                &clips[0],
                &clips,
                2.3
            )
            .is_none()
        );
        assert!(
            overlap(
                &Node::from_value(pm_model::Value::Null, 0),
                &clips[1],
                &clips,
                1.7
            )
            .is_none()
        );
    }
    #[test]
    fn v1_edges_never_become_cut_overlaps() {
        let clips = layers(false);
        assert!(
            overlap(
                &Node::from_value(pm_model::Value::Null, 0),
                &clips[0],
                &clips,
                2.
            )
            .is_none()
        );
        assert!(
            overlap(
                &Node::from_value(pm_model::Value::Null, 0),
                &clips[1],
                &clips,
                2.
            )
            .is_none()
        );
    }
    #[test]
    fn orphaned_pairs_never_extend_a_clip() {
        let clips = layers(true);
        assert!(
            overlap(
                &Node::from_value(pm_model::Value::Null, 0),
                &clips[0],
                &clips[..1],
                2.1
            )
            .is_none()
        );
    }
    #[test]
    fn later_retiming_cannot_make_an_existing_pair_hold_handle_frames() {
        for data in [
            r#"{"timeRemap":true}"#,
            r#"{"speed":{"v":1,"kf":[{"t":0,"v":1,"i":"speed-key"}]}}"#,
        ] {
            let mut clips = layers(true);
            let mut a = clips[0].to_value();
            a.as_object_mut()
                .unwrap()
                .insert("d", pm_model::json::parse(data).unwrap());
            clips[0] = Node::from_value(a, 1);
            assert!(
                overlap(
                    &Node::from_value(pm_model::Value::Null, 0),
                    &clips[0],
                    &clips,
                    2.1
                )
                .is_none()
            );
            assert!(
                overlap(
                    &Node::from_value(pm_model::Value::Null, 0),
                    &clips[1],
                    &clips,
                    1.9
                )
                .is_none()
            );
        }
    }
}
