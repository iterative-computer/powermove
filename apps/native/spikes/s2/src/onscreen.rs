//! Opt-in visible workload; uses the same distinct texture factory as --verify.
use gpui::*;
use std::{sync::Arc, time::Instant};
#[derive(Debug)]
struct Lease;
impl TextureLease for Lease {
    fn gpu_completed(&self, success: bool) {
        assert!(success);
    }
}
pub struct Fixture {
    device: metal::Device,
    tiles: Vec<Arc<NativeTexture>>,
    producer: Arc<NativeTexture>,
    started: Instant,
    producer_frame: u64,
    pub frames: usize,
}
impl Fixture {
    pub fn new(device: &metal::Device) -> Self {
        let tiles = (0..1000)
            .map(|i| {
                Self::texture(
                    device,
                    &[
                        (40 + i % 170) as u8,
                        (40 + (i / 40) % 160) as u8,
                        (60 + (i % 40) * 4) as u8,
                        255,
                    ],
                )
            })
            .collect::<Vec<_>>();
        Self {
            device: device.clone(),
            producer: tiles[0].clone(),
            tiles,
            started: Instant::now(),
            producer_frame: 0,
            frames: 0,
        }
    }
    fn texture(device: &metal::Device, pixel: &[u8]) -> Arc<NativeTexture> {
        NativeTexture::new(
            crate::verify::make_texture(device, metal::MTLPixelFormat::BGRA8Unorm, pixel),
            device.new_event(),
            0,
            Arc::new(Lease),
        )
        .unwrap()
    }
    pub fn paint(&mut self, w: f32, h: f32, window: &mut Window) {
        self.frames += 1;
        let frame = (self.started.elapsed().as_secs_f64() * 60.) as u64;
        if frame != self.producer_frame {
            // Allocate a fresh source so an outstanding GPU consumer is never overwritten.
            self.producer = Self::texture(&self.device, &[frame as u8, 80, 160, 255]);
            self.producer_frame = frame;
        }
        let top = crate::glass::toolbar_inset().max(52.) as f32 + 12.;
        let tw = (w - 510.).max(80.) / 40.;
        let th = (h - top - 208.).max(100.) / 25.;
        for (i, t) in self.tiles.iter().enumerate() {
            window.paint_texture(
                Bounds::new(
                    point(
                        px(204. + (i % 40) as f32 * tw),
                        px(top + (i / 40) as f32 * th),
                    ),
                    size(px(tw - 1.), px(th - 1.)),
                ),
                t.clone(),
            );
        }
        window.paint_texture(
            Bounds::new(point(px(440.), px(230.)), size(px(200.), px(170.))),
            self.producer.clone(),
        );
        if self.frames == 650 {
            println!(
                "S2_WORKLOAD frames=650 distinct_tiles=1000 producer_hz=60 elapsed_s={:.3}",
                self.started.elapsed().as_secs_f64()
            );
        }
    }
}
