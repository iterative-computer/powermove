//! 3D material maps read the level 1 still-compatibility bitmap, texel for
//! texel as today's app stores them.
//!
//! `fixtures/stills2/three-maps.json` holds what Electron 44 (Chromium 152)
//! stored for each fixture through the shipped importer and three.js r180's
//! map upload (`e2e/tools/threed-maps-capture.ts`: `texStorage2D` of
//! `log2(max(w, h)) + 1` levels, `texSubImage2D` of the asset's bitmap,
//! read back from a framebuffer), as SHA-256 of level 0 (identical in the
//! sRGB and linear formats) and of level 1 after `generateMipmap` per
//! format. `upload-failed` rows are the deep and arithmetic JPEGs whose
//! upload fails `INVALID_VALUE` and leaves the storage zeroed; `rejected`
//! rows fail import (no map).
mod support;

use pm_render::scene3d::textures::{MapError, Textures, decode};
use sha2::{Digest, Sha256};
use std::path::Path;

#[test]
fn maps_store_todays_texels_for_every_still_class() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stills2");
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(root.join("three-maps.json")).unwrap()).unwrap();
    let pool = pm_media::pool::FramePool::new(0);
    let (mut texels, mut failed, mut rejected) = (0, 0, 0);
    for row in &rows {
        let name = row["file"].as_str().unwrap();
        let path = root.join(name);
        match row["outcome"].as_str().unwrap() {
            "rejected" => {
                // Today's import fails: the map has no bitmap (Missing texture).
                assert!(
                    matches!(decode(&path, &pool), Err(MapError::Failed(_))),
                    "{name}"
                );
                assert!(matches!(Textures::probe(&path), Err(MapError::Failed(_))), "{name}");
                rejected += 1;
            }
            outcome => {
                assert_eq!(Textures::probe(&path), Ok(()), "{name}: drawn, never routed");
                let (w, h, rgba) = decode(&path, &pool).unwrap_or_else(|e| panic!("{name}: {e:?}"));
                assert_eq!(
                    (u64::from(w), u64::from(h)),
                    (row["width"].as_u64().unwrap(), row["height"].as_u64().unwrap()),
                    "{name}"
                );
                if outcome == "upload-failed" {
                    assert!(rgba.iter().all(|&b| b == 0), "{name}: zeroed storage");
                    failed += 1;
                } else {
                    texels += 1;
                }
                assert_eq!(
                    format!("{:x}", Sha256::digest(&rgba)),
                    row["sha256"].as_str().unwrap(),
                    "{name}: level 0 texels"
                );
            }
        }
    }
    eprintln!("3D maps: {texels} classes texel-exact, {failed} failed uploads zeroed, {rejected} rejected imports");
    assert!(texels >= 100 && failed >= 10 && rejected >= 2, "{texels} {failed} {rejected}");
}

/// Reads mip `level` of an RGBA8 texture back as tight rows.
fn read_level(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, level: u32) -> Vec<u8> {
    let w = (texture.width() >> level).max(1);
    let h = (texture.height() >> level).max(1);
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mip readback"),
        size: u64::from(row * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = buffer.slice(..).get_mapped_range().unwrap();
    (0..h as usize)
        .flat_map(|y| data[y * row as usize..y * row as usize + w as usize * 4].to_vec())
        .collect()
}

/// The pass's `generateMipmap` chain against Chromium's (ANGLE on Metal)
/// at level 1, per format (sRGB maps filter in linear light).
#[test]
fn map_mip_levels_against_todays_generate_mipmap() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stills2");
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(root.join("three-maps.json")).unwrap()).unwrap();
    let evidence = std::env::var("PM_THREED_MAPS_EVIDENCE").ok().map(std::path::PathBuf::from);
    let renderer = pm_render::Renderer::headless().expect("a Metal device");
    let (device, queue) = (renderer.device(), renderer.queue());
    let pool = pm_media::pool::FramePool::new(0);
    let mut textures = Textures::default();
    let (mut equal, mut compared, mut worst) = (0, 0, 0u8);
    for row in &rows {
        if row["outcome"] != "texels" {
            continue;
        }
        let name = row["file"].as_str().unwrap();
        let (w, h, rgba) = decode(&root.join(name), &pool).unwrap();
        if w.max(h) < 2 {
            continue;
        }
        for (srgb, key, space) in [(true, "level1Srgb", "srgb"), (false, "level1Linear", "linear")] {
            let mut encoder = device.create_command_encoder(&Default::default());
            let texture = textures.upload_rgba(device, queue, &mut encoder, w, h, &rgba, srgb, true);
            queue.submit([encoder.finish()]);
            let level1 = read_level(device, queue, &texture, 1);
            compared += 1;
            if format!("{:x}", Sha256::digest(&level1)) == row[key].as_str().unwrap() {
                equal += 1;
            } else if let Some(dir) = &evidence {
                let base = Path::new(name).file_name().unwrap().to_string_lossy().into_owned();
                let file = dir.join(format!("fixture-{base}.{space}.level1.rgba8"));
                let today = std::fs::read(&file).or_else(|_| std::fs::read(dir.join(format!("{base}.{space}.level1.rgba8"))));
                if let Ok(today) = today {
                    let d = today.iter().zip(&level1).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
                    worst = worst.max(d);
                }
            }
        }
    }
    eprintln!("3D map level 1 mips: {equal} of {compared} equal to Chromium's generateMipmap; largest difference {worst} (with PM_THREED_MAPS_EVIDENCE)");
    assert_eq!(equal, compared, "every level 1 mip equals Chromium's");
    assert!(compared >= 200);
}
