//! Gate G-I1 (docs/engine/gate-policy.md): every still fixture drawn by the
//! engine against an f64 reference conversion of the file's samples
//! (`support::still_reference`: ICC.1, H.273, BT.2100/BT.2390/BT.2408, PNG
//! Third Edition), at least as close as today's app, with outcomes
//! (rejected imports) and geometry (orientation, size) exact.
//!
//! Today is Chromium 152's upload of each fixture through the shipped
//! importer: `fixtures/stills2/chromium.json` holds the SHA-256 of each
//! premultiplied RGBA8 upload and `fixtures/stills2/today/<sha256>.png`
//! the bytes it hashes (`e2e/…` rows are `apps/desktop/e2e/fixtures`).
//! `incomplete-texture` rows are the deep and arithmetic JPEGs whose RGBA8
//! upload fails: today draws an opaque black sampler.
mod support;
use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_reference::compare::{Bounds, Class, compare};
use pm_render::media::{ProbeIssue, probe_image};
use pm_render::{RenderRequest, Renderer};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use support::doc::*;

fn fixture(name: &str) -> PathBuf {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stills2");
    match name.strip_prefix("e2e/") {
        Some(e2e) => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../apps/desktop/e2e/fixtures")
            .join(e2e),
        None => tests.join(name),
    }
}

/// The still drawn alone at its own size, at `level`, premultiplied over
/// transparent, in 32-bit float.
fn render(renderer: &mut Renderer, path: &Path, (w, h): (usize, usize), level: u32) -> Vec<f32> {
    let ext = path.extension().unwrap().to_str().unwrap().to_lowercase();
    let mut value = project(
        w as u32,
        h as u32,
        1.0,
        no_fill(),
        vec![layer(
            "still",
            "image",
            json!({"asset":"clip","w":w,"h":h}),
            w as f64 / 2.0,
            h as f64 / 2.0,
            1.0,
        )],
    );
    value["layers"][0]["compat"] = json!(level);
    asset(&mut value, "clip", "image", &ext, w as u32, h as u32, 1.0);
    let mut resolver = pm_render::MapResolver::default();
    resolver.paths.insert("clip".into(), path.to_path_buf());
    let mut request =
        RenderRequest::at_scale(Size::new(w as u32, h as u32), 1.0, Precision::Float32);
    request.background = false;
    let document = load(&value);
    assert!(
        renderer
            .unsupported_with_media(&document, "Ptest-root", &resolver)
            .is_empty(),
        "{}",
        path.display()
    );
    let frame = renderer
        .render(
            &document,
            "Ptest-root",
            support::ticks(0.0),
            &request,
            &resolver,
        )
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    renderer.read_rgba(&frame).unwrap()
}

fn today(row: &serde_json::Value, (w, h): (usize, usize)) -> Vec<f64> {
    if row["outcome"] == "incomplete-texture" {
        return [0.0, 0.0, 0.0, 1.0].repeat(w * h);
    }
    let sha = row["sha256"].as_str().unwrap();
    let path = fixture(&format!("today/{}.png", &sha[..16]));
    let (tw, th, rgba) = support::read_png(&path);
    assert_eq!((tw as usize, th as usize), (w, h), "today's size");
    assert_eq!(
        format!("{:x}", Sha256::digest(&rgba)),
        sha,
        "today's upload"
    );
    rgba.iter().map(|&v| f64::from(v) / 255.0).collect()
}

#[test]
fn stills_are_at_least_as_close_to_the_reference_as_today() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(fixture("chromium.json")).unwrap()).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let mut ledger: BTreeMap<String, (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<String>)> =
        BTreeMap::new();
    let (mut drawn, mut rejected) = (0, 0);
    let mut failures = Vec::new();
    for row in &rows {
        let name = row["file"].as_str().unwrap();
        let path = fixture(name);
        if row["outcome"] == "rejected" {
            // Today's importer rejects it; it stays unreadable media.
            assert!(
                matches!(probe_image(&path), Err(ProbeIssue::Unreadable(_))),
                "{name}: {:?}",
                probe_image(&path)
            );
            rejected += 1;
            continue;
        }
        assert_eq!(probe_image(&path), Ok(()), "{name}");
        let reference = support::still_reference::reference(&path);
        let size = (reference.width, reference.height);
        let engine: Vec<f64> = render(&mut renderer, &path, size, 1)
            .into_iter()
            .map(|v| f64::from(v).clamp(0.0, 1.0))
            .collect();
        let today = today(row, size);
        let classes = vec![Class::Resampled; size.0 * size.1];
        let report = compare(&engine, &today, &reference.rgba, &classes);
        let problems = report.failures(&Bounds::EIGHT_BIT);
        if !problems.is_empty() {
            failures.push(format!("{name}: {problems:?}{report}"));
        }
        let r = report.class(Class::Resampled);
        if std::env::var_os("PM_TRACE").is_some() {
            eprintln!("{name:<44} {} {report}", reference.reading);
        }
        let entry = ledger.entry(reference.reading.clone()).or_default();
        let collect = |s: &pm_reference::compare::Stats| [s.max, s.p999, s.mean];
        if r.engine_vs_reference.pixels > 0 {
            entry.0.extend(collect(&r.today_vs_reference));
            entry.1.extend(collect(&r.engine_vs_reference));
        } else {
            let u = report.class(Class::Unreferenced);
            entry.2.extend(collect(&u.engine_vs_today));
        }
        entry.3.push(r.engine_vs_reference.max);
        entry.4.push(name.to_string());
        drawn += 1;
    }
    for (reading, (t, e, u, _, names)) in &ledger {
        let worst = |v: &[f64], k: usize| v.iter().skip(k).step_by(3).copied().fold(0.0, f64::max);
        let mean = |v: &[f64]| {
            let m: Vec<f64> = v.iter().skip(2).step_by(3).copied().collect();
            m.iter().sum::<f64>() / m.len().max(1) as f64
        };
        if !t.is_empty() {
            eprintln!(
                "G-I1 {reading} ({} files): today {:.2}, {:.2}, {:.3} | engine {:.3}, {:.3}, {:.4}",
                names.len(),
                worst(t, 0),
                worst(t, 1),
                mean(t),
                worst(e, 0),
                worst(e, 1),
                mean(e)
            );
        } else {
            eprintln!(
                "G-I1 {reading} ({} files): engine vs today {:.2}, {:.2}, {:.3}",
                names.len(),
                worst(u, 0),
                worst(u, 1),
                mean(u)
            );
        }
    }
    eprintln!("G-I1: {drawn} drawn, {rejected} rejected");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(drawn >= 140 && rejected == 2, "{drawn} {rejected}");
}
