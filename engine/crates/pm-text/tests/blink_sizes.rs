//! Blink's font sizes on macOS, against Chromium's own `measureText`: the
//! effective size (`floorf(size × 100) / 100`) that fonts are created and
//! scaled at, the specified size HarfBuzz looks `trak` up at and `opsz`
//! follows, HarfBuzz's truncating parent-to-sub-font advance scaling, and
//! the renderer-wide font cache (`FontCacheSeed`). Runs without the test
//! harness so the registry lives on the main thread.
//!
//! Expected widths were measured with `400 <size>px "system-ui"` canvas fonts
//! in Chrome for Testing 151.0.7922.34 (headless, macOS 27.0), and in
//! Electron 44 (Chromium 152) for the corpus values.

#[cfg(all(target_os = "macos", feature = "system"))]
fn main() {
    use pm_text::compat::{Compat, FontCacheSeed, TextContent};
    use pm_text::registry::CoreTextRegistry;

    /// (size, width of "H", width of "i") in a fresh renderer.
    const FRESH: &[(f64, f64, f64)] = &[
        (11.07, 8.276321411132812, 2.795379638671875),
        (11.14, 8.3240966796875, 2.8084869384765625),
        (11.21, 8.371795654296875, 2.8215484619140625),
        (11.28, 8.419464111328125, 2.83453369140625),
        (11.35, 8.467041015625, 2.84747314453125),
        (11.42, 8.514572143554688, 2.8603363037109375),
        (11.49, 8.562057495117188, 2.8731536865234375),
        (11.56, 8.609481811523438, 2.88592529296875),
        (11.63, 8.656829833984375, 2.89862060546875),
        (11.7, 8.704147338867188, 2.911285400390625),
        (11.77, 8.75140380859375, 2.9238739013671875),
        (11.84, 8.798583984375, 2.9364013671875),
        (11.91, 8.845718383789062, 2.948883056640625),
        (13.37, 9.820541381835938, 3.2008209228515625),
        (14.41, 10.511260986328125, 3.3766326904296875),
        (15.73, 11.383956909179688, 3.5957794189453125),
        (16.21, 11.687088012695312, 3.66619873046875),
        (19.33, 13.786376953125, 4.2157745361328125),
        (21.07, 14.935928344726562, 4.51409912109375),
        (23.51, 16.675674438476562, 5.0699005126953125),
        (26.89, 19.141983032226562, 5.88079833984375),
        (33.33, 23.701034545898438, 7.2638702392578125),
        (40.17, 28.451522827148438, 8.64605712890625),
    ];

    let mut c = Compat::new(CoreTextRegistry::new());
    let content = |size: f64| TextContent { font: Some("system-ui".into()), weight: Some(400.0), size: Some(size), ..Default::default() };
    let mut checks = 0;
    let mut wrong = Vec::new();
    for &(size, h, i) in FRESH {
        for (text, want) in [("H", h), ("i", i)] {
            let got = c.measure(&content(size), text);
            if got != want {
                wrong.push(format!("{text} at {size} px: {got}, Chromium {want}"));
            }
            checks += 1;
        }
    }
    assert!(wrong.is_empty(), "widths differ from Chromium:\n  {}", wrong.join("\n  "));
    println!("ok {checks} fresh-renderer widths equal Chromium's");

    // The corpus's real project: off the 1/100 px grid, HarfBuzz tracks at
    // the specified size while the font is the effective size's.
    let high = c.measure(&content(11.867549668874172), "H");
    assert_eq!(high, 8.8115234375, "H at 11.867549668874172 px");
    assert_eq!(c.measure(&content(11.86), "H"), 8.812057495117188, "H at 11.86 px");
    println!("ok specified size for trak, effective size for the font");

    // A fresh renderer measures 24 px system-ui at opsz 24 ...
    assert_eq!(c.measure(&content(24.0), "1"), 10.8046875);
    assert_eq!(c.measure(&content(26.0), "T"), 15.4501953125);
    // ... and a renderer whose UI laid 12 and 13 px system-ui text out at a
    // device scale factor of 2 at opsz 12 and 13 (the capture).
    c.set_font_cache_seeds(vec![
        FontCacheSeed { family: "system-ui".into(), weight: 400.0, italic: false, size: 24.0, specified: 12.0 },
        FontCacheSeed { family: "System-UI".into(), weight: 400.0, italic: false, size: 26.0, specified: 13.0 },
    ]);
    assert_eq!(c.measure(&content(24.0), "1"), 11.203125);
    assert_eq!(c.measure(&content(24.0), "0"), 15.1875);
    assert_eq!(c.measure(&content(26.0), "T"), 16.6943359375);
    // Other keys keep their own entries: another size, weight, or style.
    assert_eq!(c.measure(&content(25.0), "f"), 7.928466796875);
    let bold = TextContent { weight: Some(600.0), ..content(24.0) };
    let italic = TextContent { italic: Some(true), ..content(24.0) };
    let mut fresh = Compat::new(CoreTextRegistry::new());
    assert_eq!(c.measure(&bold, "1"), fresh.measure(&bold, "1"));
    assert_eq!(c.measure(&italic, "1"), fresh.measure(&italic, "1"));
    println!("ok font cache seeds");
}

#[cfg(not(all(target_os = "macos", feature = "system")))]
fn main() {
    println!("the Blink size test needs the macOS CoreText registry; skipped");
}
