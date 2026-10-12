//! Renders the test pattern into real IOSurface-backed pool textures and checks the
//! bytes the frame bridge will hand to Chromium.
use s1_engine::pool::Surface;
use s1_engine::render::{FrameParams, Gpu, SceneKind};
use s1_mach::wire::PixelFormat;

const W: u32 = 3840;
const H: u32 = 2160;

fn f16_to_f32(h: u16) -> f32 {
    let s = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as f32;
    s * match e {
        0 => m * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1.0 + m / 1024.0) * 2f32.powi(e - 15),
    }
}

/// Reads row `y`'s first channel (R) for every column, as floats in code-value units of 1.0.
fn read_row(surface: &Surface, y: u32, format: PixelFormat) -> Vec<f32> {
    let io = &surface.iosurface.0;
    let mut seed = 0u32;
    // SAFETY: read-only lock; the GPU work that wrote the surface has completed.
    unsafe { io.lock(objc2_io_surface::IOSurfaceLockOptions::ReadOnly, &mut seed) };
    let base = io.base_address().as_ptr() as *const u8;
    let bpr = io.bytes_per_row();
    let mut out = Vec::with_capacity(W as usize);
    for x in 0..W as usize {
        // SAFETY: (x, y) is inside the surface.
        unsafe {
            let p = base.add(y as usize * bpr);
            out.push(match format {
                // BGRA byte order: R is byte 2.
                PixelFormat::Bgra8 => *p.add(x * 4 + 2) as f32 / 255.0,
                PixelFormat::Rgba16F => f16_to_f32(u16::from_ne_bytes([*p.add(x * 8), *p.add(x * 8 + 1)])),
            });
        }
    }
    unsafe { io.unlock(objc2_io_surface::IOSurfaceLockOptions::ReadOnly, &mut seed) };
    out
}

fn render(format: PixelFormat, seq: u64, rev: u64) -> (Gpu, Surface) {
    let gpu = Gpu::new(W, H, format, 0).unwrap();
    let surface = Surface::create(&gpu.device, 0, W, H, format).unwrap();
    let p = FrameParams { time_s: 0.5, seq, revision: rev, marker: [3000.0, 1500.0, 96.0, 96.0], scene: SceneKind::Pattern };
    let idx = gpu.render(&surface, &p);
    gpu.wait(idx).unwrap();
    (gpu, surface)
}

fn decode_code(row: &[f32]) -> (u32, u32) {
    let bit = |cell: usize| (row[cell * 32 + 16] > 0.5) as u32;
    let seq = (0..32).fold(0u32, |a, i| a | (bit(i) << i));
    let rev = (0..32).fold(0u32, |a, i| a | (bit(32 + i) << i));
    (seq, rev)
}

#[test]
fn rgbaf16_ramp_hdr_and_code() {
    let (_g, s) = render(PixelFormat::Rgba16F, 0xA5A5_1234, 77);
    assert_eq!(decode_code(&read_row(&s, 16, PixelFormat::Rgba16F)), (0xA5A5_1234, 77));
    let ramp = read_row(&s, 100, PixelFormat::Rgba16F);
    // Every 10-bit code value 0..1023 is present and the ramp is monotonic.
    let mut codes = std::collections::BTreeSet::new();
    let mut max_err = 0f32;
    for (x, &v) in ramp.iter().enumerate() {
        let ideal = (x as f32 + 0.5) / W as f32;
        max_err = max_err.max((v - ideal).abs());
        codes.insert((v * 1023.0).round() as u32);
        if x > 0 {
            assert!(v >= ramp[x - 1]);
        }
    }
    assert_eq!(codes.len(), 1024, "distinct 10-bit codes");
    assert!(max_err * 1023.0 < 0.5, "max error {} CV10", max_err * 1023.0);
    let patches = read_row(&s, 700, PixelFormat::Rgba16F);
    let want = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 4.0];
    for (i, w) in want.iter().enumerate() {
        let v = patches[i * W as usize / 8 + 100];
        assert_eq!(v, *w, "patch {i}");
    }
}

#[test]
fn bgra_dither_preserves_ten_bit_local_mean_and_reference_bands() {
    let (_g, s) = render(PixelFormat::Bgra8, 5, 1);
    assert_eq!(decode_code(&read_row(&s, 16, PixelFormat::Bgra8)), (5, 1));
    // Average 64 rows x 4 columns (narrower than one 8-bit step, 15 px at 3840 wide)
    // of the dithered band and of the undithered band.
    let rows_d: Vec<Vec<f32>> = (64..128).map(|y| read_row(&s, y, PixelFormat::Bgra8)).collect();
    let rows_u: Vec<Vec<f32>> = (320..384).map(|y| read_row(&s, y, PixelFormat::Bgra8)).collect();
    let (mut worst_d, mut worst_u) = (0f32, 0f32);
    for bx in 0..(W as usize / 4) {
        let mean = |rows: &Vec<Vec<f32>>| {
            rows.iter().map(|r| r[bx * 4..bx * 4 + 4].iter().sum::<f32>()).sum::<f32>() / (rows.len() * 4) as f32
        };
        let ideal = (bx as f32 * 4.0 + 2.0) / W as f32;
        worst_d = worst_d.max((mean(&rows_d) - ideal).abs() * 1023.0);
        worst_u = worst_u.max((mean(&rows_u) - ideal).abs() * 1023.0);
    }
    eprintln!("bgra: dithered worst local-mean error {worst_d:.3} CV10, undithered {worst_u:.3} CV10");
    assert!(worst_d <= 0.5, "dithered local mean off by {worst_d} CV10");
    assert!(worst_u > 1.5, "undithered band should show 8-bit steps, got {worst_u}");
    // Values above 1.0 clip on the 8-bit path.
    let patches = read_row(&s, 700, PixelFormat::Bgra8);
    assert_eq!(patches[7 * W as usize / 8 + 100], 1.0);
}
