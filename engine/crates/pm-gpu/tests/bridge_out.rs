//! Renders the bridge test source through the display transform into real
//! IOSurface-backed pool surfaces and checks the bytes Chromium will import
//! (plan section 18; the S1 quality lines).
#![cfg(target_os = "macos")]

use pm_gpu::bridge_out::display::{DisplayParams, DisplayTransform, WorkingTexture};
use pm_gpu::bridge_out::pool::SurfacePool;
use pm_gpu::bridge_out::test_source::{TestFrame, TestSource, orbit_view, view_projection};
use pm_gpu::bridge_out::{metal_device, surface::ExportedSurface};
use pm_protocol::color::{ViewerColorSpace, WorkingEncoding, display_decode, display_encode};
use pm_protocol::common::Size;
use pm_protocol::frame::{EditorCamera, Projection};
use pm_protocol::interop::{PlatformSurface, SurfaceFormat};

const W: u32 = 3840;
const H: u32 = 2160;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    transform: DisplayTransform,
    source: TestSource,
    working: WorkingTexture,
}

fn gpu() -> Gpu {
    let (device, queue, _) = metal_device("bridge-out test").unwrap();
    let transform = DisplayTransform::new(&device, &queue);
    let size = Size::new(W, H);
    let source = TestSource::new(&device, size, 0);
    let working = WorkingTexture::new(&device, size);
    Gpu {
        device,
        queue,
        transform,
        source,
        working,
    }
}

fn frame(seq: u64, view_seq: u64) -> TestFrame {
    TestFrame {
        time_s: 0.5,
        seq,
        view_seq,
        composition: [1200.0, 1200.0, 1440.0, 810.0],
        marker: [3000.0, 1500.0, 96.0, 96.0],
        camera: None,
    }
}

fn render(gpu: &Gpu, target: &ExportedSurface, frame: &TestFrame, params: DisplayParams) {
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    gpu.source
        .encode(&gpu.queue, &mut encoder, &gpu.working, frame);
    gpu.transform
        .encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &gpu.working,
            target,
            params,
        )
        .unwrap();
    let index = gpu.queue.submit([encoder.finish()]);
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: Some(std::time::Duration::from_secs(5)),
        })
        .unwrap();
}

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

/// RGB of every pixel in row `y`, in code-value units of 1.0.
struct Pixels {
    bytes: Vec<u8>,
    bpr: usize,
    format: SurfaceFormat,
}

impl Pixels {
    fn of(surface: &ExportedSurface) -> Self {
        Self {
            bytes: surface.iosurface.read(),
            bpr: surface.iosurface.bytes_per_row() as usize,
            format: surface.format,
        }
    }

    fn rgb(&self, x: u32, y: u32) -> [f32; 3] {
        let row = &self.bytes[y as usize * self.bpr..];
        match self.format {
            SurfaceFormat::Bgra8Unorm => {
                let p = &row[x as usize * 4..];
                [
                    p[2] as f32 / 255.0,
                    p[1] as f32 / 255.0,
                    p[0] as f32 / 255.0,
                ]
            }
            _ => {
                let p = &row[x as usize * 8..];
                let h = |i: usize| f16_to_f32(u16::from_ne_bytes([p[i * 2], p[i * 2 + 1]]));
                [h(0), h(1), h(2)]
            }
        }
    }

    fn r(&self, x: u32, y: u32) -> f32 {
        self.rgb(x, y)[0]
    }

    fn code(&self) -> (u32, u32) {
        let cell = TestFrame::code_cell(W);
        let bit = |c: u32| (self.r(c * cell + cell / 2, 16) > 0.5) as u32;
        let seq = (0..32).fold(0u32, |a, i| a | (bit(i) << i));
        let view = (0..32).fold(0u32, |a, i| a | (bit(32 + i) << i));
        (seq, view)
    }
}

const STANDARD_SRGB: DisplayParams = DisplayParams {
    working: WorkingEncoding::Standard,
    canvas: ViewerColorSpace::Srgb,
    noise_offset: [0, 0],
};

#[test]
fn a_pool_has_the_requested_surfaces_and_describes_itself() {
    let (device, _queue, _) = metal_device("pool test").unwrap();
    let pool = SurfacePool::new(
        &device,
        3,
        Size::new(640, 360),
        SurfaceFormat::Bgra8Unorm,
        6,
    )
    .unwrap();
    assert_eq!(pool.len(), 6);
    assert!(pool.bytes() >= 640 * 360 * 4 * 6);
    for surface in &pool.surfaces {
        assert!(surface.iosurface.mach_port().is_some());
        assert!(surface.iosurface.bytes_per_row() >= 640 * 4);
    }
    let descriptor = pool.descriptor();
    assert_eq!(descriptor.generation.get(), 3);
    assert_eq!(descriptor.surfaces.len(), 6);
    assert_eq!(
        descriptor.surfaces[5].platform,
        PlatformSurface::IoSurface { bridge_index: 5 }
    );
    assert!(pm_protocol::validate::Validate::validate(&descriptor).is_ok());
    assert!(SurfacePool::new(&device, 4, Size::new(64, 64), SurfaceFormat::Bgra8Unorm, 3).is_err());
    assert!(
        SurfacePool::new(&device, 4, Size::new(64, 64), SurfaceFormat::Rgba32Float, 4).is_err()
    );
}

