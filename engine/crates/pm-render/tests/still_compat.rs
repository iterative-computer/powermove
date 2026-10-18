mod support;
use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{RenderRequest, Renderer};
#[test]
#[ignore = "needs the private render-* corpus capture (docs/engine/media.md)"]
fn compatibility_still_corpus() {
    let out = std::env::var_os("PM_MEDIA_EVIDENCE").map(std::path::PathBuf::from);
    if let Some(p) = &out {
        std::fs::create_dir_all(p).unwrap();
    }
    let capture = support::newest_capture("render-").expect("a render-* corpus capture");
    let mut r = Renderer::headless().unwrap();
    let mut rows = Vec::new();
    for dir in support::projects(&capture) {
        let p = support::load_project(&capture, &dir);
        if !p.name.starts_with("render/still-") {
            continue;
        }
        assert!(p.missing_media.is_empty(), "{:?}", p.missing_media);
        assert!(
            r.unsupported_with_media(&p.document, &p.composition, &p.resolver)
                .is_empty(),
            "{} still routes",
            p.name
        );
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("loaded.json")).unwrap())
                .unwrap();
        let document = support::doc::load(&value);
        let s = &p.samples[0];
        let req = RenderRequest::at_scale(Size::new(s.width, s.height), 1.0, Precision::Float32);
        let frame = r
            .render(
                &document,
                &p.composition,
                support::ticks(s.time),
                &req,
                &p.resolver,
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let float = r.read_rgba(&frame).unwrap();
        let transparent_max = s.float.as_ref().map(|path| {
            let mut req = req.clone();
            req.background = false;
            let f = r
                .render(
                    &document,
                    &p.composition,
                    support::ticks(s.time),
                    &req,
                    &p.resolver,
                )
                .unwrap();
            support::float_max_cv(&r.read_rgba(&f).unwrap(), &support::read_float(path))
        });
        let native = support::opaque8(&float);
        let (w, h, reference) = support::read_png(&s.png);
        let diff = support::Diff::of(&native, &reference);
        let id = p.name.replace('/', "-");
        if let Some(out) = &out {
            support::write_png(&out.join(format!("{id}-native-as-file.png")), w, h, &native);
            support::write_png(&out.join(format!("{id}-legacy.png")), w, h, &reference);
            let mut pair = vec![0u8; w as usize * h as usize * 8];
            for y in 0..h as usize {
                for (which, bytes) in [&reference, &native].iter().enumerate() {
                    let start = (y * w as usize * 2 + which * w as usize) * 4;
                    pair[start..start + w as usize * 4]
                        .copy_from_slice(&bytes[y * w as usize * 4..(y + 1) * w as usize * 4]);
                }
            }
            support::write_png(
                &out.join(format!("{id}-legacy-left-native-right.png")),
                w * 2,
                h,
                &pair,
            );
        }
        assert!(
            diff.max <= 4 && diff.within_one * 1000 >= diff.pixels * 999,
            "{}: {diff:?}",
            p.name
        );
        assert!(
            transparent_max.is_none_or(|v| v <= 1.0),
            "{} transparent: {transparent_max:?}",
            p.name
        );
        rows.push(serde_json::json!({"case":p.name,"max_cv":diff.max,"within_one":diff.within_one,"pixels":diff.pixels,"transparent_max_cv":transparent_max,"note":"L1 compatibility bitmap; no interpretation overrides"}));
    }
    assert_eq!(rows.len(), 21, "all still corpus classes must be measured");
    if let Some(out) = out {
        std::fs::write(
            out.join("still-metrics.json"),
            serde_json::to_string_pretty(&rows).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn level_one_bitmap_does_not_quantize_the_source_or_level_two_cache() {
    use serde_json::json;
    use support::doc::*;
    let dir = std::env::temp_dir().join(format!("pm-still-depth-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("deep.png");
    let mut encoder = png::Encoder::new(std::fs::File::create(&path).unwrap(), 8, 8);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Sixteen);
    let pixel = [0x80f0u16, 0x1234, 0xabcd, 0xffff];
    let bytes: Vec<u8> = (0..64)
        .flat_map(|_| pixel.into_iter().flat_map(u16::to_be_bytes))
        .collect();
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&bytes)
        .unwrap();
    let pool = pm_media::pool::FramePool::new(0);
    let original = pm_media::image::read(&path, &pool).unwrap();
    assert_eq!(original.bit_depth, 16);
    let bitmap = pm_media::image::compat::bitmap(&original, &pool).unwrap();
    assert_eq!(bitmap.bit_depth, 8);
    assert_eq!(original.bit_depth, 16);
    let mut value = project(
        8,
        8,
        1.0,
        no_fill(),
        vec![layer(
            "still",
            "image",
            json!({"asset":"clip","w":8,"h":8}),
            4.0,
            4.0,
            1.0,
        )],
    );
    asset(&mut value, "clip", "image", "png", 8, 8, 1.0);
    let mut resolver = pm_render::MapResolver::default();
    resolver.paths.insert("clip".into(), path);
    let mut renderer = Renderer::headless().unwrap();
    let mut request = RenderRequest::at_scale(Size::new(8, 8), 1.0, Precision::Float32);
    request.background = false;
    let mut first = None;
    for level in [1, 2, 1] {
        value["layers"][0]["compat"] = json!(level);
        let frame = renderer
            .render(
                &load(&value),
                "Ptest-root",
                support::ticks(0.0),
                &request,
                &resolver,
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let rgba = renderer.read_rgba(&frame).unwrap();
        let expected = if level == 1 {
            f32::from((pixel[0] >> 8) as u8) / 255.0
        } else {
            f32::from(pixel[0]) / 65535.0
        };
        assert!(
            (rgba[0] - expected).abs() < 2e-5,
            "level{level}: {} expected {expected}",
            rgba[0]
        );
        if level == 1 {
            assert!(first.is_none_or(|v| v == rgba[0]));
            first = Some(rgba[0]);
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
