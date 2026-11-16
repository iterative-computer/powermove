//! How an effect's parameters are laid out, as v1's `parameterLayout`: start
//! and end colours as a gradient bar, `…X`/`…Y` pairs as one point control,
//! rarely used parameters under More, Amount and Mix last. From the hints a
//! definition carries (`ui`, a parameter's `advanced`); old definitions get
//! useful layouts too, and an invalid hint never swallows a parameter.
use serde_json::Value;

/// What the layout reads of one parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub key: String,
    pub label: String,
    /// `number`, `color`, `toggle`, …
    pub kind: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// The definition's `advanced`, when it says.
    pub advanced: Option<bool>,
}
impl Param {
    /// A bounded number (v1's `numeric`).
    fn numeric(&self) -> bool {
        self.kind != "color"
            && self.kind != "toggle"
            && matches!((self.min, self.max), (Some(a), Some(b)) if a.is_finite() && b.is_finite() && a < b)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Widget {
    /// A gradient bar over its two colours (which stay rows of their own
    /// below it); its midpoint goes under More, its Linear/Radial toggle
    /// right after the bar.
    Ramp {
        start: String,
        end: String,
        midpoint: Option<String>,
        mode: Option<String>,
    },
    /// Two numbers as one control; an advanced one goes under More.
    Point {
        x: String,
        y: String,
        label: String,
        advanced: bool,
    },
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub widgets: Vec<Widget>,
    /// Rows in order after the widgets.
    pub primary: Vec<String>,
    /// Rows under More, after its advanced points.
    pub more: Vec<String>,
}

const SECONDARY: [&str; 5] = ["dither", "seed", "quality", "shade", "keepluminance"];
fn amount_or_mix(key: &str) -> bool {
    key.eq_ignore_ascii_case("amount") || key.eq_ignore_ascii_case("mix")
}

pub fn layout(params: &[Param], authored: Option<&Value>) -> Layout {
    let by = |k: &str| params.iter().find(|p| p.key == k);
    let text = |w: &Value, k: &str| w.get(k).and_then(Value::as_str).map(str::to_owned);
    let mut hints: Vec<Widget> = vec![];
    for w in authored.and_then(Value::as_array).into_iter().flatten() {
        match w.get("kind").and_then(Value::as_str) {
            Some("ramp") => {
                if let (Some(start), Some(end)) = (text(w, "start"), text(w, "end")) {
                    hints.push(Widget::Ramp {
                        start,
                        end,
                        midpoint: text(w, "midpoint"),
                        mode: text(w, "mode"),
                    });
                }
            }
            Some("point") => {
                if let (Some(x), Some(y)) = (text(w, "x"), text(w, "y")) {
                    hints.push(Widget::Point {
                        label: text(w, "label").unwrap_or_else(|| "Position".into()),
                        x,
                        y,
                        advanced: w.get("advanced").and_then(Value::as_bool).unwrap_or(false),
                    });
                }
            }
            _ => {}
        }
    }
    if !hints.iter().any(|w| matches!(w, Widget::Ramp { .. })) {
        for (start, end) in [("startColor", "endColor"), ("shadow", "highlight")] {
            let colour = |k| by(k).is_some_and(|p| p.kind == "color");
            if colour(start) && colour(end) {
                hints.push(Widget::Ramp {
                    start: start.into(),
                    end: end.into(),
                    midpoint: by("midpoint")
                        .filter(|p| p.numeric())
                        .map(|_| "midpoint".into()),
                    mode: by("radial")
                        .filter(|p| p.kind == "toggle")
                        .map(|_| "radial".into()),
                });
                break;
            }
        }
    }
    for p in params {
        let Some(stem) = p.key.strip_suffix('X') else {
            continue;
        };
        let y = format!("{stem}Y");
        let claimed = hints
            .iter()
            .any(|w| matches!(w, Widget::Point { x, y: wy, .. } if *x == p.key || *wy == y));
        if p.numeric() && by(&y).is_some_and(Param::numeric) && !claimed {
            let label = p.label.trim_end();
            let label = label
                .strip_suffix(" X")
                .or_else(|| label.strip_suffix(" x"))
                .unwrap_or(label);
            hints.push(Widget::Point {
                label: if label.is_empty() {
                    p.key.clone()
                } else {
                    label.to_owned()
                },
                advanced: p
                    .advanced
                    .or(by(&y).and_then(|y| y.advanced))
                    .unwrap_or(false),
                x: p.key.clone(),
                y,
            });
        }
    }
    // Valid hints only, each parameter in at most one.
    let mut used: Vec<String> = vec![];
    let mut widgets = vec![];
    for hint in hints {
        let keys: Vec<String> = match &hint {
            Widget::Ramp {
                start,
                end,
                midpoint,
                mode,
            } => [Some(start), Some(end), midpoint.as_ref(), mode.as_ref()]
                .into_iter()
                .flatten()
                .cloned()
                .collect(),
            Widget::Point { x, y, .. } => vec![x.clone(), y.clone()],
        };
        let distinct = keys.iter().enumerate().all(|(i, k)| !keys[..i].contains(k));
        if !distinct || keys.iter().any(|k| used.contains(k) || by(k).is_none()) {
            continue;
        }
        let valid = match &hint {
            Widget::Ramp {
                start,
                end,
                midpoint,
                mode,
            } => {
                by(start).is_some_and(|p| p.kind == "color")
                    && by(end).is_some_and(|p| p.kind == "color")
                    && midpoint
                        .as_deref()
                        .is_none_or(|m| by(m).is_some_and(Param::numeric))
                    && mode
                        .as_deref()
                        .is_none_or(|m| by(m).is_some_and(|p| p.kind == "toggle"))
            }
            Widget::Point { x, y, .. } => {
                by(x).is_some_and(Param::numeric) && by(y).is_some_and(Param::numeric)
            }
        };
        if valid {
            used.extend(keys);
            widgets.push(hint);
        }
    }
    // A ramp's colours stay rows (editable, keyframeable) below the bar; its
    // midpoint goes under More, its mode follows the bar; a point's numbers
    // are the point.
    let mut hidden: Vec<&str> = vec![];
    let mut midpoints: Vec<&str> = vec![];
    for w in &widgets {
        match w {
            Widget::Ramp { midpoint, mode, .. } => {
                hidden.extend(midpoint.as_deref());
                hidden.extend(mode.as_deref());
                midpoints.extend(midpoint.as_deref());
            }
            Widget::Point { x, y, .. } => hidden.extend([x.as_str(), y.as_str()]),
        }
    }
    let ramp_colour = |k: &str| {
        widgets
            .iter()
            .any(|w| matches!(w, Widget::Ramp { start, end, .. } if start == k || end == k))
    };
    let (mut primary, mut more) = (vec![], vec![]);
    for p in params {
        if hidden.contains(&p.key.as_str()) {
            if midpoints.contains(&p.key.as_str()) {
                more.push(p.key.clone());
            }
            continue;
        }
        let advanced = !ramp_colour(&p.key)
            && p.advanced.unwrap_or_else(|| {
                SECONDARY.contains(&p.key.to_ascii_lowercase().as_str())
                    || (primary.len() >= 5 && !amount_or_mix(&p.key))
            });
        if advanced {
            more.push(p.key.clone());
        } else {
            primary.push(p.key.clone());
        }
    }
    // Amount and Mix close the primary rows (a stable sort keeps the rest).
    primary.sort_by_key(|k| amount_or_mix(k));
    Layout {
        widgets,
        primary,
        more,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn p(key: &str, kind: &str, range: Option<(f64, f64)>) -> Param {
        Param {
            key: key.into(),
            label: pm_label(key),
            kind: kind.into(),
            min: range.map(|r| r.0),
            max: range.map(|r| r.1),
            advanced: None,
        }
    }
    fn pm_label(key: &str) -> String {
        match key {
            "centerX" => "Center X".into(),
            "centerY" => "Center Y".into(),
            k => k.into(),
        }
    }
    const N: Option<(f64, f64)> = Some((0., 100.));
    #[test]
    fn colours_make_a_bar_and_xy_pairs_one_point() {
        let params = [
            p("amount", "number", N),
            p("startColor", "color", None),
            p("endColor", "color", None),
            p("midpoint", "number", N),
            p("radial", "toggle", None),
            p("centerX", "number", N),
            p("centerY", "number", N),
        ];
        let l = layout(&params, None);
        assert_eq!(
            l.widgets,
            vec![
                Widget::Ramp {
                    start: "startColor".into(),
                    end: "endColor".into(),
                    midpoint: Some("midpoint".into()),
                    mode: Some("radial".into()),
                },
                Widget::Point {
                    x: "centerX".into(),
                    y: "centerY".into(),
                    label: "Center".into(),
                    advanced: false,
                },
            ]
        );
        // The colours stay rows; Amount comes last; the midpoint is under More.
        assert_eq!(l.primary, ["startColor", "endColor", "amount"]);
        assert_eq!(l.more, ["midpoint"]);
    }
    #[test]
    fn rarely_used_and_past_the_fifth_go_under_more_unless_amount_or_mix() {
        let mut params: Vec<Param> = ["a", "b", "seed", "c", "d", "e", "f", "mix"]
            .iter()
            .map(|k| p(k, "number", N))
            .collect();
        params[1].advanced = Some(true);
        let l = layout(&params, None);
        assert_eq!(l.primary, ["a", "c", "d", "e", "f", "mix"]);
        assert_eq!(l.more, ["b", "seed"]);
        // Past five primaries, the rest go under More; Mix still stays.
        params.insert(6, p("g", "number", N));
        let l = layout(&params, None);
        assert_eq!(l.primary, ["a", "c", "d", "e", "g", "mix"]);
        assert_eq!(l.more, ["b", "seed", "f"]);
    }
    #[test]
    fn an_authored_hint_wins_and_an_invalid_one_swallows_nothing() {
        let params = [
            p("posX", "number", N),
            p("posY", "number", N),
            p("tint", "color", None),
        ];
        let ui = json!([
            {"kind": "point", "x": "posX", "y": "posY", "label": "Origin", "advanced": true},
            {"kind": "ramp", "start": "tint", "end": "missing"},
        ]);
        let l = layout(&params, Some(&ui));
        assert_eq!(
            l.widgets,
            vec![Widget::Point {
                x: "posX".into(),
                y: "posY".into(),
                label: "Origin".into(),
                advanced: true,
            }]
        );
        assert_eq!(l.primary, ["tint"]);
        assert!(l.more.is_empty());
        // An unbounded pair is no point: both stay rows.
        let loose = [p("offsetX", "number", None), p("offsetY", "number", None)];
        assert_eq!(layout(&loose, None).primary, ["offsetX", "offsetY"]);
    }
}
