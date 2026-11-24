//! Text rasters on the GPU: exact float coverage and color glyph images,
//! painted in order into float canvases
//! (`shaders/text.wgsl`), with a content-keyed raster cache.
//!
//! Eligible contours are integrated in paint pixels from cached line data.
//! Other contours use R32Float pages from the shared area renderer in the
//! same command encoder as paint. Pages are returned to the pool after
//! submission, so later layers cannot overwrite coverage still in use.
//! The CPU mode is retained for raster comparisons.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use kurbo::{Affine, BezPath, PathEl, Rect};
use pm_gpu::{Target, TargetPool, working_format};
use pm_protocol::color::{Precision, WorkingEncoding};
use pm_text::render::atlas::{self, Atlas, Cell};
use pm_text::render::scene::{ImagePixels, ItemContent, TextScene, blur_radius};
use pm_vector::render::scene::PixelRect;

use super::TextPlan;
use crate::coverage::{self, AreaCoverage};
use crate::pipelines::{UNIFORM_STRIDE, bytes_of};

/// The largest coverage page side.
pub const MAX_PAGE: u32 = 2048;
/// Rasters kept across frames (bytes of texel storage).
pub const CACHE_BYTES: u64 = 4 * 1024 * 1024;
const RETIRED_BYTES: u64 = 2 * 1024 * 1024;
const IMAGE_CACHE_BYTES: u64 = 8 * 1024 * 1024;
/// Bytes of one color glyph texel (`Rgba32Float`).
const IMAGE_TEXEL_BYTES: u64 = 16;
/// The largest side of a color glyph atlas page. A full page is 4 MiB,
/// half of [`IMAGE_CACHE_BYTES`], so every page fits the cache; an image
/// wider or taller takes a page of its own size, as before atlases.
const IMAGE_ATLAS_SIDE: u32 = 512;
/// Floats of conversion scratch kept across rasters (1 MiB).
const IMAGE_SCRATCH_KEEP: usize = 1 << 18;
const DIRECT_CACHE_BYTES: u64 = 8 * 1024 * 1024;
/// Texels of one glyph's packing at most: every index stays below 2^24,
/// where the shader's f32 metadata is exact.
const DIRECT_ARENA_TEXELS: usize = 1 << 24;
/// Texels of glyphs sharing one direct-source texture (32 KB), the scratch
/// they are written through, reused across frames.
const DIRECT_CHUNK_TEXELS: usize = 1 << 12;
const DIRECT_MAX_SIDE: f64 = 512.0;
/// Standard-size chunk textures kept for reuse (32 KiB each).
const DIRECT_FREE_CHUNKS: usize = 64;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct ItemUniform {
    rect: [f32; 4],
    map: [f32; 4],
    color: [f32; 4],
    params: [f32; 4],
    inv0: [f32; 4],
    inv1: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BlurUniform {
    width: u32,
    height: u32,
    radius: u32,
    axis: u32,
}

/// How coverage is produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// Integrate eligible contours in paint pixels; otherwise use area pages.
    Auto,
    /// Force exact GPU area coverage in R32Float pages.
    Area,
    /// CPU coverage for reference comparisons.
    Cpu,
    /// Integrate eligible contours in paint pixels, with area page fallback.
    Direct,
}

/// Counters for tests and performance reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStats {
    pub painted: u64,
    pub cache_hits: u64,
    pub pages: u64,
    pub cpu_pages: u64,
    /// Glyph outlines resolved and uploaded for direct coverage (a glyph the
    /// direct cache did not hold).
    pub glyph_uploads: u64,
    /// Color glyph images converted and uploaded (an image the image cache
    /// did not hold).
    pub image_uploads: u64,
    /// Atlas pages those images were packed into: one texture, one copy
    /// and one bind group each.
    pub image_pages: u64,
}

struct Cached {
    target: Target,
    used: u64,
}

#[derive(Clone, Copy)]
struct DirectMeta {
    origin: [f64; 2],
    /// Where the glyph's texels start in its texture; `table`, `list` and
    /// the listed point indices are relative to it.
    base: u32,
    table: u32,
    list: u32,
    row_start: i32,
    rows: u32,
    /// The texture's width in texels.
    width: u32,
}

struct DirectCacheEntry {
    // Retaining the lines makes their pointer identity safe as a key.
    _lines: Arc<BezPath>,
    /// The glyph's chunk texture bound for paint, made once per chunk.
    bind: wgpu::BindGroup,
    chunk: u64,
    meta: DirectMeta,
}

type DirectKey = (usize, [u64; 4]);

/// Glyph entries grouped by the chunk texture they share. A chunk's texture
/// is charged once, when it is made, and leaves with all of its glyphs, so
/// the retained textures stay within the bound however glyphs share them.
/// Color glyph atlas pages are cached the same way, by image content key.
struct DirectCache<C, E, K = DirectKey> {
    entries: HashMap<K, (u64, E)>,
    chunks: HashMap<u64, DirectChunk<C, K>>,
    bytes: u64,
    limit: u64,
    next: u64,
}

struct DirectChunk<C, K = DirectKey> {
    payload: C,
    bytes: u64,
    keys: Vec<K>,
    used: u64,
}

impl<C, E, K: Copy + Eq + std::hash::Hash> DirectCache<C, E, K> {
    fn new(limit: u64) -> Self {
        Self {
            entries: HashMap::new(),
            chunks: HashMap::new(),
            bytes: 0,
            limit,
            next: 0,
        }
    }

    fn hit(&mut self, key: &K, tick: u64) -> Option<&E> {
        let (chunk, entry) = self.entries.get(key)?;
        if let Some(chunk) = self.chunks.get_mut(chunk) {
            chunk.used = tick;
        }
        Some(entry)
    }

    /// Charges a new chunk of `bytes`, first dropping the least recently
    /// used chunks (with their glyphs) it would not fit beside; their
    /// payloads go to `evicted`.
    fn admit(&mut self, payload: C, bytes: u64, tick: u64, evicted: &mut Vec<C>) -> u64 {
        while self.bytes + bytes > self.limit {
            let Some((&oldest, _)) = self.chunks.iter().min_by_key(|(_, c)| c.used) else {
                break;
            };
            evicted.extend(self.evict(oldest));
        }
        let id = self.next;
        self.next += 1;
        self.bytes += bytes;
        self.chunks.insert(
            id,
            DirectChunk {
                payload,
                bytes,
                keys: Vec::new(),
                used: tick,
            },
        );
        id
    }

    fn insert(&mut self, chunk: u64, key: K, entry: E) {
        if let Some(c) = self.chunks.get_mut(&chunk) {
            c.keys.push(key);
            self.entries.insert(key, (chunk, entry));
        }
    }

    fn evict(&mut self, id: u64) -> Option<C> {
        let chunk = self.chunks.remove(&id)?;
        self.bytes -= chunk.bytes;
        for key in chunk.keys {
            // A later chunk may hold the same glyph again.
            if self.entries.get(&key).is_some_and(|(c, _)| *c == id) {
                self.entries.remove(&key);
            }
        }
        Some(chunk.payload)
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.chunks.clear();
        self.bytes = 0;
    }
}

struct DirectSource {
    bind: wgpu::BindGroup,
    chunk: u64,
    meta: DirectMeta,
    translation: [f64; 2],
}

/// A cached color glyph image: its atlas page, bound for paint (one bind
/// group per page, shared by every image on it), and its texel region.
#[derive(Clone)]
struct ImageEntry {
    bind: wgpu::BindGroup,
    page: u64,
    /// The region's origin and size in the page: x, y, width, height.
    region: [u32; 4],
}

const FRAME_GLYPH_PAGE_BYTES: u64 =
    IMAGE_ATLAS_SIDE as u64 * IMAGE_ATLAS_SIDE as u64 * IMAGE_TEXEL_BYTES;

#[derive(Default)]
struct GlyphShelf {
    x: u32,
    y: u32,
    height: u32,
}

impl GlyphShelf {
    fn append(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if width == 0 || height == 0 || width > IMAGE_ATLAS_SIDE || height > IMAGE_ATLAS_SIDE {
            return None;
        }
        let (x, y, shelf) = if self.x + width > IMAGE_ATLAS_SIDE {
            (0, self.y + self.height, 0)
        } else {
            (self.x, self.y, self.height)
        };
        if y + height > IMAGE_ATLAS_SIDE {
            return None;
        }
        self.x = x + width;
        self.y = y;
        self.height = shelf.max(height);
        Some((x, y))
    }
}

struct FrameGlyphPage {
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    shelf: GlyphShelf,
}

#[derive(Default)]
struct FrameGlyphPages {
    pages: Vec<FrameGlyphPage>,
    entries: HashMap<u64, (WorkingEncoding, ImagePixels, ImageEntry)>,
    recycle: bool,
}

impl FrameGlyphPages {
    fn bytes(&self) -> u64 {
        self.pages.len() as u64 * FRAME_GLYPH_PAGE_BYTES
    }

    fn submitted(&mut self) {
        if std::mem::take(&mut self.recycle) {
            self.entries.clear();
            for page in &mut self.pages {
                page.shelf = GlyphShelf::default();
            }
        }
    }
}

fn same_image(a: &ImagePixels, b: &ImagePixels) -> bool {
    match (a, b) {
        (ImagePixels::Rgba8(a), ImagePixels::Rgba8(b)) => {
            Arc::ptr_eq(a, b) || (a.width == b.width && a.height == b.height && a.data == b.data)
        }
        (ImagePixels::WorkingF32(a), ImagePixels::WorkingF32(b)) => {
            Arc::ptr_eq(a, b)
                || (a.width == b.width
                    && a.height == b.height
                    && a.data.len() == b.data.len()
                    && a.data
                        .iter()
                        .zip(&b.data)
                        .all(|(a, b)| a.to_bits() == b.to_bits()))
        }
        (ImagePixels::Colr(a), ImagePixels::Colr(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

const ITEM_UNIFORM_CHUNK_BYTES: u64 = 64 * 1024;
const MAX_ITEM_UNIFORM_CHUNKS: usize = 4;

#[derive(Default)]
struct UniformPool {
    chunks: Vec<UniformChunk>,
}

struct UniformChunk {
    buffer: wgpu::Buffer,
    bind: wgpu::BindGroup,
    bytes: Vec<u8>,
    flushed: usize,
}

impl UniformPool {
    fn reset(&mut self) {
        for chunk in &mut self.chunks {
            chunk.bytes.clear();
            chunk.flushed = 0;
        }
    }

    fn flush(&mut self, queue: &wgpu::Queue) {
        for chunk in &mut self.chunks {
            if chunk.flushed < chunk.bytes.len() {
                queue.write_buffer(
                    &chunk.buffer,
                    chunk.flushed as u64,
                    &chunk.bytes[chunk.flushed..],
                );
                chunk.flushed = chunk.bytes.len();
            }
        }
    }

    fn put(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        draws: &[Draw],
        use_pool: bool,
    ) -> (wgpu::BindGroup, u32) {
        let size = draws.len() as u64 * UNIFORM_STRIDE;
        let capacity = ITEM_UNIFORM_CHUNK_BYTES.min(device.limits().max_buffer_size);
        if use_pool && size <= capacity {
            let mut index = self
                .chunks
                .iter()
                .position(|chunk| chunk.bytes.len() as u64 + size <= capacity);
            if index.is_none() && self.chunks.len() < MAX_ITEM_UNIFORM_CHUNKS {
                index = Some(self.chunks.len());
                self.chunks
                    .push(UniformChunk::new(device, layout, capacity));
            }
            if let Some(index) = index {
                let chunk = &mut self.chunks[index];
                let base = chunk.append(draws);
                return (chunk.bind.clone(), base);
            }
        }
        let mut spill = UniformChunk::new(device, layout, size);
        spill.append(draws);
        queue.write_buffer(&spill.buffer, 0, &spill.bytes);
        (spill.bind, 0)
    }
}

impl UniformChunk {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, size: u64) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm text items"),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm text items"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(size_of::<ItemUniform>() as u64),
                }),
            }],
        });
        Self {
            buffer,
            bind,
            bytes: Vec::new(),
            flushed: 0,
        }
    }

    fn append(&mut self, draws: &[Draw]) -> u32 {
        let stride = UNIFORM_STRIDE as usize;
        let base = self.bytes.len();
        self.bytes.resize(base + draws.len() * stride, 0);
        for (slot, draw) in self.bytes[base..].chunks_exact_mut(stride).zip(draws) {
            slot[..size_of::<ItemUniform>()].copy_from_slice(bytes_of(&draw.u));
        }
        base as u32
    }
}

/// What a draw binds at group 1: `id` identifies the bound texture within a
/// pass, so consecutive draws from one texture bind it once.
struct ImageSource {
    bind: wgpu::BindGroup,
    id: (u8, u64),
    /// An image's region in its atlas page (`ItemUniform::map`).
    region: [u32; 4],
}

/// What each scene item draws from, by item index.
struct ItemSources {
    /// Atlas cells of coverage items.
    cells: HashMap<usize, Vec<Cell>>,
    /// Direct coverage sources (when the scene integrates outlines directly).
    direct: HashMap<usize, DirectSource>,
    /// Color glyph atlas regions and the direct sources' chunk textures.
    images: HashMap<usize, ImageSource>,
}

