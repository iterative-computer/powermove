//! Real Metal tests, runnable with a locked desktop. No AppKit/WebKit proof implied.
use anyhow::{Result, ensure};
use gpui::*;
use gpui_apple::metal_renderer::{InstanceBufferPool, MetalRenderer};
use parking_lot::Mutex;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Debug)]
struct Lease {
    completed: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}
impl TextureLease for Lease {
    fn gpu_completed(&self, success: bool) {
        assert!(success);
        self.completed.fetch_add(1, Ordering::SeqCst);
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
    Bounds::new(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(w), ScaledPixels(h)),
    )
}
fn make_texture(
    device: &metal::Device,
    format: metal::MTLPixelFormat,
    pixel: &[u8],
) -> metal::Texture {
    let d = metal::TextureDescriptor::new();
    d.set_width(2);
    d.set_height(2);
    d.set_pixel_format(format);
    d.set_storage_mode(metal::MTLStorageMode::Shared);
    d.set_usage(metal::MTLTextureUsage::ShaderRead);
    let t = device.new_texture(&d);
    let data = pixel.repeat(4);
    t.replace_region(
        metal::MTLRegion::new_2d(0, 0, 2, 2),
        0,
        data.as_ptr().cast(),
        (pixel.len() * 2) as u64,
    );
    t
}
fn scene(source: Arc<NativeTexture>, tiles: usize) -> Scene {
    let mut s = Scene::default();
    for i in 0..tiles {
        let b = if tiles == 1 {
            rect(0., 0., 64., 64.)
        } else {
            rect((i % 40) as f32 * 16., (i / 40) as f32 * 16., 16., 16.)
        };
        s.insert_primitive(PaintSurface {
            order: 0,
            bounds: b,
            content_mask: ContentMask {
                bounds: rect(0., 0., 640., 400.),
                ..Default::default()
            },
            source: source.clone().into(),
        });
    }
    s.finish();
    s
}
fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}
pub fn run() -> Result<()> {
    objc2::rc::autoreleasepool(|_| run_inner())
}
fn run_inner() -> Result<()> {
    std::fs::create_dir_all("/tmp/pm-shots")?;
    let device =
        metal::Device::system_default().ok_or_else(|| anyhow::anyhow!("Metal unavailable"))?;
    let mut renderer = MetalRenderer::new_headless_with_device(
        Arc::new(Mutex::new(InstanceBufferPool::default())),
        device.clone(),
    );
    let queue = device.new_command_queue();
    let completed = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let lease = || {
        Arc::new(Lease {
            completed: completed.clone(),
            dropped: dropped.clone(),
        }) as Arc<dyn TextureLease>
    };
    let texture = make_texture(
        &device,
        metal::MTLPixelFormat::BGRA8Unorm,
        &[0, 0, 128, 128],
    );
    let event = device.new_event();
    let signal = queue.new_command_buffer();
    signal.encode_signal_event(&event, 1);
    signal.commit();
    let source = NativeTexture::new(texture, event, 1, lease()).map_err(anyhow::Error::msg)?;
    let s = scene(source.clone(), 1);
    let image = renderer.render_scene_to_image(&s, size(DevicePixels(64), DevicePixels(64)))?;
    image.save("/tmp/pm-shots/s2-texture-bgra.png")?;
    ensure!(
        image.get_pixel(32, 32).0 == [128, 0, 0, 255],
        "premultiplied BGRA blend: {:?}",
        image.get_pixel(32, 32)
    );
    println!(
        "PASS BGRA8 premultiplied alpha: {:?}",
        image.get_pixel(32, 32)
    );
    // RGBA16F .5,0,0,.5 is already SDR encoded by producer; no HDR claim.
    let half = make_texture(
        &device,
        metal::MTLPixelFormat::RGBA16Float,
        &[0, 0x38, 0, 0, 0, 0, 0, 0x38],
    );
    let half_source =
        NativeTexture::new(half, device.new_event(), 0, lease()).map_err(anyhow::Error::msg)?;
    let half_image = renderer.render_scene_to_image(
        &scene(half_source, 1),
        size(DevicePixels(64), DevicePixels(64)),
    )?;
    half_image.save("/tmp/pm-shots/s2-texture-rgba16f.png")?;
    ensure!(
        half_image.get_pixel(32, 32).0 == [128, 0, 0, 255],
        "RGBA16F blend"
    );
    println!("PASS RGBA16F converted SDR alpha");
    // Deliberately delay producer signal on another CPU thread. GPU work must hold
    // the lease after all scene/UI references disappear, and never wait on CPU paint.
    let done = Arc::new(AtomicUsize::new(0));
    let gone = Arc::new(AtomicUsize::new(0));
    let event = device.new_event();
    let signal_event = event.clone();
    let delayed = NativeTexture::new(
        make_texture(
            &device,
            metal::MTLPixelFormat::BGRA8Unorm,
            &[0, 255, 0, 255],
        ),
        event,
        1,
        Arc::new(Lease {
            completed: done.clone(),
            dropped: gone.clone(),
        }),
    )
    .map_err(anyhow::Error::msg)?;
    let delayed_scene = scene(delayed.clone(), 1);
    let start = Instant::now();
    renderer.render_scene(&delayed_scene, size(DevicePixels(64), DevicePixels(64)))?;
    ensure!(
        start.elapsed() < Duration::from_millis(100),
        "GPU fence blocked CPU submission"
    );
    drop(delayed);
    drop(delayed_scene);
    ensure!(
        done.load(Ordering::SeqCst) == 0 && gone.load(Ordering::SeqCst) == 0,
        "lease released before consumer completion"
    );
    std::thread::sleep(Duration::from_millis(120));
    ensure!(
        gone.load(Ordering::SeqCst) == 0,
        "lease did not survive producer delay"
    );
    let q = device.new_command_queue();
    let signal = q.new_command_buffer();
    signal.encode_signal_event(&signal_event, 1);
    signal.commit();
    signal.wait_until_completed();
    let deadline = Instant::now() + Duration::from_secs(3);
    while gone.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    ensure!(
        done.load(Ordering::SeqCst) == 1 && gone.load(Ordering::SeqCst) == 1,
        "consumer completion/drop missing"
    );
    println!(
        "PASS delayed GPU completion: 120 ms producer delay, scene dropped before signal, lease released exactly once"
    );
    // Cached scenes can be submitted repeatedly and each submission has a completion.
    let before = completed.load(Ordering::SeqCst);
    for _ in 0..3 {
        renderer.render_scene_to_image(&s, size(DevicePixels(64), DevicePixels(64)))?;
    }
    ensure!(
        completed.load(Ordering::SeqCst) >= before + 3,
        "cached scene completion missing"
    );
    println!("PASS repeated scene completion");
    let mut clipped = scene(source.clone(), 1);
    clipped.surfaces[0].content_mask.bounds = rect(16., 16., 32., 32.);
    let clip =
        renderer.render_scene_to_image(&clipped, size(DevicePixels(64), DevicePixels(64)))?;
    ensure!(
        clip.get_pixel(1, 1).0 == [0, 0, 0, 255] && clip.get_pixel(32, 32).0 == [128, 0, 0, 255],
        "texture clip"
    );
    println!("PASS texture clip");
    let mut tiles = Scene::default();
    for i in 0..1000 {
        let pixel = [
            (40 + i % 170) as u8,
            (40 + (i / 40) % 160) as u8,
            (60 + (i % 40) * 4) as u8,
            255,
        ];
        let tile = NativeTexture::new(
            make_texture(&device, metal::MTLPixelFormat::BGRA8Unorm, &pixel),
            device.new_event(),
            0,
            lease(),
        )
        .map_err(anyhow::Error::msg)?;
        tiles.insert_primitive(PaintSurface {
            order: 0,
            bounds: rect((i % 40) as f32 * 16., (i / 40) as f32 * 16., 15., 15.),
            content_mask: ContentMask {
                bounds: rect(0., 0., 640., 400.),
                ..Default::default()
            },
            source: tile.into(),
        });
    }
    tiles.finish();
    renderer
        .render_scene_to_image(&tiles, size(DevicePixels(640), DevicePixels(400)))?
        .save("/tmp/pm-shots/s2-1000-tiles.png")?;
    let mut encode = Vec::new();
    let mut complete = Vec::new();
    for i in 0..180 {
        let start = Instant::now();
        renderer.render_scene(&tiles, size(DevicePixels(640), DevicePixels(400)))?;
        let submitted = start.elapsed().as_secs_f64() * 1000.;
        // A capture waits for all preceding work, bounding outstanding submissions.
        renderer.render_scene_to_image(&s, size(DevicePixels(64), DevicePixels(64)))?;
        if i >= 30 {
            encode.push(submitted);
            complete.push(start.elapsed().as_secs_f64() * 1000.);
        }
    }
    let encode95 = percentile(&mut encode, 0.95);
    let complete95 = percentile(&mut complete, 0.95);
    println!(
        "MEASURE 1000 tiles, 150 post-warmup frames: encode p95={encode95:.3} ms, submission+drain p95={complete95:.3} ms (unpaced headless, not a 60 fps display gate)"
    );
    // 60 fps producer stand-in while all 1,000 distinct tiles are sampled.
    let paced_start = Instant::now();
    let mut late = 0;
    let mut paced_times = Vec::new();
    for i in 0..120 {
        let deadline = paced_start + Duration::from_secs_f64(i as f64 / 60.);
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        let start = Instant::now();
        let animated = NativeTexture::new(
            make_texture(
                &device,
                metal::MTLPixelFormat::BGRA8Unorm,
                &[i as u8, 80, 160, 255],
            ),
            device.new_event(),
            0,
            lease(),
        )
        .map_err(anyhow::Error::msg)?;
        let mut frame = scene(animated, 1);
        frame.surfaces[0].bounds = rect(440., 230., 200., 170.);
        // Ordered primitive list from the 1,000-tile scene plus animated frame.
        let mut mixed = Scene::default();
        for tile in &tiles.surfaces {
            mixed.insert_primitive(tile.clone());
        }
        mixed.insert_primitive(frame.surfaces[0].clone());
        mixed.finish();
        renderer.render_scene(&mixed, size(DevicePixels(640), DevicePixels(400)))?;
        renderer.render_scene_to_image(&frame, size(DevicePixels(640), DevicePixels(400)))?;
        let elapsed = start.elapsed();
        if elapsed > Duration::from_secs_f64(1. / 60.) {
            late += 1;
        }
        paced_times.push(elapsed.as_secs_f64() * 1000.);
    }
    println!(
        "MEASURE paced 60 fps producer + 1000 distinct tiles: 120 frames, work p95={:.3} ms, work over 16.667 ms={late} (offscreen; no scanout claim)",
        percentile(&mut paced_times, 0.95)
    );
    let mut rows = Vec::new();
    for f in 0..10000 {
        let start = Instant::now();
        let r = crate::contract::visible_rows(5000, ((f * 17) % 119400) as f64, 600., 24.);
        std::hint::black_box(r);
        rows.push(start.elapsed().as_secs_f64() * 1000.);
    }
    println!(
        "MEASURE 5000 logical rows range calculation p95={:.6} ms; not GPUI frame-time / 120 Hz proof",
        percentile(&mut rows, 0.95)
    );
    source.retire();
    ensure!(
        renderer
            .render_scene(&s, size(DevicePixels(64), DevicePixels(64)))
            .is_err(),
        "retired texture accepted"
    );
    println!("PASS retired/device-loss source rejection");
    Ok(())
}
