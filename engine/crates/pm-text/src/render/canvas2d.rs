//! Canvas2D's macOS A8 masks: CoreGraphics smoothing followed by Skia's
//! measured paint-luminance preblend. Kept local to legacy, axis-aligned
//! glyph strikes; native level 2 and Exact outlines keep float area coverage.
use super::{
    coretext::{CoreTextReference, RefRun},
    outline::FaceRef,
};
use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_core_text::CTFont;
use std::{
    collections::HashMap,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::NonNull,
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

#[derive(Clone, Copy)]
struct MaskStyle {
    quarter: u8,
    skew: f32,
}
pub(crate) struct GlyphMask {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub coverage: Vec<u8>,
}
type Key = (u64, u32, u16, u32, Vec<(u32, u32)>, u8, u32, Option<u64>);
/// Glyph masks kept between paints. Masks are small and evicted together;
/// the fonts they are drawn with are bounded on their own
/// (`CoreTextReference`) and survive mask eviction.
const MASK_BYTES: usize = 8 << 20;
#[derive(Default)]
pub(crate) struct CanvasMasks {
    fonts: CoreTextReference,
    masks: HashMap<Key, Arc<GlyphMask>>,
    bytes: usize,
}
// SAFETY: immutable CTFont/CGFont objects are thread-safe; contexts are created and
// destroyed within one call. The cache moves with its owning render thread.
unsafe impl Send for CanvasMasks {}

/// A CTFont shared with the mask workers.
struct SharedFont(CFRetained<CTFont>);
// SAFETY: objc2 leaves CTFont !Send/!Sync because it is toll-free bridged
// with NSFont, which may be subclassed. These fonts are made here by
// CoreText (`CoreTextReference`), never by AppKit, and are immutable. Core
// Text Programming Guide, "Common Operations" / thread safety: "Font objects
// (CTFont, CTFontDescriptor, and associated objects) can be used
// simultaneously by multiple operations, work queues, or threads."
// CFRetain/CFRelease are atomic, so clones and drops may happen on any
// thread.
unsafe impl Send for SharedFont {}
unsafe impl Sync for SharedFont {}

/// One CoreGraphics mask to draw: a pure function of these fields.
pub(super) struct MaskJob {
    font: SharedFont,
    glyph: u16,
    style: MaskStyle,
    bounds: kurbo::Rect,
}
impl MaskJob {
    fn run(&self) -> Option<GlyphMask> {
        draw(&self.font.0, self.glyph, self.style, self.bounds)
    }
}

impl MaskJob {
    pub(super) fn rasterize(&self) -> Option<GlyphMask> {
        objc2::rc::autoreleasepool(|_| self.run())
    }

    pub(super) fn geometry_key(&self) -> [u64; 4] {
        [
            self.bounds.x0,
            self.bounds.y0,
            self.bounds.x1,
            self.bounds.y1,
        ]
        .map(f64::to_bits)
    }

    fn bytes(&self) -> usize {
        mask_rect(self.bounds).map_or(0, |(_, _, w, h)| w * h)
    }
}

/// Masks a paint still has to draw, deduplicated by key; drawn together
/// (in parallel when there are enough) by [`CanvasMasks::resolve`].
#[derive(Default)]
pub(crate) struct PendingMasks {
    jobs: Vec<MaskJob>,
    keys: Vec<Key>,
    index: HashMap<Key, usize>,
    font_failed: bool,
}
impl PendingMasks {
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub(super) fn jobs(&self) -> impl Iterator<Item = (&Key, &MaskJob)> {
        self.keys.iter().zip(&self.jobs)
    }

    pub(super) fn reserve_bytes(&self) -> Option<usize> {
        if self.font_failed {
            return None;
        }
        self.jobs
            .iter()
            .try_fold(0usize, |bytes, job| bytes.checked_add(job.bytes()))
    }
}

/// A mask lookup: drawn or known, or the index of a pending job.
pub(crate) enum MaskLookup {
    Ready(Option<Arc<GlyphMask>>),
    Pending(usize),
}

impl CanvasMasks {
    /// Drops every mask and font (idle `forget_glyphs`, critical memory).
    pub fn clear(&mut self) {
        self.evict_masks();
        self.fonts = CoreTextReference::default();
    }
    /// Drops the masks only: a full mask cache keeps its fonts and their
    /// providers, so the next misses do not copy and parse fonts again.
    fn evict_masks(&mut self) {
        self.masks.clear();
        self.bytes = 0;
    }
    fn keep(&mut self, key: Key, mask: Arc<GlyphMask>) {
        if self.bytes >= MASK_BYTES {
            self.evict_masks();
        }
        self.bytes += mask.coverage.len();
        self.masks.insert(key, mask);
    }
    #[allow(clippy::too_many_arguments)]
    fn key(
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u16,
        size: f32,
        quarter: u8,
        skew: f32,
        bold_path: Option<(&kurbo::BezPath, f64)>,
    ) -> Key {
        (
            face.id,
            face.index,
            glyph,
            size.to_bits(),
            coords.iter().map(|(t, v)| (*t, v.to_bits())).collect(),
            quarter,
            skew.to_bits(),
            // Fake-bold stroke width depends on the source font size before
            // its device transform; equal strike sizes can have other widths.
            bold_path.map(|(_, scale)| scale.to_bits()),
        )
    }
    /// The mask, drawing it now (one-glyph [`Self::lookup`] + [`Self::resolve`]).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn get(
        &mut self,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u16,
        size: f32,
        quarter: u8,
        skew: f32,
        bounds: kurbo::Rect,
        bold_path: Option<(&kurbo::BezPath, f64)>,
    ) -> Option<Arc<GlyphMask>> {
        let mut pending = PendingMasks::default();
        match self.lookup(
            &mut pending,
            face,
            coords,
            glyph,
            size,
            quarter,
            skew,
            bounds,
            bold_path,
        ) {
            MaskLookup::Ready(mask) => mask,
            MaskLookup::Pending(job) => self.resolve(pending).swap_remove(job),
        }
    }
    /// The cached mask, or a job in `pending` that [`Self::resolve`] draws.
    /// Fake-bold path masks are drawn at once. A key already pending in
    /// this batch gets its job: equal keys are equal masks.
    #[allow(clippy::too_many_arguments)]
    pub fn lookup(
        &mut self,
        pending: &mut PendingMasks,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u16,
        size: f32,
        quarter: u8,
        skew: f32,
        bounds: kurbo::Rect,
        bold_path: Option<(&kurbo::BezPath, f64)>,
    ) -> MaskLookup {
        let key = Self::key(face, coords, glyph, size, quarter, skew, bold_path);
        if let Some(mask) = self.masks.get(&key) {
            return MaskLookup::Ready(Some(mask.clone()));
        }
        if let Some(&job) = pending.index.get(&key) {
            return MaskLookup::Pending(job);
        }
        match bold_path {
            Some((path, scale)) => {
                let mask = draw_path(path, scale, quarter, bounds).map(Arc::new);
                if let Some(mask) = &mask {
                    self.keep(key, mask.clone());
                }
                MaskLookup::Ready(mask)
            }
            None => {
                let run = RefRun {
                    data: face.data,
                    index: face.index,
                    size,
                    coords,
                    glyphs: vec![],
                    color: [0., 0., 0., 1.],
                };
                // Fonts resolve here, on the caller's thread, in paint order.
                let Some(font) = self.fonts.font_of(face.id, &run) else {
                    pending.font_failed = true;
                    return MaskLookup::Ready(None);
                };
                let job = pending.jobs.len();
                pending.jobs.push(MaskJob {
                    font: SharedFont(font),
                    glyph,
                    style: MaskStyle { quarter, skew },
                    bounds,
                });
                pending.keys.push(key.clone());
                pending.index.insert(key, job);
                MaskLookup::Pending(job)
            }
        }
    }
    /// Each row looked up keys against the same cache snapshot. Reuse
    /// that preparation only while all its original hits remain valid.
    pub(super) fn can_admit_wave(&self, bytes: usize) -> bool {
        bytes == 0 || self.bytes.saturating_add(bytes) <= MASK_BYTES
    }

    pub(super) fn admit_row(
        &mut self,
        pending: &PendingMasks,
        mut get: impl FnMut(usize) -> Option<Arc<GlyphMask>>,
    ) -> Vec<Option<Arc<GlyphMask>>> {
        pending
            .keys
            .iter()
            .enumerate()
            .map(|(i, key)| {
                if let Some(mask) = self.masks.get(key) {
                    // A previous row may have admitted this key. A failed
                    // result was never cached, so it cannot hide a later retry.
                    return Some(mask.clone());
                }
                let mask = get(i)?;
                self.keep(key.clone(), mask.clone());
                Some(mask)
            })
            .collect()
    }

    /// Draws the pending masks, keeps the drawn ones, and returns them by
    /// job index (`None` where CoreGraphics could not draw).
    pub fn resolve(&mut self, pending: PendingMasks) -> Vec<Option<Arc<GlyphMask>>> {
        let PendingMasks { jobs, keys, .. } = pending;
        rasterize(jobs)
            .into_iter()
            .zip(keys)
            .map(|(mask, key)| {
                let mask = mask.map(Arc::new);
                if let Some(mask) = &mask {
                    self.keep(key, mask.clone());
                }
                mask
            })
            .collect()
    }
}