/// Color glyph images shelf-packed into atlas pages: each page's texel size,
/// and where each image goes (`None` for an image without texels).
#[derive(Debug, PartialEq, Eq)]
struct ShelfPack {
    pages: Vec<(u32, u32)>,
    places: Vec<Option<ShelfPlace>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShelfPlace {
    page: usize,
    x: u32,
    y: u32,
}

/// Shelf-packs images of `sizes` (width, height) into pages of at most
/// `side` texels a side, with no gutter: paint reads only inside each
/// image's own region. Images are placed tallest first along shelves of a
/// width near the square root of their area; a page closes when the next
/// shelf would pass `side`. Pages are trimmed to what they hold. An image
/// wider or taller than `side` gets a page of exactly its size, and so does
/// every image when `single` (one texture per image, the reference layout).
fn shelf_pack(sizes: &[(u32, u32)], side: u32, single: bool) -> ShelfPack {
    let side = side.max(1);
    let mut places = vec![None; sizes.len()];
    let mut pages = Vec::new();
    let mut shared = Vec::new();
    for (index, &(w, h)) in sizes.iter().enumerate() {
        if w == 0 || h == 0 {
            continue;
        }
        if single || w > side || h > side {
            places[index] = Some(ShelfPlace {
                page: pages.len(),
                x: 0,
                y: 0,
            });
            pages.push((w, h));
        } else {
            shared.push(index);
        }
    }
    if shared.is_empty() {
        return ShelfPack { pages, places };
    }
    // Tallest first keeps each shelf's height close to its images'. Ties
    // keep scene order, so the same images always pack the same way.
    shared.sort_by_key(|&index| (std::cmp::Reverse(sizes[index].1), index));
    let area: u64 = shared
        .iter()
        .map(|&index| u64::from(sizes[index].0) * u64::from(sizes[index].1))
        .sum();
    let widest = shared
        .iter()
        .map(|&index| sizes[index].0)
        .max()
        .unwrap_or(1);
    let width = ((area as f64).sqrt().ceil() as u32).clamp(widest, side);
    let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
    let mut used = (0u32, 0u32);
    let mut page = pages.len();
    for index in shared {
        let (w, h) = sizes[index];
        if x + w > width {
            // The next shelf, below the tallest image of this one.
            y += shelf;
            x = 0;
            shelf = 0;
        }
        if y + h > side {
            pages.push(used);
            page = pages.len();
            (x, y, shelf) = (0, 0, 0);
            used = (0, 0);
        }
        places[index] = Some(ShelfPlace { page, x, y });
        x += w;
        shelf = shelf.max(h);
        used = (used.0.max(x), used.1.max(y + h));
    }
    pages.push(used);
    ShelfPack { pages, places }
}

/// Writes `image`'s premultiplied texels, decoded into the working domain,
/// into the RGBA rows of a `page_width`-texel-wide page at (`x`, `y`). The
/// arithmetic is the one each image's own texture was filled with.
///
/// Premultiplied E is decoded into the project's working domain before area
/// filtering. The original texels are retained so the shader can integrate
/// each inverse output pixel against source texel squares.
fn write_working(
    image: &ImagePixels,
    working: WorkingEncoding,
    page: &mut [f32],
    page_width: u32,
    x: u32,
    y: u32,
) {
    let (width, height) = image.dimensions();
    let (width, height, page_width) = (width as usize, height as usize, page_width as usize);
    let colr = if let ImagePixels::Colr(program) = image {
        let raster = program.rasterize(working);
        debug_assert_eq!(
            (raster.width as usize, raster.height as usize),
            (width, height)
        );
        Some(raster.data)
    } else {
        None
    };
    for row in 0..height {
        let at = ((y as usize + row) * page_width + x as usize) * 4;
        let out = &mut page[at..at + width * 4];
        let o = row * width * 4;
        match image {
            ImagePixels::Rgba8(i) => {
                for (rgba, source) in out
                    .chunks_exact_mut(4)
                    .zip(i.data[o..o + width * 4].chunks_exact(4))
                {
                    let source: [f32; 4] = std::array::from_fn(|c| f32::from(source[c]) / 255.0);
                    rgba.copy_from_slice(&to_working(source, working));
                }
            }
            ImagePixels::WorkingF32(i) => out.copy_from_slice(&i.data[o..o + width * 4]),
            ImagePixels::Colr(_) => {
                let data = colr.as_deref().expect("COLR raster");
                out.copy_from_slice(&data[o..o + width * 4]);
            }
        }
    }
}

/// Coverage storage for one atlas page. Each page recorded in one encoder
/// needs its own: queue writes before the submit would otherwise replace an
/// earlier page's inputs.
#[derive(Default)]
struct AreaSlot {
    inputs: coverage::InputScratch,
    values: Option<wgpu::Buffer>,
}

impl AreaSlot {
    fn bytes(&self) -> u64 {
        self.inputs.capacity_bytes() + self.values.as_ref().map_or(0, wgpu::Buffer::size)
    }

    fn values(&mut self, device: &wgpu::Device, bytes: u64) -> &wgpu::Buffer {
        let needed = bytes.max(16).next_power_of_two();
        if self.values.as_ref().is_none_or(|b| b.size() < needed) {
            self.values = Some(coverage::buffer(device, needed));
        }
        self.values.as_ref().expect("coverage values allocated")
    }
}

/// Coverage storage kept across submitted frames.
const AREA_KEEP_BYTES: u64 = 8 * 1024 * 1024;

pub struct TextGpu {
    area: AreaCoverage,
    batch: coverage::Batch,
    area_slots: Vec<AreaSlot>,
    area_cursor: usize,
    cpu: pm_text::render::cpu::Rasterizer,
    coverage: Coverage,
    item_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    paint: [crate::pipelines::LazyRenderPipeline; 2],
    image_paint: [crate::pipelines::LazyRenderPipeline; 2],
    blur_layout: wgpu::BindGroupLayout,
    blur: [crate::pipelines::LazyRenderPipeline; 2],
    /// Color glyph images by content key, on the atlas pages that hold them
    /// (a page is charged whole and leaves with all of its images).
    images: DirectCache<wgpu::Texture, ImageEntry, u64>,
    /// Conversion scratch of one atlas page, reused across rasters.
    frame_glyphs: FrameGlyphPages,
    item_uniforms: UniformPool,
    image_scratch: Vec<f32>,
    /// Ids of atlas pages too large to cache (used by one raster only).
    uncached_pages: u64,
    /// The largest atlas page side (tests shrink it to force more pages).
    atlas_side: u32,
    /// One texture per image, the layout before atlas pages (tests only).
    #[cfg(test)]
    single_images: bool,
    direct: DirectCache<wgpu::Texture, DirectCacheEntry>,
    direct_scratch: Vec<[f32; 2]>,
    raster_pool: TargetPool,
    cache: HashMap<([u64; 2], Precision, WorkingEncoding), Cached>,
    cache_bytes: u64,
    cache_budget: u64,
    // Cached rasters can still be sampled by this frame's encoder after an
    // LRU eviction. begin_frame gives them back to this pool only after the
    // preceding frame's submission.
    retired: VecDeque<Target>,
    retired_bytes: u64,
    /// Standard-size direct chunk textures left by evicted chunks: reused
    /// from the next frame on (this frame's draws may still read them).
    chunks_retired: Vec<wgpu::Texture>,
    chunks_free: Vec<wgpu::Texture>,
    /// Staging for glyph-edge chunk uploads, reused across frames.
    belt: crate::upload::Belt,
    tick: u64,
    pub stats: TextStats,
}

fn frame_image_source(entry: &ImageEntry) -> ImageSource {
    ImageSource {
        bind: entry.bind.clone(),
        id: (3, entry.page),
        region: entry.region,
    }
}

fn glyph_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pm text color glyph atlas"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn upload_glyph_texels(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    texels: &[f32],
    [x, y, width, height]: [u32; 4],
) {
    // f32 has no padding: both routes upload the same native-endian texels.
    let bytes = unsafe {
        std::slice::from_raw_parts(texels.as_ptr().cast::<u8>(), std::mem::size_of_val(texels))
    };
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x, y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * IMAGE_TEXEL_BYTES as u32),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

fn format_index(precision: Precision) -> usize {
    match precision {
        Precision::Unorm8 => 0,
        Precision::Float16 => 0,
        Precision::Float32 => 1,
    }
}

fn text_format(precision: Precision) -> wgpu::TextureFormat {
    working_format(match precision {
        Precision::Unorm8 => Precision::Float16,
        other => other,
    })
}

fn image_key(img: &ImagePixels, working: WorkingEncoding) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    working.hash(&mut h);
    match img {
        ImagePixels::Rgba8(i) => {
            (0u8, i.width, i.height).hash(&mut h);
            i.data.hash(&mut h);
        }
        ImagePixels::WorkingF32(i) => {
            (1u8, i.width, i.height).hash(&mut h);
            for v in &i.data {
                v.to_bits().hash(&mut h);
            }
        }
        ImagePixels::Colr(program) => {
            (2u8, program.content_key()).hash(&mut h);
        }
    }
    h.finish()
}

fn to_working(mut rgba: [f32; 4], working: WorkingEncoding) -> [f32; 4] {
    if working == WorkingEncoding::LinearLight {
        let a = rgba[3];
        if a > 0.0 {
            for c in &mut rgba[..3] {
                *c = pm_color::transfer::display_decode_f32(*c / a) * a;
            }
        } else {
            rgba[..3].fill(0.0);
        }
    }
    rgba
}

/// A glyph's packing, measured before its texels are written (so a frame's
/// texels can be reserved at once).
struct DirectPlan {
    origin: [f64; 2],
    row_start: i32,
    row_end: i32,
    cursors: Vec<usize>,
    length: usize,
    table: u32,
    list: u32,
    needed: usize,
}

// Visit points in exactly the packed order. The optional predecessor
// identifies an edge; a MoveTo emits a point without an edge.
fn walk_points(
    elements: impl Iterator<Item = PathEl>,
    origin: [f64; 2],
    mut emit: impl FnMut(usize, [f32; 2], Option<[f32; 2]>),
) -> Option<usize> {
    fn append(
        p: [f32; 2],
        count: &mut usize,
        last: &mut Option<[f32; 2]>,
        emit: &mut impl FnMut(usize, [f32; 2], Option<[f32; 2]>),
    ) {
        if let Some(prev) = *last
            && prev != p
        {
            emit(*count, p, Some(prev));
            *count += 1;
        }
        *last = Some(p);
    }
    fn close(
        count: &mut usize,
        first: &mut Option<[f32; 2]>,
        last: &mut Option<[f32; 2]>,
        emit: &mut impl FnMut(usize, [f32; 2], Option<[f32; 2]>),
    ) {
        if let Some(p) = *first {
            append(p, count, last, emit);
        }
        *first = None;
        *last = None;
    }

    let (mut first, mut last) = (None, None);
    let mut count = 0;
    for element in elements {
        match element {
            PathEl::MoveTo(p) => {
                close(&mut count, &mut first, &mut last, &mut emit);
                let q = [(p.x - origin[0]) as f32, (p.y - origin[1]) as f32];
                emit(count, q, None);
                count += 1;
                first = Some(q);
                last = Some(q);
            }
            PathEl::LineTo(p) => {
                last?;
                append(
                    [(p.x - origin[0]) as f32, (p.y - origin[1]) as f32],
                    &mut count,
                    &mut last,
                    &mut emit,
                );
            }
            PathEl::ClosePath => close(&mut count, &mut first, &mut last, &mut emit),
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => return None,
        }
    }
    close(&mut count, &mut first, &mut last, &mut emit);
    Some(count)
}

fn direct_range(row_start: i32, row_end: i32, a: f32, b: f32) -> std::ops::RangeInclusive<i32> {
    let low = (a.min(b).floor() as i32 - 1).max(row_start);
    let high = (a.max(b).floor() as i32).min(row_end);
    low..=high
}

/// Measures a resolved, line-only glyph's packing: contiguous contour
/// points, a conservative Y-row table, then pairs of edge-start point
/// indices, all relative to where it starts. Only listed consecutive point
/// pairs are edges, so MoveTo jumps never contribute area. A pixel querying
/// row r has local Y in [r, r + 1), so its unit-height interval is contained
/// in [r, r + 2). An edge can contribute only for r in
/// [floor(min_y) - 1, floor(max_y)].
fn direct_plan<I: Iterator<Item = PathEl>>(elements: impl Fn() -> I) -> Option<DirectPlan> {
    let bounds = line_bounds(elements())?;
    if ![bounds.x0, bounds.y0, bounds.x1, bounds.y1]
        .into_iter()
        .all(f64::is_finite)
        || bounds.width() > DIRECT_MAX_SIDE
        || bounds.height() > DIRECT_MAX_SIDE
    {
        return None;
    }
    let origin = [bounds.x0.floor(), bounds.y0.floor()];
    let row_start = -2i32;
    let row_end = bounds.height().ceil() as i32 + 3;
    let rows = (row_end - row_start + 1) as usize;
    let mut cursors = vec![0usize; rows];
    let mut edges = 0usize;
    let point_count = walk_points(elements(), origin, |_, point, prev| {
        if let Some(prev) = prev {
            edges += 1;
            if prev[1] != point[1] {
                for row in direct_range(row_start, row_end, prev[1], point[1]) {
                    cursors[(row - row_start) as usize] += 1;
                }
            }
        }
    })?;
    if edges == 0 || point_count >= (1 << 24) {
        return None;
    }
    let mut length = 0usize;
    // Prefix counts in place; each entry becomes the insertion cursor for
    // the fill after its starting offset has been written.
    for count in &mut cursors {
        let row_count = *count;
        *count = length;
        length += row_count;
    }
    if length >= (1 << 24) {
        return None;
    }
    let table = point_count as u32;
    let list = table + rows as u32;
    let needed = list as usize + length.div_ceil(2);
    if needed as u64 * 8 > DIRECT_CACHE_BYTES / 2 || needed >= DIRECT_ARENA_TEXELS {
        return None;
    }
    Some(DirectPlan {
        origin,
        row_start,
        row_end,
        cursors,
        length,
        table,
        list,
        needed,
    })
}

