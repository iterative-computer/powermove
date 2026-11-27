//! The SDR delivery rule, shared by every SDR output and the SDR viewer:
//! each channel of a straight, display-encoded Rec.709 colour is clipped to
//! 0..1. It is what the shipped app delivered, what players show, and legal
//! levels for broadcast QC: light above white becomes white, sub-blacks black,
//! and a colour outside Rec.709 keeps its in-range channels. HDR outputs
//! never take it.

/// The lowest and highest delivered channel value (display-encoded, straight).
pub const SDR_MIN: f64 = 0.0;
pub const SDR_MAX: f64 = 1.0;

/// One straight, display-encoded Rec.709 colour as an SDR delivery holds it.
pub fn sdr(rgb: [f64; 3]) -> [f64; 3] {
    rgb.map(|c| {
        if c.is_nan() {
            SDR_MIN
        } else {
            c.clamp(SDR_MIN, SDR_MAX)
        }
    })
}

/// The same rule on a premultiplied colour of coverage `a`.
pub fn sdr_premultiplied(rgb: [f64; 3], a: f64) -> [f64; 3] {
    let a = if a.is_finite() {
        a.clamp(0.0, 1.0)
    } else {
        0.0
    };
    rgb.map(|c| {
        if c.is_nan() {
            0.0
        } else {
            c.clamp(SDR_MIN * a, SDR_MAX * a)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channels_clip_independently() {
        assert_eq!(sdr([1.6, 0.5, 0.2]), [1.0, 0.5, 0.2]);
        assert_eq!(sdr([1.5, 0.1, -0.1]), [1.0, 0.1, 0.0]);
        assert_eq!(sdr([2.0; 3]), [1.0; 3]);
        assert_eq!(sdr_premultiplied([0.8, 0.2, -0.1], 0.5), [0.5, 0.2, 0.0]);
    }
}
