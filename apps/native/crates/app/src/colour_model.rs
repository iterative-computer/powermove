//! Composition-space RGBA picker arithmetic. UI chrome belongs to pm-design.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub [f32; 4]);
impl Color {
    pub fn parse(text: &str) -> Option<Self> {
        let s = text.trim().trim_start_matches('#');
        if !matches!(s.len(), 3 | 4 | 6 | 8) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let s = if s.len() <= 4 {
            s.chars().flat_map(|c| [c, c]).collect::<String>()
        } else {
            s.into()
        };
        let n = u32::from_str_radix(&s, 16).ok()?;
        let n = if s.len() == 6 { (n << 8) | 255 } else { n };
        Some(Self([24, 16, 8, 0].map(|s| ((n >> s) & 255) as f32 / 255.)))
    }
    pub fn hex(self) -> String {
        let c = self.0.map(|c| (c.clamp(0., 1.) * 255.).round() as u8);
        let rgb = format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
        if c[3] == 255 {
            rgb
        } else {
            format!("{rgb}{:02X}", c[3])
        }
    }
    pub fn hsv(self) -> [f32; 3] {
        let [r, g, b, _] = self.0;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = if d == 0. {
            0.
        } else if max == r {
            ((g - b) / d).rem_euclid(6.)
        } else if max == g {
            (b - r) / d + 2.
        } else {
            (r - g) / d + 4.
        };
        [h * 60., if max == 0. { 0. } else { d / max }, max]
    }
    pub fn from_hsv(h: f32, s: f32, v: f32, a: f32) -> Self {
        let h = h.rem_euclid(360.) / 60.;
        let c = v * s;
        let x = c * (1. - (h.rem_euclid(2.) - 1.).abs());
        let m = v - c;
        let [r, g, b] = match h as u8 {
            0 => [c, x, 0.],
            1 => [x, c, 0.],
            2 => [0., c, x],
            3 => [0., x, c],
            4 => [x, 0., c],
            _ => [c, 0., x],
        };
        Self([r + m, g + m, b + m, a])
    }
    pub fn clipped(self) -> Self {
        Self(self.0.map(|n| n.clamp(0., 1.)))
    }
    pub fn hsl(self) -> [f32; 3] {
        let [h, s, v] = self.clipped().hsv();
        let l = v * (1. - s / 2.);
        [
            h,
            if l == 0. || l == 1. {
                0.
            } else {
                (v - l) / l.min(1. - l)
            },
            l,
        ]
    }
    pub fn from_hsl(h: f32, s: f32, l: f32, a: f32) -> Self {
        let v = l + s * l.min(1. - l);
        Self::from_hsv(h, if v == 0. { 0. } else { 2. * (1. - l / v) }, v, a)
    }
    pub fn oklab(self) -> [f32; 3] {
        let [r, g, b, _] = self.0.map(linearize);
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
        [
            0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
            1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
            0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
        ]
    }
    pub fn oklch(self) -> [f32; 3] {
        let [l, a, b] = self.oklab();
        let c = a.hypot(b);
        [
            l,
            if c < 0.00001 { 0. } else { c },
            if c < 0.00001 {
                0.
            } else {
                b.atan2(a).to_degrees().rem_euclid(360.)
            },
        ]
    }
    pub fn from_oklab(l: f32, a: f32, b: f32, alpha: f32) -> Self {
        let ll = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
        let mm = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
        let ss = (l - 0.0894841775 * a - 1.291485548 * b).powi(3);
        Self([
            delinearize(4.0767416621 * ll - 3.3077115913 * mm + 0.2309699292 * ss),
            delinearize(-1.2684380046 * ll + 2.6097574011 * mm - 0.3413193965 * ss),
            delinearize(-0.0041960863 * ll - 0.7034186147 * mm + 1.707614701 * ss),
            alpha,
        ])
    }
    pub fn from_oklch(l: f32, c: f32, h: f32, a: f32) -> Self {
        Self::from_oklab(l, c * h.to_radians().cos(), c * h.to_radians().sin(), a)
    }
    pub fn p3(self) -> Self {
        let [r, g, b, _] = self.0.map(linearize);
        let x = 0.4123908 * r + 0.35758434 * g + 0.18048079 * b;
        let y = 0.21263901 * r + 0.71516868 * g + 0.07219232 * b;
        let z = 0.01933082 * r + 0.11919478 * g + 0.95053215 * b;
        Self([
            delinearize(2.49349691 * x - 0.93138362 * y - 0.40271078 * z).clamp(0., 1.),
            delinearize(-0.82948897 * x + 1.76266406 * y + 0.02362469 * z).clamp(0., 1.),
            delinearize(0.03584583 * x - 0.07617239 * y + 0.95688452 * z).clamp(0., 1.),
            self.0[3],
        ])
    }
    pub fn from_p3(c: Self) -> Self {
        let [r, g, b, _] = c.0.map(linearize);
        let x = 0.48657095 * r + 0.26566769 * g + 0.19821729 * b;
        let y = 0.22897456 * r + 0.69173852 * g + 0.07928691 * b;
        let z = 0.04511338 * g + 1.04394437 * b;
        Self([
            delinearize(3.24096994 * x - 1.53738318 * y - 0.49861076 * z),
            delinearize(-0.96924364 * x + 1.8759675 * y + 0.04155506 * z),
            delinearize(0.05563008 * x - 0.20397696 * y + 1.05697151 * z),
            c.0[3],
        ])
    }
    pub fn outside(self) -> bool {
        self.0[..3].iter().any(|c| *c < -0.00001 || *c > 1.00001)
    }
    pub fn format(self, notation: &str) -> String {
        match notation {
            "RGB" => format!(
                "{:.0} {:.0} {:.0}",
                self.0[0].clamp(0., 1.) * 255.,
                self.0[1].clamp(0., 1.) * 255.,
                self.0[2].clamp(0., 1.) * 255.
            ),
            "HSL" => {
                let [h, s, l] = self.hsl();
                format!("{h:.0}° {:.0}% {:.0}%", s * 100., l * 100.)
            }
            "OKLCH" => {
                let [l, c, h] = self.oklch();
                format!("{:.0}% {c:.3} {h:.0}°", l * 100.)
            }
            "Display P3" => {
                let c = self.p3();
                format!("{:.3} {:.3} {:.3}", c.0[0], c.0[1], c.0[2])
            }
            _ => Self([self.0[0], self.0[1], self.0[2], 1.]).hex(),
        }
    }
    pub fn draft(self, notation: &str) -> String {
        let a = self.0[3] * 100.;
        let whole = a.round();
        let a = if (whole / 100. * 255.).round() == (self.0[3] * 255.).round() {
            format!("{whole:.0}")
        } else {
            format!("{a:.1}")
        };
        format!("{} / {a}%", self.format(notation))
    }
    pub fn parse_draft(text: &str, notation: &str, alpha: f32) -> Option<Self> {
        let lower = text.trim().to_lowercase();
        let (body, opacity) = lower
            .rsplit_once('/')
            .map_or((lower.as_str(), None), |(b, a)| {
                (b, Some(a.trim().trim_end_matches(')').trim()))
            });
        let a = if let Some(a) = opacity {
            let n = a.trim_end_matches('%').parse::<f32>().ok()?;
            if a.ends_with('%') { n / 100. } else { n }
        } else {
            alpha
        };
        if !a.is_finite() {
            return None;
        }
        if let Some(mut c) = Self::parse(body.trim()) {
            if opacity.is_some() || matches!(body.trim().trim_start_matches('#').len(), 3 | 6) {
                c.0[3] = a.clamp(0., 1.);
            }
            return Some(c);
        }
        let mode = if body.starts_with("rgb(") || body.starts_with("rgba(") {
            "RGB"
        } else if body.starts_with("hsl(") || body.starts_with("hsla(") {
            "HSL"
        } else if body.starts_with("oklch(") {
            "OKLCH"
        } else if body.starts_with("color(display-p3") {
            "Display P3"
        } else {
            notation
        };
        let body = body
            .replace("color(display-p3", "")
            .replace("oklch(", "")
            .replace("rgba(", "")
            .replace("rgb(", "")
            .replace("hsla(", "")
            .replace("hsl(", "")
            .replace([',', ')', '°'], " ");
        let parts = body.split_whitespace().collect::<Vec<_>>();
        if !(3..=4).contains(&parts.len()) {
            return None;
        }
        let mut n = [0.; 4];
        for (i, s) in parts.iter().enumerate() {
            n[i] = s.trim_end_matches('%').parse::<f32>().ok()?;
            if !n[i].is_finite() {
                return None;
            }
            if s.ends_with('%') {
                n[i] /= 100.;
            }
        }
        let a = if parts.len() == 4 {
            n[3].clamp(0., 1.)
        } else {
            a.clamp(0., 1.)
        };
        let c = match mode {
            "RGB" => Self([
                if parts[0].ends_with('%') {
                    n[0]
                } else {
                    n[0] / 255.
                },
                if parts[1].ends_with('%') {
                    n[1]
                } else {
                    n[1] / 255.
                },
                if parts[2].ends_with('%') {
                    n[2]
                } else {
                    n[2] / 255.
                },
                a,
            ])
            .clipped(),
            "HSL" => Self::from_hsl(n[0], n[1].clamp(0., 1.), n[2].clamp(0., 1.), a),
            "OKLCH" => Self::from_oklch(n[0], n[1], n[2], a),
            "Display P3" => Self::from_p3(Self([n[0], n[1], n[2], a])),
            _ => return None,
        };
        Some(c)
    }
    /// Samples the renderer's RGBA buffer, never the desktop or display transform.
    pub fn sample(rgba: &[u8], width: usize, height: usize, x: usize, y: usize) -> Option<Self> {
        if x >= width || y >= height {
            return None;
        }
        let i = y.checked_mul(width)?.checked_add(x)?.checked_mul(4)?;
        let p = rgba.get(i..i + 4)?;
        Some(Self([p[0], p[1], p[2], p[3]].map(|v| v as f32 / 255.)))
    }
}
fn linearize(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn delinearize(c: f32) -> f32 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1. / 2.4) - 0.055
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hex_rgb_hsb_round_trip() {
        for hex in ["#FF6B1A", "#000000", "#FFFFFF", "#12345680", "#09F8"] {
            let c = Color::parse(hex).unwrap();
            let [h, s, v] = c.hsv();
            assert_eq!(Color::from_hsv(h, s, v, c.0[3]).hex(), c.hex());
        }
        assert!(Color::parse("#12").is_none());
        assert!(Color::parse("ééé").is_none());
    }
    #[test]
    fn electron_notations_preserve_alpha_and_p3() {
        let c = Color::parse("#FF6B1A80").unwrap();
        assert_eq!(
            Color::parse_draft("#34C759", "Hex", c.0[3]).unwrap().hex(),
            "#34C75980"
        );
        assert_eq!(
            Color::parse_draft("rgb(255 107 26 / 50%)", "Hex", 1.)
                .unwrap()
                .hex(),
            "#FF6B1A80"
        );
        for mode in ["Hex", "RGB", "HSL", "OKLCH", "Display P3"] {
            let parsed = Color::parse_draft(&c.draft(mode), mode, 1.).unwrap();
            for (a, b) in c.0.into_iter().zip(parsed.0) {
                assert!((a - b).abs() < 0.025, "{mode}: {a} != {b}");
            }
        }
        let p = Color::from_p3(Color([1., 0., 0., 1.]));
        assert!(p.outside());
        assert_eq!(p.hex(), "#FF0000");
        let q = p.p3();
        assert!((q.0[0] - 1.).abs() < 0.0001);
        assert!(q.0[1] < 0.0001);
        assert!(Color::parse_draft("#ff0000 / NaN%", "Hex", 1.).is_none());
    }
    #[test]
    fn sample_known_composition_pixel_and_alpha() {
        let p = [255, 0, 0, 255, 10, 132, 255, 128];
        assert_eq!(Color::sample(&p, 2, 1, 1, 0).unwrap().hex(), "#0A84FF80");
        assert!(Color::sample(&p, 2, 1, 2, 0).is_none());
    }
}
