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
            texture: None,
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
fn a_snapshot_on_the_viewers_renderer_gives_the_viewer_its_policy_back() {
    // A snapshot borrows the viewer's renderer: whichever way it leaves
    // (an early return, an unwind), the viewer's frames substitute again.
    let mut renderer = Renderer::headless().unwrap();
    let (doc, resolver) = svg_doc(
        "a.svg",
        ART.as_bytes(),
        "contain",
        60.0,
        40.0,
        100.0,
        vec![],
    );
    let doc = load(&doc);
    let snapshot = |renderer: &mut Renderer, bail: bool| -> Result<(), String> {
        let mut waiting = renderer.svg_waiting();
        assert!(waiting.svg_wait());
        let pending = matches!(
            waiting.render(&doc, "Ptest-root", ticks(0.0), &request(false), &resolver),
            Err(RenderError::SvgRastersPending(_))
        );
        assert!(pending, "a snapshot waits for the raster");
        if bail {
            return Err("fonts did not answer".into());
        }
        Ok(())
    };
    assert!(snapshot(&mut renderer, true).is_err());
    assert!(!renderer.svg_wait(), "an early return restores the policy");
    assert!(snapshot(&mut renderer, false).is_ok());
    assert!(!renderer.svg_wait());
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _waiting = renderer.svg_waiting();
        panic!("a snapshot that unwinds");
    }));
    assert!(unwound.is_err());
    assert!(!renderer.svg_wait(), "an unwind restores the policy");
    // The viewer's next frame records the miss (and asks for the raster)
    // instead of failing.
    let frame = renderer
        .render(&doc, "Ptest-root", ticks(0.0), &request(false), &resolver)
        .expect("the viewer's frame draws");
    assert_eq!(frame.svg_misses.len(), 1);
    // A caller that set the policy itself keeps it.
    renderer.set_svg_wait(true);
    drop(renderer.svg_waiting());
    assert!(renderer.svg_wait());
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
        texture: None,
        content: sha(ART.as_bytes()),
        width: 40,
        height: 20
    }));
    renderer.forget_svg_rasters(Some(&sha(ART.as_bytes())));
    assert!(!renderer.has_svg_raster(&SvgRasterKey {
        texture: None,
        content: sha(ART.as_bytes()),
        width: 40,
        height: 20
    }));
    assert!(
        renderer
            .put_svg_raster(
                SvgRasterKey {
                    texture: None,
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

/// The job engine's source: an export frame fetches its missing rasters
/// before it draws, and a source that cannot names why.
#[test]
fn exports_fetch_missing_rasters_from_the_source_before_drawing() {
    use pm_render::vector::svg_rasters::{SvgRasterNeed, SvgRasterSource};
    use std::sync::{Arc, Mutex};
    struct Source(Mutex<Vec<SvgRasterKey>>, bool);
    impl SvgRasterSource for Source {
        fn fetch(&self, need: &SvgRasterNeed) -> Result<Vec<u8>, String> {
            assert_eq!(
                (need.composition.as_str(), need.layer.as_str()),
                ("Ptest-root", "svg")
            );
            self.0.lock().unwrap().push(need.request.key.clone());
            if self.1 {
                Ok(ramp(need.request.key.width, need.request.key.height, 255))
            } else {
                Err("No editor window has this project open to draw its SVG images.".into())
            }
        }
    }
    let mut renderer = Renderer::headless().unwrap();
    let (doc, resolver) = svg_doc(
        "source.svg",
        ART.as_bytes(),
        "contain",
        60.0,
        30.0,
        100.0,
        vec![],
    );
    let doc = load(&doc);
    let source = Arc::new(Source(Mutex::new(Vec::new()), true));
    renderer.set_svg_source(Some(source.clone()));
    let (px, misses) = rgba(&mut renderer, &doc, &resolver, true).unwrap();
    assert_eq!(misses, 0);
    assert!(px.iter().any(|&v| v > 0.0));
    // Once per bucket: the second frame uses what the first fetched.
    rgba(&mut renderer, &doc, &resolver, true).unwrap();
    assert_eq!(source.0.lock().unwrap().len(), 1);
    // Preview frames never block on the source.
    let mut preview = Renderer::headless().unwrap();
    preview.set_svg_source(Some(source.clone()));
    let (_, misses) = rgba(&mut preview, &doc, &resolver, false).unwrap();
    assert_eq!(misses, 1);
    assert_eq!(source.0.lock().unwrap().len(), 1);
    // No window: the export fails with the source's words.
    let mut lonely = Renderer::headless().unwrap();
    lonely.set_svg_source(Some(Arc::new(Source(Mutex::new(Vec::new()), false))));
    match lonely.render(&doc, "Ptest-root", ticks(0.0), &request(true), &resolver) {
        Err(RenderError::SvgRaster(m)) => assert!(m.contains("No editor window"), "{m}"),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("drawn without the raster"),
    }
}

#[test]
fn svg_material_maps_use_chromiums_map_upload_without_a_scene3d_refusal() {
    let path = dir().join("material-map.svg");
    std::fs::write(&path, ART).unwrap();
    let mut cube = object3d("cube", "box", json!({}), "#FFFFFF", W, H, 2.0);
    cube["d"]["data"]["object"]["material"]["maps"] = json!({"color":"art", "normal":"art"});
    let mut value = project(
        W,
        H,
        2.0,
        no_fill(),
        vec![camera("camera", [0.0, 0.0, 6.0], false, 2.0), cube],
    );
    asset(&mut value, "art", "image", "svg", 40, 20, 0.0);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("art".into(), path);
    let doc = load(&value);
    assert!(unsupported_with_media(&doc, "Ptest-root", &resolver).is_empty());
    let mut renderer = Renderer::headless().unwrap();
    let needs = match renderer.render(&doc, "Ptest-root", ticks(0.0), &request(true), &resolver) {
        Err(RenderError::SvgRastersPending(needs)) => needs,
        _ => panic!("the export must wait for Chromium's SVG map, never draw a substitute"),
    };
    assert_eq!(
        needs.len(),
        1,
        "color and data maps share identical upload bytes"
    );
    assert!(
        matches!(
            renderer.render(&doc, "Ptest-root", ticks(0.0), &request(false), &resolver),
            Err(RenderError::SvgRastersPending(_))
        ),
        "preview must keep the previous frame, not show an absent 3D object"
    );
    let key = &needs[0].request.key;
    assert_eq!((key.width, key.height, key.texture), (40, 20, Some(true)));
    let mut bucket_key = key.clone();
    bucket_key.texture = None;
    renderer
        .put_svg_raster(bucket_key, vec![0; 40 * 20 * 4])
        .unwrap();
    assert!(
        !renderer.has_svg_raster(key),
        "a canvas bucket is not a material map"
    );
    renderer
        .put_svg_raster(key.clone(), vec![255; 40 * 20 * 4])
        .unwrap();
    let frame = renderer
        .render(&doc, "Ptest-root", ticks(0.0), &request(true), &resolver)
        .unwrap();
    assert!(frame.failures.is_empty());
    assert!(frame.svg_misses.is_empty());
    let pixels = renderer.read_rgba(&frame).unwrap();
    assert!(pixels.chunks_exact(4).any(|p| p[3] > 0.9));
}

#[test]
fn a_viewer_frame_draws_without_a_bucket_the_window_could_not_draw() {
    // The viewer waits for the exact bucket (no stand-in), but a bucket
    // the window could not draw (or did not answer for) must not hold the
    // frame: it is drawn without that image, as today's compositor draws
    // the frame without a layer whose `rasterSvgAsset` throws, and named so
    // the host asks for it again later. Snapshots and exports still wait.
    let mut renderer = Renderer::headless().unwrap();
    let (small, resolver) = svg_doc(
        "fail.svg",
        ART.as_bytes(),
        "stretch",
        40.0,
        20.0,
        100.0,
        vec![],
    );
    let small = load(&small);
    let (zoomed, _) = svg_doc(
        "fail.svg",
        ART.as_bytes(),
        "stretch",
        40.0,
        20.0,
        300.0,
        vec![],
    );
    let zoomed = load(&zoomed);
    let viewer = |renderer: &mut Renderer, doc: &pm_model::Document| {
        let mut waiting = renderer.svg_waiting_viewer();
        let frame = waiting.render(doc, "Ptest-root", ticks(0.0), &request(false), &resolver)?;
        let pixels = waiting.read_rgba(&frame).unwrap();
        Ok::<_, RenderError>((pixels, frame.svg_misses))
    };
    // The 100% bucket is drawn, so a stand-in would exist for 300%.
    let a = match viewer(&mut renderer, &small) {
        Err(RenderError::SvgRastersPending(needs)) => needs[0].request.key.clone(),
        other => panic!("the viewer waits for the bucket: {:?}", other.map(|f| f.1)),
    };
    renderer.put_svg_raster(a, ramp(40, 20, 200)).unwrap();
    let (drawn, misses) = viewer(&mut renderer, &small).unwrap();
    assert!(misses.is_empty() && drawn.iter().any(|&v| v > 0.0));
    let b = match viewer(&mut renderer, &zoomed) {
        Err(RenderError::SvgRastersPending(needs)) => needs[0].request.key.clone(),
        other => panic!("no stand-in for the 300% bucket: {:?}", other.map(|f| f.1)),
    };
    assert!(!renderer.svg_raster_failed(&b));
    renderer.fail_svg_raster(b.clone());
    assert!(renderer.svg_raster_failed(&b));
    let (without, misses) = viewer(&mut renderer, &zoomed).expect("the frame draws");
    assert_eq!(misses.len(), 1, "named, so the host asks again");
    assert_eq!(misses[0].request.key, b);
    assert!(
        without.iter().all(|&v| v == 0.0),
        "drawn without the image, never the 100% bucket"
    );
    // The policy is the viewer's only: a snapshot and an export wait.
    {
        let mut waiting = renderer.svg_waiting();
        assert!(matches!(
            waiting.render(
                &zoomed,
                "Ptest-root",
                ticks(0.0),
                &request(false),
                &resolver
            ),
            Err(RenderError::SvgRastersPending(_))
        ));
    }
    {
        let mut waiting = renderer.svg_waiting_viewer();
        assert!(matches!(
            waiting.render(&zoomed, "Ptest-root", ticks(0.0), &request(true), &resolver),
            Err(RenderError::SvgRastersPending(_))
        ));
    }
    assert!(!renderer.svg_wait(), "the guards restore the policy");
    // Its raster arrives: the failure is over and the bucket is drawn.
    renderer
        .put_svg_raster(b.clone(), ramp(120, 60, 200))
        .unwrap();
    assert!(!renderer.svg_raster_failed(&b));
    let (exact, misses) = viewer(&mut renderer, &zoomed).unwrap();
    assert!(misses.is_empty() && exact.iter().any(|&v| v > 0.0));
    // A forgotten content forgets its failures.
    renderer.fail_svg_raster(b.clone());
    renderer.forget_svg_rasters(Some(&b.content));
    assert!(!renderer.svg_raster_failed(&b));
}

/// The SVG buckets of a plan, in draw order (precomps and placed content
/// included).
fn planned_buckets(plan: &pm_render::plan::CompPlan, out: &mut Vec<(u32, u32)>) {
    fn content(c: &pm_render::plan::Content, out: &mut Vec<(u32, u32)>) {
        match c {
            pm_render::plan::Content::Texture { source, .. } => match source {
                pm_render::plan::Source::Vector(v) => {
                    if let pm_render::vector::VectorSource::SvgImage { key, .. } = &**v {
                        out.push((key.width, key.height));
                    }
                }
                pm_render::plan::Source::Comp(p) | pm_render::plan::Source::Accumulate(p) => {
                    planned_buckets(p, out)
                }
                _ => {}
            },
            pm_render::plan::Content::Placed(p) => content(&p.content, out),
            other => panic!("unexpected content in the bucket grid: {other:?}"),
        }
    }
    for draw in &plan.draws {
        content(&draw.content, out);
    }
}

/// Reconstruct a captured SVG view's canvas or full-composition coordinates.
/// Canvas captures retain their recorded output scale and unrounded crop origin;
/// full-composition captures use their recorded zoom, device ratio and resolution.
fn viewer_request(cw: u32, ch: u32, row: &Value, todays_canvas: bool) -> RenderRequest {
    let f = |k: &str| row[k].as_f64().unwrap();
    if todays_canvas {
        let a = row["outputScale"].as_array().unwrap();
        let (sx, sy) = (a[0].as_f64().unwrap(), a[1].as_f64().unwrap());
        let c = row["canvas"].as_array().unwrap();
        let canvas = Size::new(c[0].as_u64().unwrap() as u32, c[1].as_u64().unwrap() as u32);
        let origin = row["crop"]
            .as_array()
            .map(|r| (r[0].as_f64().unwrap() * sx, r[1].as_f64().unwrap() * sy));
        return RenderRequest {
            scale: Some((sx, sy)),
            origin,
            ..RenderRequest::sized(canvas, Precision::Float16)
        };
    }
    let res = if row["resolution"] == "auto" {
        1.0
    } else {
        0.5
    };
    let s = f("zoom") * f("dpr").min(2.0) * res;
    let px = |v: u32| ((f64::from(v) * s).round()).clamp(1.0, 16384.0) as u32;
    RenderRequest::sized(Size::new(px(cw), px(ch)), Precision::Float16)
}

/// Today's viewer buckets over zoom 0.1 to 8 × DPR 1, 2, 3 × layer scale
/// (also Auto and Half, crop safe and unsafe projects, and an SVG inside a
/// precomp), computed with today's functions (`svgRasterDimensions`,
/// `previewRenderSize`, `previewRenderViewport`, `outputScale`;
/// apps/desktop/src/extensions/viewer/svg-buckets.test.ts keeps the
/// fixture equal to them): the engine's viewer frames plan exactly those
/// buckets when they render at today's canvas (`ViewerState::canvas`).
#[test]
fn viewer_frames_pick_todays_buckets_at_todays_viewer_scale() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/svg-viewer-buckets.json")).expect("fixture");
    let n = |v: &Value, k: &str| v[k].as_u64().unwrap() as u32;
    let (cw, ch) = (n(&fixture["comp"], "w"), n(&fixture["comp"], "h"));
    let (sub_w, sub_h) = (n(&fixture["sub"], "w"), n(&fixture["sub"], "h"));
    let (sw, sh) = (n(&fixture["source"], "w"), n(&fixture["source"], "h"));
    let (bw, bh) = (
        fixture["box"]["w"].as_f64().unwrap(),
        fixture["box"]["h"].as_f64().unwrap(),
    );
    let path = dir().join("grid.svg");
    std::fs::write(&path, ART.as_bytes()).unwrap();
    let mut resolver = MapResolver::default();
    resolver.paths.insert("art".into(), path);
    let mut renderer = Renderer::headless().unwrap();
    let rows = fixture["rows"].as_array().unwrap();
    assert!(rows.len() > 800, "{} rows", rows.len());
    let (mut wrong, mut own_differs, mut cropped) = (Vec::new(), 0, 0);
    let (mut far, mut reduced_differs) = (0, 0);
    for row in rows {
        // The far-crop rows bring their own composition.
        let (cw, ch) = row["comp"].as_array().map_or((cw, ch), |c| {
            (c[0].as_u64().unwrap() as u32, c[1].as_u64().unwrap() as u32)
        });
        let scale = row["layerScale"].as_array().unwrap();
        let nested = row["nested"].as_bool().unwrap();
        let mut svg = layer(
            "svg",
            "image",
            json!({ "asset": "art", "fit": "contain", "w": bw, "h": bh }),
            if nested {
                f64::from(sub_w) / 2.0
            } else {
                f64::from(cw) / 2.0
            },
            if nested {
                f64::from(sub_h) / 2.0
            } else {
                f64::from(ch) / 2.0
            },
            2.0,
        );
        set(
            &mut svg,
            "scale.x",
            json!(scale[0].as_f64().unwrap() * 100.0),
        );
        set(
            &mut svg,
            "scale.y",
            json!(scale[1].as_f64().unwrap() * 100.0),
        );
        let mut doc = if nested {
            let pre = layer(
                "pre",
                "precomp",
                json!({ "comp": "inner", "w": cw, "h": ch }),
                0.0,
                0.0,
                2.0,
            );
            let mut doc = project(cw, ch, 2.0, no_fill(), vec![pre]);
            composition(&mut doc, "inner", sub_w, sub_h, 2.0, vec![svg]);
            doc
        } else {
            project(cw, ch, 2.0, no_fill(), vec![svg])
        };
        asset(&mut doc, "art", "image", "svg", sw, sh, 0.0);
        let doc = load(&doc);
        let bucket = |renderer: &mut Renderer, todays_canvas: bool| {
            let (plan, failures) = renderer
                .plan(
                    &doc,
                    "Ptest-root",
                    ticks(0.0),
                    &viewer_request(cw, ch, row, todays_canvas),
                    &resolver,
                )
                .unwrap();
            assert!(failures.is_empty(), "{failures:?}");
            let mut out = Vec::new();
            planned_buckets(&plan, &mut out);
            assert_eq!(out.len(), 1, "{row}");
            out[0]
        };
        let today = row["today"].as_array().unwrap();
        let today = (
            today[0].as_u64().unwrap() as u32,
            today[1].as_u64().unwrap() as u32,
        );
        let engine = bucket(&mut renderer, true);
        if engine != today {
            wrong.push(format!("{row}: engine {engine:?}"));
        }
        // Without today's scale the engine's own (zoom × DPR) picks
        // another bucket in many of these views (the perf3 finding).
        if bucket(&mut renderer, false) != today {
            own_differs += 1;
        }
        cropped += usize::from(row["cropped"].as_bool().unwrap());
        // Before V1b a crop whose far edge passed the largest output was
        // drawn from a canvas reduced by about 16384 / (far edge): its
        // buckets were picked at that lower scale.
        if let Some(crop) = row["crop"].as_array().filter(|_| row["comp"].is_array()) {
            far += 1;
            let a = row["outputScale"].as_array().unwrap();
            let (sx, sy) = (a[0].as_f64().unwrap(), a[1].as_f64().unwrap());
            let edge =
                |o: &Value, w: &Value, s: f64| (o.as_f64().unwrap() + w.as_f64().unwrap()) * s;
            let k = (16384.0 / edge(&crop[0], &crop[2], sx))
                .min(16384.0 / edge(&crop[1], &crop[3], sy))
                .min(1.0);
            if k < 1.0 {
                let reduced = RenderRequest {
                    scale: Some((sx * k, sy * k)),
                    ..viewer_request(cw, ch, row, true)
                };
                let (plan, _) = renderer
                    .plan(&doc, "Ptest-root", ticks(0.0), &reduced, &resolver)
                    .unwrap();
                let mut out = Vec::new();
                planned_buckets(&plan, &mut out);
                if out.first() != Some(&today) {
                    reduced_differs += 1;
                }
            }
        }
    }
    eprintln!(
        "svg viewer buckets: {} views ({cropped} cropped, {far} far from the origin), {} differ from today, {own_differs} would at the engine's own scale, {reduced_differs} far ones at the reduced scale before V1b",
        rows.len(),
        wrong.len()
    );
    assert!(
        wrong.is_empty(),
        "{} of {} views:\n{}",
        wrong.len(),
        rows.len(),
        wrong.join("\n")
    );
    assert!(own_differs > 0, "the grid exercises the cap below fit");
    assert!(cropped > 0, "the grid exercises the cropped viewport");
    assert!(far >= 32, "the grid exercises crops far from the origin");
    assert!(reduced_differs > 0, "the far crops would have been reduced");
}

/// The perf3 review's case (e2e, stage 748 × 452 at DPR 2, zoom 61%): the
/// engine drew at 1.22 device pixels per composition pixel and asked for
/// the 1.25 bucket; today's viewer draws at most one pixel per composition
/// pixel there. Snapshots and exports keep their own scale (`W / comp.w`),
/// which is today's.
#[test]
fn below_fit_the_viewer_bucket_is_capped_as_today_and_exports_keep_their_scale() {
    let mut renderer = Renderer::headless().unwrap();
    // The 40 × 20 SVG stretched over a 40 × 20 box at 100% (128 × 96 project).
    let (doc, resolver) = svg_doc(
        "cap.svg",
        ART.as_bytes(),
        "stretch",
        40.0,
        20.0,
        100.0,
        vec![],
    );
    let doc = load(&doc);
    let key = |renderer: &mut Renderer, request: &RenderRequest| {
        let (plan, _) = renderer
            .plan(&doc, "Ptest-root", ticks(0.0), request, &resolver)
            .unwrap();
        let mut out = Vec::new();
        planned_buckets(&plan, &mut out);
        out[0]
    };
    // The engine's viewer at 61% × DPR 2: 156 × 117 output.
    let viewer = RenderRequest::sized(Size::new(156, 117), Precision::Float16);
    assert_eq!(
        key(&mut renderer, &viewer),
        (50, 25),
        "ceil4(1.22) = 1.25 at the engine's scale"
    );
    // Today's canvas (decision V1): the composition's size.
    let viewer = RenderRequest::sized(Size::new(128, 96), Precision::Float16);
    assert_eq!(
        key(&mut renderer, &viewer),
        (40, 20),
        "today's canvas is the composition's size"
    );
    // An export at 2× (`W = round(p.w × 2 / 2) × 2`): `W / comp.w`.
    let export = RenderRequest {
        export: true,
        ..RenderRequest::sized(Size::new(256, 192), Precision::Float16)
    };
    assert_eq!(key(&mut renderer, &export), (80, 40));
}
