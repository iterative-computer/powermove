//! Canvas2D color strings (`fillStyle`, `strokeStyle`).
//!
//! The legacy rasterizer assigns content colors to a 2D context:
//! `c.fillStyle = d.color`, `c.strokeStyle = d.strokeColor || '#fff'`,
//! `ctx.fillStyle = path.fill`. The setter parses a CSS color and ignores
//! anything it cannot parse, so the previous style (initially opaque black)
//! stays in force. [`parse`] reproduces that parsing for the syntaxes CSS
//! Color 3 defines (hex with 3, 4, 6, or 8 digits, `rgb()`/`rgba()` and
//! `hsl()`/`hsla()` in comma and space syntax, `hwb()`, the named colors,
//! `transparent`, `currentcolor`), which covers every color the inspector
//! writes (hex) and every color an agent or extension is likely to write.
//! CSS Color 4 spaces (`lab()`, `oklch()`, `color()`) return `None`, so they
//! draw with the previous style; docs/engine/vector.md lists them as a gap.
//!
//! Colors are straight (unpremultiplied) RGBA in 0..1 as Blink keeps them:
//! hex digits are exact `n / 255`, functional channels are clamped and kept
//! as floats.

/// A straight-alpha sRGB color, components in 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl CanvasColor {
    /// Canvas2D's initial `fillStyle` and `strokeStyle`.
    pub const BLACK: Self = Self::rgba8(0, 0, 0, 255);
    pub const WHITE: Self = Self::rgba8(255, 255, 255, 255);
    pub const TRANSPARENT: Self = Self::rgba8(0, 0, 0, 0);

    pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    /// The color with its alpha multiplied by `alpha` (`globalAlpha`).
    pub fn with_alpha_scale(self, alpha: f32) -> Self {
        Self {
            a: self.a * alpha,
            ..self
        }
    }

    /// Premultiplied components.
    pub fn premultiplied(&self) -> [f32; 4] {
        [self.r * self.a, self.g * self.a, self.b * self.a, self.a]
    }

    /// The color quantized to 8 bits per channel as Skia's `SkColor` holds a
    /// Canvas2D solid paint (`SkColor4f::toSkColor`: round to nearest).
    pub fn to_rgba8(&self) -> [u8; 4] {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        [q(self.r), q(self.g), q(self.b), q(self.a)]
    }
}

/// Parses a CSS color as the Canvas2D style setters do. `None` means the
/// setter ignores the value.
pub fn parse(input: &str) -> Option<CanvasColor> {
    let s = input.trim_matches(|c: char| c.is_ascii_whitespace());
    if s.is_empty() {
        return None;
    }
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    let lower = s.to_ascii_lowercase();
    if let Some(open) = lower.find('(') {
        if !lower.ends_with(')') {
            return None;
        }
        let name = &lower[..open];
        let body = &lower[open + 1..lower.len() - 1];
        return match name {
            "rgb" | "rgba" => parse_rgb(body),
            "hsl" | "hsla" => parse_hsl(body),
            "hwb" => parse_hwb(body),
            _ => None,
        };
    }
    match lower.as_str() {
        "transparent" => Some(CanvasColor::TRANSPARENT),
        // A canvas that is not in a document resolves currentcolor to black.
        "currentcolor" => Some(CanvasColor::BLACK),
        name => named(name).map(|[r, g, b]| CanvasColor::rgba8(r, g, b, 255)),
    }
}

fn hex_digit(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

fn parse_hex(hex: &str) -> Option<CanvasColor> {
    let b = hex.as_bytes();
    let digits: Option<Vec<u8>> = b.iter().map(|&c| hex_digit(c)).collect();
    let d = digits?;
    let pair = |i: usize| d[i] * 16 + d[i + 1];
    let single = |i: usize| d[i] * 17;
    match d.len() {
        3 => Some(CanvasColor::rgba8(single(0), single(1), single(2), 255)),
        4 => Some(CanvasColor::rgba8(single(0), single(1), single(2), single(3))),
        6 => Some(CanvasColor::rgba8(pair(0), pair(2), pair(4), 255)),
        8 => Some(CanvasColor::rgba8(pair(0), pair(2), pair(4), pair(6))),
        _ => None,
    }
}

/// A numeric CSS token: a number, a percentage, or `none`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Number(f64),
    Percent(f64),
    Angle(f64),
    None,
}