/// Writes `plan`'s glyph into `texels` (exactly `plan.needed` long).
fn direct_fill<I: Iterator<Item = PathEl>>(
    mut plan: DirectPlan,
    elements: impl Fn() -> I,
    texels: &mut [[f32; 2]],
) -> Option<DirectMeta> {
    let DirectPlan {
        origin,
        row_start,
        row_end,
        table,
        list,
        length,
        ..
    } = plan;
    let rows = plan.cursors.len();
    for row in 0..rows {
        let end = if row + 1 < rows {
            plan.cursors[row + 1]
        } else {
            length
        };
        texels[table as usize + row] = [plan.cursors[row] as f32, (end - plan.cursors[row]) as f32];
    }
    let cursors = &mut plan.cursors;
    walk_points(elements(), origin, |index, point, prev| {
        texels[index] = point;
        if let Some(prev) = prev
            && prev[1] != point[1]
        {
            for row in direct_range(row_start, row_end, prev[1], point[1]) {
                let position = &mut cursors[(row - row_start) as usize];
                texels[list as usize + *position / 2][*position & 1] = (index - 1) as f32;
                *position += 1;
            }
        }
    })?;
    Some(DirectMeta {
        origin,
        base: 0,
        table,
        list,
        row_start,
        rows: rows as u32,
        width: 0,
    })
}

/// [`direct_plan`] and [`direct_fill`] appended to `texels`.
#[cfg(test)]
fn direct_texels_into<I: Iterator<Item = PathEl>>(
    elements: impl Fn() -> I,
    texels: &mut Vec<[f32; 2]>,
) -> Option<DirectMeta> {
    let plan = direct_plan(&elements)?;
    let base = texels.len();
    texels.resize(base + plan.needed, [0.0, 0.0]);
    let mut meta = direct_fill(plan, elements, &mut texels[base..])?;
    meta.base = base as u32;
    Some(meta)
}

/// A shared direct-source texture's (width, height) for `texels`: the
/// fewest rows that fit, each as short as they allow, so it pads less than
/// one texel per row.
fn arena_shape(texels: usize, max_side: usize) -> (usize, usize) {
    let rows = texels.div_ceil(max_side.max(1)).max(1);
    (texels.div_ceil(rows).max(1), rows)
}

/// The bounds of a line-only path's segments (a lone MoveTo adds none),
/// or `None` for a curve.
fn line_bounds(elements: impl Iterator<Item = PathEl>) -> Option<Rect> {
    let (mut start, mut last) = (None::<kurbo::Point>, None::<kurbo::Point>);
    let mut bounds: Option<Rect> = None;
    let mut add = |a: kurbo::Point, b: kurbo::Point| {
        let r = Rect::from_points(a, b);
        bounds = Some(bounds.map_or(r, |old| old.union(r)));
    };
    for element in elements {
        match element {
            PathEl::MoveTo(p) => (start, last) = (Some(p), Some(p)),
            PathEl::LineTo(p) => {
                add(last?, p);
                last = Some(p);
            }
            PathEl::ClosePath => {
                if let (Some(a), Some(b)) = (last, start)
                    && a != b
                {
                    add(a, b);
                }
                last = start;
            }
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => return None,
        }
    }
    Some(bounds.unwrap_or(Rect::ZERO))
}

#[cfg(test)]
fn direct_texels(path: &BezPath) -> Option<(Vec<[f32; 2]>, DirectMeta)> {
    let mut texels = Vec::new();
    let meta = direct_texels_into(|| path.elements().iter().copied(), &mut texels)?;
    Some((texels, meta))
}

/// A text fragment module: `body` with the entry `fs` returning
/// `paint(pos)`. An RGBA16F target gets the nearest half of it
/// (`crate::nearest_half`): the target's own store truncates.
fn fragment_source(body: &str, paint: &str, fs: &str, format: wgpu::TextureFormat) -> String {
    let (half, value) = if format == wgpu::TextureFormat::Rgba16Float {
        (
            crate::nearest_half::NEAREST_HALF_WGSL,
            format!("pm_scene_half_color({paint}(pos))"),
        )
    } else {
        ("", format!("{paint}(pos)"))
    };
    format!(
        "{half}\n{body}\n@fragment\nfn {fs}(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {{\n    return {value};\n}}\n"
    )
}

/// Premultiplied source-over.
const SOURCE_OVER: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
};

/// The quad vertex module every paint pipeline shares.
fn vertex_module(device: &wgpu::Device) -> wgpu::ShaderModule {
    let common = include_str!("../shaders/text_common.wgsl");
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pm-render text quad"),
        source: wgpu::ShaderSource::Wgsl(
            format!("{common}\n{}", include_str!("../shaders/text_vertex.wgsl")).into(),
        ),
    })
}

/// The paint pipeline layout: an item's uniforms, then its source texture.
fn paint_layout(
    device: &wgpu::Device,
    item: &wgpu::BindGroupLayout,
    texture: &wgpu::BindGroupLayout,
) -> wgpu::PipelineLayout {
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(item), Some(texture)],
        immediate_size: 0,
    })
}

/// A text pipeline drawing `body`'s `paint` into `format`. `vertex`: the
/// quad's module, or `None` for the fragment module's own vertex entry.
fn text_pipeline(
    device: &wgpu::Device,
    vertex: Option<&wgpu::ShaderModule>,
    (label, body, paint): (&'static str, &'static str, &'static str),
    layout: &wgpu::PipelineLayout,
    (vs, fs): (&'static str, &'static str),
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> crate::pipelines::LazyRenderPipeline {
    let device = device.clone();
    let vertex = vertex.cloned();
    let layout = layout.clone();
    let module = move |device: &wgpu::Device| {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(fragment_source(body, paint, fs, format).into()),
        })
    };
    // The working format's (RGBA16F) module is made now, as every
    // text module was; an RGBA32F one when a 32-bit frame needs it.
    let ready = (format == wgpu::TextureFormat::Rgba16Float).then(|| module(&device));
    crate::pipelines::LazyRenderPipeline::new(move || {
        let fragment = ready.clone().unwrap_or_else(|| module(&device));
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fs),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: vertex.as_ref().unwrap_or(&fragment),
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    })
}

