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
    pub fn paint_tiles(&self, w: f32, h: f32, window: &mut Window) {
        let top = crate::glass::toolbar_inset().max(52.) as f32 + 12.;
        let tw = (w - 510.).max(80.) / 40.;
        let th = (h - top - 208.).max(100.) / 25.;
        let paint = |window: &mut Window| {
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
        };
        // Non-overlapping tiles share one stacking layer. Cached scene replay
        // otherwise reinserts 1,000 bounds into the order tree on every frame.
        // Keep the ungrouped path for an identical-pixels A/B control.
        if std::env::var_os("S2_UNGROUPED_TILES").is_some() {
            paint(window);
        } else {
            window.paint_layer(
                Bounds::new(point(px(204.), px(top)), size(px(40. * tw), px(25. * th))),
                paint,
            );
        }
    }

    pub fn paint(&mut self, _w: f32, _h: f32, window: &mut Window) {
        self.frames += 1;
        let frame = (self.started.elapsed().as_secs_f64() * 60.) as u64;
        if frame != self.producer_frame {
            // Allocate a fresh source so an outstanding GPU consumer is never overwritten.
            self.producer = Self::texture(&self.device, &[frame as u8, 80, 160, 255]);
            self.producer_frame = frame;
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

/// Only this entity owns the static 1,000-tile subtree. Parent animation doesn't dirty it.
pub struct TileView {
    pub fixture: std::rc::Rc<std::cell::RefCell<Fixture>>,
    pub renders: usize,
}
impl gpui::Render for TileView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        println!("S2_TILE_RENDER {}", self.renders);
        let fixture = self.fixture.clone();
        canvas(
            |_, _, _| (),
            move |b, _, w, _| {
                fixture
                    .borrow()
                    .paint_tiles(f32::from(b.size.width), f32::from(b.size.height), w)
            },
        )
        .size_full()
    }
}
