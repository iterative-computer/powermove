//! Compares this crate's WebP and AVIF decodes with the texels Chromium handed WebGL
//! (image snapshots recorded in the reference trace).
//!
//!   cargo run --example images -- out/corpus/assets/webp-avif.glb out/ref/gltf-webp-avif 392,263148
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let model = s7_geom3d::load(&std::fs::read(&a[0]).unwrap(), Default::default()).unwrap();
    let blob = std::fs::read(format!("{}/trace.bin", a[1])).unwrap();
    let offsets: Vec<usize> = a[2].split(',').map(|s| s.parse().unwrap()).collect();
    for (i, img) in model.images.iter().enumerate() {
        let Some(img) = img else {
            println!("image {i}: not decoded on this target");
            continue;
        };
        let n = (img.width * img.height * 4) as usize;
        for &off in &offsets {
            let chrome = &blob[off..off + n];
            let mut max = 0u8;
            let mut differ = 0usize;
            let mut sum = 0u64;
            for (x, y) in img.rgba.iter().zip(chrome) {
                let d = x.abs_diff(*y);
                max = max.max(d);
                sum += d as u64;
                if d > 0 {
                    differ += 1;
                }
            }
            println!("image {i} ({}, {}x{}) vs snapshot @{off}: max |Δ| {max}, {differ} of {n} channel values differ, mean |Δ| {:.4}", img.mime, img.width, img.height, sum as f64 / n as f64);
        }
    }
}
