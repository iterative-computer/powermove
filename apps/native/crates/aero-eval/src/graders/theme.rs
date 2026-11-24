//! A restyle: the active theme comes from the run's extension and its resolved
//! tokens meet the brief (hue, lightness, contrast).
use super::Context;
use crate::image::{contrast_ratio, hue_chroma, lab, parse_css_color};
use crate::verdict::Verdict;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeSpec {
    /// The active theme must be registered by an extension the run changed.
    #[serde(default = "yes")]
    pub active_new: bool,
    #[serde(default)]
    pub tokens: Vec<TokenCheck>,
    #[serde(default)]
    pub contrast: Vec<ContrastCheck>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TokenCheck {
    pub token: String,
    /// CIELAB hue angle range in degrees (wraps when lo > hi).
    pub hue_between: Option<[f64; 2]>,
    pub chroma_ge: Option<f64>,
    pub l_le: Option<f64>,
    pub l_ge: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContrastCheck {
    pub fg: String,
    pub bg: String,
    /// WCAG contrast ratio at least this.
    pub ge: f64,
}

fn yes() -> bool {
    true
}

pub fn grade(spec: &ThemeSpec, cx: &Context) -> Verdict {
    let mut v = Verdict::new("theme");
    let Some(t) = &cx.artifacts.theme else {
        v.missing("the active theme was not collected");
        return v;
    };
    if spec.active_new {
        let ours = cx.resolve("$new:theme");
        v.check(
            ours.contains(&t.active),
            format!(
                "active theme {} (themes the run registered: {})",
                t.active,
                if ours.is_empty() {
                    "none".into()
                } else {
                    ours.join(", ")
                }
            ),
        );
    }
    let color = |name: &str| t.tokens.get(name).and_then(|s| parse_css_color(s));
    for c in &spec.tokens {
        let Some(rgb) = color(&c.token) else {
            v.check(
                false,
                format!(
                    "{} is {}, not a colour this grader reads",
                    c.token,
                    t.tokens.get(&c.token).map_or("absent", String::as_str)
                ),
            );
            continue;
        };
        let (h, ch) = hue_chroma(rgb);
        let l = lab(rgb)[0];
        let mut ok = true;
        if let Some([lo, hi]) = c.hue_between {
            ok &= if lo <= hi {
                h >= lo && h <= hi
            } else {
                h >= lo || h <= hi
            };
        }
        ok &= c.chroma_ge.is_none_or(|x| ch >= x);
        ok &= c.l_le.is_none_or(|x| l <= x);
        ok &= c.l_ge.is_none_or(|x| l >= x);
        v.check(
            ok,
            format!(
                "{} {} has hue {h:.0}°, chroma {ch:.0}, L* {l:.0}",
                c.token, t.tokens[&c.token]
            ),
        );
    }
    for c in &spec.contrast {
        match (color(&c.fg), color(&c.bg)) {
            (Some(f), Some(b)) => {
                let r = contrast_ratio(f, b);
                v.check(
                    r >= c.ge,
                    format!("{} on {} contrast {r:.1}:1 (≥ {}:1)", c.fg, c.bg, c.ge),
                );
            }
            _ => v.check(
                false,
                format!("{} or {} is not a readable colour", c.fg, c.bg),
            ),
        }
    }
    v
}