#[test]
fn rgbaf16_keeps_ten_bit_ramps_and_values_above_one() {
    let gpu = gpu();
    let pool = SurfacePool::new(
        &gpu.device,
        1,
        Size::new(W, H),
        SurfaceFormat::Rgba16Float,
        4,
    )
    .unwrap();
    render(
        &gpu,
        &pool.surfaces[0],
        &frame(0xA5A5_1234, 77),
        STANDARD_SRGB,
    );
    let px = Pixels::of(&pool.surfaces[0]);
    assert_eq!(px.code(), (0xA5A5_1234, 77));
    let mut codes = std::collections::BTreeSet::new();
    let mut max_err = 0f32;
    let mut previous = -1.0;
    for x in 0..W {
        let v = px.r(x, 100);
        let ideal = (x as f32 + 0.5) / W as f32;
        max_err = max_err.max((v - ideal).abs());
        codes.insert((v * 1023.0).round() as u32);
        assert!(v >= previous, "monotonic at {x}");
        previous = v;
    }
    assert_eq!(codes.len(), 1024, "distinct 10-bit codes");
    assert!(
        max_err * 1023.0 < 0.5,
        "max error {} CV10",
        max_err * 1023.0
    );
    let want = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 4.0];
    for (i, w) in want.iter().enumerate() {
        assert_eq!(px.r(i as u32 * W / 8 + 100, 700), *w, "patch {i}");
    }
}

/// Greys are the same in BT.709 and Display P3: a display-p3 canvas's
/// transform (decode, matrix, encode) must not add banding to a grey ramp.
/// Its result is rounded to f16 before the write: Apple GPUs truncate on
/// render-target writes, which alone cost up to one f16 step (0.5 CV10 near
/// 1.0) on top of the working texture's.
#[test]
fn rgbaf16_on_a_display_p3_canvas_keeps_ten_bit_grey_ramps() {
    let gpu = gpu();
    let pool = SurfacePool::new(
        &gpu.device,
        1,
        Size::new(W, H),
        SurfaceFormat::Rgba16Float,
        4,
    )
    .unwrap();
    render(
        &gpu,
        &pool.surfaces[0],
        &frame(3, 1),
        DisplayParams {
            canvas: ViewerColorSpace::DisplayP3,
            ..STANDARD_SRGB
        },
    );
    let px = Pixels::of(&pool.surfaces[0]);
    let mut codes = std::collections::BTreeSet::new();
    let mut max_err = 0f32;
    let mut previous = -1.0;
    for x in 0..W {
        let v = px.r(x, 100);
        let ideal = (x as f32 + 0.5) / W as f32;
        max_err = max_err.max((v - ideal).abs());
        codes.insert((v * 1023.0).round() as u32);
        assert!(v >= previous, "monotonic at {x}");
        previous = v;
    }
    eprintln!(
        "display-p3 grey ramp: max error {:.3} CV10, {} codes",
        max_err * 1023.0,
        codes.len()
    );
    assert_eq!(codes.len(), 1024, "distinct 10-bit codes");
    assert!(
        max_err * 1023.0 < 0.6,
        "max error {} CV10",
        max_err * 1023.0
    );
}

#[test]
fn bgra_dither_keeps_the_ten_bit_mean_and_eight_bit_values_exact() {
    let gpu = gpu();
    let pool = SurfacePool::new(
        &gpu.device,
        1,
        Size::new(W, H),
        SurfaceFormat::Bgra8Unorm,
        4,
    )
    .unwrap();
    render(&gpu, &pool.surfaces[0], &frame(5, 1), STANDARD_SRGB);
    let px = Pixels::of(&pool.surfaces[0]);
    assert_eq!(px.code(), (5, 1));
    // The 8-bit grid ramp passes the dither untouched (what capturePage reads).
    for x in 0..W {
        let want = ((x as f32 + 0.5) / W as f32 * 255.0).round();
        assert_eq!(px.r(x, 400) * 255.0, want, "8-bit grid row at {x}");
    }
    // The continuous ramp: 64 rows × 4 columns (narrower than one 8-bit step,
    // 15 px at 3840 wide) average to the 10-bit value.
    let mut worst = 0f32;
    for bx in 0..W / 4 {
        let mut sum = 0.0;
        for y in 64..128 {
            for x in bx * 4..bx * 4 + 4 {
                sum += px.r(x, y);
            }
        }
        let mean = sum / 256.0;
        let ideal = (bx as f32 * 4.0 + 2.0) / W as f32;
        worst = worst.max((mean - ideal).abs() * 1023.0);
    }
    eprintln!("bgra dithered ramp: worst 4 × 64 local mean error {worst:.3} CV10");
    // One f16 step around each 8-bit code is kept exact (see display.wgsl),
    // which costs at most one step (0.5 CV10 near 1.0) on top of the dither.
    assert!(worst <= 0.85, "local mean off by {worst} CV10");
    // Values above 1.0 clip on the 8-bit path.
    assert_eq!(px.r(7 * W / 8 + 100, 700), 1.0);
}

