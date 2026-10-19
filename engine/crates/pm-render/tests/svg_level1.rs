//! Level 1 SVG images (decision S1; docs/engine/vector.md, "Level 1 SVG
//! images"): today's `<img>` raster at today's bucket, drawn by Chromium in
//! the document's renderer and composited natively.
//!
//! The rasters here are synthetic (the renderer's role is played by the
//! test); Chromium's own rasters against the legacy frames are compared in
//! `corpus.rs` (`PM_SVG_RASTERS`) and in the app (`e2e/engine-svg.spec.ts`).

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_protocol::svg_raster::SvgRasterKey;
use pm_render::vector::svg_rasters::SvgRasterizer;
use pm_render::{MapResolver, RenderError, RenderRequest, Renderer, unsupported_with_media};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::doc::*;
use support::ticks;

const W: u32 = 128;
const H: u32 = 96;
const ART: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#4C8DFF"/></svg>"##;

fn dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pm-render-svg1-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// One level 1 SVG image layer (`fit`, box `bw` × `bh`, scale percent)
/// over the SVG file `name`, plus `extra` layers.
fn svg_doc(
    name: &str,
    svg: &[u8],
    fit: &str,
    bw: f64,
    bh: f64,
    scale: f64,
    extra: Vec<Value>,
) -> (Value, MapResolver) {
    let path = dir().join(name);
    std::fs::write(&path, svg).unwrap();
    let mut l = layer(
        "svg",
        "image",
        json!({ "asset": "art", "fit": fit, "w": bw, "h": bh }),
        64.0,
        48.0,
        2.0,
    );
    set(&mut l, "scale.x", json!(scale));
    set(&mut l, "scale.y", json!(scale));
    let mut layers = vec![l];
    layers.extend(extra);
    let mut doc = project(W, H, 2.0, no_fill(), layers);
    asset(&mut doc, "art", "image", "svg", 40, 20, 0.0);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("art".into(), path);
    (doc, resolver)
}

fn request(export: bool) -> RenderRequest {
    let mut r = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    r.export = export;
    r
}

/// A premultiplied RGBA8 raster: a horizontal ramp with alpha `a`.
fn ramp(w: u32, h: u32, a: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for _y in 0..h {
        for x in 0..w {
            let v = ((u32::from(a) * x) / w.max(1)) as u8;
            out.extend_from_slice(&[v, v / 2, a / 3, a]);
        }
    }
    out
}

fn rgba(
    renderer: &mut Renderer,
    doc: &pm_model::Document,
    resolver: &MapResolver,
    export: bool,
) -> Result<(Vec<f32>, usize), RenderError> {
    let frame = renderer.render(doc, "Ptest-root", ticks(0.0), &request(export), resolver)?;
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    Ok((renderer.read_rgba(&frame).unwrap(), frame.svg_misses.len()))
}

#[test]
fn a_frame_names_todays_bucket_and_the_svg_bytes() {
    let mut renderer = Renderer::headless().unwrap();
    // A 60 × 30 contain box at 250%: `svgRasterDimensions(40, 20, 60, 30,
    // world)` is 40 × 20 at ceil4(2.5 × 1.5) = 3.75 → 150 × 75.
    let (doc, resolver) = svg_doc(
        "bucket.svg",
        ART.as_bytes(),
        "contain",
        60.0,
        30.0,
        250.0,
        vec![],
    );
    let doc = load(&doc);
    assert_eq!(
        unsupported_with_media(&doc, "Ptest-root", &resolver),
        vec![]
    );
    let frame = renderer
        .render(&doc, "Ptest-root", ticks(0.0), &request(false), &resolver)
        .unwrap();
    assert!(frame.failures.is_empty());
    assert_eq!(frame.svg_misses.len(), 1);
    let need = &frame.svg_misses[0];
    assert_eq!(
        need.request.key,
        SvgRasterKey {
            content: sha(ART.as_bytes()),
            width: 150,
            height: 75
        }
    );
    assert_eq!(need.request.asset, "art");
    assert_eq!(need.path, resolver.paths["art"]);
    // Nothing drew: no substitute exists yet.
    assert!(
        renderer
            .read_rgba(&frame)
            .unwrap()
            .iter()
            .all(|&v| v == 0.0)
    );
    // Exports never draw without the raster.
    match renderer.render(&doc, "Ptest-root", ticks(0.0), &request(true), &resolver) {
        Err(RenderError::SvgRastersPending(needs)) => {
            assert_eq!(needs[0].request.key, need.request.key)
        }
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("an export drew a missing SVG raster"),
    }
    // Snapshots wait too.
    renderer.set_svg_wait(true);
    assert!(matches!(
        renderer.render(&doc, "Ptest-root", ticks(0.0), &request(false), &resolver),
        Err(RenderError::SvgRastersPending(_))
    ));
}