impl TextGpu {
    pub fn new(device: &wgpu::Device) -> TextGpu {
        // Each text module has one entry point. Metal's shader translation
        // can borrow its IR directly, and plain glyphs never compile the
        // polygon clipping functions used only by bitmap images. Fragment
        // modules are made per target format (`fragment_source`).
        let vertex_module = vertex_module(device);
        let uniform = |binding, dynamic, size: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let item_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm text item"),
            entries: &[uniform(0, true, size_of::<ItemUniform>() as u64)],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm text source"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let layout = |groups: &[&wgpu::BindGroupLayout]| {
            let groups: Vec<Option<&wgpu::BindGroupLayout>> =
                groups.iter().map(|g| Some(*g)).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &groups,
                immediate_size: 0,
            })
        };
        let paint_layout = paint_layout(device, &item_layout, &texture_layout);
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm text Gaussian"),
            entries: &[
                uniform(0, false, size_of::<BlurUniform>() as u64),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let blur_pipeline_layout = layout(&[&blur_layout]);
        let formats = [Precision::Float16, Precision::Float32].map(working_format);
        let outline = (
            "pm-render text outline",
            concat!(
                include_str!("../shaders/text_common.wgsl"),
                "\n",
                include_str!("../shaders/text.wgsl")
            ),
            "paint_outline",
        );
        let image = (
            "pm-render text image",
            concat!(
                include_str!("../shaders/text_common.wgsl"),
                "\n",
                include_str!("../shaders/text_image.wgsl")
            ),
            "paint_image",
        );
        let gaussian = (
            "pm-render text Gaussian",
            include_str!("../shaders/text_blur.wgsl"),
            "blur_pass",
        );
        let paint = formats.map(|f| {
            text_pipeline(
                device,
                Some(&vertex_module),
                outline,
                &paint_layout,
                ("vs", "fs_outline"),
                f,
                Some(SOURCE_OVER),
            )
        });
        let image_paint = formats.map(|f| {
            text_pipeline(
                device,
                Some(&vertex_module),
                image,
                &paint_layout,
                ("vs", "fs_image"),
                f,
                Some(SOURCE_OVER),
            )
        });
        let blur = formats.map(|f| {
            text_pipeline(
                device,
                None,
                gaussian,
                &blur_pipeline_layout,
                ("vs_blur", "fs_blur"),
                f,
                None,
            )
        });
        TextGpu {
            area: AreaCoverage::new(device),
            batch: coverage::Batch::default(),
            area_slots: Vec::new(),
            area_cursor: 0,
            cpu: pm_text::render::cpu::Rasterizer::new(),
            coverage: Coverage::Auto,
            item_layout,
            texture_layout,
            paint,
            image_paint,
            blur_layout,
            blur,
            images: DirectCache::new(IMAGE_CACHE_BYTES),
            frame_glyphs: FrameGlyphPages::default(),
            item_uniforms: UniformPool::default(),
            image_scratch: Vec::new(),
            uncached_pages: 0,
            atlas_side: IMAGE_ATLAS_SIDE,
            #[cfg(test)]
            single_images: false,
            direct: DirectCache::new(DIRECT_CACHE_BYTES),
            direct_scratch: Vec::new(),
            raster_pool: TargetPool::new(),
            cache: HashMap::new(),
            cache_bytes: 0,
            cache_budget: CACHE_BYTES,
            retired: VecDeque::new(),
            retired_bytes: 0,
            chunks_retired: Vec::new(),
            chunks_free: Vec::new(),
            belt: Default::default(),
            tick: 0,
            stats: TextStats::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn built_count(&self) -> usize {
        self.paint
            .iter()
            .chain(self.image_paint.iter())
            .filter(|pipeline| pipeline.is_built())
            .count()
            + self
                .blur
                .iter()
                .filter(|pipeline| pipeline.is_built())
                .count()
    }

    /// The reference layout and shader of color glyph images before atlas
    /// pages: one texture per image, painted by the image shader as it was
    /// (`image_reference.wgsl`), which bounds texels by the texture's size.
    #[cfg(test)]
    pub(crate) fn use_reference_images(&mut self, device: &wgpu::Device) {
        let vertex = vertex_module(device);
        let layout = paint_layout(device, &self.item_layout, &self.texture_layout);
        let image = (
            "pm-render text image reference",
            concat!(
                include_str!("../shaders/text_common.wgsl"),
                "\n",
                include_str!("image_reference.wgsl")
            ),
            "paint_image",
        );
        self.image_paint = [Precision::Float16, Precision::Float32]
            .map(working_format)
            .map(|f| {
                text_pipeline(
                    device,
                    Some(&vertex),
                    image,
                    &layout,
                    ("vs", "fs_image"),
                    f,
                    Some(SOURCE_OVER),
                )
            });
        self.single_images = true;
        self.clear();
    }

    /// Compiles the text pipelines frames at `precision` draw with (outline
    /// and image paint, blur), ahead of the first text that needs them.
    pub(crate) fn prewarm(&self, precision: Precision) {
        let i = format_index(precision);
        for pipeline in [&self.paint[i], &self.image_paint[i], &self.blur[i]] {
            pipeline.get();
        }
    }

    #[cfg(test)]
    pub(crate) fn probe_all(&self) -> Vec<(String, std::time::Duration)> {
        let mut times = Vec::new();
        for (name, pipeline) in [
            ("outline paint f16", &self.paint[0]),
            ("outline paint f32", &self.paint[1]),
            ("image paint f16", &self.image_paint[0]),
            ("image paint f32", &self.image_paint[1]),
            ("Gaussian f16", &self.blur[0]),
            ("Gaussian f32", &self.blur[1]),
        ] {
            let start = std::time::Instant::now();
            pipeline.get();
            times.push((format!("text {name}"), start.elapsed()));
        }
        times
    }

    pub fn set_coverage(&mut self, coverage: Coverage) {
        self.coverage = coverage;
        self.clear();
    }

    /// Marks a new submitted frame so area buffers can be reused without
    /// aliasing two text pages recorded in the same command encoder.
    pub fn begin_frame(&mut self) {
        self.frame_glyphs.submitted();
        self.item_uniforms.reset();
        self.area_cursor = 0;
        let mut kept = 0;
        self.area_slots.retain(|slot| {
            kept += slot.bytes();
            kept <= AREA_KEEP_BYTES
        });
        if self.batch.capacity_bytes() > AREA_KEEP_BYTES {
            self.batch = coverage::Batch::default();
        }
        for target in self.retired.drain(..) {
            self.raster_pool.give(target);
        }
        self.chunks_free.append(&mut self.chunks_retired);
        self.chunks_free.truncate(DIRECT_FREE_CHUNKS);
        self.retired_bytes = 0;
    }

    /// Drops cached rasters and color glyph textures. The private raster
    /// pool releases accounting without explicitly destroying textures that
    /// might still be referenced by recorded commands.
    pub fn clear(&mut self) {
        self.area_slots.clear();
        self.area_cursor = 0;
        self.batch = coverage::Batch::default();
        self.direct_scratch = Vec::new();
        self.belt = Default::default();
        for (_, c) in self.cache.drain() {
            self.raster_pool.release(c.target);
        }
        for target in self.retired.drain(..) {
            self.raster_pool.release(target);
        }
        self.retired_bytes = 0;
        self.raster_pool.trim(0);
        self.cache_bytes = 0;
        self.images.clear();
        self.images.limit = IMAGE_CACHE_BYTES;
        self.frame_glyphs = FrameGlyphPages::default();
        self.item_uniforms = UniformPool::default();
        self.image_scratch = Vec::new();
        self.direct.clear();
        self.chunks_retired.clear();
        self.chunks_free.clear();
    }

    pub fn cached_bytes(&self) -> u64 {
        self.area_slots.iter().map(AreaSlot::bytes).sum::<u64>()
            + self.batch.capacity_bytes()
            + self.raster_pool.bytes()
            + self.images.bytes
            + self.frame_glyphs.bytes()
            + self
                .item_uniforms
                .chunks
                .iter()
                .map(|c| c.buffer.size())
                .sum::<u64>()
            + self.direct.bytes
            + self
                .chunks_retired
                .iter()
                .chain(&self.chunks_free)
                .map(|t| {
                    u64::from(t.width())
                        * u64::from(t.height())
                        * u64::from(t.format().block_copy_size(None).unwrap_or(16))
                })
                .sum::<u64>()
    }

    /// The caller records and submits every reader before beginning a new
    /// frame. Within that window, a page is append-only across all rasters.
    fn place_frame_image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: u64,
        image: &ImagePixels,
        working: WorkingEncoding,
    ) -> Option<ImageSource> {
        let (width, height) = image.dimensions();
        if width > IMAGE_ATLAS_SIDE
            || height > IMAGE_ATLAS_SIDE
            || device.limits().max_texture_dimension_2d < IMAGE_ATLAS_SIDE
        {
            return None;
        }
        let placement = self
            .frame_glyphs
            .pages
            .iter_mut()
            .enumerate()
            .find_map(|(index, page)| page.shelf.append(width, height).map(|(x, y)| (index, x, y)));
        let (index, x, y) = if let Some(placement) = placement {
            placement
        } else {
            let reserved = self.frame_glyphs.bytes() + FRAME_GLYPH_PAGE_BYTES;
            if reserved + self.images.bytes > IMAGE_CACHE_BYTES {
                self.frame_glyphs.recycle = true;
                return None;
            }
            let texture = glyph_texture(device, IMAGE_ATLAS_SIDE, IMAGE_ATLAS_SIDE);
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind = self.source_bind(device, &view);
            let mut shelf = GlyphShelf::default();
            let (x, y) = shelf.append(width, height)?;
            let index = self.frame_glyphs.pages.len();
            self.frame_glyphs.pages.push(FrameGlyphPage {
                texture,
                bind,
                shelf,
            });
            self.images.limit = IMAGE_CACHE_BYTES - reserved;
            self.stats.image_pages += 1;
            (index, x, y)
        };
        let page = &self.frame_glyphs.pages[index];
        let mut scratch = std::mem::take(&mut self.image_scratch);
        scratch.resize(width as usize * height as usize * 4, 0.0);
        write_working(image, working, &mut scratch, width, 0, 0);
        upload_glyph_texels(queue, &page.texture, &scratch, [x, y, width, height]);
        if scratch.capacity() <= IMAGE_SCRATCH_KEEP {
            scratch.clear();
            self.image_scratch = scratch;
        }
        self.stats.image_uploads += 1;
        let entry = ImageEntry {
            bind: page.bind.clone(),
            page: index as u64,
            region: [x, y, width, height],
        };
        let source = frame_image_source(&entry);
        self.frame_glyphs
            .entries
            .insert(key, (working, image.clone(), entry));
        Some(source)
    }

    /// The atlas regions of the color glyph images `wanted` (item index,
    /// image), by item index. Images the cache holds keep their regions.
    /// The rest are decoded into the working domain once per content key
    /// (as each image's own texture was filled) and shelf-packed into atlas
    /// pages: per page one texture, one copy and one bind group, not one per
    /// image. Pages within half the image cache are cached whole.
    ///
    /// Paint reads an image's texels at its region's whole-texel origin
    /// plus the same integer coordinates, and treats texels outside the
    /// region's size as outside the image, so every painted value is the
    /// one an image's own texture gave.
    fn place_images(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        wanted: &[(usize, &ImagePixels)],
        working: WorkingEncoding,
    ) -> HashMap<usize, ImageSource> {
        let source = |entry: &ImageEntry| ImageSource {
            bind: entry.bind.clone(),
            id: (1, entry.page),
            region: entry.region,
        };
        let mut out = HashMap::with_capacity(wanted.len());
        #[cfg(test)]
        let single = self.single_images;
        #[cfg(not(test))]
        let single = false;
        let requested: Vec<_> = wanted
            .iter()
            .copied()
            .filter(|(_, image)| {
                let (w, h) = image.dimensions();
                w > 0 && h > 0
            })
            .map(|(i, image)| (i, image, image_key(image, working)))
            .collect();
        let mut distinct = HashMap::new();
        let shared = !single
            && self.atlas_side == IMAGE_ATLAS_SIDE
            && requested.iter().all(|&(_, image, key)| {
                distinct
                    .insert(key, image)
                    .is_none_or(|other| same_image(image, other))
                    && self
                        .frame_glyphs
                        .entries
                        .get(&key)
                        .is_none_or(|(domain, other, _)| {
                            working == *domain && same_image(image, other)
                        })
            });
        // Images the cache misses, once per content key, in scene order.
        let mut missing: Vec<(u64, &ImagePixels)> = Vec::new();
        let mut missing_at: HashMap<u64, usize> = HashMap::new();
        let mut staged: Vec<(usize, usize)> = Vec::new();
        for (i, image, key) in requested {
            if let Some(entry) = self.images.hit(&key, self.tick) {
                out.insert(i, source(entry));
            } else if shared && let Some((_, _, entry)) = self.frame_glyphs.entries.get(&key) {
                out.insert(i, frame_image_source(entry));
            } else if shared
                && let Some(entry) = self.place_frame_image(device, queue, key, image, working)
            {
                out.insert(i, entry);
            } else if let Some(&at) = missing_at.get(&key) {
                staged.push((i, at));
            } else {
                missing_at.insert(key, missing.len());
                staged.push((i, missing.len()));
                missing.push((key, image));
            }
        }
        if missing.is_empty() {
            return out;
        }
        let sizes: Vec<(u32, u32)> = missing
            .iter()
            .map(|(_, image)| image.dimensions())
            .collect();
        let side = self
            .atlas_side
            .min(device.limits().max_texture_dimension_2d)
            .max(1);
        let pack = shelf_pack(&sizes, side, single || shared);
        let mut on_page: Vec<Vec<usize>> = vec![Vec::new(); pack.pages.len()];
        for (m, place) in pack.places.iter().enumerate() {
            if let Some(place) = place {
                on_page[place.page].push(m);
            }
        }
        let mut placed: Vec<Option<ImageEntry>> = vec![None; missing.len()];
        let mut scratch = std::mem::take(&mut self.image_scratch);
        for (&(width, height), images) in pack.pages.iter().zip(&on_page) {
            scratch.clear();
            scratch.resize(width as usize * height as usize * 4, 0.0);
            for &m in images {
                let place = pack.places[m].expect("placed on this page");
                write_working(missing[m].1, working, &mut scratch, width, place.x, place.y);
            }
            self.stats.image_uploads += images.len() as u64;
            self.stats.image_pages += 1;
            let texture = glyph_texture(device, width, height);
            upload_glyph_texels(queue, &texture, &scratch, [0, 0, width, height]);
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind = self.source_bind(device, &view);
            let page_bytes = u64::from(width) * u64::from(height) * IMAGE_TEXEL_BYTES;
            // A page the cache cannot hold is this raster's alone; recorded
            // commands keep its texture until they complete.
            let chunk = (page_bytes <= self.images.limit.min(IMAGE_CACHE_BYTES / 2)).then(|| {
                self.images
                    .admit(texture, page_bytes, self.tick, &mut Vec::new())
            });
            let page = chunk.unwrap_or_else(|| {
                self.uncached_pages += 1;
                u64::MAX - self.uncached_pages
            });
            for &m in images {
                let place = pack.places[m].expect("placed on this page");
                let (w, h) = sizes[m];
                let entry = ImageEntry {
                    bind: bind.clone(),
                    page,
                    region: [place.x, place.y, w, h],
                };
                if let Some(chunk) = chunk {
                    self.images.insert(chunk, missing[m].0, entry.clone());
                }
                placed[m] = Some(entry);
            }
        }
        // Keep a common page's scratch; not an unusually large one's.
        if scratch.capacity() <= IMAGE_SCRATCH_KEEP {
            scratch.clear();
            self.image_scratch = scratch;
        }
        for (i, at) in staged {
            if let Some(entry) = &placed[at] {
                out.insert(i, source(entry));
            }
        }
        out
    }

    /// The cached direct source of `lines`, or `None` to stage it.
    fn direct_hit(
        &mut self,
        lines: &pm_text::render::scene::CoverageLines,
        translation: [f64; 2],
    ) -> Option<DirectSource> {
        let entry = self.direct.hit(&lines.key(), self.tick)?;
        Some(DirectSource {
            bind: entry.bind.clone(),
            chunk: entry.chunk,
            meta: entry.meta,
            translation,
        })
    }

    /// Uploads the planned glyphs a chunk at a time through the reused
    /// scratch: one texture and one copy per chunk of at most
    /// [`DIRECT_CHUNK_TEXELS`] (a larger glyph takes a chunk alone), and
    /// caches each glyph in its chunk's texture. Returns the sources in
    /// planning order; `None` if a glyph cannot be written.
    fn direct_upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        planned: Vec<(pm_text::render::scene::CoverageLines, DirectPlan)>,
    ) -> Option<Vec<(wgpu::BindGroup, u64, DirectMeta)>> {
        let max_side = device.limits().max_texture_dimension_2d.max(1) as usize;
        self.stats.glyph_uploads += planned.len() as u64;
        let mut out = Vec::with_capacity(planned.len());
        let mut chunk: Vec<(pm_text::render::scene::CoverageLines, DirectMeta, usize)> = Vec::new();
        let mut scratch = std::mem::take(&mut self.direct_scratch);
        let sizes: Vec<usize> = planned.iter().map(|(_, p)| p.needed).collect();
        let mut planned = planned.into_iter();
        let mut at = 0;
        while at < sizes.len() {
            // This chunk's glyphs, and its texture's texels reserved at once.
            let mut end = at + 1;
            let mut total = sizes[at];
            while end < sizes.len() && total + sizes[end] <= DIRECT_CHUNK_TEXELS {
                total += sizes[end];
                end += 1;
            }
            // A chunk within the standard size takes a standard texture,
            // reused across frames; a larger glyph gets its own.
            let standard = total <= DIRECT_CHUNK_TEXELS;
            let (w, h) = arena_shape(if standard { DIRECT_CHUNK_TEXELS } else { total }, max_side);
            scratch.clear();
            scratch.reserve(w * h);
            chunk.clear();
            for _ in at..end {
                let (lines, plan) = planned.next().expect("sized");
                let base = scratch.len();
                let needed = plan.needed;
                scratch.resize(base + needed, [0.0, 0.0]);
                let mut meta = direct_fill(plan, || lines.device_elements(), &mut scratch[base..])?;
                meta.base = base as u32;
                chunk.push((lines, meta, needed));
            }
            at = end;
            let (width, height) = (w, h);
            // A standard texture (one row) is written only as far as used.
            let written = if standard && height == 1 {
                scratch.len().max(1)
            } else {
                scratch.resize(width * height, [0.0, 0.0]);
                width
            };
            let size = wgpu::Extent3d {
                width: width as u32,
                height: height as u32,
                depth_or_array_layers: 1,
            };
            let reused = standard.then(|| self.chunks_free.pop()).flatten();
            let texture = reused.unwrap_or_else(|| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("pm text direct resolved edges"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rg32Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            });
            // [f32; 2] has no padding; both routes copy the bytes before
            // returning.
            let bytes = unsafe {
                std::slice::from_raw_parts(scratch.as_ptr().cast::<u8>(), scratch.len() * 8)
            };
            let destination = wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            };
            let extent = wgpu::Extent3d {
                width: written as u32,
                ..size
            };
            if height == 1 {
                // One row needs no row pitch: staged in the reused belt,
                // not a fresh staging buffer per chunk.
                let (buffer, offset) = self.belt.stage(device, bytes);
                encoder.copy_buffer_to_texture(
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset,
                            bytes_per_row: None,
                            rows_per_image: None,
                        },
                    },
                    destination,
                    extent,
                );
            } else {
                queue.write_texture(
                    destination,
                    bytes,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(written as u32 * 8),
                        rows_per_image: None,
                    },
                    extent,
                );
            }
            let view = texture.create_view(&Default::default());
            // One bind group per chunk, shared by its glyphs' draws.
            let bind = self.source_bind(device, &view);
            // The whole texture is retained while any of its glyphs is.
            let mut evicted = Vec::new();
            let id = self.direct.admit(
                texture,
                (width * height) as u64 * 8,
                self.tick,
                &mut evicted,
            );
            let standard_size = arena_shape(DIRECT_CHUNK_TEXELS, max_side);
            self.chunks_retired.extend(
                evicted
                    .into_iter()
                    .filter(|t| (t.width() as usize, t.height() as usize) == standard_size),
            );
            for (lines, mut meta, _) in chunk.drain(..) {
                meta.width = width as u32;
                let key = lines.key();
                self.direct.insert(
                    id,
                    key,
                    DirectCacheEntry {
                        _lines: lines.lines,
                        bind: bind.clone(),
                        chunk: id,
                        meta,
                    },
                );
                out.push((bind.clone(), id, meta));
            }
        }
        // Keep the common chunk's scratch; not an unusually large glyph's.
        if scratch.capacity() <= 2 * DIRECT_CHUNK_TEXELS {
            scratch.clear();
            self.direct_scratch = scratch;
        }
        Some(out)
    }

    /// Renders the coverage pages of `atlas` into pool targets, which the
    /// caller returns to the pool after the frame is submitted.
    fn pages(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        scene: &TextScene,
        atlas: &Atlas,
    ) -> Result<Vec<Target>, String> {
        let mut targets = Vec::with_capacity(atlas.pages.len());
        for (page, &(w, h)) in atlas.pages.iter().enumerate() {
            let use_gpu = self.coverage != Coverage::Cpu;
            // The atlas cells keep their exact logical coordinates. Only the
            // R32 target allocation is bucketed so a growing text raster can
            // reuse the same pool texture across several adjacent sizes.
            // Paint loads integer cell texels and never samples the spare area.
            let (target_w, target_h) = if use_gpu {
                (w.max(1).next_power_of_two(), h.max(1).next_power_of_two())
            } else {
                (w.max(1), h.max(1))
            };
            let target = pool.take(
                device,
                target_w,
                target_h,
                if use_gpu {
                    wgpu::TextureFormat::R32Float
                } else {
                    wgpu::TextureFormat::R8Unorm
                },
                wgpu::TextureUsages::TEXTURE_BINDING
                    | if use_gpu {
                        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC
                    } else {
                        wgpu::TextureUsages::COPY_DST
                    },
            );
            self.stats.pages += 1;
            if use_gpu {
                let entries = area_items(scene, atlas, page, self.cpu.coverage_cache())?;
                let items: Vec<_> = entries
                    .iter()
                    .map(|entry| coverage::TransformedItem {
                        path: &entry.path,
                        transform: entry.transform,
                        bounds: entry.bounds,
                        at: entry.at,
                    })
                    .collect();
                self.batch.reset_transformed(&items);
                if self.area_cursor == self.area_slots.len() {
                    self.area_slots.push(AreaSlot::default());
                }
                let slot = &mut self.area_slots[self.area_cursor];
                self.area_cursor += 1;
                let values = slot.values(device, self.batch.bytes()).clone();
                self.area.encode_pooled(
                    device,
                    queue,
                    encoder,
                    &self.batch,
                    coverage::CoverageOutput {
                        values: &values,
                        page: Some(&target.view),
                    },
                    &mut slot.inputs,
                )?;
            } else {
                let (pw, ph, data) = self.cpu.render_page_r8(scene, atlas, page);
                self.stats.cpu_pages += 1;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &target.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(pw),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width: pw,
                        height: ph,
                        depth_or_array_layers: 1,
                    },
                );
            }
            targets.push(target);
        }
        Ok(targets)
    }

    fn source_bind(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm text source"),
            layout: &self.texture_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            }],
        })
    }

    /// The painted raster of `plan`: a top-down working texture of the
    /// plan's crop, premultiplied.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        taken: &mut Vec<Target>,
        plan: &TextPlan,
        precision: Precision,
        working: WorkingEncoding,
    ) -> Result<wgpu::TextureView, String> {
        self.tick += 1;
        if let Some(c) = self.cache.get_mut(&(plan.key, precision, working)) {
            c.used = self.tick;
            self.stats.cache_hits += 1;
            pm_perf::bump(pm_perf::Counter::TextCacheHits);
            return Ok(c.target.view.clone());
        }
        pm_perf::bump(pm_perf::Counter::TextCacheMisses);
        let _span = pm_perf::Span::new(pm_perf::Counter::TextRenderMicros);
        let scene = &plan.scene;
        let region = Rect::new(plan.crop[0] as f64, plan.crop[1] as f64,
            plan.crop[2] as f64, plan.crop[3] as f64);
        let needed = |item: &pm_text::render::scene::Item| {
            let reach = f64::from(blur_radius(item.blur)) + 1.;
            let r = item.bounds.inflate(reach,reach).intersect(region);
            r.width()>0. && r.height()>0.
        };
        // Glyphs, sizes, or transforms the cache has not met (a layer first
        // drawn, or animated) get their geometry on several threads at once.
        let coverage = pm_perf::Span::new(pm_perf::Counter::TextCoverageMicros);
        self.cpu.coverage_cache().prepare(
            scene.items.iter().filter(|item| needed(item)).flat_map(|item| match &item.content {
                ItemContent::Coverage { paths } => paths.as_slice(),
                _ => &[],
            }),
            |n, make| crate::parallel::map(n, make),
        );
        drop(coverage);
        let atlas = atlas::pack_region(
            scene,
            MAX_PAGE,
            Rect::new(
                plan.crop[0] as f64,
                plan.crop[1] as f64,
                plan.crop[2] as f64,
                plan.crop[3] as f64,
            ),
        );
        // A scene uses one coverage source throughout, including when any
        // contour is too large or otherwise unsuitable for direct integration.
        let mut direct = matches!(self.coverage, Coverage::Auto | Coverage::Direct);
        let mut direct_sources: HashMap<usize, DirectSource> = HashMap::new();
        if direct {
            // Glyphs the direct cache misses are uploaded together: one
            // texture and one copy per frame, not one per glyph, reserved
            // once from every glyph's plan.
            let mut planned = Vec::new();
            let mut planned_at: HashMap<(usize, [u64; 4]), usize> = HashMap::new();
            let mut staged_items = Vec::new();
            for (i, item) in scene.items.iter().enumerate() {
                if !needed(item) { continue; }
                let ItemContent::Coverage { paths } = &item.content else {
                    continue;
                };
                let [placed] = paths.as_slice() else {
                    direct = false;
                    break;
                };
                let lines = self.cpu.coverage_cache().get(placed);
                let [_, _, _, _, x, y] = placed.transform.as_coeffs();
                if let Some(source) = self.direct_hit(&lines, [x, y]) {
                    direct_sources.insert(i, source);
                } else if let Some(&at) = planned_at.get(&lines.key()) {
                    staged_items.push((i, at, [x, y]));
                } else if let Some(plan) = direct_plan(|| lines.device_elements()) {
                    planned_at.insert(lines.key(), planned.len());
                    staged_items.push((i, planned.len(), [x, y]));
                    planned.push((lines, plan));
                } else {
                    direct = false;
                    break;
                }
            }
            if direct && !planned.is_empty() {
                let uploaded = self.direct_upload(device, queue, encoder, planned);
                self.belt.close(encoder);
                match uploaded {
                    Some(flushed) => {
                        for (i, at, translation) in staged_items {
                            let (bind, chunk, meta) = &flushed[at];
                            direct_sources.insert(
                                i,
                                DirectSource {
                                    bind: bind.clone(),
                                    chunk: *chunk,
                                    meta: *meta,
                                    translation,
                                },
                            );
                        }
                    }
                    None => direct = false,
                }
            }
        }
        if !direct {
            direct_sources.clear();
        }
        let pages: Vec<wgpu::TextureView> = if direct {
            Vec::new()
        } else {
            let targets = self.pages(device, queue, encoder, pool, scene, &atlas)?;
            let views = targets.iter().map(|t| t.view.clone()).collect();
            taken.extend(targets);
            views
        };
        let (tw, th) = (plan.width(), plan.height());
        let format = text_format(precision);
        let raster = self.raster_pool.take(
            device,
            tw,
            th,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let crop = [f64::from(plan.crop[0]), f64::from(plan.crop[1])];
        let mut cells: HashMap<usize, Vec<Cell>> = HashMap::new();
        for c in &atlas.cells {
            cells.entry(c.item).or_default().push(*c);
        }
        // Color glyph atlas pages (uploaded before the passes are recorded).
        let wanted: Vec<(usize, &ImagePixels)> = scene
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| needed(item))
            .filter_map(|(i, item)| match &item.content {
                ItemContent::Image { image, .. } => Some((i, image)),
                ItemContent::Coverage { .. } => None,
            })
            .collect();
        let mut images = self.place_images(device, queue, &wanted, working);
        for (&i, source) in &direct_sources {
            images.insert(
                i,
                ImageSource {
                    bind: source.bind.clone(),
                    id: (2, source.chunk),
                    region: [0; 4],
                },
            );
        }
        let sources = ItemSources {
            cells,
            direct: direct_sources,
            images,
        };
        let mut cleared = false;
        let mut i = 0;
        while i < scene.items.len() {
            if scene.items[i].blur > 0.0 {
                self.blurred(
                    device,
                    queue,
                    encoder,
                    pool,
                    taken,
                    plan,
                    i,
                    &sources,
                    &pages,
                    &raster,
                    precision,
                    working,
                    &mut cleared,
                );
                i += 1;
                continue;
            }
            // A run of unblurred draws in one pass.
            let start = i;
            while i < scene.items.len() && scene.items[i].blur <= 0.0 {
                i += 1;
            }
            let draws = self.draws(scene, start..i, crop, (tw, th), &sources, working);
            self.pass(
                device,
                queue,
                encoder,
                &raster,
                &draws,
                &pages,
                &sources.images,
                precision,
                &mut cleared,
            );
        }
        if !cleared {
            self.pass(
                device,
                queue,
                encoder,
                &raster,
                &[],
                &pages,
                &sources.images,
                precision,
                &mut cleared,
            );
        }
        // Some callers submit this encoder immediately. Send only the new
        // tails now, and never rewrite a range used by any earlier raster.
        self.item_uniforms.flush(queue);
        self.stats.painted += 1;
        let view = raster.view.clone();
        self.insert(plan, precision, working, raster);
        self.raster_pool.trim(self.cache_budget);
        Ok(view)
    }

    /// Sets the standing raster cache ceiling (bytes of texel storage).
    pub fn set_cache_budget(&mut self, bytes: u64) {
        self.cache_budget = bytes;
        self.evict(self.cache_budget, true);
        self.raster_pool.trim(self.cache_budget);
    }

    /// A frame's commands are submitted and own what they read: the rasters
    /// it retired go, and the raster pool keeps at most the raster cache
    /// budget idle. The glyph caches stay. Each holds at most its own cap
    /// (rasters, [`IMAGE_CACHE_BYTES`] of color glyph images,
    /// [`DIRECT_CACHE_BYTES`] of glyph outlines), so text a later frame draws
    /// unchanged is neither resolved nor uploaded again.
    pub fn end_frame(&mut self) {
        self.frame_glyphs.submitted();
        self.item_uniforms.reset();
        for target in self.retired.drain(..) {
            self.raster_pool.release(target);
        }
        self.retired_bytes = 0;
        self.raster_pool.trim(self.cache_budget);
    }

    /// Evicts least recently used rasters until the cache holds at most
    /// `bytes` (lease pressure). Discarded handles are never reused.
    pub fn trim(&mut self, bytes: u64) {
        if bytes == 0 {
            self.area_slots.clear();
            self.area_cursor = 0;
            self.batch = coverage::Batch::default();
            self.direct_scratch = Vec::new();
            self.image_scratch = Vec::new();
            self.item_uniforms = UniformPool::default();
            self.belt = Default::default();
        }
        if self.images.bytes + self.frame_glyphs.bytes() + self.direct.bytes > bytes {
            self.frame_glyphs = FrameGlyphPages::default();
            self.images.limit = IMAGE_CACHE_BYTES;
            self.images.clear();
            self.direct.clear();
            self.chunks_retired.clear();
            self.chunks_free.clear();
        }
        self.evict(bytes, false);
        for target in self.retired.drain(..) {
            self.raster_pool.release(target);
        }
        self.retired_bytes = 0;
        self.raster_pool.trim(bytes);
    }

    fn retire(&mut self, target: Target) {
        let bytes = target.bytes();
        if bytes > RETIRED_BYTES {
            self.raster_pool.release(target);
            return;
        }
        self.retired_bytes += bytes;
        self.retired.push_back(target);
        while self.retired_bytes > RETIRED_BYTES {
            let oldest = self
                .retired
                .pop_front()
                .expect("retired bytes have a target");
            self.retired_bytes -= oldest.bytes();
            // Drop without Texture::destroy: the current encoder may still
            // sample this raster, but it will never be pooled or overwritten.
            self.raster_pool.release(oldest);
        }
    }

    fn evict(&mut self, bytes: u64, recycle: bool) {
        while self.cache_bytes > bytes {
            let Some((&k, _)) = self.cache.iter().min_by_key(|(_, c)| c.used) else {
                break;
            };
            if let Some(c) = self.cache.remove(&k) {
                self.cache_bytes = self.cache_bytes.saturating_sub(c.target.bytes());
                if recycle {
                    self.retire(c.target);
                } else {
                    self.raster_pool.release(c.target);
                }
            }
        }
    }

    fn insert(
        &mut self,
        plan: &TextPlan,
        precision: Precision,
        working: WorkingEncoding,
        target: Target,
    ) {
        let bytes = target.bytes();
        let budget = self.cache_budget;
        if bytes > budget / 2 {
            self.raster_pool.release(target);
            return;
        }
        self.evict(budget.saturating_sub(bytes), true);
        self.cache_bytes += bytes;
        self.cache.insert(
            (plan.key, precision, working),
            Cached {
                target,
                used: self.tick,
            },
        );
    }

    /// The draw uniforms of items `range`, in a target whose pixel (0, 0)
    /// is device pixel `origin`.
    fn draws(
        &self,
        scene: &TextScene,
        range: std::ops::Range<usize>,
        origin: [f64; 2],
        size: (u32, u32),
        sources: &ItemSources,
        working: WorkingEncoding,
    ) -> Vec<Draw> {
        let (tw, th) = (size.0 as f32, size.1 as f32);
        let mut out = Vec::new();
        for i in range {
            let item = &scene.items[i];
            if matches!(item.content, ItemContent::Image { .. }) && !sources.images.contains_key(&i) { continue; }
            match &item.content {
                ItemContent::Coverage { .. } => {
                    let a = item.color[3] * item.opacity;
                    // The smoothing's coverage curve (`params.y`, `inv1.zw`).
                    let [weight, light, dark] = item.curve;
                    let rgb = pm_color::linear::color_to_working(
                        [item.color[0], item.color[1], item.color[2]],
                        working,
                    );
                    let color = [
                        (rgb[0] * a) as f32,
                        (rgb[1] * a) as f32,
                        (rgb[2] * a) as f32,
                        a as f32,
                    ];
                    for c in sources.cells.get(&i).map(Vec::as_slice).unwrap_or(&[]) {
                        let x0 = c.device_x as f64 - origin[0];
                        let y0 = c.device_y as f64 - origin[1];
                        let direct = sources.direct.get(&i);
                        let (page, image, map, params, inv0, inv1) = if let Some(source) = direct {
                            // Split the local origin in the render target into
                            // whole pixels and a subpixel phase before f32.
                            let px = source.meta.origin[0] + source.translation[0] - origin[0];
                            let py = source.meta.origin[1] + source.translation[1] - origin[1];
                            let whole = [px.floor(), py.floor()];
                            (
                                None,
                                Some(i),
                                [
                                    whole[0] as f32,
                                    whole[1] as f32,
                                    (px - whole[0]) as f32,
                                    (py - whole[1]) as f32,
                                ],
                                [3.0, weight, tw, th],
                                [
                                    (source.meta.base + source.meta.table) as f32,
                                    (source.meta.base + source.meta.list) as f32,
                                    source.meta.row_start as f32,
                                    source.meta.width as f32,
                                ],
                                [
                                    source.meta.rows as f32,
                                    source.meta.base as f32,
                                    light,
                                    dark,
                                ],
                            )
                        } else {
                            (
                                Some(c.page),
                                None,
                                [
                                    (f64::from(c.x) - x0) as f32,
                                    (f64::from(c.y) - y0) as f32,
                                    0.0,
                                    0.0,
                                ],
                                [0.0, weight, tw, th],
                                [0.0; 4],
                                [0.0, 0.0, light, dark],
                            )
                        };
                        out.push(Draw {
                            page,
                            image,
                            u: ItemUniform {
                                rect: [
                                    x0 as f32,
                                    y0 as f32,
                                    (x0 + f64::from(c.width)) as f32,
                                    (y0 + f64::from(c.height)) as f32,
                                ],
                                map,
                                color,
                                params,
                                inv0,
                                inv1,
                            },
                        });
                    }
                }
                ItemContent::Image { transform, .. } => {
                    let Some(source) = sources.images.get(&i) else {
                        continue;
                    };
                    let [ax, ay, aw, ah] = source.region;
                    let b: Rect = item.bounds.inflate(1.0, 1.0);
                    let rect = [
                        (b.x0.floor() - origin[0]) as f32,
                        (b.y0.floor() - origin[1]) as f32,
                        (b.x1.ceil() - origin[0]) as f32,
                        (b.y1.ceil() - origin[1]) as f32,
                    ];
                    // Target px → device px → image px → padded image px.
                    let inv = transform.inverse() * Affine::translate((origin[0], origin[1]));
                    let [a, bb, c, d, e, f] = inv.as_coeffs();
                    // Any source-axis footprint wider than one texel needs
                    // area integration, including mixed shrink/enlarge maps.
                    let xx = a * a + c * c;
                    let xy = a * bb + c * d;
                    let yy = bb * bb + d * d;
                    let major =
                        ((xx + yy + ((xx - yy).powi(2) + 4.0 * xy * xy).sqrt()) * 0.5).sqrt();
                    let downscale = major > 1.0;
                    let alpha = (item.color[3] * item.opacity) as f32;
                    out.push(Draw {
                        page: None,
                        image: Some(i),
                        u: ItemUniform {
                            rect,
                            color: [0.0, 0.0, 0.0, alpha],
                            // The image's atlas region: whole texels, exact.
                            map: [ax as f32, ay as f32, aw as f32, ah as f32],
                            params: [1.0, if downscale { 1.0 } else { 0.0 }, tw, th],
                            inv0: [a as f32, c as f32, e as f32, 0.0],
                            inv1: [bb as f32, d as f32, f as f32, 0.0],
                            ..Default::default()
                        },
                    });
                }
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &Target,
        draws: &[Draw],
        pages: &[wgpu::TextureView],
        images: &HashMap<usize, ImageSource>,
        precision: Precision,
        cleared: &mut bool,
    ) {
        #[cfg(test)]
        let pooled = !self.single_images;
        #[cfg(not(test))]
        let pooled = true;
        let uniforms = (!draws.is_empty()).then(|| {
            self.item_uniforms
                .put(device, queue, &self.item_layout, draws, pooled)
        });
        let page_binds: Vec<wgpu::BindGroup> =
            pages.iter().map(|v| self.source_bind(device, v)).collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm text paint"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if *cleared {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        *cleared = true;
        let mut bound_image = None;
        // The texture bound at group 1: draws reading the same atlas page,
        // direct chunk or coverage page bind it once.
        let mut bound_source = None;
        for (k, d) in draws.iter().enumerate() {
            let (bind, id) = match (d.page, d.image) {
                (Some(p), _) => (&page_binds[p], (0, p as u64)),
                (None, Some(i)) => {
                    let source = &images[&i];
                    (&source.bind, source.id)
                }
                (None, None) => continue,
            };
            let is_image = d.u.params[0] == 1.0;
            if bound_image != Some(is_image) {
                let pipeline = if is_image {
                    &self.image_paint[format_index(precision)]
                } else {
                    &self.paint[format_index(precision)]
                };
                pass.set_pipeline(pipeline.get());
                bound_image = Some(is_image);
                bound_source = None;
            }
            let (uniform_bind, base) = uniforms.as_ref().expect("draw has uniforms");
            pass.set_bind_group(
                0,
                uniform_bind,
                &[*base + (k as u64 * UNIFORM_STRIDE) as u32],
            );
            if bound_source != Some(id) {
                pass.set_bind_group(1, bind, &[]);
                bound_source = Some(id);
            }
            pass.draw(0..4, 0..1);
        }
    }

    /// A draw painted alone, convolved with the pixel-integrated Gaussian,
    /// then composited into the float text target.
    #[allow(clippy::too_many_arguments)]
    fn blurred(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        taken: &mut Vec<Target>,
        plan: &TextPlan,
        index: usize,
        sources: &ItemSources,
        pages: &[wgpu::TextureView],
        raster: &Target,
        precision: Precision,
        working: WorkingEncoding,
        cleared: &mut bool,
    ) {
        let scene = &plan.scene;
        let item = &scene.items[index];
        let r = blur_radius(item.blur) as f64;
        // The draw's own pixels, grown by the kernel, within what can reach
        // the raster.
        let own = match &item.content {
            ItemContent::Coverage { .. } => {
                let mut u: Option<Rect> = None;
                for c in sources.cells.get(&index).map(Vec::as_slice).unwrap_or(&[]) {
                    let rc = Rect::new(
                        c.device_x as f64,
                        c.device_y as f64,
                        (c.device_x + i64::from(c.width)) as f64,
                        (c.device_y + i64::from(c.height)) as f64,
                    );
                    u = Some(u.map_or(rc, |a| a.union(rc)));
                }
                u
            }
            ItemContent::Image { .. } => Some(item.bounds.inflate(1.0, 1.0)),
        };
        let Some(own) = own else { return };
        let crop = Rect::new(
            f64::from(plan.crop[0]),
            f64::from(plan.crop[1]),
            f64::from(plan.crop[2]),
            f64::from(plan.crop[3]),
        );
        let reach = own.inflate(r, r).intersect(crop.inflate(r, r));
        if reach.width() <= 0.0 || reach.height() <= 0.0 {
            return;
        }
        let rx0 = reach.x0.floor();
        let ry0 = reach.y0.floor();
        let lw = (reach.x1.ceil() - rx0) as u32;
        let lh = (reach.y1.ceil() - ry0) as u32;
        let a = pool.take(
            device,
            lw,
            lh,
            text_format(precision),
            pm_gpu::target::working_usage(),
        );
        let draws = self.draws(
            scene,
            index..index + 1,
            [rx0, ry0],
            (lw, lh),
            sources,
            working,
        );
        let mut fresh = false;
        self.pass(
            device,
            queue,
            encoder,
            &a,
            &draws,
            pages,
            &sources.images,
            precision,
            &mut fresh,
        );
        let weights = gaussian_kernel(item.blur);
        let tmp = pool.take(
            device,
            lw,
            lh,
            text_format(precision),
            pm_gpu::target::working_usage(),
        );
        let filtered = pool.take(
            device,
            lw,
            lh,
            text_format(precision),
            pm_gpu::target::working_usage(),
        );
        self.blur_passes(
            device,
            queue,
            encoder,
            [(&a, &tmp), (&tmp, &filtered)],
            &weights,
            precision,
        );
        let (tw, th) = (plan.width() as f32, plan.height() as f32);
        let x0 = (rx0 - crop.x0).max(0.0);
        let y0 = (ry0 - crop.y0).max(0.0);
        let x1 = (rx0 + f64::from(lw) - crop.x0).min(f64::from(plan.width()));
        let y1 = (ry0 + f64::from(lh) - crop.y0).min(f64::from(plan.height()));
        let copy = Draw {
            page: Some(0),
            image: None,
            u: ItemUniform {
                rect: [x0 as f32, y0 as f32, x1 as f32, y1 as f32],
                map: [(crop.x0 - rx0) as f32, (crop.y0 - ry0) as f32, 0.0, 0.0],
                params: [2.0, 1.0, tw, th],
                ..Default::default()
            },
        };
        self.pass(
            device,
            queue,
            encoder,
            raster,
            &[copy],
            std::slice::from_ref(&filtered.view),
            &sources.images,
            precision,
            cleared,
        );
        taken.extend([a, tmp, filtered]);
    }

    /// The separable Gaussian: `passes[axis]` is the (source, target) of the
    /// horizontal (0) then the vertical (1) pass, sharing one weight buffer.
    fn blur_passes(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        passes: [(&Target, &Target); 2],
        weights: &[f32],
        precision: Precision,
    ) {
        let wb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm text Gaussian weights"),
            size: std::mem::size_of_val(weights) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let weight_bytes: Vec<u8> = weights.iter().flat_map(|w| w.to_ne_bytes()).collect();
        queue.write_buffer(&wb, 0, &weight_bytes);
        for (axis, (source, target)) in (0u32..).zip(passes) {
            let uniform = BlurUniform {
                width: target.width(),
                height: target.height(),
                radius: (weights.len() / 2) as u32,
                axis,
            };
            let ub = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pm text blur uniform"),
                size: size_of::<BlurUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&ub, 0, bytes_of(&uniform));
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pm text Gaussian"),
                layout: &self.blur_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ub.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&source.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wb.as_entire_binding(),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pm text Gaussian"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(self.blur[format_index(precision)].get());
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..4, 0..1);
        }
    }
}

