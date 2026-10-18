//! CSS colors as a canvas `fillStyle` or `strokeStyle` setter parses them.
//!
//! Parsed: hex, `rgb()`/`rgba()`, `hsl()`/`hsla()`, `hwb()`, every CSS
//! named color and `transparent` (the sRGB colors a canvas stores as 8-bit
//! channels). A string the setter rejects keeps the context's previous
//! style; callers pass that fallback ([`Rejected::Invalid`]). Valid CSS
//! colors this module does not reproduce (`lab()`, `lch()`, `oklab()`,
//! `oklch()`, `color()`, `color-mix()`, relative colors, `calc()`,
//! `currentcolor`, system colors) are [`Rejected::Unsupported`]: the
//! renderer refuses such a layer rather than drawing the fallback. Any
//! functional notation that fails to parse counts as unsupported, so a
//! string is only ever drawn as the fallback when it is certainly not a
//! color.
//!
//! Values are E-encoded straight RGBA in 0..1 (the working encoding:
//! a hex code's value is the working value, section 9).

/// Straight (unpremultiplied) RGBA, E-encoded, 0..1.
pub type Rgba = [f64; 4];

pub const BLACK: Rgba = [0.0, 0.0, 0.0, 1.0];
pub const WHITE: Rgba = [1.0, 1.0, 1.0, 1.0];

fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn hex(s: &str) -> Option<Rgba> {
    let b = s.as_bytes();
    let d: Option<Vec<u8>> = b.iter().map(|c| hex_digit(*c)).collect();
    let d = d?;
    let byte = |hi: u8, lo: u8| f64::from(hi * 16 + lo) / 255.0;
    match d.len() {
        3 => Some([byte(d[0], d[0]), byte(d[1], d[1]), byte(d[2], d[2]), 1.0]),
        4 => Some([
            byte(d[0], d[0]),
            byte(d[1], d[1]),
            byte(d[2], d[2]),
            byte(d[3], d[3]),
        ]),
        6 => Some([byte(d[0], d[1]), byte(d[2], d[3]), byte(d[4], d[5]), 1.0]),
        8 => Some([
            byte(d[0], d[1]),
            byte(d[2], d[3]),
            byte(d[4], d[5]),
            byte(d[6], d[7]),
        ]),
        _ => None,
    }
}

/// One `rgb()`/`rgba()` component: a number (0..255) or a percentage.
fn channel(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(p) = s.strip_suffix('%') {
        let v: f64 = p.trim().parse().ok()?;
        if !v.is_finite() {
            return None;
        }
        // A percentage channel is stored as 8 bits too.
        return Some(byte_channel(v / 100.0));
    }
    let v: f64 = s.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    // CSS rounds the 0..255 value to the 8-bit color the canvas stores.
    Some((v.clamp(0.0, 255.0)).round() / 255.0)
}

fn alpha(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(p) = s.strip_suffix('%') {
        let v: f64 = p.trim().parse().ok()?;
        return Some((v / 100.0).clamp(0.0, 1.0));
    }
    let v: f64 = s.parse().ok()?;
    // `inf` and `NaN` are not CSS numbers.
    if !v.is_finite() {
        return None;
    }
    Some(v.clamp(0.0, 1.0))
}

/// `rgb()`/`rgba()` arguments.
fn functional(args: &str) -> Option<Rgba> {
    let ([r, g, b], a) = components(args)?;
    // The legacy syntax takes three numbers or three percentages; the
    // modern one may mix them. Mixed channels are not reproduced either way.
    let percents = [r, g, b].iter().filter(|c| c.ends_with('%')).count();
    if percents != 0 && percents != 3 {
        return None;
    }
    Some([
        channel(r)?,
        channel(g)?,
        channel(b)?,
        match a {
            Some(a) => alpha(a)?,
            None => 1.0,
        },
    ])
}

