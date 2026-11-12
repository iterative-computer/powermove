//! Colour arithmetic for the token table: roles are written in OKLCH (even
//! lightness steps, hue kept while chroma changes) and stored as opaque sRGB.
//! Neutrals are one ink per appearance composited over a ground at an alpha.

/// An OKLCH colour as opaque 0xRRGGBB, clipped to sRGB per channel.
pub fn oklch(lightness: f32, chroma: f32, hue: f32) -> u32 {
    let (b, a) = hue.to_radians().sin_cos();
    let (a, b) = (a * chroma, b * chroma);
    let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let linear = [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ];
    linear.into_iter().fold(0, |out, v| {
        let v = v.clamp(0., 1.);
        let v = if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        };
        (out << 8) | (v * 255.).round() as u32
    })
}

/// (lightness, chroma, hue in degrees) of an opaque 0xRRGGBB.
pub fn to_oklch(rgb: u32) -> (f32, f32, f32) {
    let [r, g, b] = [16, 8, 0].map(|shift| {
        let v = ((rgb >> shift) & 255) as f32 / 255.;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    (
        lightness,
        a.hypot(b),
        b.atan2(a).to_degrees().rem_euclid(360.),
    )
}

/// `b` composited onto `a` at `amount` (0–1): `a` moved that far toward `b`,
/// per sRGB channel, as opaque 0xRRGGBB.
pub fn mix(a: u32, b: u32, amount: f32) -> u32 {
    [0, 8, 16].into_iter().fold(0, |out, s| {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        out | (((x + (y - x) * amount).round() as u32) << s)
    })
}

/// WCAG contrast ratio of two opaque colours.
pub fn contrast(a: u32, b: u32) -> f32 {
    let luminance = |c: u32| {
        let ch = |s: u32| {
            let v = ((c >> s) & 255) as f32 / 255.;
            if v <= 0.039_28 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
    };
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `ink` over `ground` at the lowest alpha (searched from `alpha`, in 1 %
/// steps) that reaches `ratio` against every colour in `on`: a text step that
/// stays an ink alpha and still clears its contrast floor.
pub fn ink_step(ground: u32, ink: u32, alpha: f32, ratio: f32, on: &[u32]) -> u32 {
    let mut alpha = alpha;
    loop {
        let colour = mix(ground, ink, alpha);
        if alpha >= 1. || on.iter().all(|bg| contrast(colour, *bg) >= ratio) {
            return colour;
        }
        alpha = (alpha + 0.01).min(1.);
    }
}
