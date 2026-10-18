//! Compare source compatibility bitmaps to actual app-uploaded texture bytes.
//! Private fixtures and evidence stay outside Git. An unreachable import or
//! failed Chromium upload is recorded explicitly, never counted as parity.
use pm_media::{frame::PlaneFormat, image, pool::FramePool};
use serde_json::json;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(
        std::env::var_os("PM_SVGSTILLS_EVIDENCE").expect("private evidence directory"),
    );
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let pool = FramePool::new(0);
    let mut results = Vec::new();
    for row in rows {
        let name = row["name"].as_str().unwrap();
        if row["reachable"] != true || row["error"] != 0 {
            results.push(
                json!({"case":name,"app":row,"pass":false,"status":"no usable app-upload bitmap"}),
            );
            continue;
        }
        let path = PathBuf::from(row["file"].as_str().unwrap());
        let attempt = (|| -> pm_media::Result<_> {
            let original = image::read(&path, &pool)?;
            if original.float
                || original.color.icc_class == Some(image::icc::IccClass::ConvertedToSrgb)
            {
                let native = original.frame.with_planes(|planes| {
                    let p = &planes[0];
                    let mut out = Vec::new();
                    for y in 0..p.desc.height {
                        out.extend_from_slice(unsafe {
                            std::slice::from_raw_parts(
                                p.ptr.add(y * p.stride),
                                p.desc.width * p.desc.format.bytes(),
                            )
                        });
                    }
                    out
                })?;
                std::fs::write(root.join(format!("{name}-original-native.bin")), native)?;
            }

            let bitmap = if original.format == image::ImageFormat::Avif {
                image::avif_compat::read(&path, &original, &pool)?
            } else {
                image::compat::bitmap_at(&path, &original, &pool)?
            };
            let pixels = bitmap.frame.with_planes(|planes| {
                let p = &planes[0];
                assert_eq!(p.desc.format, PlaneFormat::Rgba8);
                let mut out = Vec::new();
                for y in 0..p.desc.height {
                    out.extend_from_slice(unsafe {
                        std::slice::from_raw_parts(p.ptr.add(y * p.stride), p.desc.width * 4)
                    });
                }
                out
            })?;
            Ok((original, bitmap, pixels))
        })();
        match attempt {
            Err(error) => {
                eprintln!("{name}: {error}");
                results.push(json!({"case":name,"error":error.to_string(),"pass":false}));
            }
            Ok((original, bitmap, pixels)) => {
                let reference = std::fs::read(root.join(row["raw"].as_str().unwrap())).unwrap();
                if name.contains("-alpha.") {
                    icc_trials(&root, name, &original, &reference);
                }
                let w = bitmap.frame.format.width;
                let h = bitmap.frame.format.height;
                if pixels.len() != reference.len() || row["w"] != w || row["h"] != h {
                    results.push(json!({"case":name,"native_size":[w,h],"app_size":[row["w"],row["h"]],"pass":false}));
                    continue;
                }
                let mut max = 0;
                let mut over1 = 0;
                let mut alpha = 0;
                for (a, b) in pixels.chunks_exact(4).zip(reference.chunks_exact(4)) {
                    let d = a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                    max = max.max(d);
                    over1 += usize::from(d > 1);
                    alpha = alpha.max(a[3].abs_diff(b[3]));
                }
                let pass = max <= 4 && over1 * 1000 <= w * h;
                eprintln!(
                    "{name}: max{max} over1={over1}/{} alpha{alpha} pass={pass}",
                    w * h
                );
                std::fs::write(root.join(format!("{name}-native.rgba8")), &pixels).unwrap();
                let mut pair = vec![0u8; w * h * 8];
                for y in 0..h {
                    for (side, src) in [&reference, &pixels].iter().enumerate() {
                        let start = (y * w * 2 + side * w) * 4;
                        pair[start..start + w * 4]
                            .copy_from_slice(&src[y * w * 4..(y + 1) * w * 4]);
                        // These are premultiplied texture bytes, shown over black.
                        for px in pair[start..start + w * 4].chunks_exact_mut(4) {
                            px[3] = 255;
                        }
                    }
                }
                ::image::save_buffer(
                    root.join(format!("{name}-legacy-left-native-right.png")),
                    &pair,
                    (w * 2) as u32,
                    h as u32,
                    ::image::ColorType::Rgba8,
                )
                .unwrap();
                results.push(json!({"case":name,"max_cv":max,"over1":over1,"pixels":w*h,"alpha_cv":alpha,"pass":pass,"native":{"format":original.format,"depth":original.bit_depth,"float":original.float,"icc":original.color.icc_class,"orientation":original.orientation}}));
            }
        }
    }
    std::fs::write(
        root.join("comparison.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
    if std::env::args().any(|a| a == "--expect-parity") {
        assert!(
            results.iter().all(|r| r["pass"] == true),
            "one or more still classes have no parity proof"
        );
    }
}

fn icc_trials(root: &std::path::Path, name: &str, original: &image::StillImage, reference: &[u8]) {
    use pm_media::sys::skcms as s;
    let Some(profile) = &original.color.icc else {
        return;
    };
    let input = original
        .frame
        .with_planes(|p| {
            assert_eq!(p[0].desc.format, PlaneFormat::Rgba8);
            let mut out = Vec::new();
            for y in 0..p[0].desc.height {
                out.extend_from_slice(unsafe {
                    std::slice::from_raw_parts(p[0].ptr.add(y * p[0].stride), p[0].desc.width * 4)
                });
            }
            out
        })
        .unwrap();
    let mut padded = profile.clone();
    padded.extend([0; 4]);
    let mut parsed: s::skcms_ICCProfile = unsafe { std::mem::zeroed() };
    assert!(unsafe {
        s::skcms_ParseWithA2BPriority(
            padded.as_ptr().cast(),
            profile.len(),
            [0, 1].as_ptr(),
            2,
            &mut parsed,
        )
    });
    let mut rows = Vec::new();
    for premul_source in [false, true] {
        for float in [false, true] {
            for premul_dest in [false, true] {
                let mut src = input.clone();
                if premul_source
                    && original.color.alpha != pm_protocol::color::AlphaMode::Premultiplied
                {
                    for p in src.chunks_exact_mut(4) {
                        let a = u32::from(p[3]);
                        for c in &mut p[..3] {
                            *c = ((u32::from(*c) * a * 257 + 257 * 128) >> 16) as u8;
                        }
                    }
                }
                let mut out8 = vec![0u8; src.len()];
                let mut out32 = vec![0.0f32; src.len()];
                let (dst, format) = if float {
                    (out32.as_mut_ptr().cast(), s::skcms_PixelFormat_RGBA_ffff)
                } else {
                    (out8.as_mut_ptr().cast(), s::skcms_PixelFormat_RGBA_8888)
                };
                assert!(unsafe {
                    s::skcms_Transform(
                        src.as_ptr().cast(),
                        s::skcms_PixelFormat_RGBA_8888,
                        if premul_source {
                            s::skcms_AlphaFormat_PremulAsEncoded
                        } else {
                            s::skcms_AlphaFormat_Unpremul
                        },
                        &parsed,
                        dst,
                        format,
                        if premul_dest {
                            s::skcms_AlphaFormat_PremulAsEncoded
                        } else {
                            s::skcms_AlphaFormat_Unpremul
                        },
                        s::skcms_sRGB_profile(),
                        src.len() / 4,
                    )
                });
                if float {
                    for p in out32.chunks_exact_mut(4) {
                        if !premul_dest {
                            let a = p[3];
                            for c in &mut p[..3] {
                                *c *= a;
                            }
                        }
                    }
                    for (a, b) in out8.iter_mut().zip(out32) {
                        *a = (b * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                } else if !premul_dest {
                    for p in out8.chunks_exact_mut(4) {
                        let a = u32::from(p[3]);
                        for c in &mut p[..3] {
                            *c = ((u32::from(*c) * a * 257 + 257 * 128) >> 16) as u8;
                        }
                    }
                }
                let mut max = 0;
                let mut over1 = 0;
                for (a, b) in out8.chunks_exact(4).zip(reference.chunks_exact(4)) {
                    let d = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
                    max = max.max(d);
                    over1 += usize::from(d > 1);
                }
                eprintln!(
                    "ICC {name} srcPremul={premul_source} float={float} dstPremul={premul_dest}: max{max} over1={over1}"
                );
                std::fs::write(
                    root.join(format!(
                        "{name}-trial-{premul_source}-{float}-{premul_dest}.rgba8"
                    )),
                    &out8,
                )
                .unwrap();
                rows.push(json!({"source_premul":premul_source,"float":float,"dest_premul":premul_dest,"max_cv":max,"over1":over1,"pixels":reference.len()/4}));
            }
        }
    }
    std::fs::write(
        root.join(format!("{name}-icc-trials.json")),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}
