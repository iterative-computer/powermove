//! U02 backing, pinned main c7f46a056: PointField and GeneratedPanel. Numerical
//! authored increments/group membership only; no widget layout or visual choices.
use super::{
    generated::{Binding, binding},
    invalid,
};
use crate::Result;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisDomain {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}
impl AxisDomain {
    pub fn new(min: f64, max: f64, authored_step: Option<f64>) -> Result<Self> {
        if !min.is_finite() || !max.is_finite() || min >= max || !(max - min).is_finite() {
            return Err(invalid("invalid point range"));
        }
        let step = authored_step
            .filter(|n| n.is_finite() && *n > 0.)
            .unwrap_or((max - min) / 100.);
        if step <= 0. {
            return Err(invalid("point increment underflow"));
        }
        Ok(Self { min, max, step })
    }
    /// Quantization is anchored at the authored minimum. JS Math.round ties go
    /// toward positive infinity; f64::round alone differs on negative half steps.
    pub fn quantize(self, value: f64) -> Result<f64> {
        if !value.is_finite() {
            return Err(invalid("invalid point value"));
        }
        let steps = ((value - self.min) / self.step + 0.5).floor();
        let rounded = self.min + steps * self.step;
        if !rounded.is_finite() {
            return Err(invalid("point quantization overflow"));
        }
        let rounded = fixed_six(rounded)?;
        Ok(rounded.clamp(self.min, self.max))
    }
}
/// ECMAScript Number::toFixed(6) rounds an exact binary halfway value away from
/// zero. Rust formatting uses ties-to-even, which would alter authored steps
/// such as 1/128. Work in integer significands, then parse once to avoid double
/// rounding through a scaled f64. Large values follow JS's >=1e21 pass-through.
fn fixed_six(value: f64) -> Result<f64> {
    if value.abs() >= 1e21 {
        return Ok(value);
    }
    let bits = value.abs().to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let significand = (bits & ((1u64 << 52) - 1)) | if exponent == 0 { 0 } else { 1u64 << 52 };
    let power = if exponent == 0 {
        -1074
    } else {
        exponent - 1023 - 52
    };
    let scaled = u128::from(significand) * 1_000_000;
    let integer = if power >= 0 {
        scaled
            .checked_shl(power as u32)
            .ok_or_else(|| invalid("point decimal overflow"))?
    } else {
        let shift = (-power) as u32;
        if shift >= 128 {
            0
        } else {
            (scaled >> shift)
                + u128::from((scaled & ((1u128 << shift) - 1)) >= (1u128 << (shift - 1)))
        }
    };
    format!(
        "{}{}.{:06}",
        if value.is_sign_negative() { "-" } else { "" },
        integer / 1_000_000,
        integer % 1_000_000
    )
    .parse()
    .map_err(|_| invalid("point decimal conversion failed"))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlGroups {
    pub secondary: Vec<usize>,
    pub point_pairs: Vec<(usize, usize)>,
    pub gradient_pair: Option<(usize, usize)>,
}
fn name(c: &Value) -> String {
    c["stateKey"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| c["param"].as_str().filter(|s| !s.is_empty()))
        .or_else(|| c["label"].as_str())
        .unwrap_or("")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}
fn local(binding: &Binding) -> bool {
    matches!(binding, Binding::Local(_))
}
pub fn groups(controls: &[Value]) -> Result<ControlGroups> {
    let mut primary = 0;
    let mut secondary = Vec::new();
    for (i, c) in controls.iter().enumerate() {
        let advanced = if matches!(c["type"].as_str(), Some("button" | "readout")) {
            c["advanced"] == true
        } else {
            c["advanced"].as_bool().unwrap_or(primary >= 5)
        };
        if advanced {
            secondary.push(i);
        } else if !matches!(c["type"].as_str(), Some("button" | "readout")) {
            primary += 1;
        }
    }
    let mut pairs = Vec::new();
    for (x, cx) in controls
        .iter()
        .enumerate()
        .filter(|(_, c)| c["type"] == "slider" && name(c).ends_with('x'))
    {
        let xn = name(cx);
        let yn = format!("{}y", &xn[..xn.len() - 1]);
        if let Some((y, cy)) = controls
            .iter()
            .enumerate()
            .find(|(_, c)| c["type"] == "slider" && name(c) == yn)
        {
            let xb = binding(cx)?;
            let yb = binding(cy)?;
            let bounded = |c: &Value| {
                c["min"]
                    .as_f64()
                    .zip(c["max"].as_f64())
                    .is_some_and(|(min, max)| max > min)
            };
            if local(&xb) == local(&yb) && bounded(cx) && bounded(cy) {
                pairs.push((x, y));
            }
        }
    }
    let start = controls
        .iter()
        .position(|c| c["type"] == "color" && name(c) == "startcolor");
    let end = controls
        .iter()
        .position(|c| c["type"] == "color" && name(c) == "endcolor");
    let gradient_pair = start
        .zip(end)
        .filter(|(a, b)| !secondary.contains(a) && !secondary.contains(b));
    Ok(ControlGroups {
        secondary,
        point_pairs: pairs,
        gradient_pair,
    })
}