/// The arguments of a functional notation: three components and an
/// optional alpha, in the legacy comma syntax or the space syntax with
/// `/ alpha`.
fn components(args: &str) -> Option<([&str; 3], Option<&str>)> {
    let (main, slash_alpha) = match args.split_once('/') {
        Some((m, a)) => (m, Some(a)),
        None => (args, None),
    };
    let comma = main.contains(',');
    let parts: Vec<&str> = if comma {
        main.split(',').map(str::trim).collect()
    } else {
        main.split_whitespace().collect()
    };
    if comma && slash_alpha.is_some() {
        return None;
    }
    let slash_alpha = slash_alpha.map(str::trim);
    // Every component is one token (`50 %` is not a percentage), and the
    // legacy comma syntax has no `none`.
    let bad = |p: &&str| {
        p.is_empty() || p.contains(char::is_whitespace) || (comma && p.eq_ignore_ascii_case("none"))
    };
    if parts.iter().any(bad) || slash_alpha.iter().any(bad) {
        return None;
    }
    match (parts.len(), slash_alpha) {
        (3, a) => Some(([parts[0], parts[1], parts[2]], a)),
        (4, None) if comma => Some(([parts[0], parts[1], parts[2]], Some(parts[3]))),
        _ => None,
    }
}

/// A hue in degrees: a number, or an angle in `deg`, `rad`, `grad`, `turn`.
fn hue(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    let lower = s.to_ascii_lowercase();
    let (num, factor) = if let Some(v) = lower.strip_suffix("grad") {
        (v, 360.0 / 400.0)
    } else if let Some(v) = lower.strip_suffix("deg") {
        (v, 1.0)
    } else if let Some(v) = lower.strip_suffix("rad") {
        (v, 180.0 / std::f64::consts::PI)
    } else if let Some(v) = lower.strip_suffix("turn") {
        (v, 360.0)
    } else {
        (lower.as_str(), 1.0)
    };
    let v: f64 = num.trim().parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    Some((v * factor).rem_euclid(360.0))
}

/// An `hsl()`/`hwb()` percentage component as 0..1: a percentage, or in
/// the space syntax a number on the 0..100 scale (CSS Color 4).
fn percent(s: &str, number_ok: bool) -> Option<f64> {
    let s = s.trim();
    if number_ok && s.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    let (num, ok) = match s.strip_suffix('%') {
        Some(p) => (p.trim(), true),
        None => (s, number_ok),
    };
    if !ok {
        return None;
    }
    let v: f64 = num.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    Some((v / 100.0).clamp(0.0, 1.0))
}

/// HSL to sRGB (CSS Color 4 `hslToRgb`), unrounded.
fn hsl_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    let f = |n: f64| {
        let k = (n + h / 30.0).rem_euclid(12.0);
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

/// A channel in 0..1 rounded to the 8-bit value the canvas stores.
fn byte_channel(v: f64) -> f64 {
    (v.clamp(0.0, 1.0) * 255.0).round() / 255.0
}

fn hsl(args: &str) -> Option<Rgba> {
    let ([h, s, l], a) = components(args)?;
    let numbers = !args.contains(',');
    let rgb = hsl_rgb(hue(h)?, percent(s, numbers)?, percent(l, numbers)?);
    let a = match a {
        Some(a) => alpha(a)?,
        None => 1.0,
    };
    Some([
        byte_channel(rgb[0]),
        byte_channel(rgb[1]),
        byte_channel(rgb[2]),
        a,
    ])
}

fn hwb(args: &str) -> Option<Rgba> {
    // `hwb()` has only the space syntax.
    if args.contains(',') {
        return None;
    }
    let ([h, w, b], a) = components(args)?;
    let (h, mut w, mut b) = (hue(h)?, percent(w, true)?, percent(b, true)?);
    if w + b >= 1.0 {
        let sum = w + b;
        w /= sum;
        b /= sum;
    }
    let base = hsl_rgb(h, 1.0, 0.5);
    let ch = |c: f64| byte_channel(c * (1.0 - w - b) + w);
    let a = match a {
        Some(a) => alpha(a)?,
        None => 1.0,
    };
    Some([ch(base[0]), ch(base[1]), ch(base[2]), a])
}

/// Why a canvas style setter does not take a string as a color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// Not a CSS color: the setter keeps its previous style.
    Invalid,
    /// A CSS color (or possibly one) this parser does not reproduce: the
    /// layer must be refused, not drawn with the previous style.
    Unsupported,
}

