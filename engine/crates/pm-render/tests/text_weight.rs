//! TEXT-SMOOTH-1: text draws with today's macOS weight by default and
//! exactly on request.
//!
//! Today's app (and every video exported from it) draws text through
//! Canvas2D on macOS, heavier than the outlines. `tools/visual-parity/
//! text-weight-probe.mjs` measured it: a Helvetica "l" stem's coverage width
//! in today's export path, Regular and Bold, white on black and black on
//! white, 12 to 96 px. This gate draws the same cases with the default
//! weight: within 0.01 px of today on average and 0.02 px at worst. This
//! rejects the old outline smoothing fit (0.096 px mean) and guards the
//! Canvas mask path. The exact weight stays paint-independent.
//!
//! `PM_TEXT_WEIGHT_PROFILE=<file>` writes each case's exact middle-row
//! coverage, the data the fit was made from.
//!
//! Runs without the harness (the CoreText registry needs the main thread).

mod support;

use pm_text::render::smoothing::CANVAS2D_STEMS as TODAY;

/// The middle row's coverage of a centred "l" on a 256 px frame, as today's
/// probe measures it: `(L − background) / (paint − background)`.
#[cfg(target_os = "macos")]
fn row(renderer: &mut pm_render::Renderer, weight: u32, white: bool, size: f64) -> Vec<f64> {
    use pm_protocol::{color::Precision, common::Size};
    use pm_render::{MapResolver, RenderRequest};
    use support::doc::{layer, project, solid_fill};
    const W: usize = 256;
    let (paint, background) = if white {
        ("#FFFFFF", "#000000")
    } else {
        ("#000000", "#FFFFFF")
    };
    let d = serde_json::json!({
        "text": "l", "font": "Helvetica", "weight": weight, "size": size,
        "color": paint, "align": "center"
    });
    let v = project(
        W as u32,
        W as u32,
        1.0,
        solid_fill(background),
        vec![layer("l", "text", d, 128.0, 128.0, 1.0)],
    );
    let doc = pm_model::Document::new(Some(pm_model::json::parse(&v.to_string()).unwrap()), 1);
    let frame = renderer
        .render(
            &doc,
            "Ptest-root",
            support::ticks(0.0),
            &RenderRequest::sized(Size::new(W as u32, W as u32), Precision::Float32),
            &MapResolver::default(),
        )
        .expect("a text frame");
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let px = renderer.read_rgba(&frame).unwrap();
    let luma = |x: usize, y: usize| {
        let i = (y * W + x) * 4;
        0.2126 * f64::from(px[i]) + 0.7152 * f64::from(px[i + 1]) + 0.0722 * f64::from(px[i + 2])
    };
    let cov = |x: usize, y: usize| {
        let l = luma(x, y);
        (if white { l } else { 1.0 - l }).clamp(0.0, 1.0)
    };
    let rows: Vec<usize> = (0..W)
        .filter(|&y| (0..W).any(|x| cov(x, y) > 0.5))
        .collect();
    let y = rows[rows.len() / 2];
    (0..W).map(|x| cov(x, y)).collect()
}

#[cfg(target_os = "macos")]
fn main() {
    use pm_render::Renderer;
    use pm_render::text::TextWeight;
    let fonts = || {
        Box::new(support::text::MainThreadFonts(
            pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new()),
            None,
        ))
    };
    let mut renderer = Renderer::headless().expect("a Metal device");
    renderer.set_text_fonts(Some(fonts()));
    assert_eq!(
        renderer.text().weight(),
        TextWeight::Smoothed,
        "the default"
    );

    if let Some(file) = std::env::var_os("PM_TEXT_WEIGHT_PROFILE") {
        renderer.text().set_weight(TextWeight::Exact);
        let cases: Vec<_> = TODAY
            .iter()
            .map(|&(weight, white, size, today)| {
                serde_json::json!({"weight": weight, "white": white, "size": size,
                    "today": today, "exact": row(&mut renderer, weight, white, size)})
            })
            .collect();
        std::fs::write(file, serde_json::to_string(&cases).unwrap()).unwrap();
        renderer.text().set_weight(TextWeight::Smoothed);
    }

    let mut failures = Vec::new();
    let (mut sum_exact, mut sum_smoothed) = (0.0, 0.0);
    for &(weight, white, size, today) in TODAY {
        let smoothed: f64 = row(&mut renderer, weight, white, size).iter().sum();
        renderer.text().set_weight(TextWeight::Exact);
        let exact_row = row(&mut renderer, weight, white, size);
        let other: f64 = row(&mut renderer, weight, !white, size).iter().sum();
        renderer.text().set_weight(TextWeight::Smoothed);
        let exact: f64 = exact_row.iter().sum();
        println!(
            "{weight} {} {size:>3} px: today {today:.3}, smoothed {smoothed:.3}, exact {exact:.3}",
            if white { "white" } else { "black" }
        );
        sum_exact += (exact - today).abs();
        sum_smoothed += (smoothed - today).abs();
        if (exact - other).abs() > 1e-3 {
            failures.push(format!(
                "exact weight depends on paint at {size} px: {exact} {other}"
            ));
        }
        // Independent Canvas captures, allowing a few RGBA8 code values
        // across the stem. The old fitted coverage is not accurate enough.
        if (smoothed - today).abs() > 0.02 {
            failures.push(format!(
                "{weight} {} {size} px: smoothed {smoothed:.3} against today's {today:.3}",
                if white { "white" } else { "black" }
            ));
        }
    }
    let n = TODAY.len() as f64;
    println!(
        "mean |width − today|: exact {:.3} px, smoothed {:.3} px",
        sum_exact / n,
        sum_smoothed / n
    );
    if sum_smoothed >= sum_exact {
        failures.push("the default weight is not closer to today than the exact one".into());
    }
    if sum_smoothed / n > 0.01 {
        failures.push(format!(
            "mean |width − today| {:.3} px exceeds 0.01",
            sum_smoothed / n
        ));
    }
    for f in &failures {
        println!("FAILED {f}");
    }
    assert!(failures.is_empty());
}

#[cfg(not(target_os = "macos"))]
fn main() {}
