#![cfg(target_os = "macos")]
use image::{Rgba, RgbaImage};
use pm_wave::*;
use std::path::Path;
fn load(name: &str) -> RgbaImage {
    image::open(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
    .to_rgba8()
}
fn thumbnail(img: &RgbaImage) -> RgbaImage {
    RgbaImage::from_fn(155, 100, |x, y| {
        let mut sum = [0u32; 4];
        for yy in 0..4 {
            for xx in 0..4 {
                for (sum, v) in sum.iter_mut().zip(img.get_pixel(x * 4 + xx, y * 4 + yy).0) {
                    *sum += u32::from(v);
                }
            }
        }
        Rgba(sum.map(|v| ((v + 8) / 16) as u8))
    })
}
// A global SSIM and normalized RGB mean absolute difference, both printed per frame.
fn difference(a: &RgbaImage, b: &RgbaImage) -> (f64, f64) {
    let mut abs = 0.;
    let mut sx = 0.;
    let mut sy = 0.;
    let mut xx = 0.;
    let mut yy = 0.;
    let mut xy = 0.;
    let n = (a.width() * a.height() * 3) as f64;
    for (a, b) in a.pixels().zip(b.pixels()) {
        for i in 0..3 {
            let x = f64::from(a[i]) / 255.;
            let y = f64::from(b[i]) / 255.;
            abs += (x - y).abs();
            sx += x;
            sy += y;
            xx += x * x;
            yy += y * y;
            xy += x * y;
        }
    }
    let mx = sx / n;
    let my = sy / n;
    let vx = xx / n - mx * mx;
    let vy = yy / n - my * my;
    let cov = xy / n - mx * my;
    (
        ((2. * mx * my + 0.0001) * (2. * cov + 0.0009))
            / ((mx * mx + my * my + 0.0001) * (vx + vy + 0.0009)),
        abs / n,
    )
}
#[test]
fn webgl_reference_frames() {
    let device = metal::Device::system_default().expect("Metal target required");
    let mut r = WaveRenderer::new(&device).expect("Metal shader compilation");
    let from = load("from.png");
    let to = load("to.png");
    let output = std::env::var_os("PM_WAVE_FRAMES").map(std::path::PathBuf::from);
    if let Some(p) = &output {
        std::fs::create_dir_all(p).unwrap();
    }
    for (kind, name, src) in [
        (
            WaveType::Vertical,
            "vertical",
            Rect {
                x: 235.,
                y: 330.,
                w: 150.,
                h: 44.,
            },
        ),
        (
            WaveType::Horizontal,
            "horizontal",
            Rect {
                x: 12.,
                y: 120.,
                w: 44.,
                h: 160.,
            },
        ),
        (
            WaveType::Center,
            "center",
            Rect {
                x: 280.,
                y: 170.,
                w: 60.,
                h: 60.,
            },
        ),
    ] {
        for (palette, pal_name) in [
            (Palette::Colorful, "colorful"),
            (Palette::Green, "green"),
            (Palette::Ocean, "ocean"),
            (Palette::Sunset, "sunset"),
            (Palette::Mono, "mono"),
            (Palette::hex("#528dff").unwrap(), "528dff"),
        ] {
            let mut style = WaveStyle::new(kind);
            style.palette = palette;
            let style = style.for_size(620., 400., 28.);
            let k = WaveStyle::scale(620., 400.);
            let src = Rect {
                x: src.x / k,
                y: src.y / k,
                w: src.w / k,
                h: src.h / k,
            };
            r.prepare(&from, Some(&to), 620. / k, &style).unwrap();
            let mut worst: f64 = 1.;
            let mut worst_mae: f64 = 0.;
            for i in 0..=10 {
                let frame = r
                    .render(f64::from(i) / 10., src, 620. / k, 400. / k, None, false)
                    .unwrap();
                let mut img = frame.read_image().unwrap();
                for px in img.pixels_mut() {
                    let a = u32::from(px[3]);
                    for (i, bg) in [16u32, 24, 34].into_iter().enumerate() {
                        px[i] = (u32::from(px[i]) + ((255 - a) * bg + 127) / 255).min(255) as u8;
                    }
                    px[3] = 255;
                }
                if let Some(out) = &output {
                    img.save(out.join(format!("{name}-{pal_name}-{i}.png")))
                        .unwrap();
                }
                let native = thumbnail(&img);
                let atlas = load(&format!("{name}-{pal_name}.png"));
                let reference = image::imageops::crop_imm(&atlas, i * 155, 0, 155, 100).to_image();
                let (ssim, mae) = difference(&native, &reference);
                worst = worst.min(ssim);
                worst_mae = worst_mae.max(mae);
                println!(
                    "{name}/{pal_name} scrub={:.1} SSIM={ssim:.6} MAE={mae:.6}",
                    f64::from(i) / 10.
                );
                assert!(
                    ssim > 0.9998 && mae < 0.001,
                    "frame {name}/{pal_name}/{i}: SSIM {ssim}, MAE {mae}"
                );
            }
            println!("SUMMARY {name}/{pal_name}: min SSIM={worst:.6}, max MAE={worst_mae:.6}");
            r.release();
            assert_eq!(
                r.texture_count(),
                0,
                "all screenshot/output leases released"
            );
        }
    }
}