/// CSS system colors and `currentcolor`: valid, but resolved by the
/// document, not by this parser.
const DOCUMENT_COLORS: &[&str] = &[
    "accentcolor",
    "accentcolortext",
    "activeborder",
    "activecaption",
    "activetext",
    "appworkspace",
    "background",
    "buttonborder",
    "buttonface",
    "buttonhighlight",
    "buttonshadow",
    "buttontext",
    "canvas",
    "canvastext",
    "captiontext",
    "currentcolor",
    "field",
    "fieldtext",
    "graytext",
    "highlight",
    "highlighttext",
    "inactiveborder",
    "inactivecaption",
    "inactivecaptiontext",
    "infobackground",
    "infotext",
    "linktext",
    "mark",
    "marktext",
    "menu",
    "menutext",
    "scrollbar",
    "selecteditem",
    "selecteditemtext",
    "threeddarkshadow",
    "threedface",
    "threedhighlight",
    "threedlightshadow",
    "threedshadow",
    "visitedtext",
    "window",
    "windowframe",
    "windowtext",
];

/// Classifies a string as the canvas setter would see it: the color, or
/// why it is not drawn as one.
pub fn classify(value: &str) -> Result<Rgba, Rejected> {
    let s = value.trim();
    if let Some(h) = s.strip_prefix('#') {
        return hex(h).ok_or(Rejected::Invalid);
    }
    let lower = s.to_ascii_lowercase();
    if let Some((name, rest)) = lower.split_once('(') {
        let name = name.trim_end();
        let parsed = rest.trim_end().strip_suffix(')').and_then(|inner| {
            // Nested functions (`calc()`, relative colors) are not parsed.
            if inner.contains('(') || inner.contains(" from ") || inner.starts_with("from ") {
                return None;
            }
            match name {
                "rgb" | "rgba" => functional(inner),
                "hsl" | "hsla" => hsl(inner),
                "hwb" => hwb(inner),
                _ => None,
            }
        });
        return parsed.ok_or(Rejected::Unsupported);
    }
    if lower == "transparent" {
        return Ok([0.0, 0.0, 0.0, 0.0]);
    }
    if let Some([r, g, b]) = named(&lower) {
        return Ok([
            f64::from(r) / 255.0,
            f64::from(g) / 255.0,
            f64::from(b) / 255.0,
            1.0,
        ]);
    }
    if DOCUMENT_COLORS.contains(&lower.as_str()) {
        return Err(Rejected::Unsupported);
    }
    Err(Rejected::Invalid)
}

/// Parses a CSS color as the canvas setter does, or `None` (the setter
/// keeps its previous value, or the color is not reproduced; see
/// [`classify`]).
pub fn parse(value: &str) -> Option<Rgba> {
    classify(value).ok()
}