#[test]
fn the_raster_is_placed_as_todays_image_quad() {
    // An opaque raster drawn as an SVG covers exactly what the same pixels
    // as a PNG image layer cover: the box, the fit UVs, and the quad are
    // today's image quad's. The texture is RGBA8, as today's canvas upload
    // is (and as level 1 shape rasters are, exact on the corpus); the PNG
    // path keeps decoded stills at float precision, so bilinear samples
    // between texels differ from it by at most one code value.
    let mut renderer = Renderer::headless().unwrap();
    let (doc, resolver) = svg_doc(
        "same.svg",
        ART.as_bytes(),
        "cover",
        50.0,
        40.0,
        130.0,
        vec![],
    );
    let doc = load(&doc);
    let miss = renderer
        .render(&doc, "Ptest-root", ticks(0.0), &request(false), &resolver)
        .unwrap();
    let key = miss.svg_misses[0].request.key.clone();
    let pixels = ramp(key.width, key.height, 255);
    renderer
        .put_svg_raster(key.clone(), pixels.clone())
        .unwrap();
    let (svg, misses) = rgba(&mut renderer, &doc, &resolver, true).unwrap();
    assert_eq!(misses, 0);
    let png = dir().join("same.png");
    support::write_png(&png, key.width, key.height, &pixels);
    let mut image = svg_doc(
        "unused.svg",
        ART.as_bytes(),
        "cover",
        50.0,
        40.0,
        130.0,
        vec![],
    )
    .0;
    image["assets"]["art"]["format"] = json!("png");
    image["assets"]["art"]["w"] = json!(key.width);
    image["assets"]["art"]["h"] = json!(key.height);
    let mut png_resolver = MapResolver::default();
    png_resolver.paths.insert("art".into(), png);
    let (reference, _) = rgba(&mut renderer, &load(&image), &png_resolver, true).unwrap();
    let to8 = |v: &[f32]| v.iter().map(|&x| support::unorm8(x)).collect::<Vec<_>>();
    let (a, b) = (to8(&svg), to8(&reference));
    assert!(a.iter().any(|&v| v > 0), "the SVG drew");
    let alpha_differs = a
        .iter()
        .zip(&b)
        .skip(3)
        .step_by(4)
        .filter(|(x, y)| x != y)
        .count();
    let max = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
    assert_eq!(
        alpha_differs, 0,
        "coverage differs from the PNG image layer"
    );
    assert!(max <= 1, "colour differs by {max}");
}

#[test]
fn preview_substitutes_the_latest_bucket_for_one_frame() {
    let mut renderer = Renderer::headless().unwrap();
    let (doc, resolver) = svg_doc(
        "zoom.svg",
        ART.as_bytes(),
        "stretch",
        40.0,
        20.0,
        100.0,
        vec![],
    );
    let small = load(&doc);
    let first = renderer
        .render(&small, "Ptest-root", ticks(0.0), &request(false), &resolver)
        .unwrap();
    let a = first.svg_misses[0].request.key.clone();
    assert_eq!((a.width, a.height), (40, 20));
    renderer
        .put_svg_raster(a.clone(), ramp(40, 20, 200))
        .unwrap();
    let (drawn, misses) = rgba(&mut renderer, &small, &resolver, false).unwrap();
    assert_eq!(misses, 0);
    assert!(drawn.iter().any(|&v| v > 0.0));
    // Zoomed to 300%: a new bucket; the 40 × 20 one stands in meanwhile.
    let (zoomed, _) = svg_doc(
        "zoom.svg",
        ART.as_bytes(),
        "stretch",
        40.0,
        20.0,
        300.0,
        vec![],
    );
    let zoomed = load(&zoomed);
    let frame = renderer
        .render(
            &zoomed,
            "Ptest-root",
            ticks(0.0),
            &request(false),
            &resolver,
        )
        .unwrap();
    let b = frame.svg_misses[0].request.key.clone();
    assert_eq!((b.width, b.height), (120, 60));
    assert_eq!(b.content, a.content);
    let stand_in = renderer.read_rgba(&frame).unwrap();
    assert!(
        stand_in.iter().any(|&v| v > 0.0),
        "the latest bucket stands in"
    );
    // The asked bucket arrives: the next frame draws it and asks nothing.
    renderer
        .put_svg_raster(b.clone(), ramp(120, 60, 200))
        .unwrap();
    let (exact, misses) = rgba(&mut renderer, &zoomed, &resolver, false).unwrap();
    assert_eq!(misses, 0);
    assert_ne!(exact, stand_in);
    // An export never draws a stand-in: a third bucket fails pending.
    let (huge, _) = svg_doc(
        "zoom.svg",
        ART.as_bytes(),
        "stretch",
        40.0,
        20.0,
        700.0,
        vec![],
    );
    assert!(matches!(
        renderer.render(
            &load(&huge),
            "Ptest-root",
            ticks(0.0),
            &request(true),
            &resolver
        ),
        Err(RenderError::SvgRastersPending(_))
    ));
}