/// Fewer jobs than this draw on the caller's thread: waking a worker costs
/// about as much as one small mask.
const PARALLEL_JOBS: usize = 3;
/// Mask workers besides the caller. The render thread draws too, and other
/// engine threads (decode, GPU submission) need cores.
const MAX_HELPERS: usize = 3;

/// One batch of jobs shared by the caller and the workers. Each claims the
/// next job index and reports exactly one result for it.
struct Batch {
    jobs: Vec<MaskJob>,
    next: AtomicUsize,
    done: mpsc::Sender<(usize, std::thread::Result<Option<GlyphMask>>)>,
}
impl Batch {
    fn work(&self) {
        // Workers have no pool of their own; anything CoreText autoreleases
        // is drained per batch.
        objc2::rc::autoreleasepool(|_| {
            loop {
                let i = self.next.fetch_add(1, Ordering::Relaxed);
                let Some(job) = self.jobs.get(i) else { break };
                let mask = catch_unwind(AssertUnwindSafe(|| job.run()));
                // The caller holds the receiver until every job reported.
                let _ = self.done.send((i, mask));
            }
        });
    }
}

struct MaskPool {
    tickets: Mutex<mpsc::Sender<Arc<Batch>>>,
    helpers: usize,
}

/// The process's mask workers, started on first use; `None` on one core or
/// when no thread could start. They block on the ticket queue when idle.
fn mask_pool() -> Option<&'static MaskPool> {
    static POOL: OnceLock<Option<MaskPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let helpers = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .saturating_sub(1)
            .min(MAX_HELPERS);
        let (tickets, queue) = mpsc::channel::<Arc<Batch>>();
        let queue = Arc::new(Mutex::new(queue));
        let mut started = 0;
        for i in 0..helpers {
            let queue = queue.clone();
            let worker = std::thread::Builder::new()
                .name(format!("pm-text-masks-{i}"))
                .spawn(move || {
                    loop {
                        // The guard is released at the end of this statement,
                        // before the batch runs.
                        let batch =
                            match queue.lock().unwrap_or_else(PoisonError::into_inner).recv() {
                                Ok(batch) => batch,
                                Err(_) => return,
                            };
                        batch.work();
                    }
                });
            if worker.is_ok() {
                started += 1;
            }
        }
        (started > 0).then(|| MaskPool {
            tickets: Mutex::new(tickets),
            helpers: started,
        })
    })
    .as_ref()
}

