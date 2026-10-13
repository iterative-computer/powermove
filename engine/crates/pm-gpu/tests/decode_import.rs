//! Zero-copy import and section 13 normalization on the GPU: every backend's
//! frames (VideoToolbox `420v`, `x420`, `x422`, `v216`, `y416`, `v0a8`;
//! FFmpeg planar LSB formats including 4:1:1 DV and planar RGB; dav1d; the
//! paired alpha decoders; image readers) reach wgpu as textures over the
//! decoder's memory, and the GPU normalization equals the f64 CPU reference
//! rounded to f32 within 1 ULP (Metal division is not correctly rounded; S6).

use pm_gpu::decode_in::{DecodeImporter, Normalized, required_features};
use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::decode::{Backend, backends};
use pm_media::frame::DecodedFrame;
use pm_media::vt::DecodePurpose;
use std::path::{Path, PathBuf};

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    importer: DecodeImporter,
}

fn gpu() -> Gpu {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::METAL;
    let instance = wgpu::Instance::new(desc);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("a Metal adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("pm-gpu decode import test"),
        required_features: required_features(),
        ..Default::default()
    }))
    .expect("a device");
    let importer = DecodeImporter::new(&device).expect("importer");
    Gpu {
        device,
        queue,
        importer,
    }
}

fn read_texture(g: &Gpu, t: &wgpu::Texture) -> Vec<[f32; 4]> {
    let (w, h) = (t.width(), t.height());
    let bpp = t.format().block_copy_size(None).unwrap();
    let row = (w * bpp).div_ceil(256) * 256;
    let buf = g.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = g.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        t.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    g.queue.submit([enc.finish()]);
    let s = buf.slice(..);
    s.map_async(wgpu::MapMode::Read, |_| {});
    g.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let data = s.get_mapped_range().unwrap();
    let comps = (bpp / 4) as usize;
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let r = &data[(y * row) as usize..][..(w * bpp) as usize];
        for x in 0..w as usize {
            let mut v = [0f32; 4];
            for c in 0..comps {
                v[c] = f32::from_ne_bytes(r[(x * comps + c) * 4..][..4].try_into().unwrap());
            }
            out.push(v);
        }
    }
    out
}

fn ulps(a: f32, b: f32) -> u32 {
    let k = |x: f32| {
        let i = x.to_bits() as i32;
        if i < 0 { i32::MIN.wrapping_sub(i) } else { i }
    };
    (k(a) as i64 - k(b) as i64)
        .unsigned_abs()
        .min(u32::MAX as u64) as u32
}

/// Imports, normalizes, and compares with the CPU reference; returns the
/// worst ULP difference.
fn check_frame(g: &Gpu, f: &DecodedFrame, what: &str) -> u32 {
    let imp = g
        .importer
        .import(f)
        .unwrap_or_else(|e| panic!("{what}: import: {e}"));
    assert!(
        imp.aliased,
        "{what}: textures do not alias the decoder's memory"
    );
    assert_eq!(imp.textures.len(), f.format.planes.len());
    let mut enc = g.device.create_command_encoder(&Default::default());
    let n = g.importer.normalize(&g.queue, &mut enc, &imp).unwrap();
    g.queue.submit([enc.finish()]);
    let (luma, chroma) = f.reference().unwrap();
    let mut worst = 0;
    match n {
        Normalized::YCbCr { luma: l, chroma: c } => {
            let gl = read_texture(g, &l);
            let gc = read_texture(g, &c);
            assert_eq!(gl.len(), luma.len());
            assert_eq!(gc.len(), chroma.len());
            for (gv, cv) in gl.iter().zip(&luma) {
                worst = worst.max(ulps(gv[0], cv[0])).max(ulps(gv[3], cv[1]));
            }
            for (gv, cv) in gc.iter().zip(&chroma) {
                worst = worst.max(ulps(gv[0], cv[0])).max(ulps(gv[1], cv[1]));
            }
        }
        Normalized::Rgb { rgba } => {
            // The CPU reference holds [G, A] and [B, R] for RGB layouts.
            let gv = read_texture(g, &rgba);
            for (i, v) in gv.iter().enumerate() {
                worst = worst
                    .max(ulps(v[1], luma[i][0]))
                    .max(ulps(v[3], luma[i][1]));
                worst = worst
                    .max(ulps(v[2], chroma[i][0]))
                    .max(ulps(v[0], chroma[i][1]));
            }
        }
    }
    g.importer.flush_cache();
    worst
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../apps/desktop/e2e/fixtures")
}

fn corpus() -> PathBuf {
    let d = std::env::var_os("PM_MEDIA_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/media-corpus")
        });
    assert!(
        d.is_dir(),
        "run engine/crates/pm-media/tests/corpus/gen.sh first"
    );
    d
}