fn gaussian_kernel(sigma: f64) -> Vec<f32> {
    pm_text::render::paint::gaussian(sigma)
        .into_iter()
        .map(|w| w as f32)
        .collect()
}

struct Draw {
    /// Page index (coverage) or layer (kind 2, page 0 of the slice passed).
    page: Option<usize>,
    image: Option<usize>,
    u: ItemUniform,
}

struct AreaItem {
    path: Arc<BezPath>,
    /// Maps `path` to device space (cached local lines, then translation).
    transform: Affine,
    bounds: PixelRect,
    at: [u32; 2],
}

/// Outlines and the atlas cell each outline covers. The cache holds the f64
/// resolved outline in its own space; the linear transform and glyph
/// placement stay f64 until area rasterization.
fn area_items(
    scene: &TextScene,
    atlas: &Atlas,
    page: usize,
    cache: &mut pm_text::render::scene::CoveragePathCache,
) -> Result<Vec<AreaItem>, String> {
    let mut out = Vec::new();
    for cell in atlas.cells.iter().filter(|c| c.page == page) {
        let ItemContent::Coverage { paths, .. } = &scene.items[cell.item].content else {
            continue;
        };
        let (path, transform) = if let [one] = paths.as_slice() {
            let [_, _, _, _, x, y] = one.transform.as_coeffs();
            let lines = cache.get(one);
            (lines.lines, Affine::translate((x, y)) * lines.linear)
        } else {
            let mut combined = BezPath::new();
            for p in paths {
                let [_, _, _, _, x, y] = p.transform.as_coeffs();
                let to_device = Affine::translate((x, y));
                for el in cache.get(p).device_elements() {
                    combined.push(to_device * el);
                }
            }
            (Arc::new(combined), Affine::IDENTITY)
        };
        let x = i32::try_from(cell.device_x).map_err(|_| "text cell x outside i32".to_owned())?;
        let y = i32::try_from(cell.device_y).map_err(|_| "text cell y outside i32".to_owned())?;
        out.push(AreaItem {
            path,
            transform,
            bounds: PixelRect::new(x, y, cell.width, cell.height),
            at: [cell.x, cell.y],
        });
    }
    Ok(out)
}

