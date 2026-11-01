//! Picker color math. Preserve out-of-sRGB components until document serialization.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}
impl Default for Color {
    fn default() -> Self {
        Self::new(0., 0., 0., 1.)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notation {
    Hex,
    Rgb,
    Hsl,
    Oklch,
    DisplayP3,
}
fn clamp(n: f64) -> f64 {
    n.clamp(0., 1.)
}
fn byte(n: f64) -> u8 {
    (clamp(n) * 255.).round() as u8
}
fn lin(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn delinear(c: f64) -> f64 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1. / 2.4) - 0.055
    }
}
impl Color {
    pub const fn new(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }
    pub fn clipped(self) -> Self {
        Self::new(clamp(self.r), clamp(self.g), clamp(self.b), self.a)
    }
    pub fn outside_srgb(self) -> bool {
        [self.r, self.g, self.b]
            .iter()
            .any(|c| *c < -0.00001 || *c > 1.00001)
    }
    pub fn hex(self) -> String {
        format!(
            "#{:02X}{:02X}{:02X}",
            byte(self.r),
            byte(self.g),
            byte(self.b)
        )
    }
    pub fn stored_hex(self) -> String {
        if byte(self.a) == 255 {
            self.hex()
        } else {
            format!("{}{:02X}", self.hex(), byte(self.a))
        }
    }
    pub fn rgba(self) -> gpui::Rgba {
        gpui::Rgba {
            r: clamp(self.r) as f32,
            g: clamp(self.g) as f32,
            b: clamp(self.b) as f32,
            a: clamp(self.a) as f32,
        }
    }
    pub fn from_hsv(h: f64, s: f64, v: f64, a: f64) -> Self {
        let h = h.rem_euclid(360.);
        let f = |n: f64| {
            let k = (n + h / 60.) % 6.;
            v - v * s * k.min(4. - k).min(1.).max(0.)
        };
        Self::new(f(5.), f(3.), f(1.), a)
    }
    pub fn hsv(self) -> (f64, f64, f64) {
        let max = self.r.max(self.g).max(self.b);
        let min = self.r.min(self.g).min(self.b);
        let d = max - min;
        let mut h = if d == 0. {
            0.
        } else if max == self.r {
            ((self.g - self.b) / d) % 6.
        } else if max == self.g {
            (self.b - self.r) / d + 2.
        } else {
            (self.r - self.g) / d + 4.
        };
        h = (h * 60.).rem_euclid(360.);
        if h > 359.9999 {
            h = 0.;
        }
        (h, if max != 0. { d / max } else { 0. }, max)
    }
    pub fn from_hsl(h: f64, s: f64, l: f64, a: f64) -> Self {
        let v = l + s * l.min(1. - l);
        Self::from_hsv(h, if v != 0. { 2. * (1. - l / v) } else { 0. }, v, a)
    }
    pub fn hsl(self) -> (f64, f64, f64) {
        let (h, s, v) = self.clipped().hsv();
        let l = v * (1. - s / 2.);
        (
            h,
            if l == 0. || l == 1. {
                0.
            } else {
                (v - l) / l.min(1. - l)
            },
            l,
        )
    }
    pub fn oklch(self) -> (f64, f64, f64) {
        let (r, g, b) = (lin(self.r), lin(self.g), lin(self.b));
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
        let ll = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
        let aa = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
        let bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
        let c = aa.hypot(bb);
        (
            ll,
            if c < 0.00001 { 0. } else { c },
            if c < 0.00001 {
                0.
            } else {
                bb.atan2(aa).to_degrees().rem_euclid(360.)
            },
        )
    }
    pub fn oklab(self) -> (f64, f64, f64) {
        let (l, c, h) = self.oklch();
        (l, c * h.to_radians().cos(), c * h.to_radians().sin())
    }
    pub fn from_oklab(l: f64, a: f64, b: f64, alpha: f64) -> Self {
        Self::from_oklch(l, a.hypot(b), b.atan2(a).to_degrees(), alpha)
    }
    pub fn from_oklch(l: f64, c: f64, h: f64, a: f64) -> Self {
        let aa = c * h.to_radians().cos();
        let b = c * h.to_radians().sin();
        let ll = (l + 0.3963377774 * aa + 0.2158037573 * b).powi(3);
        let m = (l - 0.1055613458 * aa - 0.0638541728 * b).powi(3);
        let s = (l - 0.0894841775 * aa - 1.291485548 * b).powi(3);
        Self::new(
            delinear(4.0767416621 * ll - 3.3077115913 * m + 0.2309699292 * s),
            delinear(-1.2684380046 * ll + 2.6097574011 * m - 0.3413193965 * s),
            delinear(-0.0041960863 * ll - 0.7034186147 * m + 1.707614701 * s),
            a,
        )
    }
    fn in_gamut(self) -> bool {
        [self.r, self.g, self.b]
            .iter()
            .all(|c| (0. ..=1.).contains(c))
    }
    pub fn keeping_lightness(l: f64, c: f64, h: f64, a: f64) -> Self {
        let n = Self::from_oklch(l, c, h, a);
        if n.in_gamut() {
            return n;
        }
        let (mut low, mut high) = (0., c);
        for _ in 0..24 {
            let mid = (low + high) / 2.;
            if Self::from_oklch(l, mid, h, a).in_gamut() {
                low = mid;
            } else {
                high = mid;
            }
        }
        Self::from_oklch(l, low, h, a)
    }
    pub fn keeping_chroma(l: f64, c: f64, h: f64, a: f64) -> Self {
        for step in 0..=200 {
            let d = step as f64 * 0.005;
            for ll in [l - d, l + d] {
                if (0. ..=1.).contains(&ll) {
                    let n = Self::from_oklch(ll, c, h, a);
                    if n.in_gamut() {
                        return n;
                    }
                }
            }
        }
        Self::keeping_lightness(l, c, h, a)
    }
    pub fn p3(self) -> Self {
        let (r, g, b) = (lin(self.r), lin(self.g), lin(self.b));
        let x = 0.4123908 * r + 0.35758434 * g + 0.18048079 * b;
        let y = 0.21263901 * r + 0.71516868 * g + 0.07219232 * b;
        let z = 0.01933082 * r + 0.11919478 * g + 0.95053215 * b;
        Self::new(
            clamp(delinear(2.49349691 * x - 0.93138362 * y - 0.40271078 * z)),
            clamp(delinear(-0.82948897 * x + 1.76266406 * y + 0.02362469 * z)),
            clamp(delinear(0.03584583 * x - 0.07617239 * y + 0.95688452 * z)),
            self.a,
        )
    }
    pub fn from_p3(p: Self) -> Self {
        let (r, g, b) = (lin(p.r), lin(p.g), lin(p.b));
        let x = 0.48657095 * r + 0.26566769 * g + 0.19821729 * b;
        let y = 0.22897456 * r + 0.69173852 * g + 0.07928691 * b;
        let z = 0.04511338 * g + 1.04394437 * b;
        Self::new(
            delinear(3.24096994 * x - 1.53738318 * y - 0.49861076 * z),
            delinear(-0.96924364 * x + 1.8759675 * y + 0.04155506 * z),
            delinear(0.05563008 * x - 0.20397696 * y + 1.05697151 * z),
            p.a,
        )
    }
    pub fn format(self, n: Notation) -> String {
        match n {
            Notation::Hex => self.hex(),
            Notation::Rgb => format!("{} {} {}", byte(self.r), byte(self.g), byte(self.b)),
            Notation::Hsl => {
                let (h, s, l) = self.hsl();
                format!(
                    "{}° {}% {}%",
                    h.round(),
                    (s * 100.).round(),
                    (l * 100.).round()
                )
            }
            Notation::Oklch => {
                let (l, c, h) = self.oklch();
                format!("{}% {c:.3} {}°", (l * 100.).round(), h.round())
            }
            Notation::DisplayP3 => {
                let p = self.p3();
                [p.r, p.g, p.b]
                    .map(|n| {
                        format!("{:.3}", n)
                            .trim_end_matches('0')
                            .trim_end_matches('.')
                            .to_string()
                    })
                    .join(" ")
            }
        }
    }
}
pub fn opacity_text(alpha: f64) -> String {
    let w = (clamp(alpha) * 100.).round();
    if byte(w / 100.) == byte(alpha) {
        w.to_string()
    } else {
        format!("{}", (clamp(alpha) * 1000.).round() / 10.)
    }
}
fn fraction(s: &str) -> Option<f64> {
    s.trim_end_matches(['%', '°'])
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .map(|n| if s.ends_with('%') { n / 100. } else { n })
}
pub fn parse(raw: &str) -> Option<Color> {
    let s = raw.trim().to_lowercase();
    if s == "transparent" {
        return Some(Color::new(0., 0., 0., 0.));
    }
    let hex = s.strip_prefix('#').unwrap_or(&s);
    if [3, 4, 6, 8].contains(&hex.len()) && hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        let h = if hex.len() <= 4 {
            hex.chars().flat_map(|c| [c, c]).collect::<String>()
        } else {
            hex.into()
        };
        let ch = |i| {
            u8::from_str_radix(&h[i..i + 2], 16)
                .ok()
                .map(|n| n as f64 / 255.)
        };
        return Some(Color::new(
            ch(0)?,
            ch(2)?,
            ch(4)?,
            if h.len() == 8 { ch(6)? } else { 1. },
        ));
    }
    let (name, body) = s.split_once('(')?;
    let body = body.strip_suffix(')')?.replace([',', '/'], " ");
    let mut parts = body.split_whitespace();
    let p3 = name == "color";
    if p3 && parts.next()? != "display-p3" {
        return None;
    }
    let parts = parts.collect::<Vec<_>>();
    if parts.len() < 3 || parts.len() > 4 {
        return None;
    }
    let (a, b, c) = (
        fraction(parts[0])?,
        fraction(parts[1])?,
        fraction(parts[2])?,
    );
    let alpha = if parts.len() == 4 {
        clamp(fraction(parts[3])?)
    } else {
        1.
    };
    Some(match name {
        "rgb" | "rgba" => {
            let ch = |v: f64, s: &str| clamp(if s.ends_with('%') { v } else { v / 255. });
            Color::new(ch(a, parts[0]), ch(b, parts[1]), ch(c, parts[2]), alpha)
        }
        "hsl" | "hsla" => Color::from_hsl(a, clamp(b), clamp(c), alpha),
        "hsv" | "hsb" => Color::from_hsv(a, clamp(b), clamp(c), alpha),
        "oklch" => Color::from_oklch(a, b, c, alpha),
        "color" => Color::from_p3(Color::new(a, b, c, alpha)),
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pasted_notations() {
        for (s, h) in [
            ("#ff8800", "#FF8800"),
            ("rgb(255, 136, 0)", "#FF8800"),
            ("hsl(32 100% 50%)", "#FF8800"),
            ("hsb(240, 100%, 100%)", "#0000FF"),
            ("#ff880080", "#FF880080"),
            ("rgb(255 136 0 / 25%)", "#FF880040"),
            ("transparent", "#00000000"),
        ] {
            assert_eq!(parse(s).unwrap().stored_hex(), h, "{s}");
        }
        assert_eq!(parse("rgb(255, 0, 0"), None);
    }
    #[test]
    fn alpha_storage() {
        let mut c = Color::new(1., 0., 0., 1.);
        assert_eq!(c.stored_hex(), "#FF0000");
        c.a = 0.999;
        assert_eq!(c.stored_hex(), "#FF0000");
        c.a = 0.5;
        assert_eq!(c.stored_hex(), "#FF000080");
        assert_eq!(c.hex(), "#FF0000");
        assert_eq!(opacity_text(parse("#FF000080").unwrap().a), "50");
        assert_eq!(opacity_text(0.333), "33.3");
    }
    #[test]
    fn wide_gamut_and_formats() {
        let red = parse("color(display-p3 1 0 0)").unwrap();
        assert!(red.outside_srgb());
        assert_eq!(red.hex(), "#FF0000");
        assert_eq!(red.format(Notation::DisplayP3), "1 0 0");
        assert!(!Color::from_p3(Color::new(0.5, 0.5, 0.5, 1.)).outside_srgb());
        let o = parse("#FF6B1A").unwrap();
        assert_eq!(o.format(Notation::Rgb), "255 107 26");
        assert_eq!(o.format(Notation::Hsl), "21° 100% 55%");
        assert!(o.format(Notation::Oklch).starts_with("70% 0.19"));
    }
    #[test]
    fn gamut_fit_and_roundtrips() {
        let c = Color::keeping_lightness(0.7, 0.4, 140., 1.);
        assert!(!c.outside_srgb());
        assert!((c.oklch().0 - 0.7).abs() < 0.001);
        let c = parse("#2386DD80").unwrap();
        let (h, s, v) = c.hsv();
        assert_eq!(Color::from_hsv(h, s, v, c.a).stored_hex(), c.stored_hex());
        let (l, a, b) = c.oklab();
        assert_eq!(Color::from_oklab(l, a, b, c.a).stored_hex(), c.stored_hex());
    }
}