#[test]
fn rasters_are_uploaded_once_per_content_and_bucket_across_layers() {
    let mut renderer = Renderer::headless().unwrap();
    // Two layers of the same SVG at the same bucket, a third at another.
    let twin = |id: &str, scale: f64, x: f64| {
        let mut l = layer(
            id,
            "image",
            json!({ "asset": "art", "fit": "stretch", "w": 40, "h": 20 }),
            x,
            48.0,
            2.0,
        );
        set(&mut l, "scale.x", json!(scale));
        set(&mut l, "scale.y", json!(scale));
        l
    };
    let (doc, resolver) = svg_doc(
        "shared.svg",
        ART.as_bytes(),
        "stretch",
        40.0,
        20.0,
        100.0,
        vec![twin("twin", 100.0, 20.0), twin("big", 200.0, 100.0)],
    );
    let doc = load(&doc);
    let frame = renderer
        .render(&doc, "Ptest-root", ticks(0.0), &request(false), &resolver)
        .unwrap();
    let mut keys: Vec<_> = frame
        .svg_misses
        .iter()
        .map(|m| (m.request.key.width, m.request.key.height))
        .collect();
    keys.sort();
    assert_eq!(keys, vec![(40, 20), (80, 40)], "one request per bucket");
    for m in &frame.svg_misses {
        let k = m.request.key.clone();
        renderer
            .put_svg_raster(k.clone(), ramp(k.width, k.height, 255))
            .unwrap();
    }
    let (_, misses) = rgba(&mut renderer, &doc, &resolver, true).unwrap();
    assert_eq!(misses, 0);
    // A replaced file is another content: it is asked for again, and the
    // old content never stands in for it.
    let other = ART.replace("#4C8DFF", "#FF0000");
    std::fs::write(&resolver.paths["art"], &other).unwrap();
    let frame = renderer
        .render(&doc, "Ptest-root", ticks(0.0), &request(false), &resolver)
        .unwrap();
    assert!(
        frame
            .svg_misses
            .iter()
            .all(|m| m.request.key.content == sha(other.as_bytes()))
    );
    assert_eq!(frame.svg_misses.len(), 2);
    assert!(
        renderer
            .read_rgba(&frame)
            .unwrap()
            .iter()
            .all(|&v| v == 0.0)
    );
    // Forgetting a content drops its rasters (an engine restart starts
    // empty the same way).
    assert!(renderer.has_svg_raster(&SvgRasterKey {
        content: sha(ART.as_bytes()),
        width: 40,
        height: 20
    }));
    renderer.forget_svg_rasters(Some(&sha(ART.as_bytes())));
    assert!(!renderer.has_svg_raster(&SvgRasterKey {
        content: sha(ART.as_bytes()),
        width: 40,
        height: 20
    }));
    assert!(
        renderer
            .put_svg_raster(
                SvgRasterKey {
                    content: sha(b"x"),
                    width: 2,
                    height: 2
                },
                vec![0; 15]
            )
            .is_err()
    );
}

#[test]
fn broken_and_offline_documents_ask_for_nothing() {
    let mut renderer = Renderer::headless().unwrap();
    // An undeclared entity without a DTD: Chromium's broken `<img>`.
    let broken = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><text>&nbsp;</text></svg>"#;
    let (doc, resolver) = svg_doc("broken.svg", broken, "stretch", 40.0, 20.0, 100.0, vec![]);
    let (px, misses) = rgba(&mut renderer, &load(&doc), &resolver, true).unwrap();
    assert_eq!(misses, 0);
    assert!(px.iter().all(|&v| v == 0.0));
    // Offline media is a named layer failure, as for every image.
    let (doc, _) = svg_doc(
        "offline.svg",
        ART.as_bytes(),
        "stretch",
        40.0,
        20.0,
        100.0,
        vec![],
    );
    let frame = renderer
        .render(
            &load(&doc),
            "Ptest-root",
            ticks(0.0),
            &request(true),
            &MapResolver::default(),
        )
        .unwrap();
    assert_eq!(frame.failures.len(), 1);
    assert!(
        frame.failures[0].message.contains("offline"),
        "{:?}",
        frame.failures
    );
}

#[test]
fn pm_cli_draws_an_approximation_with_resvg_and_counts_it() {
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_svg_rasterizer(SvgRasterizer::Resvg);
    let (doc, resolver) = svg_doc(
        "cli.svg",
        ART.as_bytes(),
        "stretch",
        40.0,
        20.0,
        100.0,
        vec![],
    );
    let (px, misses) = rgba(&mut renderer, &load(&doc), &resolver, true).unwrap();
    assert_eq!(misses, 0);
    assert_eq!(renderer.svg_approximated(), 1);
    // The box's center is the SVG's blue.
    let at = ((48 * W + 64) * 4) as usize;
    let to8 = |v: f32| support::unorm8(v);
    assert_eq!(
        [
            to8(px[at]),
            to8(px[at + 1]),
            to8(px[at + 2]),
            to8(px[at + 3])
        ],
        [0x4C, 0x8D, 0xFF, 255]
    );
}
