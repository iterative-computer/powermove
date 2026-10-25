//! Original SDR HEIF routing and source frames, independently referenced by
//! ImageIO/ColorSync. Generated media stays in a temporary directory.
#![cfg(target_os = "macos")]

use std::path::Path;
use std::process::Command;

use pm_media::image::{self, ImageFormat, icc::IccClass};
use pm_media::pool::FramePool;
use pm_protocol::color::AlphaMode;
use pm_protocol::common::{Size, Ticks};
use pm_protocol::snapshot::SourceFit;
use pm_render::Renderer;
use pm_render::media::{StillReader, probe_still};

// The encoder/decoder and reference draw use Apple's native APIs, like the
// ImageIO orientation cases in tests/stills.rs. Reference color conversion
// uses extended linear sRGB floats so gamut clips only after premultiplying
// the final display values, rather than clipping straight colors to RGBA8.
const IMAGEIO_REFERENCE: &str = r#"
import Foundation
import CoreGraphics
import ImageIO

let root = URL(fileURLWithPath: CommandLine.arguments[1])
let out = URL(fileURLWithPath: CommandLine.arguments[2])
let cases: [(String, String, Bool, Bool, Int)] = [
    ("existing", "apps/desktop/e2e/fixtures/still.heic", false, false, 1),
    ("deep", "engine/crates/pm-render/tests/fixtures/stills2/png-chrm-srgb-gama-16.png", true, false, 1),
    ("deep-alpha", "engine/crates/pm-render/tests/fixtures/stills2/png-cicp-1-13-16-alpha.png", true, false, 1),
    ("p3-alpha-oriented", "apps/desktop/e2e/fixtures/still-alpha.png", true, true, 6),
]
var report: [[String: Any]] = []
for (name, input, encode, p3, orientation) in cases {
    var path = root.appendingPathComponent(input)
    if encode {
        let src = CGImageSourceCreateWithURL(path as CFURL, nil)!
        let image = CGImageSourceCreateImageAtIndex(src, 0, nil)!
        let space = CGColorSpace(name: p3 ? CGColorSpace.displayP3 : CGColorSpace.sRGB)!
        let tagged = image.copy(colorSpace: space)!
        path = out.appendingPathComponent(name + ".heic")
        let dst = CGImageDestinationCreateWithURL(path as CFURL, "public.heic" as CFString, 1, nil)!
        let props: [CFString: Any] = [
            kCGImageDestinationLossyCompressionQuality: 1.0,
            kCGImagePropertyDepth: image.bitsPerComponent > 8 ? 10 : 8,
            kCGImagePropertyOrientation: orientation,
        ]
        CGImageDestinationAddImage(dst, tagged, props as CFDictionary)
        precondition(CGImageDestinationFinalize(dst), "HEIF encoding failed: " + name)
    }
    let src = CGImageSourceCreateWithURL(path as CFURL, nil)!
    let decoded = CGImageSourceCreateImageAtIndex(src, 0, nil)!
    let props = CGImageSourceCopyPropertiesAtIndex(src, 0, nil)! as NSDictionary
    let applied = (props[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1
    let options: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceThumbnailMaxPixelSize: max(decoded.width, decoded.height),
    ]
    let oriented = CGImageSourceCreateThumbnailAtIndex(src, 0, options as CFDictionary)!
    let w = oriented.width, h = oriented.height
    var linear = [Float](repeating: 0, count: w * h * 4)
    linear.withUnsafeMutableBytes { bytes in
        let info = CGBitmapInfo.floatComponents.rawValue
            | CGBitmapInfo.byteOrder32Little.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue
        let context = CGContext(data: bytes.baseAddress, width: w, height: h,
            bitsPerComponent: 32, bytesPerRow: w * 16,
            space: CGColorSpace(name: CGColorSpace.extendedLinearSRGB)!, bitmapInfo: info)!
        context.interpolationQuality = .none
        context.draw(oriented, in: CGRect(x: 0, y: 0, width: CGFloat(w), height: CGFloat(h)))
    }
    func display(_ x: Double) -> Double {
        let v = abs(x)
        let e = v <= 0.0031308 ? 12.92 * v : 1.055 * pow(v, 1.0 / 2.4) - 0.055
        return x < 0 ? -e : e
    }
    func code(_ x: Double) -> UInt8 { UInt8((min(1, max(0, x)) * 255).rounded()) }
    var rgba = [UInt8](repeating: 0, count: linear.count)
    for i in stride(from: 0, to: linear.count, by: 4) {
        let a = Double(linear[i + 3])
        for c in 0..<3 { rgba[i + c] = code(a > 0 ? display(Double(linear[i + c]) / a) * a : 0) }
        rgba[i + 3] = code(a)
    }
    try Data(rgba).write(to: out.appendingPathComponent(name + ".rgba"))
    report.append(["name": name, "path": path.path, "width": w, "height": h,
        "depth": decoded.bitsPerComponent, "orientation": applied])
}
try JSONSerialization.data(withJSONObject: report).write(to: out.appendingPathComponent("cases.json"))
"#;

#[test]
fn original_sdr_heif_depth_alpha_orientation_and_profile_reach_source_frames() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("heif-reference.swift");
    std::fs::write(&script, IMAGEIO_REFERENCE).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    assert!(
        Command::new("/usr/bin/swift")
            .arg(&script)
            .arg(root)
            .arg(temp.path())
            .status()
            .expect("ImageIO test cases require Xcode command line tools")
            .success()
    );
    let cases: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(temp.path().join("cases.json")).unwrap()).unwrap();
    let pool = FramePool::new(0);
    let mut renderer = Renderer::headless().unwrap();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let path = Path::new(case["path"].as_str().unwrap());
        let decoded = image::read(path, &pool).unwrap();
        assert_eq!(decoded.format, ImageFormat::Heif);
        assert!(!decoded.float, "{name}: this test covers integer SDR");
        if name.starts_with("deep") {
            assert!(
                decoded.bit_depth > 8,
                "{name}: encoder must retain higher depth"
            );
            assert!(case["depth"].as_u64().unwrap() > 8);
        }
        if name.contains("alpha") {
            assert_ne!(decoded.color.alpha, AlphaMode::Opaque, "{name}");
        }
        if name.starts_with("p3") {
            assert_eq!(decoded.color.icc_class, Some(IccClass::DisplayP3));
            assert!(
                pm_render::stills::container_transform(path, ImageFormat::Heif)
                    .unwrap()
                    .is_some()
            );
        } else {
            assert!(
                matches!(
                    decoded.color.icc_class,
                    Some(IccClass::Srgb | IccClass::DisplayP3 | IccClass::ConvertedToSrgb)
                ),
                "{name}: actual profile {:?}",
                decoded.color.icc_class
            );
        }
        assert_eq!(
            u64::from(decoded.orientation),
            case["orientation"].as_u64().unwrap()
        );
        let size = Size::new(
            case["width"].as_u64().unwrap() as u32,
            case["height"].as_u64().unwrap() as u32,
        );
        assert_eq!(
            (decoded.frame.format.width, decoded.frame.format.height),
            (size.width as usize, size.height as usize)
        );
        for reader in [StillReader::Chromium, StillReader::Native] {
            assert_eq!(probe_still(path, reader), Ok(()), "{name}: {reader:?}");
        }
        let expected = std::fs::read(temp.path().join(format!("{name}.rgba"))).unwrap();
        if name.contains("alpha") {
            assert!(
                expected.chunks_exact(4).any(|p| p[3] < 255)
                    && expected.chunks_exact(4).any(|p| p[3] > 0),
                "{name}: visible and transparent pixels"
            );
        }
        if name == "deep-alpha" {
            assert!(
                expected.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255),
                "higher-depth case must exercise partial alpha"
            );
        }
        let monitor = renderer
            .source_rgba8(path, Ticks::ZERO, size, SourceFit::Stretch, None, false)
            .unwrap();
        let poster = renderer
            .source_rgba8_export(path, Ticks::ZERO, size, SourceFit::Stretch, None, false)
            .unwrap();
        let strip = renderer
            .source_rgba8_batch(
                path,
                &[Ticks::ZERO, Ticks::from_whole_seconds(1)],
                size,
                SourceFit::Stretch,
                None,
                false,
            )
            .unwrap();
        assert_eq!(monitor, poster, "{name}: poster/source contract");
        assert!(
            strip.iter().all(|frame| frame == &monitor),
            "{name}: still filmstrip contract"
        );
        let native = renderer
            .source_rgba8_composed(
                path,
                Ticks::ZERO,
                size,
                SourceFit::Stretch,
                &Default::default(),
                false,
                pm_render::sampling::Sampling::Final,
            )
            .unwrap();
        assert_eq!(
            monitor, native,
            "{name}: every native source consumer keeps the same reading"
        );
        for (reading, actual) in [("display", monitor), ("native", native)] {
            assert_eq!(actual.len(), expected.len());
            let mut errors: Vec<_> = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b))
                .collect();
            errors.sort_unstable();
            let max = *errors.last().unwrap();
            let p999 =
                errors[((errors.len() as f64 * 0.999).ceil() as usize - 1).min(errors.len() - 1)];
            let mean = errors.iter().map(|v| f64::from(*v)).sum::<f64>() / errors.len() as f64;
            eprintln!(
                "{name} {reading}: reference error max={max}, p99.9={p999}, mean={mean:.6} CV"
            );
            assert!(
                max <= 1,
                "{name} {reading}: ImageIO/ColorSync reference differs by {max} codes"
            );
        }
    }
}