/// The CPU reference of a plan's raster (tests).
pub fn render_cpu(plan: &TextPlan) -> pm_text::render::paint::Image {
    let full = pm_text::render::paint::render_cpu(&plan.scene);
    let (w, h) = (plan.width(), plan.height());
    let mut out = pm_text::render::paint::Image::new(w, h);
    for y in 0..h as usize {
        let sy = y + plan.crop[1] as usize;
        let src =
            &full.data[(sy * full.width as usize + plan.crop[0] as usize) * 4..][..w as usize * 4];
        out.data[y * w as usize * 4..(y + 1) * w as usize * 4].copy_from_slice(src);
    }
    out
}

#[cfg(test)]
mod direct_tests {
    use super::*;

    #[test]
    fn shared_chunk_textures_are_charged_once_and_stay_within_the_bound() {
        // Four glyphs share each 4 KB chunk: the chunk, not each glyph, is
        // what the cache retains and charges.
        let mut cache: DirectCache<(), u32> = DirectCache::new(10 * 1024);
        let key = |n: usize| (n, [0u64; 4]);
        let mut tick = 0;
        for chunk in 0..2 {
            tick += 1;
            let id = cache.admit((), 4096, tick, &mut Vec::new());
            for glyph in 0..4 {
                cache.insert(id, key(chunk * 4 + glyph), glyph as u32);
            }
        }
        assert_eq!(cache.bytes, 8192);
        assert_eq!(cache.entries.len(), 8);
        // The first chunk is used again; the third evicts the second, whole.
        tick += 1;
        assert!(cache.hit(&key(0), tick).is_some());
        tick += 1;
        let id = cache.admit((), 4096, tick, &mut Vec::new());
        cache.insert(id, key(8), 0);
        assert_eq!(cache.bytes, 8192, "two chunks retained");
        assert!(cache.bytes <= cache.limit);
        assert!((4..8).all(|n| cache.hit(&key(n), tick).is_none()));
        assert!((0..4).all(|n| cache.hit(&key(n), tick).is_some()));
        // A glyph uploaded again into a newer chunk stays when the older
        // chunk leaves.
        let older = cache.admit((), 4096, tick, &mut Vec::new());
        cache.insert(older, key(9), 1);
        let newer = cache.admit((), 4096, tick, &mut Vec::new());
        cache.insert(newer, key(9), 7);
        assert!(cache.bytes <= cache.limit);
        cache.evict(older);
        assert_eq!(cache.hit(&key(9), tick), Some(&7));
    }