#[cfg(test)]
thread_local! {
    /// Tests: draw every batch on the caller's thread.
    pub(super) static SERIAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
static PARALLEL_BATCHES: AtomicUsize = AtomicUsize::new(0);

/// Draws `jobs`, returning their masks in job order. Each mask is drawn in
/// its own bitmap context by one thread, so the masks do not depend on how
/// the jobs are spread over threads.
fn rasterize(jobs: Vec<MaskJob>) -> Vec<Option<GlyphMask>> {
    #[cfg(test)]
    let serial = SERIAL.with(|s| s.get());
    #[cfg(not(test))]
    let serial = false;
    let pool = if serial || jobs.len() < PARALLEL_JOBS {
        None
    } else {
        mask_pool()
    };
    let Some(pool) = pool else {
        return objc2::rc::autoreleasepool(|_| jobs.iter().map(MaskJob::run).collect());
    };
    #[cfg(test)]
    PARALLEL_BATCHES.fetch_add(1, Ordering::Relaxed);
    let n = jobs.len();
    let (done, results) = mpsc::channel();
    let batch = Arc::new(Batch {
        jobs,
        next: AtomicUsize::new(0),
        done,
    });
    {
        let tickets = pool.tickets.lock().unwrap_or_else(PoisonError::into_inner);
        for _ in 0..pool.helpers.min(n - 1) {
            if tickets.send(batch.clone()).is_err() {
                break;
            }
        }
    }
    // The caller draws too, so a batch never waits on a busy worker.
    batch.work();
    let mut out: Vec<Option<GlyphMask>> = (0..n).map(|_| None).collect();
    let mut panicked = None;
    for _ in 0..n {
        // `batch` keeps a sender alive, and every claimed job reports once.
        let (i, mask) = results.recv().expect("every mask job reports");
        match mask {
            Ok(mask) => out[i] = mask,
            Err(payload) => panicked = Some(payload),
        }
    }
    if let Some(payload) = panicked {
        // As if drawn on this thread.
        std::panic::resume_unwind(payload);
    }
    out
}
const PAD: i32 = 4;
/// SkTypeface_Mac::onFilterRec uses useStrokeForFakeBold. Such strikes go
/// through SkScalerContext::GenerateImageFromPath, without CoreGraphics' font
/// dilation. Their geometric A8 coverage still takes the same preblend table.
fn draw_path(
    path: &kurbo::BezPath,
    scale: f64,
    quarter: u8,
    bounds: kurbo::Rect,
) -> Option<GlyphMask> {
    if !scale.is_finite()
        || scale <= 0.
        || ![bounds.x0, bounds.y0, bounds.x1, bounds.y1]
            .iter()
            .all(|v| v.is_finite())
    {
        return None;
    }
    let left = bounds.x0.floor() as i32 - PAD;
    let top = bounds.y0.floor() as i32 - PAD;
    let w = (bounds.x1.ceil() as i32 + PAD + 1 - left).max(1) as u32;
    let h = (bounds.y1.ceil() as i32 + PAD - top).max(1) as u32;
    if w > 4096 || h > 4096 {
        return None;
    }
    let mut context = vello_cpu::RenderContext::new(w as u16, h as u16);
    context.set_paint(vello_cpu::color::palette::css::WHITE);
    context.set_transform(
        kurbo::Affine::translate((f64::from(-left) + f64::from(quarter) / 4., f64::from(-top)))
            * kurbo::Affine::scale(scale),
    );
    context.fill_path(path);
    context.flush();
    let mut pixels = vello_cpu::Pixmap::new(w as u16, h as u16);
    context.render(&mut pixels, &mut vello_cpu::Resources::new());
    Some(GlyphMask {
        width: w,
        height: h,
        left,
        top,
        coverage: pixels.data().iter().map(|p| p.a).collect(),
    })
}
fn mask_rect(bounds: kurbo::Rect) -> Option<(i32, i32, usize, usize)> {
    if ![bounds.x0, bounds.y0, bounds.x1, bounds.y1]
        .into_iter()
        .all(f64::is_finite)
    {
        return None;
    }
    let left = (bounds.x0.floor() as i32).checked_sub(PAD)?;
    let top = (bounds.y0.floor() as i32).checked_sub(PAD)?;
    let right = (bounds.x1.ceil() as i32).checked_add(PAD + 1)?;
    let bottom = (bounds.y1.ceil() as i32).checked_add(PAD)?;
    let w = right.checked_sub(left)?.max(1) as usize;
    let h = bottom.checked_sub(top)?.max(1) as usize;
    (w <= 4096 && h <= 4096).then_some((left, top, w, h))
}

fn draw(
    font: &CFRetained<CTFont>,
    glyph: u16,
    style: MaskStyle,
    bounds: kurbo::Rect,
) -> Option<GlyphMask> {
    let sub_x = f64::from(style.quarter) / 4.0;
    let (left, top, w, h) = mask_rect(bounds)?;
    // Skia's offscreen: device RGB, 32 bits, no alpha, host byte order.
    let mut pixels = vec![0xffff_ffffu32; w * h];
    let space = CGColorSpace::new_device_rgb()?;
    let info = CGImageAlphaInfo::NoneSkipFirst.0 | CGImageByteOrderInfo::Order32Little.0;
    let ctx = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            w,
            h,
            8,
            w * 4,
            Some(&space),
            info,
        )
    }?;
    let c = Some(&*ctx);
    CGContext::set_allows_font_subpixel_quantization(c, false);
    CGContext::set_should_subpixel_quantize_fonts(c, false);
    CGContext::set_allows_font_subpixel_positioning(c, true);
    CGContext::set_should_subpixel_position_fonts(c, true);
    CGContext::set_text_drawing_mode(c, objc2_core_graphics::CGTextDrawingMode::Fill);

    // The pen is the CTM's origin, so the text matrix (synthetic italic)
    // shears the glyph about its own origin.
    CGContext::translate_ctm(c, f64::from(-left) + sub_x, (h as i32 + top) as f64);
    if style.skew != 0.0 {
        // CoreGraphics' y runs up: y-down skew `x + s·y` is `c = −s`.
        CGContext::set_text_matrix(
            c,
            objc2_core_foundation::CGAffineTransform {
                a: 1.0,
                b: 0.0,
                c: -f64::from(style.skew),
                d: 1.0,
                tx: 0.0,
                ty: 0.0,
            },
        );
    }
    CGContext::set_should_antialias(c, true);
    CGContext::set_allows_font_smoothing(c, true);
    CGContext::set_should_smooth_fonts(c, true);
    CGContext::set_gray_fill_color(c, 0.0, 1.0);
    // The pen: `-left + sub_x` from the bitmap's left edge, on the baseline
    // `-top` rows below its top (CoreGraphics' y runs up from the bottom);
    // the CTM is translated there.
    let pos = CGPoint { x: 0.0, y: 0.0 };
    unsafe {
        font.draw_glyphs(
            NonNull::new(&glyph as *const u16 as *mut u16).expect("non-null"),
            NonNull::new(&pos as *const CGPoint as *mut CGPoint).expect("non-null"),
            1,
            &ctx,
        )
    };
    CGContext::flush(c);
    drop(ctx);
    let coverage = pixels
        .iter()
        .map(|p| {
            let (r, g, b) = ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff);
            // SkComputeLuminance of the inverted pixel.
            let (r, g, b) = (255 - r, 255 - g, 255 - b);
            ((r * 54 + g * 183 + b * 19) >> 8) as u8
        })
        .collect();
    Some(GlyphMask {
        width: w as u32,
        height: h as u32,
        left,
        top,
        coverage,
    })
}

