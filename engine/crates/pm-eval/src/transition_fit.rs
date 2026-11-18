//! Read-only fitting of stored cut-transition intent against current media.
use crate::{cut_transition::needs_signed_retiming, node};
use pm_model::Node;

pub const RETIMING_REASON: &str =
    "Cross dissolve needs clips without time remapping. Try a fade instead.";
pub const SHORTENED_REASON: &str = "Shortened to fit media";
const SOURCE_TIMING_REASON: &str =
    "Source timing is unavailable for a cut transition. Try a fade instead.";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fitted {
    pub duration: f64,
    pub offset: f64,
}
impl Fitted {
    pub fn shortened(self, intent: f64) -> bool {
        self.duration < intent - 1e-6
    }
}

/// `offset` is relative to the unchanged cut. A centred intent stays centred
/// while both sides have handles; a zero handle shifts it wholly to one side.
/// Offset alignment is preserved while shortening. Nothing writes
/// the derived result back to the document, so undo restores it automatically.
pub fn fit(
    root: &Node,
    a: &Node,
    b: &Node,
    duration: f64,
    offset: f64,
) -> Result<Fitted, &'static str> {
    if needs_signed_retiming(a) || needs_signed_retiming(b) {
        return Err(RETIMING_REASON);
    }
    if !duration.is_finite() || !(0.02..=600.).contains(&duration) {
        return Err("Transition duration must be between 0.02 and 600 seconds");
    }
    if !offset.is_finite() {
        return Err("Transition offset must be finite");
    }
    let length = |clip: &Node| node::number(clip.get("dur")).unwrap_or(0.);
    if length(a) <= 0. || length(b) <= 0. {
        return Err("Transition clips must have a positive duration");
    }
    let (_, tail) = handles(root, a)?;
    let (head, _) = handles(root, b)?;
    let pre = head.min(length(a));
    let post = tail.min(length(b));
    let fraction = (-offset / duration).clamp(0., 1.);
    let (duration, offset) = if pre == 0. {
        (duration.min(post), 0.)
    } else if post == 0. {
        let d = duration.min(pre);
        (d, -d)
    } else {
        // Preserve the intended alignment while shortening. This is also
        // continuous through the centre, so offset drags never jump in size.
        let before = if fraction > 0. {
            pre / fraction
        } else {
            f64::INFINITY
        };
        let after = if fraction < 1. {
            post / (1. - fraction)
        } else {
            f64::INFINITY
        };
        let d = duration.min(before).min(after);
        (d, -fraction * d)
    };
    Ok(Fitted { duration, offset })
}