    #[test]
    fn closed_form_clamped_integral_matches_piecewise_reference() {
        // Independently split the parameter interval at the two clamp
        // crossings, as the original shader did. The shader now integrates
        // the sorted value interval without constructing these cuts.
        fn piecewise(u0: f64, u1: f64) -> f64 {
            let delta = u1 - u0;
            if delta == 0.0 {
                return u0.clamp(0.0, 1.0);
            }
            let mut cuts = vec![0.0, 1.0];
            for crossing in [-u0 / delta, (1.0 - u0) / delta] {
                if crossing > 0.0 && crossing < 1.0 {
                    cuts.push(crossing);
                }
            }
            cuts.sort_by(f64::total_cmp);
            cuts.windows(2)
                .map(|segment| {
                    let [left, right] = [segment[0], segment[1]];
                    let a = (u0 + delta * left).clamp(0.0, 1.0);
                    let b = (u0 + delta * right).clamp(0.0, 1.0);
                    (a + b) * (right - left) * 0.5
                })
                .sum()
        }

        fn closed_form(u0: f64, u1: f64) -> f64 {
            let lo = u0.min(u1);
            let hi = u0.max(u1);
            if hi <= 0.0 {
                return 0.0;
            }
            if lo >= 1.0 {
                return 1.0;
            }
            let span = hi - lo;
            if span == 0.0 {
                return lo.clamp(0.0, 1.0);
            }
            let a = lo.max(0.0);
            let b = hi.min(1.0);
            let middle = (b - a).max(0.0) * (0.5 * (a + b));
            let tail = (hi - lo.max(1.0)).max(0.0);
            (middle + tail) / span
        }

        let values = [
            -1e6,
            -2.0,
            -1.0,
            -1e-12,
            0.0,
            1e-12,
            0.25,
            0.5,
            1.0 - 1e-12,
            1.0,
            1.0 + 1e-12,
            2.0,
            1e6,
        ];
        for u0 in values {
            for u1 in values {
                let actual = closed_form(u0, u1);
                let expected = piecewise(u0, u1);
                // The old crossing calculation can lose a few ulps when
                // the endpoints span a million units in either direction.
                assert!(
                    (actual - expected).abs() <= 1e-10,
                    "u0={u0}, u1={u1}: closed={actual}, piecewise={expected}"
                );
                assert!((0.0..=1.0).contains(&actual));
            }
        }
    }

    #[test]
    fn row_index_includes_every_edge_at_fractional_phases() {
        let mut path = BezPath::new();
        let epsilon = 1.0 / 4096.0;
        let rectangles = [
            (0.0, 0.0, 9.5, epsilon),
            (12.0, epsilon, 9.5, 1.0 - 2.0 * epsilon),
            (24.0, 1.0 - epsilon, 9.5, 2.0 * epsilon),
            (36.0, 1.0, 9.5, epsilon),
            (48.0, 1.0 + epsilon, 9.5, 1.0 - epsilon),
            (60.0, 2.5, 9.5, 3.5),
            (72.0, 3.0 - epsilon, 9.5, 1.0 + epsilon),
            (84.0, 4.0, 9.5, 0.5),
        ];
        for (x, y, w, h) in rectangles {
            path.move_to((x, y));
            path.line_to((x + w, y));
            path.line_to((x + w, y + h));
            path.line_to((x, y + h));
            path.close_path();
        }
        let (data, meta) = direct_texels(&path).expect("small contours fit");
        // Each closed contour has four edges and a duplicated first point.
        for phase in (0..64)
            .map(|step| step as f32 / 64.0)
            .chain([1.0 - f32::EPSILON])
        {
            for output_y in -3..11 {
                let local_y = output_y as f32 - phase;
                let row = local_y.floor() as i32 - meta.row_start;
                let listed: Vec<u32> = if row < 0 || row >= meta.rows as i32 {
                    Vec::new()
                } else {
                    let [start, count] = data[meta.table as usize + row as usize];
                    (start as u32..start as u32 + count as u32)
                        .map(|position| {
                            let pair = data[meta.list as usize + (position / 2) as usize];
                            if position & 1 == 0 {
                                pair[0] as u32
                            } else {
                                pair[1] as u32
                            }
                        })
                        .collect()
                };
                for contour in 0..rectangles.len() {
                    let first = contour as u32 * 5;
                    for edge in first..first + 4 {
                        let a = data[edge as usize][1];
                        let b = data[edge as usize + 1][1];
                        if a != b && a.min(b) < local_y + 1.0 && a.max(b) > local_y {
                            assert!(
                                listed.contains(&edge),
                                "missing edge {edge} at phase {phase}, row {row}, [{a}, {b}]"
                            );
                        }
                    }
                    assert!(!listed.contains(&(first + 4)), "indexed a contour jump");
                }
            }
        }
    }

    /// Chunks evicted to admit a new one come back as payloads (their
    /// textures are reused from the next frame instead of dropped).
    #[test]
    fn evicted_chunks_hand_back_their_payloads() {
        let mut cache: DirectCache<u32, ()> = DirectCache::new(2 * 4096);
        let mut evicted = Vec::new();
        cache.admit(1, 4096, 1, &mut evicted);
        cache.admit(2, 4096, 2, &mut evicted);
        assert!(evicted.is_empty());
        cache.admit(3, 4096, 3, &mut evicted);
        assert_eq!(evicted, [1], "the least recently used chunk");
        cache.admit(4, 2 * 4096, 4, &mut evicted);
        evicted.sort();
        assert_eq!(evicted, [1, 2, 3]);
    }
}

#[cfg(test)]
mod atlas_tests {
    use super::*;
    use pm_text::render::bitmap::Rgba8;
    use pm_text::render::colr::RgbaF32;
    use pm_text::render::scene::{Item, PathStyle, PlacedPath};

    /// Every placed image lies inside its page, pages stay within `side`
    /// unless they hold one oversized image, and no two images on a page
    /// share a texel.
    fn assert_valid(sizes: &[(u32, u32)], pack: &ShelfPack, side: u32) {
        assert_eq!(pack.places.len(), sizes.len());
        let mut on_page: Vec<Vec<(u32, u32, u32, u32)>> = vec![Vec::new(); pack.pages.len()];
        for (index, (&(w, h), place)) in sizes.iter().zip(&pack.places).enumerate() {
            let Some(place) = place else {
                assert!(w == 0 || h == 0, "image {index} has texels but no place");
                continue;
            };
            let (pw, ph) = pack.pages[place.page];
            assert!(
                place.x + w <= pw && place.y + h <= ph,
                "image {index} ({w}×{h} at {},{}) leaves its {pw}×{ph} page",
                place.x,
                place.y
            );
            on_page[place.page].push((place.x, place.y, w, h));
        }
        for (page, rects) in on_page.iter().enumerate() {
            let (pw, ph) = pack.pages[page];
            assert!(!rects.is_empty(), "page {page} holds nothing");
            assert!(
                (pw <= side && ph <= side) || rects.len() == 1,
                "page {page} is {pw}×{ph} beyond side {side} with several images"
            );
            for (a, &(ax, ay, aw, ah)) in rects.iter().enumerate() {
                for &(bx, by, bw, bh) in &rects[a + 1..] {
                    let apart = ax + aw <= bx || bx + bw <= ax || ay + ah <= by || by + bh <= ay;
                    assert!(apart, "images overlap on page {page}");
                }
            }
        }
    }

    #[test]
    fn shelf_pack_of_nothing_is_empty() {
        let pack = shelf_pack(&[], 64, false);
        assert!(pack.pages.is_empty() && pack.places.is_empty());
    }

    #[test]
    fn one_image_gets_a_page_of_its_size() {
        let pack = shelf_pack(&[(5, 7)], 64, false);
        assert_eq!(pack.pages, [(5, 7)]);
        assert_eq!(
            pack.places,
            [Some(ShelfPlace {
                page: 0,
                x: 0,
                y: 0
            })]
        );
    }

    #[test]
    fn images_fill_shelves_tallest_first() {
        // 56 texels: shelves 8 wide. The tallest starts the first shelf.
        let sizes = [(4, 4), (4, 8), (4, 2)];
        let pack = shelf_pack(&sizes, 64, false);
        assert_eq!(pack.pages, [(8, 10)]);
        assert_eq!(
            pack.places,
            [
                Some(ShelfPlace {
                    page: 0,
                    x: 4,
                    y: 0
                }),
                Some(ShelfPlace {
                    page: 0,
                    x: 0,
                    y: 0
                }),
                Some(ShelfPlace {
                    page: 0,
                    x: 0,
                    y: 8
                }),
            ]
        );
        assert_valid(&sizes, &pack, 64);
    }

    #[test]
    fn an_exact_fit_fills_one_page() {
        let sizes = [(8, 8); 4];
        let pack = shelf_pack(&sizes, 16, false);
        assert_eq!(pack.pages, [(16, 16)]);
        let corners: Vec<_> = pack.places.iter().map(|p| p.map(|p| (p.x, p.y))).collect();
        assert_eq!(
            corners,
            [Some((0, 0)), Some((8, 0)), Some((0, 8)), Some((8, 8))]
        );
        assert_valid(&sizes, &pack, 16);
    }

    #[test]
    fn a_full_page_opens_the_next() {
        let sizes = [(8, 8); 5];
        let pack = shelf_pack(&sizes, 16, false);
        assert_eq!(pack.pages, [(16, 16), (8, 8)]);
        assert_eq!(
            pack.places[4],
            Some(ShelfPlace {
                page: 1,
                x: 0,
                y: 0
            })
        );
        assert_valid(&sizes, &pack, 16);
    }

    #[test]
    fn oversized_and_empty_images() {
        // Wider or taller than a page: a page of its own size. No texels:
        // no place, no page.
        let sizes = [(3, 3), (40, 2), (0, 9), (2, 33), (9, 0), (4, 4)];
        let pack = shelf_pack(&sizes, 32, false);
        assert_eq!(pack.places[2], None);
        assert_eq!(pack.places[4], None);
        let own = |index: usize| {
            let place = pack.places[index].expect("placed");
            (place.x, place.y, pack.pages[place.page])
        };
        assert_eq!(own(1), (0, 0, (40, 2)));
        assert_eq!(own(3), (0, 0, (2, 33)));
        assert_eq!(pack.places[0].unwrap().page, pack.places[5].unwrap().page);
        assert_eq!(pack.pages.len(), 3);
        assert_valid(&sizes, &pack, 32);
    }

    #[test]
    fn single_mode_gives_every_image_its_own_texture() {
        let sizes = [(3, 3), (5, 2), (0, 4), (7, 7)];
        let pack = shelf_pack(&sizes, 64, true);
        assert_eq!(pack.pages, [(3, 3), (5, 2), (7, 7)]);
        assert!(pack.places.iter().flatten().all(|p| (p.x, p.y) == (0, 0)));
        assert_valid(&sizes, &pack, 64);
    }