/// `SkMaskGamma::CanonicalColor` for one channel: its top 3 bits,
/// replicated (`sk_t_scale255<3>`).
fn canonical(v: u32) -> u32 {
    let b = v >> 5;
    (b << 5) | (b << 2) | (b >> 1)
}

/// The luminance bucket Skia's Mac scaler picks a glyph mask table by, for
/// straight E-encoded paint `rgb` (0..1): the paint color canonicalized,
/// halved (smoothed text without subpixel coverage,
/// `SkCTFontSmoothBehavior::some`), canonicalized again, reduced to one
/// luminance for A8 masks (`SkComputeLuminance`), and canonicalized; its
/// top bits are the bucket. Halving leaves only buckets 0 to 3. Verified
/// on ten colors of the fit capture (black, primaries, grays, yellow, cyan).
pub fn luminance_bucket(rgb: [f64; 3]) -> usize {
    let c = |v: f64| canonical((v.clamp(0.0, 1.0) * 255.0).round() as u32);
    let halve = |v: u32| canonical(v / 2);
    let (r, g, b) = (halve(c(rgb[0])), halve(c(rgb[1])), halve(c(rgb[2])));
    let lum = (r * 54 + g * 183 + b * 19) >> 8;
    (canonical(lum) >> 5) as usize
}

