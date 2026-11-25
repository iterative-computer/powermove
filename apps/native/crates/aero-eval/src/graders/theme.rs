//! A restyle: the active theme comes from the run's extension and its resolved
//! tokens meet the brief (hue, lightness, contrast). Alpha counts: a
//! foreground is composited over the background it is drawn on before its
//! contrast is measured, and a translucent token whose surface is unknown (a
//! background, or a token checked on its own) is refused, never read as
//! opaque.
use super::Context;
use crate::image::{contrast_ratio, hue_chroma, lab, over, parse_css_color, parse_css_rgba};
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
    let raw = |name: &str| t.tokens.get(name).map_or("absent", String::as_str);
    let color = |name: &str| t.tokens.get(name).and_then(|s| parse_css_color(s));
    for c in &spec.tokens {
        let Some(rgb) = color(&c.token) else {
            v.check(
                false,
                format!(
                    "{} is {}, not an opaque colour this grader reads",
                    c.token,
                    raw(&c.token)
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
                c.token,
                raw(&c.token)
            ),
        );
    }
    for c in &spec.contrast {
        let fg = t.tokens.get(&c.fg).and_then(|s| parse_css_rgba(s));
        match (fg, color(&c.bg)) {
            (Some((f, alpha)), Some(b)) => {
                let shown = over(f, alpha, b);
                let r = contrast_ratio(shown, b);
                v.check(
                    r >= c.ge,
                    format!(
                        "{} ({}) on {} ({}) contrast {r:.1}:1 (≥ {}:1){}",
                        c.fg,
                        raw(&c.fg),
                        c.bg,
                        raw(&c.bg),
                        c.ge,
                        if alpha < 1.0 {
                            format!(", composited at alpha {alpha:.2}")
                        } else {
                            String::new()
                        }
                    ),
                );
            }
            _ => v.check(
                false,
                format!(
                    "{} ({}) on {} ({}): the foreground is not a colour or the background is not opaque",
                    c.fg,
                    raw(&c.fg),
                    c.bg,
                    raw(&c.bg)
                ),
            ),
        }
    }
    v
}