    #[test]
    fn random_packs_are_disjoint_and_within_their_pages() {
        let mut state = 0x2545_f491_u32;
        let mut next = |limit: u32| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) % limit
        };
        for side in [16, 64, 512] {
            for count in [1, 7, 64, 400] {
                let sizes: Vec<(u32, u32)> = (0..count).map(|_| (next(48), next(48))).collect();
                let pack = shelf_pack(&sizes, side, false);
                assert_valid(&sizes, &pack, side);
                assert_eq!(pack, shelf_pack(&sizes, side, false), "deterministic");
            }
        }
    }

    fn gpu() -> pm_gpu::Gpu {
        pm_gpu::Gpu::new("text glyph atlas bit identity").expect("a Metal device")
    }

    /// A deterministic premultiplied RGBA8 glyph mask with transparent,
    /// partial and opaque texels.
    fn mask(width: u32, height: u32, seed: u32) -> ImagePixels {
        let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(12_345);
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let a = match (state >> 24) as u8 {
                a if a < 48 => 0,
                a if a > 224 => 255,
                a => a,
            };
            for c in 0..3u32 {
                let v = (state >> (c * 7)) & 0xff;
                data.push((v * u32::from(a) / 255) as u8);
            }
            data.push(a);
        }
        ImagePixels::Rgba8(Arc::new(Rgba8 {
            width,
            height,
            data,
        }))
    }

    fn image_item(image: ImagePixels, transform: Affine, alpha: f64, blur: f64) -> Item {
        let (w, h) = image.dimensions();
        Item {
            bounds: transform.transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(w), f64::from(h))),
            content: ItemContent::Image { image, transform },
            color: [1.0, 1.0, 1.0, alpha],
            opacity: 1.0,
            blur,
            curve: pm_text::render::smoothing::IDENTITY,
        }
    }

    /// An underline-like coverage bar drawn between glyph images.
    fn bar(x: f64, y: f64, w: f64, h: f64) -> Item {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((w, 0.0));
        path.line_to((w, h));
        path.line_to((0.0, h));
        path.close_path();
        Item {
            content: ItemContent::Coverage {
                paths: vec![PlacedPath {
                    path: Arc::new(path),
                    transform: Affine::translate((x, y)),
                    style: PathStyle::Fill,
                }],
            },
            color: [0.2, 0.6, 0.9, 0.8],
            opacity: 1.0,
            blur: 0.0,
            bounds: Rect::new(x, y, x + w, y + h),
            curve: pm_text::render::smoothing::IDENTITY,
        }
    }

    /// Three lines of kerned, subpixel-placed glyph masks at `scale` (glyphs
    /// repeat, so masks are shared), coverage bars between them, a rotated
    /// translucent mask, a blurred one (its own passes) and a working-domain
    /// float image.
    fn scene(scale: f64, phase: f64, extra: Option<Item>) -> TextScene {
        let glyphs: Vec<ImagePixels> = (0..14)
            .map(|g| mask(6 + (g * 3) % 17, 9 + (g * 5) % 19, g))
            .collect();
        let mut items = Vec::new();
        for line in 0..3u32 {
            let mut x = 5.0 + phase + f64::from(line) * 0.37;
            let y = 6.0 + f64::from(line) * (4.0 + 28.0 * scale) + 0.21 * f64::from(line);
            for k in 0..12usize {
                let glyph = &glyphs[(line as usize * 5 + k * 3) % glyphs.len()];
                let transform = Affine::translate((x, y)) * Affine::scale(scale);
                items.push(image_item(glyph.clone(), transform, 0.9, 0.0));
                // Kerned: the next glyph overlaps this one, at a new phase.
                x += f64::from(glyph.dimensions().0) * scale * 0.8 + 0.173;
                if k == 5 {
                    items.push(bar(x - 9.0, y + 18.0 * scale, 14.25, 1.5));
                }
            }
        }
        items.push(image_item(
            mask(20, 16, 99),
            Affine::translate((40.3, 70.6)) * Affine::rotate(0.35) * Affine::scale(scale * 1.3),
            0.6,
            0.0,
        ));
        items.push(image_item(
            glyphs[3].clone(),
            Affine::translate((60.45, 20.7)) * Affine::scale(scale),
            1.0,
            1.25,
        ));
        items.push(image_item(
            ImagePixels::WorkingF32(Arc::new(RgbaF32 {
                width: 6,
                height: 5,
                data: (0..30)
                    .flat_map(|i| {
                        let a = (i % 7) as f32 / 6.0;
                        [0.3 * a, 0.7 * a, 0.1 * a, a]
                    })
                    .collect(),
            })),
            Affine::translate((90.6, 50.2)) * Affine::scale(scale * 1.7),
            0.85,
            0.0,
        ));
        items.extend(extra);
        TextScene {
            width: 320,
            height: 160,
            items,
        }
    }

    fn render(
        painter: &mut TextGpu,
        gpu: &pm_gpu::Gpu,
        plan: &TextPlan,
        precision: Precision,
        working: WorkingEncoding,
    ) -> Vec<u32> {
        let mut pool = TargetPool::new();
        let mut held = Vec::new();
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("text glyph atlas bit identity"),
            });
        let view = painter
            .render(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &mut pool,
                &mut held,
                plan,
                precision,
                working,
            )
            .expect("text raster");
        gpu.queue.submit([encoder.finish()]);
        let pixels = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, view.texture())
            .expect("text raster readback");
        for target in held {
            pool.give(target);
        }
        // The frame is submitted.
        painter.begin_frame();
        pixels.into_iter().map(f32::to_bits).collect()
    }

    fn differing(a: &[u32], b: &[u32]) -> usize {
        assert_eq!(a.len(), b.len());
        a.iter().zip(b).filter(|(a, b)| a != b).count()
    }

    /// Glyph masks packed into shared atlas pages paint every texel of the
    /// raster with exactly the bits one texture per mask painted with the
    /// image shader as it was: across scales that enlarge (bilinear) and
    /// shrink (area integration, e.g. 0.25 × 1.03), subpixel phases,
    /// overlapping translucent glyphs, coverage drawn between them, rotation,
    /// blur, both working encodings, f32 and f16 rasters, small pages (many
    /// pages, own pages for larger masks) and masks the cache already holds.
    #[test]
    fn atlas_pages_paint_the_bits_of_one_texture_per_image() {
        let gpu = gpu();
        let mut reference = TextGpu::new(&gpu.device);
        reference.use_reference_images(&gpu.device);
        let mut atlas = TextGpu::new(&gpu.device);
        let mut small = TextGpu::new(&gpu.device);
        small.atlas_side = 24;
        let mut checked = 0usize;
        // Masks are cached by content and working encoding: only the first
        // raster of each encoding uploads them.
        let mut uploaded: Vec<WorkingEncoding> = Vec::new();
        for precision in [Precision::Float32, Precision::Float16] {
            for working in [WorkingEncoding::Standard, WorkingEncoding::LinearLight] {
                for scale in [1.0, 0.25 * 1.03, 0.25 * 1.03 * 4.0, 0.7, 1.6] {
                    for phase in [0.0, 0.37] {
                        let plan = TextPlan::whole(scene(scale, phase, None));
                        let before = atlas.stats;
                        let want = render(&mut reference, &gpu, &plan, precision, working);
                        let got = render(&mut atlas, &gpu, &plan, precision, working);
                        let paged = render(&mut small, &gpu, &plan, precision, working);
                        let label =
                            format!("{precision:?} {working:?} scale {scale} phase {phase}");
                        assert_eq!(differing(&want, &got), 0, "atlas pages, {label}");
                        assert_eq!(differing(&want, &paged), 0, "small atlas pages, {label}");
                        let painted = want
                            .chunks_exact(4)
                            .filter(|p| f32::from_bits(p[3]) > 0.0)
                            .count();
                        assert!(painted > 100, "{label}: only {painted} pixels painted");
                        let uploads = atlas.stats.image_uploads - before.image_uploads;
                        let pages = atlas.stats.image_pages - before.image_pages;
                        if !uploaded.contains(&working) {
                            uploaded.push(working);
                            // 14 masks (one also blurred), a rotated mask
                            // and a float image, on one page.
                            assert_eq!((uploads, pages), (16, 1), "{label}");
                        } else {
                            // The same masks at another scale, phase or
                            // precision: all cached.
                            assert_eq!((uploads, pages), (0, 0), "{label}");
                        }
                        checked += want.len();
                    }
                }
            }
        }
        assert_eq!(
            reference.stats.image_pages, reference.stats.image_uploads,
            "the reference keeps one texture per image"
        );
        assert!(small.stats.image_pages > 2 * atlas.stats.image_pages);
        println!("{checked} channel values compared: 0 differing bits");
    }

    #[test]
    fn glyph_shelf_never_reuses_an_uploaded_subrect() {
        let mut shelf = GlyphShelf::default();
        let first = shelf.append(256, 128).unwrap();
        let second = shelf.append(256, 256).unwrap();
        let third = shelf.append(512, 256).unwrap();
        assert_eq!((first, second, third), ((0, 0), (256, 0), (0, 256)));
        assert_eq!(shelf.append(1, 1), None);
        assert_eq!(shelf.append(513, 1), None);
    }

    #[test]
    fn multiple_unsubmitted_rasters_share_pages_and_keep_their_original_bits() {
        let gpu = gpu();
        let mut atlas = TextGpu::new(&gpu.device);
        let mut reference = TextGpu::new(&gpu.device);
        reference.use_reference_images(&gpu.device);
        let plans: Vec<_> = [(16, 12), (16, 12), (512, 512), (512, 512), (513, 12)]
            .into_iter()
            .enumerate()
            .map(|(seed, (w, h))| {
                TextPlan::whole(TextScene {
                    width: 48,
                    height: 32,
                    items: vec![image_item(
                        mask(w, h, 10 + seed as u32),
                        Affine::translate((1.125, 2.5)),
                        0.75,
                        0.0,
                    )],
                })
            })
            .collect();
        let mut pool = TargetPool::new();
        let mut taken = Vec::new();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let mut views = Vec::new();
        for plan in &plans {
            views.push(
                atlas
                    .render(
                        &gpu.device,
                        &gpu.queue,
                        &mut encoder,
                        &mut pool,
                        &mut taken,
                        plan,
                        Precision::Float32,
                        WorkingEncoding::Standard,
                    )
                    .expect("text raster"),
            );
        }
        assert_eq!(atlas.frame_glyphs.bytes(), IMAGE_CACHE_BYTES);
        assert_eq!(atlas.images.bytes, 0, "overflow images are transient");
        assert_eq!(atlas.frame_glyphs.entries.len(), 3);
        assert_eq!(atlas.item_uniforms.chunks.len(), 1);
        let regions: Vec<_> = plans[..2]
            .iter()
            .map(|p| {
                let ItemContent::Image { image, .. } = &p.scene.items[0].content else {
                    panic!("expected test image");
                };
                let entry =
                    &atlas.frame_glyphs.entries[&image_key(image, WorkingEncoding::Standard)].2;
                (entry.page, entry.region)
            })
            .collect();
        assert_eq!(regions, vec![(0, [0, 0, 16, 12]), (0, [16, 0, 16, 12])]);
        gpu.queue.submit([encoder.finish()]);
        atlas.end_frame();
        assert!(
            atlas.frame_glyphs.entries.is_empty(),
            "full pages can recycle after submit"
        );

        for (plan, view) in plans.iter().zip(views) {
            let want = render(
                &mut reference,
                &gpu,
                plan,
                Precision::Float32,
                WorkingEncoding::Standard,
            );
            let got: Vec<_> = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, view.texture())
                .expect("text raster readback")
                .into_iter()
                .map(f32::to_bits)
                .collect();
            assert!(want.iter().any(|&value| value != 0));
            assert_eq!(differing(&want, &got), 0);
        }
    }

    /// A mask without texels draws nothing (one texture per image could not
    /// even make its texture).
    #[test]
    fn an_empty_mask_draws_nothing() {
        let gpu = gpu();
        let mut atlas = TextGpu::new(&gpu.device);
        let empty = image_item(
            ImagePixels::Rgba8(Arc::new(Rgba8 {
                width: 0,
                height: 3,
                data: Vec::new(),
            })),
            Affine::translate((30.5, 30.5)),
            1.0,
            0.0,
        );
        let plain = TextPlan::whole(scene(1.0, 0.0, None));
        let with_empty = TextPlan::whole(scene(1.0, 0.0, Some(empty)));
        let a = render(
            &mut atlas,
            &gpu,
            &plain,
            Precision::Float32,
            WorkingEncoding::Standard,
        );
        let b = render(
            &mut atlas,
            &gpu,
            &with_empty,
            Precision::Float32,
            WorkingEncoding::Standard,
        );
        assert_eq!(differing(&a, &b), 0);
    }

    /// Pages stay within the image cache's bytes, and clearing the caches
    /// releases them.
    #[test]
    fn atlas_pages_are_bounded_and_released() {
        let gpu = gpu();
        let mut atlas = TextGpu::new(&gpu.device);
        for phase in 0..40 {
            // New masks every raster: the cache evicts whole pages.
            let glyphs: Vec<Item> = (0..60u32)
                .map(|g| {
                    image_item(
                        mask(40, 48, phase * 100 + g),
                        Affine::translate((f64::from(g % 10) * 30.0, f64::from(g / 10) * 25.0)),
                        1.0,
                        0.0,
                    )
                })
                .collect();
            let plan = TextPlan::whole(TextScene {
                width: 320,
                height: 160,
                items: glyphs,
            });
            render(
                &mut atlas,
                &gpu,
                &plan,
                Precision::Float16,
                WorkingEncoding::Standard,
            );
            assert!(atlas.images.bytes + atlas.frame_glyphs.bytes() <= IMAGE_CACHE_BYTES);
        }
        assert!(atlas.images.bytes + atlas.frame_glyphs.bytes() > 0);
        atlas.trim(0);
        assert_eq!(atlas.images.bytes + atlas.frame_glyphs.bytes(), 0);
        assert!(atlas.image_scratch.capacity() == 0);
    }
}