/// The mask table for paint `rgb` (`mask_tables::TABLES`).
pub fn mask_table(rgb: [f64; 3]) -> &'static [u8; 256] {
    &super::mask_tables::TABLES[luminance_bucket(rgb).min(3)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_mask_wave_keeps_failed_keys_retryable_and_reserves_before_eviction() {
        let mut canvas = CanvasMasks::default();
        let pending = PendingMasks {
            keys: vec![(1, 0, 10, 24.0f32.to_bits(), Vec::new(), 0, 0, None)],
            ..PendingMasks::default()
        };
        assert!(canvas.admit_row(&pending, |_| None)[0].is_none());
        assert!(canvas.masks.is_empty());

        canvas.bytes = MASK_BYTES - 1;
        assert!(canvas.can_admit_wave(1));
        assert!(!canvas.can_admit_wave(2));
        let mask = Arc::new(GlyphMask {
            width: 1,
            height: 1,
            left: 0,
            top: 0,
            coverage: vec![128],
        });
        let result = canvas.admit_row(&pending, |_| Some(mask.clone()));
        assert!(Arc::ptr_eq(result[0].as_ref().unwrap(), &mask));
        assert_eq!(canvas.bytes, MASK_BYTES);
        assert!(!canvas.can_admit_wave(1));
        let cached = canvas.admit_row(&pending, |_| panic!("cached mask should not be retried"));
        assert!(Arc::ptr_eq(cached[0].as_ref().unwrap(), &mask));
    }
    use kurbo::Shape;

    /// Selection/seek repaints nested text at new device sizes while Properties
    /// follows the selection. Exercise actual masks, not just empty font runs.
    #[test]
    fn painted_seek_cycles_bound_font_providers_and_clear_on_idle() {
        use read_fonts::TableProvider;
        let faces: Vec<_> = ["Arial", "Times New Roman", "Courier New"]
            .map(|name| {
                std::fs::read(format!("/System/Library/Fonts/Supplemental/{name}.ttf"))
                    .expect("system fixture font")
            })
            .into_iter()
            .collect();
        let glyphs: Vec<_> = faces
            .iter()
            .map(|data| {
                read_fonts::FontRef::new(data)
                    .unwrap()
                    .cmap()
                    .unwrap()
                    .map_codepoint('H' as u32)
                    .unwrap()
                    .to_u32() as u16
            })
            .collect();
        let source_bytes: usize = faces.iter().map(Vec::len).sum();
        let mut cache = CanvasMasks::default();
        let mut first = None;
        for cycle in 0..50 {
            for (index, data) in faces.iter().enumerate() {
                let mask = cache
                    .get(
                        FaceRef {
                            data,
                            index: 0,
                            id: index as u64 + 1,
                        },
                        &[],
                        glyphs[index],
                        20. + cycle as f32 * 0.125,
                        0,
                        0.,
                        kurbo::Rect::new(-8., -80., 80., 20.),
                        None,
                    )
                    .expect("painted glyph");
                assert!(mask.coverage.iter().any(|&v| v > 0));
                if cycle == 0 && index == 0 {
                    first = Some(mask.coverage.clone());
                }
            }
            let (bytes, strikes) = cache.fonts.retained_for_test();
            assert_eq!(bytes, source_bytes, "each provider is retained only once");
            assert!(strikes <= 128, "native strikes must stay bounded");
            assert!(
                cache.bytes < 8 << 20,
                "mask eviction must not hide font growth"
            );
        }
        // This is the clear reached by TextContext::forget_glyphs on idle and
        // Renderer::trim(0) under critical pressure.
        cache.clear();
        assert_eq!(cache.fonts.retained_for_test(), (0, 0));
        assert!(cache.masks.is_empty());
        assert_eq!(cache.bytes, 0);
        let again = cache
            .get(
                FaceRef {
                    data: &faces[0],
                    index: 0,
                    id: 1,
                },
                &[],
                glyphs[0],
                20.,
                0,
                0.,
                kurbo::Rect::new(-8., -80., 80., 20.),
                None,
            )
            .unwrap();
        assert_eq!(
            Some(&again.coverage),
            first.as_ref(),
            "idle must preserve pixels"
        );
    }

    const WGHT: u32 = u32::from_be_bytes(*b"wght");
    const OPSZ: u32 = u32::from_be_bytes(*b"opsz");
    /// The fixture's text: SF Pro (hidden `.SFNS`, the CGFont variation
    /// path) and a visible variable face (the descriptor path).
    fn sf() -> Vec<u8> {
        std::fs::read("/System/Library/Fonts/SFNS.ttf").expect("SF Pro system font")
    }
    fn geist() -> Vec<u8> {
        include_bytes!("../../tests/fixtures/Geist-Variable.ttf").to_vec()
    }
    fn gids(data: &[u8], text: &str) -> Vec<u16> {
        use read_fonts::TableProvider;
        let cmap = read_fonts::FontRef::new(data).unwrap().cmap().unwrap();
        let mut out: Vec<u16> = text
            .chars()
            .filter_map(|c| cmap.map_codepoint(c as u32))
            .map(|g| g.to_u32() as u16)
            .collect();
        out.sort_unstable();
        out.dedup();
        assert!(out.len() > 8, "fixture glyphs map");
        out
    }
    /// Device sizes under `Powermove UI 3`: layer pt × 25 % × (1.05 → 0.93)
    /// × the viewer's 1.5, as f32 like the painter's strike size.
    fn animated_sizes(steps: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for pt in [25.9f64, 43.2, 62.9] {
            for k in 0..steps {
                let t = k as f64 / (steps - 1) as f64;
                let scale = 0.25 * (1.05 + (0.93 - 1.05) * t) * 1.5;
                out.push((pt * scale) as f32);
            }
        }
        out
    }
    fn glyph_bounds(size: f32) -> kurbo::Rect {
        let s = f64::from(size);
        kurbo::Rect::new(-0.25 * s, -1.15 * s, 1.3 * s, 0.45 * s)
    }
    /// The uncached, single-threaded reference: new font objects from a new
    /// CoreTextReference, drawn on this thread.
    fn fresh(
        fonts: &mut CoreTextReference,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u16,
        size: f32,
        style: MaskStyle,
    ) -> Option<GlyphMask> {
        let run = RefRun {
            data: face.data,
            index: face.index,
            size,
            coords,
            glyphs: vec![],
            color: [0., 0., 0., 1.],
        };
        let font = fonts.font_of(face.id, &run)?;
        draw(&font, glyph, style, glyph_bounds(size))
    }
    fn assert_same(a: &GlyphMask, b: &GlyphMask, what: &str) {
        assert_eq!(
            (a.width, a.height, a.left, a.top),
            (b.width, b.height, b.left, b.top),
            "{what}: mask box"
        );
        assert!(a.coverage == b.coverage, "{what}: mask bytes");
    }
    /// One strike's masks as one batch: every glyph at every quarter, each
    /// asked twice (the second is the first's job).
    fn batch(
        cache: &mut CanvasMasks,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyphs: &[u16],
        size: f32,
        skew: f32,
    ) -> Vec<(u16, u8, Option<Arc<GlyphMask>>)> {
        let mut pending = PendingMasks::default();
        let mut asked = Vec::new();
        for &glyph in glyphs {
            for quarter in 0..4u8 {
                let mut ask = || {
                    cache.lookup(
                        &mut pending,
                        face,
                        coords,
                        glyph,
                        size,
                        quarter,
                        skew,
                        glyph_bounds(size),
                        None,
                    )
                };
                let first = ask();
                let again = ask();
                if let (MaskLookup::Pending(a), MaskLookup::Pending(b)) = (&first, &again) {
                    assert_eq!(a, b, "equal keys share one job");
                }
                asked.push((glyph, quarter, first));
            }
        }
        let drawn = if pending.is_empty() {
            Vec::new()
        } else {
            cache.resolve(pending)
        };
        asked
            .into_iter()
            .map(|(glyph, quarter, l)| {
                let mask = match l {
                    MaskLookup::Ready(mask) => mask,
                    MaskLookup::Pending(job) => drawn[job].clone(),
                };
                (glyph, quarter, mask)
            })
            .collect()
    }

    /// The cached fonts, the per-coordinates variation instances and the
    /// parallel batch draw the exact masks an uncached, single-threaded
    /// CoreText draw makes: every weight, the optical size axis, both
    /// variation paths, animated fractional sizes, all quarters, italic.
    #[test]
    fn batched_parallel_masks_match_fresh_serial_masks() {
        let (sf, geist) = (sf(), geist());
        let text = "Powermove GOOD TIMES Text edit 0:11 Adjustment";
        let faces: Vec<(FaceRef<'_>, Vec<u16>, Vec<Vec<(u32, f32)>>)> = vec![
            (
                FaceRef {
                    data: &sf,
                    index: 0,
                    id: 1,
                },
                gids(&sf, text),
                [400f32, 450., 500., 550., 600., 650.]
                    .map(|w| vec![(WGHT, w)])
                    .into_iter()
                    .chain([vec![(WGHT, 500.), (OPSZ, 17.)], vec![]])
                    .collect(),
            ),
            (
                FaceRef {
                    data: &geist,
                    index: 0,
                    id: 2,
                },
                gids(&geist, text),
                [300f32, 500., 700.]
                    .map(|w| vec![(WGHT, w)])
                    .into_iter()
                    .collect(),
            ),
        ];
        let before = PARALLEL_BATCHES.load(Ordering::Relaxed);
        let mut cache = CanvasMasks::default();
        let mut strikes = 0;
        for (face, glyphs, weights) in &faces {
            for coords in weights {
                for (k, size) in animated_sizes(6).into_iter().enumerate() {
                    let skew = if k % 3 == 2 { -0.2 } else { 0.0 };
                    let masks = batch(&mut cache, *face, coords, glyphs, size, skew);
                    // Fresh font objects per strike: no instance is shared.
                    let mut reference = CoreTextReference::default();
                    for (glyph, quarter, mask) in &masks {
                        let want = fresh(
                            &mut reference,
                            *face,
                            coords,
                            *glyph,
                            size,
                            MaskStyle {
                                quarter: *quarter,
                                skew,
                            },
                        );
                        let what =
                            format!("face {} {coords:?} {size} g{glyph} q{quarter}", face.id);
                        match (mask, &want) {
                            (Some(got), Some(want)) => assert_same(got, want, &what),
                            (None, None) => {}
                            _ => panic!("{what}: drawn on one side only"),
                        }
                    }
                    // A second paint hits the cache with the same masks.
                    let again = batch(&mut cache, *face, coords, glyphs, size, skew);
                    for ((_, _, a), (_, _, b)) in masks.iter().zip(&again) {
                        assert!(match (a, b) {
                            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                            (None, None) => true,
                            _ => false,
                        });
                    }
                    strikes += 1;
                }
            }
        }
        assert!(strikes > 150);
        if mask_pool().is_some() {
            assert!(
                PARALLEL_BATCHES.load(Ordering::Relaxed) - before >= strikes,
                "the batches went through the workers"
            );
        }
    }

    /// The same batch drawn by the workers and on the caller's thread alone.
    #[test]
    fn worker_and_caller_thread_batches_draw_the_same_masks() {
        let sf = sf();
        let face = FaceRef {
            data: &sf,
            index: 0,
            id: 1,
        };
        let glyphs = gids(&sf, "Powermove UI GOOD TIMES 0123456789");
        for size in animated_sizes(4) {
            let coords = [(WGHT, 550.)];
            let parallel = batch(
                &mut CanvasMasks::default(),
                face,
                &coords,
                &glyphs,
                size,
                0.,
            );
            SERIAL.with(|s| s.set(true));
            let serial = batch(
                &mut CanvasMasks::default(),
                face,
                &coords,
                &glyphs,
                size,
                0.,
            );
            SERIAL.with(|s| s.set(false));
            assert_eq!(parallel.len(), serial.len());
            for ((g, q, a), (_, _, b)) in parallel.iter().zip(&serial) {
                let what = format!("{size} g{g} q{q}");
                assert_same(a.as_ref().unwrap(), b.as_ref().unwrap(), &what);
            }
        }
    }

    /// Several painters on their own threads share the mask workers at once,
    /// over and over: every mask is the single-threaded one.
    #[test]
    fn concurrent_painters_share_the_workers_deterministically() {
        let (sf, geist) = (sf(), geist());
        let faces = [
            (
                FaceRef {
                    data: &sf,
                    index: 0,
                    id: 1,
                },
                gids(&sf, "Powermove GOOD TIMES"),
            ),
            (
                FaceRef {
                    data: &geist,
                    index: 0,
                    id: 2,
                },
                gids(&geist, "Powermove GOOD TIMES"),
            ),
        ];
        let sizes = animated_sizes(3);
        let coords = [(WGHT, 600.)];
        SERIAL.with(|s| s.set(true));
        let mut reference = CanvasMasks::default();
        let want: Vec<_> = faces
            .iter()
            .flat_map(|(face, glyphs)| {
                sizes
                    .iter()
                    .map(|&size| batch(&mut reference, *face, &coords, glyphs, size, 0.))
                    .collect::<Vec<_>>()
            })
            .collect();
        SERIAL.with(|s| s.set(false));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let mut cache = CanvasMasks::default();
                    for round in 0..8 {
                        let mut i = 0;
                        for (face, glyphs) in &faces {
                            for &size in &sizes {
                                let got = batch(&mut cache, *face, &coords, glyphs, size, 0.);
                                for ((g, q, a), (_, _, b)) in got.iter().zip(&want[i]) {
                                    let what =
                                        format!("round {round} face {} {size} g{g} q{q}", face.id);
                                    assert_same(a.as_ref().unwrap(), b.as_ref().unwrap(), &what);
                                }
                                i += 1;
                            }
                        }
                        // Draw everything again next round.
                        cache.evict_masks();
                    }
                });
            }
        });
    }

    /// Thousands of fractional strike sizes: masks stay within their budget,
    /// strikes stay bounded, one variation instance per weight, and mask
    /// eviction keeps the font provider (no 7.9 MB SF copy per eviction).
    #[test]
    fn fractional_sizes_stay_bounded_and_keep_fonts_across_mask_eviction() {
        let sf = sf();
        let face = FaceRef {
            data: &sf,
            index: 0,
            id: 1,
        };
        let glyphs = gids(&sf, "Powermove UI");
        let mut cache = CanvasMasks::default();
        let mut providers = None;
        let mut evictions = 0;
        for i in 0..1000 {
            let size = 12.0 + i as f32 * 0.006;
            for w in [450., 600.] {
                let before = cache.bytes;
                for (_, _, mask) in batch(&mut cache, face, &[(WGHT, w)], &glyphs, size, 0.) {
                    assert!(mask.is_some());
                }
                if cache.bytes < before {
                    evictions += 1;
                }
                assert!(cache.bytes < MASK_BYTES + (64 << 10), "mask budget");
                let (bytes, strikes) = cache.fonts.retained_for_test();
                assert_eq!(bytes, sf.len(), "one provider");
                assert!(strikes <= 128, "strikes stay bounded");
                assert!(
                    cache.fonts.variants_for_test() <= 2,
                    "one instance per weight"
                );
                let now = cache.fonts.providers_for_test();
                assert_eq!(
                    *providers.get_or_insert_with(|| now.clone()),
                    now,
                    "provider kept"
                );
            }
        }
        assert!(
            evictions >= 2,
            "the mask cache filled and evicted ({evictions})"
        );
        assert_eq!(cache.fonts.variants_for_test(), 2);
        // Many distinct weights at one size: instances stay bounded too.
        for i in 0..1000 {
            let w = 100. + i as f32 * 0.8;
            assert!(
                cache
                    .get(
                        face,
                        &[(WGHT, w)],
                        glyphs[0],
                        20.,
                        0,
                        0.,
                        glyph_bounds(20.),
                        None
                    )
                    .is_some()
            );
            assert!(cache.fonts.variants_for_test() <= 64);
            assert!(cache.fonts.retained_for_test().1 <= 128);
        }
    }

    #[test]
    fn synthetic_bold_paths_have_area_coverage_without_font_dilation() {
        let rect = kurbo::Rect::new(0., 0., 2.25, 8.);
        let mask = draw_path(&rect.to_path(0.01), 1., 1, rect).unwrap();
        let pixel = |x: i32, y: i32| {
            mask.coverage[((y - mask.top) as u32 * mask.width + (x - mask.left) as u32) as usize]
        };
        // A quarter-pixel pen: [0.25, 2.5]. No fitted expansion or vertical
        // offset may thicken this independently specified outline.
        assert!((i32::from(pixel(0, 3)) - 191).abs() <= 1);
        assert_eq!(pixel(1, 3), 255);
        assert!((i32::from(pixel(2, 3)) - 128).abs() <= 1);
        assert_eq!(pixel(0, -1), 0);
        assert_eq!(pixel(0, 8), 0);
        assert!(
            (mask.coverage.iter().map(|v| f64::from(*v)).sum::<f64>() / 255. - 18.).abs() < 0.04
        );
    }
}