fn parse_token(t: &str) -> Option<Token> {
    if t == "none" {
        return Some(Token::None);
    }
    if let Some(p) = t.strip_suffix('%') {
        return parse_number(p).map(Token::Percent);
    }
    for (unit, scale) in [
        ("deg", 1.0),
        ("grad", 0.9),
        ("rad", 180.0 / core::f64::consts::PI),
        ("turn", 360.0),
    ] {
        if let Some(n) = t.strip_suffix(unit) {
            return parse_number(n).map(|v| Token::Angle(v * scale));
        }
    }
    parse_number(t).map(Token::Number)
}

/// A CSS `<number>`: optional sign, digits with an optional fraction, an
/// optional exponent. Rejects anything else (`Number()`'s looser forms such as
/// `0x10` or `Infinity` are not CSS numbers).
fn parse_number(t: &str) -> Option<f64> {
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - digits_start;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        digits += i - frac;
    }
    if digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp {
            i = j;
        }
    }
    if i != b.len() {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Splits a functional body into channel tokens and an optional alpha:
/// either the legacy comma syntax (`a, b, c[, alpha]`) or the modern space
/// syntax (`a b c[ / alpha]`).
fn split_args(body: &str) -> Option<(Vec<Token>, Option<Token>, bool)> {
    let body = body.trim();
    if body.contains(',') {
        let parts: Vec<&str> = body.split(',').map(str::trim).collect();
        if !(parts.len() == 3 || parts.len() == 4) || parts.iter().any(|p| p.is_empty()) {
            return None;
        }
        let tokens: Option<Vec<Token>> = parts.iter().map(|p| parse_token(p)).collect();
        let mut tokens = tokens?;
        if tokens.contains(&Token::None) {
            return None;
        }
        let alpha = (tokens.len() == 4).then(|| tokens.pop()).flatten();
        Some((tokens, alpha, true))
    } else {
        let (channels, alpha) = match body.split_once('/') {
            Some((c, a)) => (c, Some(a.trim())),
            None => (body, None),
        };
        let tokens: Option<Vec<Token>> = channels.split_ascii_whitespace().map(parse_token).collect();
        let tokens = tokens?;
        if tokens.len() != 3 {
            return None;
        }
        let alpha = match alpha {
            Some(a) if a.split_ascii_whitespace().count() == 1 => Some(parse_token(a)?),
            Some(_) => return None,
            None => None,
        };
        Some((tokens, alpha, false))
    }
}

fn alpha_of(t: Option<Token>) -> Option<f64> {
    match t {
        None => Some(1.0),
        Some(Token::Number(v)) => Some(v.clamp(0.0, 1.0)),
        Some(Token::Percent(p)) => Some((p / 100.0).clamp(0.0, 1.0)),
        Some(Token::None) => Some(0.0),
        Some(Token::Angle(_)) => None,
    }
}

fn color(r: f64, g: f64, b: f64, a: f64) -> CanvasColor {
    CanvasColor {
        r: r.clamp(0.0, 1.0) as f32,
        g: g.clamp(0.0, 1.0) as f32,
        b: b.clamp(0.0, 1.0) as f32,
        a: a.clamp(0.0, 1.0) as f32,
    }
}

fn parse_rgb(body: &str) -> Option<CanvasColor> {
    let (tokens, alpha, legacy) = split_args(body)?;
    // The legacy syntax requires all three channels to be numbers or all
    // three percentages.
    if legacy {
        let percents = tokens.iter().filter(|t| matches!(t, Token::Percent(_))).count();
        if percents != 0 && percents != 3 {
            return None;
        }
    }
    let mut c = [0.0; 3];
    for (i, t) in tokens.iter().enumerate() {
        c[i] = match *t {
            Token::Number(v) => v / 255.0,
            Token::Percent(p) => p / 100.0,
            Token::None => 0.0,
            Token::Angle(_) => return None,
        };
    }
    Some(color(c[0], c[1], c[2], alpha_of(alpha)?))
}

fn hue_of(t: Token) -> Option<f64> {
    match t {
        Token::Number(v) | Token::Angle(v) => Some(v),
        Token::None => Some(0.0),
        Token::Percent(_) => None,
    }
}

fn percent_of(t: Token, legacy: bool) -> Option<f64> {
    match t {
        Token::Percent(p) => Some(p / 100.0),
        Token::Number(v) if !legacy => Some(v / 100.0),
        Token::None if !legacy => Some(0.0),
        _ => None,
    }
}

/// HSL to RGB (CSS Color 4, sample code).
fn hsl_to_rgb(hue: f64, sat: f64, light: f64) -> [f64; 3] {
    let hue = hue.rem_euclid(360.0);
    let sat = sat.clamp(0.0, 1.0);
    let light = light.clamp(0.0, 1.0);
    let f = |n: f64| {
        let k = (n + hue / 30.0) % 12.0;
        let a = sat * light.min(1.0 - light);
        light - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

fn parse_hsl(body: &str) -> Option<CanvasColor> {
    let (tokens, alpha, legacy) = split_args(body)?;
    let hue = hue_of(tokens[0])?;
    let sat = percent_of(tokens[1], legacy)?;
    let light = percent_of(tokens[2], legacy)?;
    let [r, g, b] = hsl_to_rgb(hue, sat, light);
    Some(color(r, g, b, alpha_of(alpha)?))
}

fn parse_hwb(body: &str) -> Option<CanvasColor> {
    let (tokens, alpha, legacy) = split_args(body)?;
    if legacy {
        return None;
    }
    let hue = hue_of(tokens[0])?;
    let white = percent_of(tokens[1], false)?.clamp(0.0, 1.0);
    let black = percent_of(tokens[2], false)?.clamp(0.0, 1.0);
    if white + black >= 1.0 {
        let gray = white / (white + black);
        return Some(color(gray, gray, gray, alpha_of(alpha)?));
    }
    let [r, g, b] = hsl_to_rgb(hue, 1.0, 0.5);
    let scale = 1.0 - white - black;
    let ch = |v: f64| v * scale + white;
    Some(color(ch(r), ch(g), ch(b), alpha_of(alpha)?))
}

/// The CSS named colors.
fn named(name: &str) -> Option<[u8; 3]> {
    const NAMES: &[(&str, [u8; 3])] = &[
        ("aliceblue", [240, 248, 255]),
        ("antiquewhite", [250, 235, 215]),
        ("aqua", [0, 255, 255]),
        ("aquamarine", [127, 255, 212]),
        ("azure", [240, 255, 255]),
        ("beige", [245, 245, 220]),
        ("bisque", [255, 228, 196]),
        ("black", [0, 0, 0]),
        ("blanchedalmond", [255, 235, 205]),
        ("blue", [0, 0, 255]),
        ("blueviolet", [138, 43, 226]),
        ("brown", [165, 42, 42]),
        ("burlywood", [222, 184, 135]),
        ("cadetblue", [95, 158, 160]),
        ("chartreuse", [127, 255, 0]),
        ("chocolate", [210, 105, 30]),
        ("coral", [255, 127, 80]),
        ("cornflowerblue", [100, 149, 237]),
        ("cornsilk", [255, 248, 220]),
        ("crimson", [220, 20, 60]),
        ("cyan", [0, 255, 255]),
        ("darkblue", [0, 0, 139]),
        ("darkcyan", [0, 139, 139]),
        ("darkgoldenrod", [184, 134, 11]),
        ("darkgray", [169, 169, 169]),
        ("darkgreen", [0, 100, 0]),
        ("darkgrey", [169, 169, 169]),
        ("darkkhaki", [189, 183, 107]),
        ("darkmagenta", [139, 0, 139]),
        ("darkolivegreen", [85, 107, 47]),
        ("darkorange", [255, 140, 0]),
        ("darkorchid", [153, 50, 204]),
        ("darkred", [139, 0, 0]),
        ("darksalmon", [233, 150, 122]),
        ("darkseagreen", [143, 188, 143]),
        ("darkslateblue", [72, 61, 139]),
        ("darkslategray", [47, 79, 79]),
        ("darkslategrey", [47, 79, 79]),
        ("darkturquoise", [0, 206, 209]),
        ("darkviolet", [148, 0, 211]),
        ("deeppink", [255, 20, 147]),
        ("deepskyblue", [0, 191, 255]),
        ("dimgray", [105, 105, 105]),
        ("dimgrey", [105, 105, 105]),
        ("dodgerblue", [30, 144, 255]),
        ("firebrick", [178, 34, 34]),
        ("floralwhite", [255, 250, 240]),
        ("forestgreen", [34, 139, 34]),
        ("fuchsia", [255, 0, 255]),
        ("gainsboro", [220, 220, 220]),
        ("ghostwhite", [248, 248, 255]),
        ("gold", [255, 215, 0]),
        ("goldenrod", [218, 165, 32]),
        ("gray", [128, 128, 128]),
        ("green", [0, 128, 0]),
        ("greenyellow", [173, 255, 47]),
        ("grey", [128, 128, 128]),
        ("honeydew", [240, 255, 240]),
        ("hotpink", [255, 105, 180]),
        ("indianred", [205, 92, 92]),
        ("indigo", [75, 0, 130]),
        ("ivory", [255, 255, 240]),
        ("khaki", [240, 230, 140]),
        ("lavender", [230, 230, 250]),
        ("lavenderblush", [255, 240, 245]),
        ("lawngreen", [124, 252, 0]),
        ("lemonchiffon", [255, 250, 205]),
        ("lightblue", [173, 216, 230]),
        ("lightcoral", [240, 128, 128]),
        ("lightcyan", [224, 255, 255]),
        ("lightgoldenrodyellow", [250, 250, 210]),
        ("lightgray", [211, 211, 211]),
        ("lightgreen", [144, 238, 144]),
        ("lightgrey", [211, 211, 211]),
        ("lightpink", [255, 182, 193]),
        ("lightsalmon", [255, 160, 122]),
        ("lightseagreen", [32, 178, 170]),
        ("lightskyblue", [135, 206, 250]),
        ("lightslategray", [119, 136, 153]),
        ("lightslategrey", [119, 136, 153]),
        ("lightsteelblue", [176, 196, 222]),
        ("lightyellow", [255, 255, 224]),
        ("lime", [0, 255, 0]),
        ("limegreen", [50, 205, 50]),
        ("linen", [250, 240, 230]),
        ("magenta", [255, 0, 255]),
        ("maroon", [128, 0, 0]),
        ("mediumaquamarine", [102, 205, 170]),
        ("mediumblue", [0, 0, 205]),
        ("mediumorchid", [186, 85, 211]),
        ("mediumpurple", [147, 112, 219]),
        ("mediumseagreen", [60, 179, 113]),
        ("mediumslateblue", [123, 104, 238]),
        ("mediumspringgreen", [0, 250, 154]),
        ("mediumturquoise", [72, 209, 204]),
        ("mediumvioletred", [199, 21, 133]),
        ("midnightblue", [25, 25, 112]),
        ("mintcream", [245, 255, 250]),
        ("mistyrose", [255, 228, 225]),
        ("moccasin", [255, 228, 181]),
        ("navajowhite", [255, 222, 173]),
        ("navy", [0, 0, 128]),
        ("oldlace", [253, 245, 230]),
        ("olive", [128, 128, 0]),
        ("olivedrab", [107, 142, 35]),
        ("orange", [255, 165, 0]),
        ("orangered", [255, 69, 0]),
        ("orchid", [218, 112, 214]),
        ("palegoldenrod", [238, 232, 170]),
        ("palegreen", [152, 251, 152]),
        ("paleturquoise", [175, 238, 238]),
        ("palevioletred", [219, 112, 147]),
        ("papayawhip", [255, 239, 213]),
        ("peachpuff", [255, 218, 185]),
        ("peru", [205, 133, 63]),
        ("pink", [255, 192, 203]),
        ("plum", [221, 160, 221]),
        ("powderblue", [176, 224, 230]),
        ("purple", [128, 0, 128]),
        ("rebeccapurple", [102, 51, 153]),
        ("red", [255, 0, 0]),
        ("rosybrown", [188, 143, 143]),
        ("royalblue", [65, 105, 225]),
        ("saddlebrown", [139, 69, 19]),
        ("salmon", [250, 128, 114]),
        ("sandybrown", [244, 164, 96]),
        ("seagreen", [46, 139, 87]),
        ("seashell", [255, 245, 238]),
        ("sienna", [160, 82, 45]),
        ("silver", [192, 192, 192]),
        ("skyblue", [135, 206, 235]),
        ("slateblue", [106, 90, 205]),
        ("slategray", [112, 128, 144]),
        ("slategrey", [112, 128, 144]),
        ("snow", [255, 250, 250]),
        ("springgreen", [0, 255, 127]),
        ("steelblue", [70, 130, 180]),
        ("tan", [210, 180, 140]),
        ("teal", [0, 128, 128]),
        ("thistle", [216, 191, 216]),
        ("tomato", [255, 99, 71]),
        ("turquoise", [64, 224, 208]),
        ("violet", [238, 130, 238]),
        ("wheat", [245, 222, 179]),
        ("white", [255, 255, 255]),
        ("whitesmoke", [245, 245, 245]),
        ("yellow", [255, 255, 0]),
        ("yellowgreen", [154, 205, 50]),
    ];
    NAMES
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .ok()
        .map(|i| NAMES[i].1)
}

/// A canvas style slot: assigning parses and ignores invalid values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StyleSlot(pub CanvasColor);

impl Default for StyleSlot {
    fn default() -> Self {
        Self(CanvasColor::BLACK)
    }
}

impl StyleSlot {
    /// `ctx.fillStyle = value` for a string value.
    pub fn set(&mut self, value: &str) {
        if let Some(c) = parse(value) {
            self.0 = c;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba8(s: &str) -> Option<[u8; 4]> {
        parse(s).map(|c| c.to_rgba8())
    }

    #[test]
    fn hex_forms() {
        assert_eq!(rgba8("#FF6B1A"), Some([255, 107, 26, 255]));
        assert_eq!(rgba8("#4C8DFF80"), Some([76, 141, 255, 128]));
        assert_eq!(rgba8("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(rgba8("#0008"), Some([0, 0, 0, 136]));
        assert_eq!(rgba8(" #00000000 "), Some([0, 0, 0, 0]));
        assert_eq!(rgba8("#12345"), None);
        assert_eq!(rgba8("#gggggg"), None);
        assert_eq!(rgba8("FF6B1A"), None);
    }

    #[test]
    fn functional_forms() {
        assert_eq!(rgba8("rgb(255, 0, 0)"), Some([255, 0, 0, 255]));
        assert_eq!(rgba8("rgba(0, 0, 255, 0.5)"), Some([0, 0, 255, 128]));
        assert_eq!(rgba8("rgb(0 128 0 / 50%)"), Some([0, 128, 0, 128]));
        assert_eq!(rgba8("rgb(100%, 0%, 0%)"), Some([255, 0, 0, 255]));
        assert_eq!(rgba8("rgb(100%, 0, 0)"), None);
        assert_eq!(rgba8("rgb(300, -5, 0)"), Some([255, 0, 0, 255]));
        assert_eq!(rgba8("hsl(120, 100%, 50%)"), Some([0, 255, 0, 255]));
        assert_eq!(rgba8("hsl(0.5turn 100% 50%)"), Some([0, 255, 255, 255]));
        assert_eq!(rgba8("hsla(240, 100%, 50%, .25)"), Some([0, 0, 255, 64]));
        assert_eq!(rgba8("hwb(0 0% 0%)"), Some([255, 0, 0, 255]));
        assert_eq!(rgba8("hwb(0 50% 50%)"), Some([128, 128, 128, 255]));
        assert_eq!(rgba8("rgb(1,2)"), None);
        assert_eq!(rgba8("oklch(0.5 0.1 120)"), None);
    }

    #[test]
    fn names_keywords_and_slots() {
        assert_eq!(rgba8("RebeccaPurple"), Some([102, 51, 153, 255]));
        assert_eq!(rgba8("transparent"), Some([0, 0, 0, 0]));
        assert_eq!(rgba8("nonsense"), None);
        let mut slot = StyleSlot::default();
        slot.set("bad");
        assert_eq!(slot.0, CanvasColor::BLACK);
        slot.set("#E8E2CF");
        assert_eq!(slot.0.to_rgba8(), [232, 226, 207, 255]);
        slot.set("");
        assert_eq!(slot.0.to_rgba8(), [232, 226, 207, 255]);
    }

    #[test]
    fn named_table_is_sorted() {
        // `named` binary-searches the table.
        assert!(named("aliceblue").is_some() && named("yellowgreen").is_some());
        assert_eq!(named("grey"), Some([128, 128, 128]));
    }
}