/// Every backend of a file: three frames each (first, middle, last).
fn run(g: &Gpu, path: &Path, purpose: DecodePurpose) -> Vec<String> {
    let src = MediaSource::open(path).unwrap();
    let svc = DecodeService::new(ServiceConfig {
        cache_bytes: 0,
        ..Default::default()
    });
    let mut lines = vec![];
    let n = src.index.frames.len();
    for b in backends(&src.info, &src.index) {
        let mut s = match svc.session_with(&src, 1, purpose, Some(b)) {
            Ok(s) => s,
            Err(e) if b != Backend::Ffmpeg => {
                lines.push(format!("{}: {b:?} unavailable: {e}", path.display()));
                continue;
            }
            Err(e) => panic!("{}: {e}", path.display()),
        };
        let mut worst = 0;
        let mut label = String::new();
        for k in [0, n / 2, n - 1] {
            let f = s.frame(k).unwrap();
            label = f.format.label.clone();
            worst = worst.max(check_frame(
                g,
                &f,
                &format!("{} [{b:?}] frame {k}", path.display()),
            ));
        }
        assert!(
            worst <= 1,
            "{} [{b:?} {label}]: GPU normalization differs from the CPU reference by {worst} ULP",
            path.display()
        );
        lines.push(format!(
            "{} [{b:?} {label}]: aliased, max {worst} ULP",
            path.file_name().unwrap().to_string_lossy()
        ));
    }
    lines
}

#[test]
fn e2e_fixtures() {
    let g = gpu();
    let mut files: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("mp4" | "mov" | "mkv" | "webm")
            )
        })
        .collect();
    files.sort();
    for f in files {
        for l in run(&g, &f, DecodePurpose::Display) {
            eprintln!("{l}");
        }
    }
}

#[test]
fn still_images() {
    let g = gpu();
    let pool = pm_media::pool::FramePool::new(0);
    for name in [
        "still-red.png",
        "still-alpha.png",
        "still.tiff",
        "still.webp",
        "still.heic",
        "still.avif",
    ] {
        let img = pm_media::image::read(&fixtures().join(name), &pool).unwrap();
        // Image readers copy into pool memory once (decoded pixels are not
        // GPU-shareable as they come out of the decoders); the import itself
        // is zero-copy over that memory.
        let imp = g.importer.import(&img.frame).unwrap();
        assert_eq!(imp.textures.len(), 1);
        let mut enc = g.device.create_command_encoder(&Default::default());
        let Normalized::Rgb { rgba } = g.importer.normalize(&g.queue, &mut enc, &imp).unwrap()
        else {
            panic!("{name}: RGB expected")
        };
        g.queue.submit([enc.finish()]);
        let gv = read_texture(&g, &rgba);
        let (l, c) = img.frame.reference().unwrap();
        let worst = gv.iter().enumerate().fold(0, |w, (i, v)| {
            w.max(ulps(v[1], l[i][0]))
                .max(ulps(v[3], l[i][1]))
                .max(ulps(v[2], c[i][0]))
                .max(ulps(v[0], c[i][1]))
        });
        assert!(worst <= 1, "{name}: {worst} ULP");
    }
}

/// Planar RGB (FFmpeg `gbrp10le` from DPX) normalizes to R, G, B, A.
#[test]
fn planar_rgb_dpx() {
    let g = gpu();
    let dir = std::env::temp_dir().join(format!("pm-gpu-dpx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dpx = dir.join("frame.dpx");
    let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        })
        .join("bin/ffmpeg");
    let ok = std::process::Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x36:rate=1",
            "-frames:v",
            "1",
            "-pix_fmt",
            "gbrp10le",
        ])
        .arg(&dpx)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let pool = pm_media::pool::FramePool::new(0);
    let img = pm_media::image::read(&dpx, &pool).unwrap();
    assert_eq!(img.frame.format.label, "gbrp10le");
    let worst = check_frame(&g, &img.frame, "frame.dpx");
    assert!(worst <= 1, "{worst} ULP");
}

#[test]
#[ignore]
fn corpus_layouts() {
    let g = gpu();
    let c = corpus();
    for (name, purpose) in [
        ("prores422hq-vt.mov", DecodePurpose::Export),   // v216
        ("prores4444xq-vt.mov", DecodePurpose::Display), // y416
        ("hevc-main10.mov", DecodePurpose::Display),     // x420 and yuv420p10le
        ("hevc-422-10.mov", DecodePurpose::Display),     // x422 and yuv422p10le
        ("h264-422-10.mov", DecodePurpose::Display),
        ("av1-10bit.mp4", DecodePurpose::Display), // VideoToolbox AV1 and dav1d
        ("vp9-alpha.webm", DecodePurpose::Display),
        ("vp8-alpha.webm", DecodePurpose::Display),
        ("vp9-10bit.webm", DecodePurpose::Display),
        ("dnxhr-444.mxf", DecodePurpose::Display),
        ("cineform.mov", DecodePurpose::Display),
        ("dv-ntsc.dv", DecodePurpose::Display), // yuv411p
        ("mjpeg.avi", DecodePurpose::Display),  // yuvj422p, full range
        ("uhd-prores422hq.mov", DecodePurpose::Display),
    ] {
        for l in run(&g, &c.join(name), purpose) {
            eprintln!("{l}");
        }
    }
}
