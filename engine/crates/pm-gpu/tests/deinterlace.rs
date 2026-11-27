//! bwdif on the GPU against FFmpeg's bwdif (our LGPL build) on interlaced
//! corpus files: field-rate output, both fields of frames at the start, in
//! the middle, and at the end, compared at the source's code depth. The port
//! filters integer codes as FFmpeg does, so the output must be identical.

use pm_gpu::decode_in::deinterlace::{CodeSpace, Deinterlacer, FieldPass};
use pm_gpu::decode_in::{DecodeImporter, Normalized, required_features};
use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::probe::FieldOrder;
use pm_media::vt::DecodePurpose;
use std::path::PathBuf;

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

fn ffmpeg() -> PathBuf {
    std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        })
        .join("bin/ffmpeg")
}

fn device() -> (wgpu::Device, wgpu::Queue) {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::METAL;
    let instance = wgpu::Instance::new(desc);
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: required_features(),
        ..Default::default()
    }))
    .unwrap()
}

fn read(device: &wgpu::Device, queue: &wgpu::Queue, t: &wgpu::Texture) -> Vec<[f32; 4]> {
    let (w, h) = (t.width(), t.height());
    let bpp = t.format().block_copy_size(None).unwrap();
    let row = (w * bpp).div_ceil(256) * 256;
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
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
    queue.submit([enc.finish()]);
    let s = buf.slice(..);
    s.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let data = s.get_mapped_range().unwrap();
    let comps = (bpp / 4) as usize;
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w as usize {
            let mut v = [0f32; 4];
            for c in 0..comps {
                v[c] = f32::from_ne_bytes(
                    data[(y * row) as usize + (x * comps + c) * 4..][..4]
                        .try_into()
                        .unwrap(),
                );
            }
            out.push(v);
        }
    }
    out
}

/// Field-rate bwdif of `frames` (presented indices), compared plane by plane
/// with FFmpeg's `bwdif=mode=send_field` output. Returns (max code
/// difference, share of samples that differ).
fn compare(name: &str, pix_fmt: &str, frames: &[usize]) -> (i32, f64) {
    let path = corpus().join(name);
    let (device, queue) = device();
    let importer = DecodeImporter::new(&device).unwrap();
    let deint = Deinterlacer::new(&device);
    let src = MediaSource::open(&path).unwrap();
    let fields = &src.probe.video.as_ref().unwrap().fields;
    let tff = fields.order == FieldOrder::TopFirst;
    let (w, h) = (src.info.width as usize, src.info.height as usize);
    let (lw, lh) = (src.info.log2_chroma_w, src.info.log2_chroma_h);
    let (cw, ch) = ((w + (1 << lw) - 1) >> lw, (h + (1 << lh) - 1) >> lh);
    // FFmpeg's reference, 8-bit planar.
    let raw = std::env::temp_dir().join(format!("pm-gpu-bwdif-{}-{name}.raw", std::process::id()));
    let ok = std::process::Command::new(ffmpeg())
        .args(["-v", "error", "-y", "-i"])
        .arg(&path)
        .args([
            "-map",
            "0:v:0",
            "-vf",
            "bwdif=mode=send_field:parity=auto:deint=all",
            "-f",
            "rawvideo",
            "-pix_fmt",
            pix_fmt,
        ])
        .arg(&raw)
        .status()
        .unwrap()
        .success();
    assert!(ok, "ffmpeg bwdif");
    let reference = std::fs::read(&raw).unwrap();
    let frame_bytes = w * h + 2 * cw * ch;
    assert_eq!(
        reference.len(),
        frame_bytes * 2 * src.index.frames.len(),
        "two fields per frame"
    );
    let svc = DecodeService::new(ServiceConfig {
        cache_bytes: 64 << 20,
        ..Default::default()
    });
    let mut s = svc.session(&src, 1, DecodePurpose::Display).unwrap();
    let norm = |f: &pm_media::frame::DecodedFrame| {
        let imp = importer.import(f).unwrap();
        let mut enc = device.create_command_encoder(&Default::default());
        let n = importer.normalize(&queue, &mut enc, &imp).unwrap();
        queue.submit([enc.finish()]);
        n
    };
    let (mut worst, mut differing, mut total) = (0i32, 0usize, 0usize);
    for &n in frames {
        let ff = s.fields(n).unwrap();
        let (p, c, nx) = (norm(&ff.prev), norm(&ff.cur), norm(&ff.next));
        for second in [false, true] {
            let mut enc = device.create_command_encoder(&Default::default());
            let codes = CodeSpace::of(&ff.cur.format);
            let out = deint
                .run(
                    &queue,
                    &mut enc,
                    &p,
                    &c,
                    &nx,
                    FieldPass::new(tff, second, ff.first, ff.last),
                    &codes,
                )
                .unwrap();
            queue.submit([enc.finish()]);
            let Normalized::YCbCr { luma, chroma } = out else {
                panic!("Y'CbCr")
            };
            let l = read(&device, &queue, &luma);
            let cc = read(&device, &queue, &chroma);
            let refp = &reference[(2 * n + second as usize) * frame_bytes..][..frame_bytes];
            let code_y = |v: f32| ((v * 219.0 + 16.0).round() as i32).clamp(0, 255);
            let code_c = |v: f32| ((v * 224.0 + 128.0).round() as i32).clamp(0, 255);
            let mut rows = std::collections::BTreeMap::<usize, usize>::new();
            let mut first: Option<(usize, usize, i32, u8)> = None;
            let mut cmp = |ours: i32, theirs: u8| {
                let d = (ours - theirs as i32).abs();
                worst = worst.max(d);
                differing += (d != 0) as usize;
                total += 1;
                d
            };
            for i in 0..w * h {
                let d = cmp(code_y(l[i][0]), refp[i]);
                if d > 1 {
                    *rows.entry(i / w).or_default() += 1;
                    first.get_or_insert((i % w, i / w, code_y(l[i][0]), refp[i]));
                }
            }
            if std::env::var_os("PM_GPU_TRACE").is_some() && !rows.is_empty() {
                eprintln!(
                    "frame {n} second {second}: luma rows with >1 differences: {} (first {:?}); rows {:?}",
                    rows.len(),
                    first,
                    rows.iter().take(12).collect::<Vec<_>>()
                );
            }
            for i in 0..cw * ch {
                cmp(code_c(cc[i][0]), refp[w * h + i]);
                cmp(code_c(cc[i][1]), refp[w * h + cw * ch + i]);
            }
        }
    }
    let _ = std::fs::remove_file(&raw);
    (worst, differing as f64 / total as f64)
}

#[test]
#[ignore]
fn xdcam_1080i_top_field_first() {
    let n = 45;
    let (worst, share) = compare(
        "mxf-xdcam422-1080i-tff.mxf",
        "yuv422p",
        &[0, 1, 2, 20, n - 2, n - 1],
    );
    eprintln!(
        "XDCAM 1080i TFF: max {worst} code, {:.4}% of samples differ",
        share * 100.0
    );
    assert_eq!(
        worst, 0,
        "bwdif must equal FFmpeg's: max difference {worst} codes"
    );
}

#[test]
#[ignore]
fn dv_ntsc_bottom_field_first() {
    let (worst, share) = compare("dv-ntsc.dv", "yuv411p", &[0, 1, 2, 30, 58, 59]);
    eprintln!(
        "DV NTSC BFF 4:1:1: max {worst} code, {:.4}% of samples differ",
        share * 100.0
    );
    assert_eq!(
        worst, 0,
        "bwdif must equal FFmpeg's: max difference {worst} codes"
    );
}