#[test]
fn the_dither_moves_with_the_noise_offset_but_keeps_its_mean() {
    let gpu = gpu();
    let pool = SurfacePool::new(
        &gpu.device,
        1,
        Size::new(W, H),
        SurfaceFormat::Bgra8Unorm,
        4,
    )
    .unwrap();
    let mean_and_row = |offset: [u32; 2]| {
        render(
            &gpu,
            &pool.surfaces[0],
            &frame(1, 1),
            DisplayParams {
                noise_offset: offset,
                ..STANDARD_SRGB
            },
        );
        let px = Pixels::of(&pool.surfaces[0]);
        let row: Vec<f32> = (0..W).map(|x| px.r(x, 100)).collect();
        (row.iter().sum::<f32>() / W as f32, row)
    };
    let (a_mean, a) = mean_and_row(DisplayParams::noise_offset_for(1));
    let (b_mean, b) = mean_and_row(DisplayParams::noise_offset_for(2));
    assert_ne!(a, b);
    assert!(
        (a_mean - b_mean).abs() * 1023.0 < 0.25,
        "{a_mean} vs {b_mean}"
    );
}

#[test]
fn a_display_p3_canvas_gets_p3_values_and_linear_input_gets_encoded() {
    let gpu = gpu();
    let pool = SurfacePool::new(
        &gpu.device,
        1,
        Size::new(W, H),
        SurfaceFormat::Rgba16Float,
        4,
    )
    .unwrap();
    // The marker is (1, 0.5, 0) in E, BT.709.
    let m = [
        [0.822_462_1, 0.177_538_0, 0.0],
        [0.033_194_1, 0.966_805_8, 0.0],
        [0.017_082_7, 0.072_397_4, 0.910_519_9],
    ];
    let linear = [
        display_decode(1.0),
        display_decode(0.5),
        display_decode(0.0),
    ];
    let want: Vec<f64> = (0..3)
        .map(|r| display_encode((0..3).map(|c| m[r][c] * linear[c]).sum()))
        .collect();
    render(
        &gpu,
        &pool.surfaces[0],
        &frame(1, 1),
        DisplayParams {
            canvas: ViewerColorSpace::DisplayP3,
            ..STANDARD_SRGB
        },
    );
    let got = Pixels::of(&pool.surfaces[0]).rgb(3040, 1540);
    for c in 0..3 {
        assert!(
            (f64::from(got[c]) - want[c]).abs() < 2e-3,
            "channel {c}: {got:?} vs {want:?}"
        );
    }
    // Linear-light working values are encoded with E on the way out.
    render(
        &gpu,
        &pool.surfaces[0],
        &frame(1, 1),
        DisplayParams {
            working: WorkingEncoding::LinearLight,
            ..STANDARD_SRGB
        },
    );
    let px = Pixels::of(&pool.surfaces[0]);
    let x = 1900;
    let v = (x as f64 + 0.5) / f64::from(W);
    assert!((f64::from(px.r(x, 100)) - display_encode(v)).abs() < 2e-3);
}

#[test]
fn the_cube_field_draws_under_the_frame_code() {
    let gpu = gpu();
    let pool = SurfacePool::new(
        &gpu.device,
        1,
        Size::new(W, H),
        SurfaceFormat::Bgra8Unorm,
        4,
    )
    .unwrap();
    let camera = EditorCamera {
        view: orbit_view(0.6, 0.35, 26.0, 0.0),
        projection: Projection::Perspective {
            fov_y_degrees: 51.566,
            near: 0.1,
            far: 200.0,
        },
    };
    let test = TestFrame {
        camera: Some(view_projection(&camera, f64::from(W) / f64::from(H))),
        ..frame(42, 9)
    };
    render(&gpu, &pool.surfaces[0], &test, STANDARD_SRGB);
    let px = Pixels::of(&pool.surfaces[0]);
    assert_eq!(px.code(), (42, 9));
    // Lit cubes: many distinct shades in the middle of the frame.
    let shades: std::collections::BTreeSet<u32> = (0..W)
        .step_by(7)
        .map(|x| (px.r(x, H / 2) * 255.0) as u32)
        .collect();
    assert!(shades.len() > 8, "{} shades", shades.len());
}
