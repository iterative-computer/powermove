//! Independent f64 display reference. D65 Lab, CIEDE2000; no screenshot pixels.
fn decode(v: f64) -> f64 {
    let m = v.abs();
    (if m <= 0.04045 {
        m / 12.92
    } else {
        ((m + 0.055) / 1.055).powf(2.4)
    })
    .copysign(v)
}
pub fn p3(encoded: &[f32]) -> [f64; 3] {
    let a = encoded[3] as f64;
    if a <= 0. {
        return [0.; 3];
    }
    let c = [encoded[0], encoded[1], encoded[2]].map(|c| decode(c as f64 / a));
    [
        0.8224621 * c[0] + 0.1775380 * c[1],
        0.0331941 * c[0] + 0.9668058 * c[1],
        0.0170827 * c[0] + 0.0723974 * c[1] + 0.9105199 * c[2],
    ]
    .map(|v| v * a)
}
fn lab(c: [f64; 3]) -> [f64; 3] {
    let xyz = [
        0.48657095 * c[0] + 0.26566769 * c[1] + 0.19821729 * c[2],
        0.22897456 * c[0] + 0.69173852 * c[1] + 0.07928691 * c[2],
        0.04511338 * c[1] + 1.04394437 * c[2],
    ];
    let f = |v: f64| {
        if v > 216. / 24389. {
            v.cbrt()
        } else {
            (24389. / 27. * v + 16.) / 116.
        }
    };
    let [x, y, z] = [f(xyz[0] / 0.95047), f(xyz[1]), f(xyz[2] / 1.08883)];
    [116. * y - 16., 500. * (x - y), 200. * (y - z)]
}
pub fn delta(a: [f64; 3], b: [f64; 3]) -> f64 {
    delta_lab(lab(a), lab(b))
}
fn delta_lab(a: [f64; 3], b: [f64; 3]) -> f64 {
    let [l1, a1, b1] = a;
    let [l2, a2, b2] = b;
    let c = (a1.hypot(b1) + a2.hypot(b2)) / 2.;
    let g = 0.5 * (1. - (c.powi(7) / (c.powi(7) + 25_f64.powi(7))).sqrt());
    let (ap1, ap2) = ((1. + g) * a1, (1. + g) * a2);
    let (c1, c2) = (ap1.hypot(b1), ap2.hypot(b2));
    let hue = |b: f64, a: f64| b.atan2(a).to_degrees().rem_euclid(360.);
    let (h1, h2) = (hue(b1, ap1), hue(b2, ap2));
    let dl = l2 - l1;
    let dc = c2 - c1;
    let mut dh = h2 - h1;
    if c1 * c2 == 0. {
        dh = 0.;
    } else if dh > 180. {
        dh -= 360.;
    } else if dh < -180. {
        dh += 360.;
    }
    let d_h = 2. * (c1 * c2).sqrt() * (dh.to_radians() / 2.).sin();
    let lm = (l1 + l2) / 2.;
    let cm = (c1 + c2) / 2.;
    let hm = if c1 * c2 == 0. {
        h1 + h2
    } else if (h1 - h2).abs() <= 180. {
        (h1 + h2) / 2.
    } else if h1 + h2 < 360. {
        (h1 + h2 + 360.) / 2.
    } else {
        (h1 + h2 - 360.) / 2.
    };
    let cos = |d: f64| d.to_radians().cos();
    let t = 1. - 0.17 * cos(hm - 30.) + 0.24 * cos(2. * hm) + 0.32 * cos(3. * hm + 6.)
        - 0.20 * cos(4. * hm - 63.);
    let sl = 1. + 0.015 * (lm - 50.).powi(2) / (20. + (lm - 50.).powi(2)).sqrt();
    let sc = 1. + 0.045 * cm;
    let sh = 1. + 0.015 * cm * t;
    let rt = -2.
        * (cm.powi(7) / (cm.powi(7) + 25_f64.powi(7))).sqrt()
        * (60. * (-((hm - 275.) / 25.).powi(2)).exp())
            .to_radians()
            .sin();
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (d_h / sh).powi(2) + rt * (dc / sc) * (d_h / sh))
        .max(0.)
        .sqrt()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn published_ciede2000_reference_pair() {
        assert!((delta_lab([50., 2.6772, -79.7751], [50., 0., -82.7485]) - 2.0425).abs() < 0.0001);
    }
    #[test]
    fn identity_and_extended_range() {
        assert_eq!(delta([2., 1., 0.5], [2., 1., 0.5]), 0.);
        assert!(p3(&[2., 2., 2., 1.])[0] > 1.);
    }
}