/// Parses `value`, keeping `previous` when the setter would reject it.
pub fn parse_or(value: &str, previous: Rgba) -> Rgba {
    parse(value).unwrap_or(previous)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_documents_hold() {
        assert_eq!(parse("#fff"), Some(WHITE));
        assert_eq!(parse("#FF000080"), Some([1.0, 0.0, 0.0, 128.0 / 255.0]));
        assert_eq!(parse("rgba(255,0,0,0.5)"), Some([1.0, 0.0, 0.0, 0.5]));
        assert_eq!(
            parse(" rgb(0, 128, 255) "),
            Some([0.0, 128.0 / 255.0, 1.0, 1.0])
        );
        assert_eq!(parse("rgb(0 0 0 / 25%)"), Some([0.0, 0.0, 0.0, 0.25]));
        assert_eq!(parse("Red"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse("transparent"), Some([0.0; 4]));
        assert_eq!(parse("#12"), None);
        assert_eq!(parse("nonsense"), None);
        assert_eq!(parse_or("nonsense", BLACK), BLACK);
    }

    fn rgb8(r: u8, g: u8, b: u8, a: f64) -> Rgba {
        [
            f64::from(r) / 255.0,
            f64::from(g) / 255.0,
            f64::from(b) / 255.0,
            a,
        ]
    }

    #[test]
    fn parses_hsl_and_hwb_as_chromium_stores_them() {
        // Chromium (Electron, 2026-10-16) serializes these fillStyle values
        // as the codes below.
        assert_eq!(parse("hsl(0, 100%, 50%)"), Some(rgb8(255, 0, 0, 1.0)));
        assert_eq!(parse("hsl(120deg 100% 25%)"), Some(rgb8(0, 128, 0, 1.0)));
        assert_eq!(parse("hsl(210 50% 40%)"), Some(rgb8(51, 102, 153, 1.0)));
        assert_eq!(parse("hsl(0.5turn 100 50)"), Some(rgb8(0, 255, 255, 1.0)));
        assert_eq!(
            parse("hsla(240, 100%, 50%, 0.5)"),
            Some(rgb8(0, 0, 255, 0.5))
        );
        assert_eq!(
            parse("HSL(300 100% 50% / 25%)"),
            Some(rgb8(255, 0, 255, 0.25))
        );
        assert_eq!(parse("hsl(-120, 100%, 50%)"), Some(rgb8(0, 0, 255, 1.0)));
        assert_eq!(parse("hwb(0 0% 0%)"), Some(rgb8(255, 0, 0, 1.0)));
        assert_eq!(parse("hwb(120 20% 30%)"), Some(rgb8(51, 179, 51, 1.0)));
        assert_eq!(parse("hwb(0 60% 60%)"), Some(rgb8(128, 128, 128, 1.0)));
        assert_eq!(parse("hsl(37 61% 47%)"), Some(rgb8(0xc1, 0x89, 0x2f, 1.0)));
        assert_eq!(parse("hwb(200 13% 41%)"), Some(rgb8(0x21, 0x6f, 0x96, 1.0)));
        assert_eq!(parse("hsl(none 100% 50%)"), Some(rgb8(255, 0, 0, 1.0)));
        // The legacy comma syntax needs percentages (Chromium rejects it);
        // a functional notation that does not parse is refused, not guessed.
        assert_eq!(classify("hsl(0, 100, 50)"), Err(Rejected::Unsupported));
    }

    #[test]
    fn strings_outside_the_grammar_are_refused_not_guessed() {
        // Each of these is either invalid CSS or a form whose handling the
        // parser does not reproduce; none may draw as a parsed color.
        for s in [
            "hsl(none, 100%, 50%)",
            "rgb(none, 0, 0)",
            "rgb(255, 50%, 0)",
            "rgb(255 50% 0)",
            "rgb(50 % 0% 0%)",
            "hsl(0 100 % 50%)",
            "rgb(0 0 0 / inf)",
            "rgb(0 0 0 / NaN)",
            "rgb(0, 0, 0, infinity)",
            "rgb(1,,2,3)",
            "rgb(1 2 3 /)",
        ] {
            assert_eq!(classify(s), Err(Rejected::Unsupported), "{s}");
        }
        assert_eq!(parse("rgb(100%, 0%, 50%)"), Some(rgb8(255, 0, 128, 1.0)));
        assert_eq!(parse("rgb( 1 , 2 , 3 , .5 )"), Some(rgb8(1, 2, 3, 0.5)));
    }

    #[test]
    fn refuses_colors_it_does_not_reproduce() {
        for s in [
            "oklch(70% 0.1 200)",
            "oklab(0.5 0.1 0.1)",
            "lab(50% 40 59.5)",
            "lch(52% 72 50)",
            "color(display-p3 1 0 0)",
            "color-mix(in srgb, red, blue)",
            "rgb(from red r g b)",
            "rgb(calc(255) 0 0)",
            "rgb(255 none 0)",
            "light-dark(red, blue)",
            "currentcolor",
            "CanvasText",
        ] {
            assert_eq!(classify(s), Err(Rejected::Unsupported), "{s}");
        }
        for s in ["", "nonsense", "undefined", "null", "#12", "#ggg", "5"] {
            assert_eq!(classify(s), Err(Rejected::Invalid), "{s:?}");
        }
    }

    #[test]
    fn named_colors_are_sorted_for_the_search() {
        assert_eq!(
            parse("yellowgreen"),
            Some([154.0 / 255.0, 205.0 / 255.0, 50.0 / 255.0, 1.0])
        );
        assert_eq!(parse("aliceblue").map(|c| c[0]), Some(240.0 / 255.0));
        assert_eq!(parse("rebeccapurple").map(|c| c[2]), Some(153.0 / 255.0));
    }
}
