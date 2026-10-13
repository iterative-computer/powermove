//! Interlaced sources (section 13 "Variable frame rate and interlacing"):
//! the field order in effect (probe, or the Interpret Footage override,
//! including a progressive/PsF override), whether to deinterlace to field
//! rate or frame rate for a composition, and which source frame and field a
//! composition time shows (section 5 rule 7 at the field rate).
//!
//! Deinterlacing itself (bwdif, in Y′CbCr before chroma upsampling) runs on
//! the GPU in `pm-gpu`; it reads the previous, current, and next frames,
//! which [`crate::decode::service::SessionGuard::fields`] decodes ("random
//! access includes neighboring fields").

use crate::probe::{FieldInfo, FieldOrder};
use crate::time::cmp_frac;
use pm_time::{Rate, Rational, TICKS_PER_SECOND, Tick};
use serde::Serialize;

/// Interpret Footage's field order setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldOverride {
    /// Use what the probe found.
    #[default]
    Auto,
    TopFirst,
    BottomFirst,
    /// Treat as progressive (PsF: progressive segmented frames carried as fields).
    Progressive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeinterlaceOutput {
    /// Progressive source (or the PsF override): frames are used as they are.
    Off,
    /// One output picture per field (the composition rate is at least the
    /// field rate).
    FieldRate,
    /// One output picture per frame, from its first field.
    FrameRate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldPlan {
    pub order: FieldOrder,
    pub output: DeinterlaceOutput,
}

impl FieldPlan {
    pub fn top_field_first(&self) -> bool {
        self.order != FieldOrder::BottomFirst
    }
}

/// Section 13: deinterlace to field rate when the composition rate is at
/// least the field rate, otherwise to frame rate. Unknown field order on an
/// interlaced override defaults to top field first.
pub fn plan(fields: &FieldInfo, field_override: FieldOverride, composition: Rate, source: Rate) -> FieldPlan {
    let order = match field_override {
        FieldOverride::Auto => fields.order,
        FieldOverride::TopFirst => FieldOrder::TopFirst,
        FieldOverride::BottomFirst => FieldOrder::BottomFirst,
        FieldOverride::Progressive => FieldOrder::Progressive,
    };
    let output = match order {
        FieldOrder::Progressive | FieldOrder::Unknown => DeinterlaceOutput::Off,
        _ => {
            // composition ≥ 2 × source  ⇔  c.num · s.den ≥ 2 · s.num · c.den
            let c = composition.num() as i128 * source.den() as i128;
            let f = 2 * source.num() as i128 * composition.den() as i128;
            if c >= f { DeinterlaceOutput::FieldRate } else { DeinterlaceOutput::FrameRate }
        }
    };
    FieldPlan { order, output }
}

/// Which field of which presented frame shows at source time `t` (ticks,
/// presentation timeline) at field rate: rule 7 over field times
/// `pts_k + j · d/2` (j = 0 is the first field in display order), with q the
/// timebase unit capped at a quarter of a field. `pts` is presentation-ordered.
pub fn select_field(pts: &[i64], timebase: Rational, rate: Rate, t: Tick) -> Option<(usize, u8)> {
    // Field duration in seconds: rate.den / (2 · rate.num).
    let (fd_num, fd_den) = (rate.den() as i128, 2 * rate.num() as i128);
    let (tn, td) = (timebase.num() as i128, timebase.den() as i128);
    let (q_num, q_den) = if cmp_frac(tn, td, fd_num, 4 * fd_den).is_le() { (tn, td) } else { (fd_num, 4 * fd_den) };
    // limit = t + q/2 in seconds.
    let lim_num = 2 * t as i128 * q_den + q_num * TICKS_PER_SECOND as i128;
    let lim_den = 2 * TICKS_PER_SECOND as i128 * q_den;
    // Frame k qualifies when pts_k ≤ limit; its second field when
    // pts_k + field ≤ limit.
    let k = pts.partition_point(|&p| cmp_frac(p as i128 * tn, td, lim_num, lim_den).is_le()).checked_sub(1)?;
    // pts_k·tn/td + fd_num/fd_den ≤ lim  ⇔  (pts_k·tn·fd_den + fd_num·td)/(td·fd_den) ≤ lim
    let second = cmp_frac(pts[k] as i128 * tn * fd_den + fd_num * td, td * fd_den, lim_num, lim_den).is_le();
    Some((k, second as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(order: FieldOrder) -> FieldInfo {
        FieldInfo { order, source: "test".into() }
    }

    #[test]
    fn field_rate_when_the_composition_is_fast_enough() {
        let tff = info(FieldOrder::TopFirst);
        assert_eq!(plan(&tff, FieldOverride::Auto, Rate::FPS_59_94, Rate::FPS_29_97).output, DeinterlaceOutput::FieldRate);
        assert_eq!(plan(&tff, FieldOverride::Auto, Rate::FPS_29_97, Rate::FPS_29_97).output, DeinterlaceOutput::FrameRate);
        assert_eq!(plan(&tff, FieldOverride::Auto, Rate::FPS_50, Rate::FPS_25).output, DeinterlaceOutput::FieldRate);
        assert_eq!(plan(&tff, FieldOverride::Auto, Rate::FPS_48, Rate::FPS_25).output, DeinterlaceOutput::FrameRate);
        assert_eq!(plan(&tff, FieldOverride::Progressive, Rate::FPS_60, Rate::FPS_29_97).output, DeinterlaceOutput::Off);
        let p = plan(&info(FieldOrder::Progressive), FieldOverride::BottomFirst, Rate::FPS_50, Rate::FPS_25);
        assert_eq!((p.order, p.output), (FieldOrder::BottomFirst, DeinterlaceOutput::FieldRate));
        assert!(!p.top_field_first());
    }

    #[test]
    fn fields_are_selected_at_field_rate() {
        // 29.97 fps at 1/30000: frame k at 1001·k; fields at 0 and 500.5.
        let tb = Rational::new(1, 30000).unwrap();
        let pts: Vec<i64> = (0..10).map(|k| 1001 * k).collect();
        let field = |n: i64| Rate::FPS_59_94.frame_start(n).unwrap();
        for n in 0..20 {
            assert_eq!(select_field(&pts, tb, Rate::FPS_29_97, field(n)), Some(((n / 2) as usize, (n % 2) as u8)), "field {n}");
        }
        assert_eq!(select_field(&pts, tb, Rate::FPS_29_97, -field(1)), None);
    }
}