/// Generated and still content is unbounded. Unknown moving media admits no
/// extension; known media with an already-invalid clip range is refused.
fn handles(root: &Node, clip: &Node) -> Result<(f64, f64), &'static str> {
    let precomp = node::is_str(clip.get("type"), "precomp");
    let moving = precomp || node::is_str(clip.get("type"), "video");
    if !moving {
        return Ok((f64::INFINITY, f64::INFINITY));
    }
    let d = clip.get("d");
    let scalar = |field: &str, default: f64| {
        let Some(value) = d.and_then(|d| d.get(field)) else {
            return default;
        };
        if matches!(value.kind(), pm_model::NodeKind::Null) {
            return default;
        }
        if matches!(value.kind(), pm_model::NodeKind::Object(_)) {
            if node::array(value.get("kf")).is_some_and(|keys| !keys.is_empty())
                || node::truthy(value.get("expr"))
            {
                return f64::NAN;
            }
            node::number(value.get("v")).unwrap_or(f64::NAN)
        } else {
            node::number(Some(value)).unwrap_or(f64::NAN)
        }
    };
    let speed = scalar("speed", 1.);
    let trim = scalar("trim", 0.);
    if !speed.is_finite() || speed <= 0. || !trim.is_finite() || trim < 0. {
        return Err(SOURCE_TIMING_REASON);
    }
    let id = node::string(d.and_then(|d| d.get(if precomp { "comp" } else { "asset" })));
    let source = id.and_then(|id| {
        root.get(if precomp { "comps" } else { "assets" })?
            .get(&id.to_string())
    });
    // `dur` describes the imported timeline. A conformed frame rate uses
    // the decoder's actual frame count, which this read-only snapshot does
    // not contain for every source (notably VFR). Do not guess an extent
    // from average fps and risk extending into the decoder's held frame.
    if !precomp
        && source
            .and_then(|s| s.get("interpret"))
            .and_then(|interpret| interpret.get("frameRate"))
            .is_some_and(|rate| !matches!(rate.kind(), pm_model::NodeKind::Null))
    {
        return Err(SOURCE_TIMING_REASON);
    }
    let Some(source_duration) = node::number(source.and_then(|s| s.get("dur"))) else {
        return Ok((0., 0.));
    };
    let length = node::number(clip.get("dur")).unwrap_or(0.);
    if !source_duration.is_finite() || trim + length * speed > source_duration + 1e-6 {
        return Err("Clip exceeds its source media. Trim it before adding a transition.");
    }
    Ok((
        trim / speed,
        ((source_duration - trim) / speed - length).max(0.),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(json: &str) -> Node {
        Node::from_value(pm_model::json::parse(json).unwrap(), 0)
    }
    #[test]
    fn cut_transition_fitting_preserves_intent_through_handle_loss_and_undo() {
        let root = node(r#"{"assets":{"a":{"dur":2.3},"b":{"dur":3}}}"#);
        let a = node(r#"{"type":"video","dur":2,"d":{"asset":"a","speed":1,"trim":0}}"#);
        let b = node(r#"{"type":"video","dur":1,"d":{"asset":"b","speed":1,"trim":0.3}}"#);
        assert_eq!(
            fit(&root, &a, &b, 0.5, -0.25).unwrap(),
            Fitted {
                duration: 0.5,
                offset: -0.25
            }
        );
        let rolled_a = node(r#"{"type":"video","dur":2.2,"d":{"asset":"a","speed":1,"trim":0}}"#);
        let rolled_b = node(r#"{"type":"video","dur":0.8,"d":{"asset":"b","speed":1,"trim":0.5}}"#);
        let fitted = fit(&root, &rolled_a, &rolled_b, 0.5, -0.25).unwrap();
        assert!((fitted.duration - 0.2).abs() < 1e-6);
        assert!(fitted.shortened(0.5));
        let shifted = fit(&root, &rolled_a, &rolled_b, 0.5, -0.25001).unwrap();
        assert!(
            (shifted.duration - fitted.duration).abs() < 0.001,
            "offsetting through the centre must not jump back to the full stored duration"
        );
        assert_eq!(fit(&root, &a, &b, 0.5, -0.25).unwrap().duration, 0.5);
        let exhausted = node(r#"{"assets":{"a":{"dur":2},"b":{"dur":1}}}"#);
        let zero_b = node(r#"{"type":"video","dur":1,"d":{"asset":"b","trim":0}}"#);
        assert_eq!(
            fit(&exhausted, &a, &zero_b, 0.5, -0.25).unwrap().duration,
            0.
        );
    }

    #[test]
    fn cut_transition_refuses_rate_conforming_without_a_measured_source_extent() {
        // Four seconds imported at 30 fps become two seconds at 60 fps.
        // The asset's stored dur stays 4; using it as a handle bound would
        // extend A past the conformed end and make the decoder hold a frame.
        let root = node(
            r#"{"assets":{"a":{"dur":4,"fps":30,"interpret":{"frameRate":60}},"b":{"dur":3}}}"#,
        );
        let a = node(r#"{"type":"video","dur":1.8,"d":{"asset":"a","trim":0,"speed":1}}"#);
        let b = node(r#"{"type":"video","dur":1,"d":{"asset":"b","trim":0.3,"speed":1}}"#);
        assert!(fit(&root, &a, &b, 0.5, -0.25).is_err());
        let restored = node(
            r#"{"assets":{"a":{"dur":4,"fps":30,"interpret":{"frameRate":null}},"b":{"dur":3}}}"#,
        );
        assert_eq!(fit(&restored, &a, &b, 0.5, -0.25).unwrap().duration, 0.5);
    }
}
