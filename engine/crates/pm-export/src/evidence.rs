//! Picture evidence (section 17, "Verification"): what an export keeps of
//! the frames it hands the encoder, and how the decoded file is held to it.
//!
//! The rule (`docs/engine/delivery.md`, "The file check"): an export fails
//! only on evidence that no codec loss at any supported data rate can
//! explain. Anything that is a matter of degree is judged against what the
//! same encode does elsewhere, or reported as a note with the file kept.
//!
//! Two things are kept. Of every frame, a coarse signature: the frame's
//! channel means and an 8 × 8 grid of means of each of Y′, Cb, Cr and alpha
//! ([`FrameSignature`]). Of the first, middle and last frame, the samples of
//! 48 × 48 regions at nine places ([`BlockSample`]), each nine 16 × 16
//! blocks that are judged against each other.
//!
//! What fails an export (each grid cell is the mean of 1,024 samples of an
//! eighth of the frame each way, which every codec keeps at every rate):
//!
//! * a frame that holds another frame's picture: clearly nearer a
//!   neighbour's reference than its own, while its own distance stands out
//!   against the export's typical one (stale, repeated, swapped frames);
//! * a frame, or a part of one, that is not the picture: its distance, or
//!   one cell's, many times the export's typical one;
//! * a channel lost across a frame: the frame keeps under a quarter of the
//!   picture its luma, colour or alpha grid carries;
//! * a level, range or matrix mistake: a shift of most of the picture that
//!   every frame shares, beyond what a codec's quantisation does to the
//!   flattest saturated picture, or a gain of the whole export's grid;
//! * for ProRes, which codes each block on its own from its own samples, a
//!   16-pixel block that went flat, or holds a picture of its own, where
//!   its neighbours of the same kind kept theirs; and for every codec a
//!   picture displaced as a whole.
//!
//! What a long-GOP codec does to one block of a predicted frame (a skipped
//! or intra-predicted block shows a picture that is not the frame's own at
//! a low rate) is reported as a note.
//!
//! What this cannot tell: a stale or swapped frame whose neighbours look
//! alike at the grid's scale, damage smaller than a grid cell away from the
//! sampled regions, and one wrong block of a long-GOP predicted frame.

use crate::convert::{EncoderInput, sample_ycbcra};
use crate::settings::{Codec, ProResProfile};

/// Side of a block judged against its neighbours.
pub const BLOCK: u32 = 16;

/// Side of a sampled region: three blocks each way.
pub const REGION: u32 = 3 * BLOCK;

/// Side of a cell of a block's low-frequency fingerprint.
const CELL: u32 = 4;

/// A frame signature's grids are this many cells each way.
pub const GRID: usize = 8;

const CELLS: usize = GRID * GRID;

/// Samples taken across each grid cell, each way.
const CELL_SAMPLES: usize = 32;

/// A region of the picture: normalized Y′, Cb, Cr, A.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockSample {
    pub frame: u64,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub mean: [f64; 4],
    /// How busy each channel is: half the spread of the middle four fifths
    /// of its samples (the 10th to the 90th percentile).
    pub activity: [f64; 4],
    /// The samples, row by row.
    pub samples: Vec<[f32; 4]>,
}

impl BlockSample {
    fn new(frame: u64, region: BlockRegion, values: Vec<[f64; 4]>) -> Self {
        let n = values.len() as f64;
        let mut mean = [0.0; 4];
        for v in &values {
            for k in 0..4 {
                mean[k] += v[k];
            }
        }
        let mean = mean.map(|v| v / n);
        let activity = std::array::from_fn(|k| {
            let mut channel: Vec<f64> = values.iter().map(|v| v[k]).collect();
            channel.sort_by(f64::total_cmp);
            let tail = channel.len() / 10;
            (channel[channel.len() - 1 - tail] - channel[tail]) / 2.0
        });
        Self {
            frame,
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
            mean,
            activity,
            samples: values.iter().map(|v| v.map(|c| c as f32)).collect(),
        }
    }

    fn at(&self, k: usize, x: u32, y: u32) -> f64 {
        f64::from(self.samples[(y * self.width + x) as usize][k])
    }

    /// The region's [`BLOCK`]-sided blocks, row by row, with their place in
    /// the region's grid; a region smaller than a block is one block.
    fn blocks(&self) -> Vec<((u32, u32), BlockSample)> {
        let (across, down) = (self.width / BLOCK, self.height / BLOCK);
        if across == 0 || down == 0 {
            return vec![((0, 0), self.clone())];
        }
        (0..down)
            .flat_map(|j| (0..across).map(move |i| (i, j)))
            .map(|(i, j)| {
                let values = (j * BLOCK..(j + 1) * BLOCK)
                    .flat_map(|y| (i * BLOCK..(i + 1) * BLOCK).map(move |x| (x, y)))
                    .map(|(x, y)| self.samples[(y * self.width + x) as usize].map(f64::from))
                    .collect();
                let region = BlockRegion {
                    x: self.x + i * BLOCK,
                    y: self.y + j * BLOCK,
                    width: BLOCK,
                    height: BLOCK,
                };
                ((i, j), BlockSample::new(self.frame, region, values))
            })
            .collect()
    }

    /// The channel's 4 × 4 cell means with their own mean taken off
    /// (`None`: the block has fewer than four cells).
    fn cells(&self, k: usize) -> Option<Vec<f64>> {
        let (across, down) = (self.width / CELL, self.height / CELL);
        if across * down < 4 || self.samples.is_empty() {
            return None;
        }
        let mut cells: Vec<f64> = (0..down)
            .flat_map(|j| (0..across).map(move |i| (i, j)))
            .map(|(i, j)| {
                let mut sum = 0.0;
                for y in j * CELL..(j + 1) * CELL {
                    for x in i * CELL..(i + 1) * CELL {
                        sum += self.at(k, x, y);
                    }
                }
                sum / f64::from(CELL * CELL)
            })
            .collect();
        let mean = cells.iter().sum::<f64>() / cells.len() as f64;
        cells.iter_mut().for_each(|c| *c -= mean);
        Some(cells)
    }
}

/// One of the verification regions in full-frame, top-down coordinates.
/// WebCodecs can convert just these pixels without changing the sampled
/// locations used by the decoded-file check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlockRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The frames whose regions are checked: first, middle, last.
pub fn sampled_frames(frames: u64) -> Vec<u64> {
    let mut v = vec![0, frames / 2, frames.saturating_sub(1)];
    v.dedup();
    v
}

/// The deduplicated 3 × 3 grid of regions, in the same order as the block
/// samples: each is centred on a 16-pixel-aligned block of a 3 × 3 grid
/// over the frame, moved inside the frame at its edges.
pub(crate) fn block_regions(width: u32, height: u32) -> Vec<BlockRegion> {
    let size_x = REGION.min(width);
    let size_y = REGION.min(height);
    let place = |extent: u32, size: u32, i: u32| -> u32 {
        let block = BLOCK.min(extent);
        let at = (extent - block) * i / 2 / BLOCK * BLOCK;
        at.saturating_sub(BLOCK).min(extent - size)
    };
    let mut out = vec![];
    for j in 0..3 {
        for i in 0..3 {
            let (x, y) = (place(width, size_x, i), place(height, size_y, j));
            if out.iter().any(|b: &BlockRegion| b.x == x && b.y == y) {
                continue;
            }
            out.push(BlockRegion {
                x,
                y,
                width: size_x,
                height: size_y,
            });
        }
    }
    out
}

/// The regions of a 3 × 3 grid.
pub fn block_means(input: &EncoderInput, frame: u64) -> Vec<BlockSample> {
    block_regions(input.width, input.height)
        .into_iter()
        .map(|region| {
            let values = (region.y..region.y + region.height)
                .flat_map(|y| (region.x..region.x + region.width).map(move |x| (x, y)))
                .map(|(x, y)| sample_ycbcra(input, x as usize, y as usize))
                .collect();
            BlockSample::new(frame, region, values)
        })
        .collect()
}

/// A frame's coarse picture: what tells it from its neighbours and from a
/// frame with other levels, colours or alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameSignature {
    /// The means of Y′, Cb, Cr and A over the sampled pixels.
    pub mean: [f32; 4],
    /// Means of a [`GRID`] × [`GRID`] grid over the frame, row by row, of
    /// each of Y′, Cb, Cr and A.
    pub cells: [[f32; CELLS]; 4],
}

/// Sample positions along `extent`: up to [`CELL_SAMPLES`] in each of
/// [`GRID`] equal spans, evenly spaced.
fn sample_positions(extent: usize) -> [Vec<usize>; GRID] {
    std::array::from_fn(|cell| {
        let (start, end) = (cell * extent / GRID, (cell + 1) * extent / GRID);
        let span = end - start;
        let count = span.min(CELL_SAMPLES);
        (0..count)
            .map(|t| start + (2 * t + 1) * span / (2 * count))
            .collect()
    })
}

/// The signature of a frame, from 65,536 of its pixels at most.
pub fn frame_signature(input: &EncoderInput) -> FrameSignature {
    signature(input.width as usize, input.height as usize, |x, y| {
        sample_ycbcra(input, x, y)
    })
}

/// The signature of a `width` × `height` frame whose pixel (x, y) is
/// `sample(x, y)` (normalized Y′, Cb, Cr, A).
pub(crate) fn signature(
    width: usize,
    height: usize,
    sample: impl Fn(usize, usize) -> [f64; 4],
) -> FrameSignature {
    let xs = sample_positions(width);
    let ys = sample_positions(height);
    let mut cells = [[0.0f32; CELLS]; 4];
    let (mut total, mut count) = ([0.0f64; 4], 0usize);
    for (j, rows) in ys.iter().enumerate() {
        for (i, columns) in xs.iter().enumerate() {
            let mut sum = [0.0f64; 4];
            for &y in rows {
                for &x in columns {
                    let s = sample(x, y);
                    for k in 0..4 {
                        sum[k] += s[k];
                    }
                }
            }
            let n = rows.len() * columns.len();
            for k in 0..4 {
                cells[k][j * GRID + i] = if n == 0 {
                    0.0
                } else {
                    (sum[k] / n as f64) as f32
                };
                total[k] += sum[k];
            }
            count += n;
        }
    }
    FrameSignature {
        mean: total.map(|v| (v / count.max(1) as f64) as f32),
        cells,
    }
}

/// What an export keeps of the frames it hands the encoder.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Evidence {
    /// The regions of the sampled frames.
    pub blocks: Vec<BlockSample>,
    /// Every frame's signature, in frame order.
    pub frames: Vec<FrameSignature>,
}

impl Evidence {
    /// Takes in frame `index` of `frames` (frames arrive in order).
    pub fn add(&mut self, index: u64, frames: u64, input: &EncoderInput) {
        if sampled_frames(frames).contains(&index) {
            self.blocks.extend(block_means(input, index));
        }
        self.frames.push(frame_signature(input));
    }
}

/// What the file decoded to: the sampled frames whole, and the signatures
/// of the frames that were decoded, by index.
#[derive(Clone, Debug, Default)]
pub struct Decoded {
    pub sampled: Vec<(u64, EncoderInput)>,
    pub frames: Vec<(u64, FrameSignature)>,
    /// Every frame of the file was decoded.
    pub complete: bool,
}

impl Decoded {
    /// Takes in decoded frame `index` of `frames`, in presentation order.
    pub fn add(&mut self, index: u64, frames: u64, frame: EncoderInput) {
        self.frames.push((index, frame_signature(&frame)));
        if sampled_frames(frames).contains(&index) {
            self.sampled.push((index, frame));
        }
    }

    /// [`Decoded::add`] for a frame that is read only as far as it is
    /// needed: its signature, and the frame whole (`read`) when it is one
    /// of the sampled ones.
    pub fn add_with<E>(
        &mut self,
        index: u64,
        frames: u64,
        signature: FrameSignature,
        read: impl FnOnce() -> Result<EncoderInput, E>,
    ) -> Result<(), E> {
        self.frames.push((index, signature));
        if sampled_frames(frames).contains(&index) {
            self.sampled.push((index, read()?));
        }
        Ok(())
    }
}

/// What was measured of one sampled 16-pixel block.
#[derive(Clone, Debug, PartialEq)]
pub struct PixelCheck {
    pub frame: u64,
    pub x: u32,
    pub y: u32,
    pub expected: [f64; 4],
    pub decoded: [f64; 4],
    /// Largest channel difference of the block's mean, in code values at
    /// the codec's depth.
    pub error_cv: f64,
    /// The largest channel activity of the block the encoder was handed, in
    /// code values (how busy the picture is there).
    pub activity_cv: f64,
    /// The mean's difference beyond the codec's flat noise, as a fraction
    /// of the same channel's activity (the largest of the channels).
    pub excess: f64,
    /// The lowest correlation of a channel's low-frequency picture with the
    /// reference's (channels with enough contrast to judge).
    pub correlation: Option<f64>,
    /// The fraction of the block's luma activity the file keeps (blocks
    /// busy enough to judge).
    pub detail: Option<f64>,
    /// The largest part of a channel's picture that the reference does not
    /// explain (RMS, code values): a codec's noise, or a picture of its own.
    pub foreign_cv: f64,
}

/// What the picture checks measured over the whole file.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PictureReport {
    /// Frames whose signature was held to the encoder input's.
    pub frames_checked: u64,
    /// The largest signed difference of a frame's channel mean, in codes.
    pub frame_bias_cv: f64,
    /// The largest signed shift of most of the picture that every frame
    /// shares, in codes.
    pub level_cv: f64,
    /// The export's typical distance of a frame's grids from its own
    /// reference (the median over frames, RMS, in luma codes).
    pub typical_cv: f64,
    /// The largest distance of a frame's grids from its own reference.
    pub frame_distance_cv: f64,
    /// The largest difference of one grid cell from its reference beyond
    /// the frame's shift (any channel, in luma codes).
    pub frame_cell_cv: f64,
    /// The smallest ratio over frames of the distance to a neighbour's
    /// reference to the distance to its own: a frame holding a neighbour's
    /// picture gives well under one (`None`: no neighbours).
    pub identity: Option<f64>,
    /// The least fraction of a channel's picture a frame keeps, over the
    /// channels with a picture to judge.
    pub kept: Option<f64>,
    /// The largest departure from one of a channel's gain over the export.
    pub gain: Option<f64>,
    /// The smallest ratio of the sampled regions' squared luma difference
    /// with the reference moved one pixel to the difference in place
    /// (`None`: the regions are too close to the reference to tell).
    pub registration: Option<f64>,
}

/// A codec's noise on a flat picture, in code values at the codec's depth:
/// lossless PNG is exact; ProRes within 4 CV of 10 bits; the long-GOP
/// codecs within 6 CV at 8 bits (24 at 10 bits).
pub fn pixel_tolerance(codec: Codec) -> f64 {
    match codec {
        Codec::Png { .. } => 0.0,
        Codec::ProRes(p) if p.is_4444() => 16.0,
        Codec::ProRes(_) => 4.0,
        Codec::Hevc { ten_bit: true, .. } => 24.0,
        Codec::H264 { .. } | Codec::Hevc { .. } | Codec::Vp9 { .. } | Codec::Vp8 { .. } => 6.0,
    }
}

/// How far most of the picture of every frame may sit from the encoder
/// input's the same way, in code values at the codec's depth, for channel
/// `k` (Y′, Cb, Cr, A). A wrong level, range or matrix moves every frame;
/// a codec's quantisation of a flat saturated picture can too, in every
/// frame of a slate, and that sets the allowance: twice the largest such
/// shift measured on full-frame primary and secondary slates, very dark
/// and flat frames, at the lowest rate of each codec
/// (`tests/hard_delivery.rs`).
pub fn level_tolerance(codec: Codec, k: usize) -> f64 {
    let chroma = k == 1 || k == 2;
    match codec {
        Codec::Png { .. } => 0.0,
        Codec::ProRes(p) if p.is_4444() => match k {
            3 => 64.0,
            _ if chroma => 16.0,
            _ => 8.0,
        },
        Codec::ProRes(ProResProfile::Proxy) if chroma => 16.0,
        Codec::ProRes(_) if chroma => 4.0,
        Codec::ProRes(_) => 2.0,
        Codec::Hevc { ten_bit: true, .. } if chroma => 12.0,
        Codec::Hevc { ten_bit: true, .. } => 6.0,
        _ if chroma || k == 3 => 3.0,
        _ => 1.5,
    }
}

/// A frame holds a neighbour's picture when it is this many times nearer
/// that neighbour's reference than its own ...
const IDENTITY_RATIO: f64 = 3.0;
/// ... and its distance from its own is this many times the export's
/// typical one, plus [`IDENTITY_FLOOR`].
const IDENTITY_OUTLIER: f64 = 3.0;
/// Normalized: 0.09 of a 10-bit code.
const IDENTITY_FLOOR: f64 = 1e-4;

/// A frame is not its picture when its distance from its own reference is
/// this many times the export's typical one plus [`FOREIGN_FLOOR`]
/// (normalized, about 1.8 codes of 10 bits over every grid of the frame).
const FOREIGN: f64 = 8.0;
const FOREIGN_FLOOR: f64 = 2e-3;
/// A part of a frame is not its picture when one cell sits this many times
/// the export's typical largest cell from its reference, plus
/// [`CELL_FLOOR`] (normalized, about 7 codes of 10 bits).
const CELL_OUTLIER: f64 = 8.0;
const CELL_FLOOR: f64 = 8e-3;

/// A channel is lost when a frame keeps less than this part of the
/// picture its grid carries, and what is left of it is under twice that
/// part of it (lost rather than replaced by another picture) ...
const LOST: f64 = 0.25;
/// ... where that picture is at least [`LOST_SIGNAL`] (normalized RMS,
/// about 9 codes of 10 bits) and this many times the export's typical
/// spread of the channel's cell differences.
const LOST_SIGNAL: f64 = 1e-2;
const LOST_NOISE: f64 = 6.0;

/// A frame's gain of a channel is judged when its reference grid's cells
/// spread this far (normalized standard deviation), and refused when every
/// judged frame's gain is beyond [`GAIN`] of one. A video/full range
/// mistake is a gain of 0.86 or 1.17.
const GAIN_SPREAD: f64 = 0.05;
const GAIN: f64 = 0.05;

/// A channel's low-frequency picture is measured when its contrast is at
/// least this many times the codec's flat noise and at least
/// [`FINGERPRINT_SHARE`] of the block's activity.
const FINGERPRINT_CONTRAST: f64 = 2.0;
const FINGERPRINT_SHARE: f64 = 0.5;

/// A block's luma detail is judged from this many times the flat noise.
const DETAIL_ACTIVITY: f64 = 4.0;

/// A block has gone flat when it keeps less than [`DETAIL_FLAT`] of its
/// luma activity while its neighbours of the same kind (activity within
/// [`ALIKE`] either way) keep [`DETAIL_KEPT`] of theirs at the median.
const DETAIL_FLAT: f64 = 0.1;
const DETAIL_KEPT: f64 = 0.5;
const ALIKE: f64 = 2.0;

/// A block holds a picture of its own when the part of a channel its
/// reference does not explain is over [`FOREIGN_BLOCK`] times its
/// neighbours' at the median and over [`FOREIGN_SHARE`] of the block's own
/// picture, each plus twice the flat noise.
const FOREIGN_BLOCK: f64 = 4.0;
const FOREIGN_SHARE: f64 = 0.5;

/// Neighbours a block needs to be judged against them.
const NEIGHBOURS_JUDGED: usize = 4;

/// Moving the reference one pixel must not bring the squared difference
/// under this fraction of the difference in place (a picture in place
/// gives 1 or more), in most of the frame's regions.
const REGISTRATION: f64 = 0.8;

/// Code values per unit of each normalized channel (Y′, Cb, Cr, A; or R, G, B, A
/// for RGB codecs) at the codec's depth.
pub fn channel_scale(codec: Codec, k: usize) -> f64 {
    let d = codec.depth() as i32;
    if !codec.is_ycbcr() {
        return 2f64.powi(d) - 1.0;
    }
    match k {
        0 => 219.0 * 2f64.powi(d - 8),
        1 | 2 => 224.0 * 2f64.powi(d - 8),
        // Alpha is 8-bit in HEVC with alpha and libvpx; 16-bit in ProRes.
        _ => match codec {
            Codec::ProRes(_) => 65535.0,
            _ => 255.0,
        },
    }
}

/// The codec codes each block from its own samples only (no prediction
/// from other frames or from neighbouring blocks' pixels): what one block
/// of it shows is certain evidence about that block.
fn codes_blocks_alone(codec: Codec) -> bool {
    matches!(codec, Codec::ProRes(_) | Codec::Png { .. })
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(values[values.len() / 2])
}

fn rms(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, n) = values.fold((0.0, 0usize), |(s, n), v| (s + v * v, n + 1));
    (sum / n.max(1) as f64).sqrt()
}

/// Compares sampled decoded frames with the encoder input's regions.
pub fn check_pixels(
    codec: Codec,
    blocks: &[BlockSample],
    decoded: &[(u64, EncoderInput)],
    picture: &mut PictureReport,
    problems: &mut Vec<String>,
    notes: &mut Vec<String>,
) -> Vec<PixelCheck> {
    let sampled = decoded
        .iter()
        .flat_map(|(frame, input)| block_means(input, *frame));
    check_pixel_blocks(codec, blocks, sampled, picture, problems, notes)
}

/// One block's measures, for judging it against its neighbours.
struct Judged {
    at: (u32, u32),
    place: (u32, u32),
    /// Luma activity of the reference, code values.
    activity: f64,
    /// Fraction of its luma activity kept (busy enough to judge).
    kept: Option<f64>,
    /// Per channel: the decoded picture the reference does not explain,
    /// and the reference's own picture (RMS about the mean), code values.
    foreign: [f64; 4],
    own: [f64; 4],
}

/// Compares already extracted decoded regions with the encoder input's.
pub(crate) fn check_pixel_blocks(
    codec: Codec,
    blocks: &[BlockSample],
    decoded: impl IntoIterator<Item = BlockSample>,
    picture: &mut PictureReport,
    problems: &mut Vec<String>,
    notes: &mut Vec<String>,
) -> Vec<PixelCheck> {
    let mut pixels = vec![];
    let lossless = matches!(codec, Codec::Png { .. });
    let flat = pixel_tolerance(codec);
    let scale = |k: usize| channel_scale(codec, k);
    let mut regions = 0usize;
    // Per frame: each judged region's best one-pixel move and its ratio.
    let mut moves: std::collections::BTreeMap<u64, Vec<(usize, f64)>> = Default::default();
    let mut squares_by_frame: std::collections::BTreeMap<u64, [f64; 5]> = Default::default();
    let mut local: Vec<(u64, String)> = vec![];
    for got in decoded {
        let Some(want) = blocks
            .iter()
            .find(|b| b.frame == got.frame && b.x == got.x && b.y == got.y)
        else {
            continue;
        };
        regions += 1;
        let comparable = (got.width, got.height, got.samples.len())
            == (want.width, want.height, want.samples.len());
        if !comparable {
            problems.push(format!(
                "frame {} region ({}, {}) decoded at another size",
                got.frame, got.x, got.y
            ));
            continue;
        }
        let pairs: Vec<_> = want.blocks().into_iter().zip(got.blocks()).collect();
        let mut judged = vec![];
        for (((i, j), w), (_, g)) in &pairs {
            let error = |k: usize| (g.mean[k] - w.mean[k]).abs() * scale(k);
            let activity = |k: usize| w.activity[k] * scale(k);
            let mut correlation = None::<f64>;
            for k in 0..4 {
                let (Some(r), Some(d)) = (w.cells(k), g.cells(k)) else {
                    continue;
                };
                let (rr, dd) = (rms(r.iter().copied()), rms(d.iter().copied()));
                if rr * scale(k) < FINGERPRINT_CONTRAST * flat.max(1.0)
                    || rr < FINGERPRINT_SHARE * w.activity[k]
                {
                    continue;
                }
                let dot: f64 = r.iter().zip(&d).map(|(a, b)| a * b).sum();
                let c = if dd > 0.0 {
                    dot / (r.len() as f64 * rr * dd)
                } else {
                    0.0
                };
                correlation = Some(correlation.map_or(c, |v| v.min(c)));
            }
            // The part of each channel's decoded picture the reference
            // does not explain, and the reference's own picture.
            let mut foreign = [0.0; 4];
            let mut own = [0.0; 4];
            for k in 0..4 {
                let n = w.samples.len() as f64;
                let (mut ww, mut gg, mut wg) = (0.0, 0.0, 0.0);
                for (a, b) in w.samples.iter().zip(&g.samples) {
                    let (a, b) = (f64::from(a[k]) - w.mean[k], f64::from(b[k]) - g.mean[k]);
                    ww += a * a;
                    gg += b * b;
                    wg += a * b;
                }
                let unexplained = if ww > 0.0 { gg - wg * wg / ww } else { gg };
                foreign[k] = (unexplained.max(0.0) / n).sqrt() * scale(k);
                own[k] = (ww / n).sqrt() * scale(k);
            }
            let detail = (activity(0) >= DETAIL_ACTIVITY * flat.max(1.0))
                .then(|| g.activity[0] / w.activity[0]);
            pixels.push(PixelCheck {
                frame: g.frame,
                x: g.x,
                y: g.y,
                expected: w.mean,
                decoded: g.mean,
                error_cv: (0..4).map(error).fold(0.0, f64::max),
                activity_cv: (0..4).map(activity).fold(0.0, f64::max),
                excess: (0..4)
                    .filter(|&k| activity(k) > 0.5)
                    .map(|k| (error(k) - flat).max(0.0) / activity(k))
                    .fold(0.0, f64::max),
                correlation,
                detail,
                foreign_cv: foreign.iter().copied().fold(0.0, f64::max),
            });
            if lossless {
                if (0..4).any(|k| error(k) > 1e-9) {
                    problems.push(format!(
                        "frame {} block ({}, {}) differs from what was rendered",
                        g.frame, g.x, g.y
                    ));
                }
                continue;
            }
            judged.push(Judged {
                at: (g.x, g.y),
                place: (*i, *j),
                activity: activity(0),
                kept: detail,
                foreign,
                own,
            });
        }
        // Each block against its neighbours in the region.
        for b in &judged {
            let neighbours: Vec<&Judged> = judged
                .iter()
                .filter(|o| {
                    o.place != b.place
                        && o.place.0.abs_diff(b.place.0) <= 1
                        && o.place.1.abs_diff(b.place.1) <= 1
                })
                .collect();
            if neighbours.len() < NEIGHBOURS_JUDGED {
                continue;
            }
            let at = format!("frame {} at ({}, {})", got.frame, b.at.0, b.at.1);
            if let Some(kept) = b.kept {
                let alike: Vec<f64> = neighbours
                    .iter()
                    .filter(|o| {
                        o.activity * ALIKE >= b.activity && o.activity <= b.activity * ALIKE
                    })
                    .filter_map(|o| o.kept)
                    .collect();
                if kept < DETAIL_FLAT
                    && alike.len() >= NEIGHBOURS_JUDGED
                    && median(alike.clone()).is_some_and(|m| m >= DETAIL_KEPT)
                {
                    local.push((
                        got.frame,
                        format!(
                            "{at}: a block lost its detail (kept {:.0} % where its neighbours kept {:.0} %)",
                            kept * 100.0,
                            median(alike).unwrap_or(0.0) * 100.0
                        ),
                    ));
                }
            }
            for k in 0..4 {
                let around =
                    median(neighbours.iter().map(|o| o.foreign[k]).collect()).unwrap_or(0.0);
                let f = b.foreign[k];
                if f > FOREIGN_BLOCK * around + 2.0 * flat
                    && f > FOREIGN_SHARE * b.own[k] + 2.0 * flat
                {
                    local.push((
                        got.frame,
                        format!(
                            "{at}: a block holds a picture that was not rendered ({f:.1} codes of it, {around:.1} around it)"
                        ),
                    ));
                    break;
                }
            }
        }
        // Registration of the region's luma as a whole.
        if !lossless && want.width > 2 && want.height > 2 {
            let mut squares = [0.0f64; 5];
            for (m, (dx, dy)) in MOVES.iter().enumerate() {
                for y in 1..want.height - 1 {
                    for x in 1..want.width - 1 {
                        let reference = want.at(0, (x as i32 - dx) as u32, (y as i32 - dy) as u32);
                        let d = (got.at(0, x, y) - reference) * scale(0);
                        squares[m] += d * d;
                    }
                }
            }
            let samples = f64::from((want.width - 2) * (want.height - 2));
            let pooled = squares_by_frame.entry(got.frame).or_default();
            for m in 0..5 {
                pooled[m] += squares[m];
            }
            // Judged where the region differs from the reference by more
            // than a code.
            if squares[0] > samples {
                let (m, moved) = (1..5)
                    .map(|m| (m, squares[m]))
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .expect("four moves");
                moves
                    .entry(got.frame)
                    .or_default()
                    .push((m, moved / squares[0]));
            }
        }
    }
    let certain = |frame: u64| codes_blocks_alone(codec) || frame == 0;
    for (frame, squares) in &squares_by_frame {
        if squares[0] > 0.0 {
            let moved = squares[1..].iter().copied().fold(f64::INFINITY, f64::min);
            let ratio = moved / squares[0];
            picture.registration = Some(picture.registration.map_or(ratio, |v| v.min(ratio)));
        }
        // Displaced as a whole: most of the judged regions, at least three,
        // nearer the reference moved the same way.
        let judged = moves.get(frame).map_or(&[][..], |v| v.as_slice());
        for (m, (dx, dy)) in MOVES.iter().enumerate().skip(1) {
            let agreeing = judged
                .iter()
                .filter(|(mv, r)| *mv == m && *r < REGISTRATION)
                .count();
            if judged.len() >= 3 && agreeing * 3 >= judged.len() * 2 {
                let line =
                    format!("frame {frame}: the picture is displaced by ({dx}, {dy}) pixels");
                if certain(*frame) {
                    problems.push(line);
                } else {
                    local.push((*frame, line));
                }
            }
        }
    }
    for (frame, line) in local {
        if certain(frame) {
            problems.push(line);
        } else if notes.len() < 3 {
            notes.push(format!(
                "Fine detail in part of frame {frame} differs from what was rendered, as the encoder's data rate allows ({line})."
            ));
        }
    }
    if regions != blocks.len() {
        problems.push(format!(
            "checked {regions} of {} sampled regions",
            blocks.len()
        ));
    }
    pixels
}

const MOVES: [(i32, i32); 5] = [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)];

/// RMS difference of two frames' grids over every channel, normalized.
fn distance(a: &FrameSignature, b: &FrameSignature) -> f64 {
    rms(a
        .cells
        .iter()
        .zip(&b.cells)
        .flat_map(|(p, q)| p.iter().zip(q).map(|(x, y)| f64::from(x - y))))
}

/// What was measured of one decoded frame against its own reference and
/// its neighbours'.
struct Measured {
    n: u64,
    /// Signed difference of each channel's mean, code values.
    bias: [f64; 4],
    /// Signed shift of most of each channel's grid (the median cell
    /// difference), normalized.
    shift: [f64; 4],
    /// RMS of each channel's cell differences about that shift, normalized.
    spread: [f64; 4],
    /// The largest cell difference beyond its channel's shift, normalized.
    cell: f64,
    /// Distance from its own reference, normalized.
    own: f64,
    /// The nearest neighbour's reference and its distance.
    nearest: Option<(u64, f64)>,
    /// Per channel: the fraction of the reference grid's picture kept, that
    /// picture's strength, and the strength of the decoded grid's own
    /// picture (normalized RMS).
    kept: [(f64, f64, f64); 4],
}

/// The neutral of channel `k` of a grid: no colour for Cb and Cr; the
/// grid's own mean for luma and alpha, whose picture is their contrast.
fn neutral(cells: &[f32; CELLS], k: usize) -> f64 {
    match k {
        1 | 2 => 0.0,
        _ => cells.iter().map(|c| f64::from(*c)).sum::<f64>() / CELLS as f64,
    }
}

fn measure(
    n: u64,
    got: &FrameSignature,
    want: &FrameSignature,
    neighbours: impl Iterator<Item = (u64, f64)>,
    codec: Codec,
) -> Measured {
    let mut shift = [0.0; 4];
    let mut spread = [0.0; 4];
    let mut cell = 0.0f64;
    let mut kept = [(1.0, 0.0, 0.0); 4];
    for k in 0..4 {
        let d: Vec<f64> = got.cells[k]
            .iter()
            .zip(&want.cells[k])
            .map(|(g, w)| f64::from(g - w))
            .collect();
        shift[k] = median(d.clone()).unwrap_or(0.0);
        spread[k] = rms(d.iter().map(|v| v - shift[k]));
        cell = d.iter().map(|v| (v - shift[k]).abs()).fold(cell, f64::max);
        let (nw, ng) = (neutral(&want.cells[k], k), neutral(&got.cells[k], k));
        let (mut rr, mut gr, mut gg) = (0.0, 0.0, 0.0);
        for (g, w) in got.cells[k].iter().zip(&want.cells[k]) {
            let (r, g) = (f64::from(*w) - nw, f64::from(*g) - ng);
            rr += r * r;
            gr += g * r;
            gg += g * g;
        }
        kept[k] = (
            if rr > 0.0 { gr / rr } else { 1.0 },
            (rr / CELLS as f64).sqrt(),
            (gg / CELLS as f64).sqrt(),
        );
    }
    Measured {
        n,
        bias: std::array::from_fn(|k| {
            f64::from(got.mean[k] - want.mean[k]) * channel_scale(codec, k)
        }),
        shift,
        spread,
        cell,
        own: distance(got, want),
        nearest: neighbours.min_by(|a, b| a.1.total_cmp(&b.1)),
        kept,
    }
}

/// The slope of a decoded grid against its reference, where the
/// reference's cells spread [`GAIN_SPREAD`] or more.
fn gain(want: &[f32; CELLS], got: &[f32; CELLS]) -> Option<f64> {
    let mean = |v: &[f32; CELLS]| v.iter().map(|c| f64::from(*c)).sum::<f64>() / CELLS as f64;
    let (mw, mg) = (mean(want), mean(got));
    let (mut ww, mut wg) = (0.0, 0.0);
    for (w, g) in want.iter().zip(got) {
        let (w, g) = (f64::from(*w) - mw, f64::from(*g) - mg);
        ww += w * w;
        wg += w * g;
    }
    ((ww / CELLS as f64).sqrt() >= GAIN_SPREAD).then(|| wg / ww)
}

/// Frames to either side whose reference is compared with a decoded frame.
const NEIGHBOURS: i64 = 2;

/// Holds decoded frames' signatures to the encoder input's (see the module
/// documentation for what fails). `expected` gives the signature of an
/// encoded frame; `decoded` holds frames by index; `encoded` is the number
/// of frames encoded when the file was decoded whole.
pub fn check_frames<'a>(
    codec: Codec,
    expected: &dyn Fn(u64) -> Option<&'a FrameSignature>,
    encoded: Option<usize>,
    decoded: &[(u64, FrameSignature)],
    picture: &mut PictureReport,
    problems: &mut Vec<String>,
) {
    if let Some(encoded) = encoded
        && decoded.len() != encoded
    {
        problems.push(format!(
            "the file decodes to {} frames, {encoded} were rendered",
            decoded.len(),
        ));
    }
    let luma = channel_scale(codec, 0);
    let measured: Vec<Measured> = decoded
        .iter()
        .filter_map(|(n, got)| {
            let want = expected(*n)?;
            let neighbours = (-NEIGHBOURS..=NEIGHBOURS)
                .filter(|d| *d != 0)
                .filter_map(|d| {
                    let m = u64::try_from(*n as i64 + d).ok()?;
                    expected(m).map(|s| (m, distance(got, s)))
                });
            Some(measure(*n, got, want, neighbours, codec))
        })
        .collect();
    picture.frames_checked += measured.len() as u64;
    if measured.is_empty() {
        return;
    }
    let typical = median(measured.iter().map(|m| m.own).collect()).unwrap_or(0.0);
    let typical_cell = median(measured.iter().map(|m| m.cell).collect()).unwrap_or(0.0);
    let typical_spread: [f64; 4] = std::array::from_fn(|k| {
        median(measured.iter().map(|m| m.spread[k]).collect()).unwrap_or(0.0)
    });
    picture.typical_cv = typical * luma;
    let mut found: Vec<String> = vec![];
    for m in &measured {
        let n = m.n;
        for k in 0..4 {
            if m.bias[k].abs() > picture.frame_bias_cv.abs() {
                picture.frame_bias_cv = m.bias[k];
            }
        }
        picture.frame_distance_cv = picture.frame_distance_cv.max(m.own * luma);
        picture.frame_cell_cv = picture.frame_cell_cv.max(m.cell * luma);
        if let Some((_, d)) = m.nearest
            && m.own > 0.0
        {
            let ratio = d / m.own;
            picture.identity = Some(picture.identity.map_or(ratio, |v| v.min(ratio)));
        }
        let outlier = m.own > IDENTITY_OUTLIER * typical + IDENTITY_FLOOR;
        if let Some((other, d)) = m.nearest
            && outlier
            && IDENTITY_RATIO * d < m.own
        {
            found.push(format!("frame {n} shows the picture of frame {other}"));
            continue;
        }
        let mut lost = vec![];
        for (k, (&(part, signal, left), noise)) in m.kept.iter().zip(typical_spread).enumerate() {
            if signal < LOST_SIGNAL || signal < LOST_NOISE * noise {
                continue;
            }
            picture.kept = Some(picture.kept.map_or(part, |v| v.min(part)));
            // Lost, not replaced: what is left of the channel's picture is
            // small beside what it should carry.
            if part < LOST && left < 2.0 * LOST * signal {
                let what = match k {
                    0 => "its picture",
                    1 | 2 => "its colour",
                    _ => "its alpha",
                };
                if !lost.contains(&what) {
                    lost.push(what);
                }
            }
        }
        if !lost.is_empty() {
            found.push(format!("frame {n} lost {}", lost.join(" and ")));
            continue;
        }
        if m.own > FOREIGN * typical + FOREIGN_FLOOR {
            found.push(format!("frame {n} is not the picture that was rendered"));
            continue;
        }
        if m.cell > CELL_OUTLIER * typical_cell + CELL_FLOOR {
            found.push(format!(
                "part of frame {n} is not the picture that was rendered"
            ));
        }
    }
    // What every frame shares: a level, range or matrix mistake.
    let names = ["luma", "Cb", "Cr", "alpha"];
    for (k, name) in names.iter().enumerate() {
        let least = measured
            .iter()
            .map(|m| m.shift[k])
            .min_by(|a, b| a.abs().total_cmp(&b.abs()))
            .unwrap_or(0.0);
        let shared = if measured.iter().all(|m| m.shift[k] * least > 0.0) {
            least * channel_scale(codec, k)
        } else {
            0.0
        };
        if shared.abs() > picture.level_cv.abs() {
            picture.level_cv = shared;
        }
        if shared.abs() > level_tolerance(codec, k) {
            found.insert(
                0,
                format!("{name} is {shared:+.1} codes from what was rendered in every frame"),
            );
        }
    }
    // The gain of each channel, where every judged frame departs from
    // one the same way (a range or matrix mistake is in all of them; one
    // frame's loss of a channel is judged above).
    for (k, name) in names.iter().enumerate().take(3) {
        let gains: Vec<f64> = decoded
            .iter()
            .filter_map(|(index, got)| gain(&expected(*index)?.cells[k], &got.cells[k]))
            .collect();
        let Some(least) = gains
            .iter()
            .copied()
            .min_by(|a, b| (a - 1.0).abs().total_cmp(&(b - 1.0).abs()))
        else {
            continue;
        };
        let shared = if gains.iter().all(|g| (g - 1.0) * (least - 1.0) > 0.0) {
            least
        } else {
            1.0
        };
        let off = (shared - 1.0).abs();
        picture.gain = Some(picture.gain.map_or(off, |v: f64| v.max(off)));
        if off > GAIN {
            found.insert(
                0,
                format!("{name}'s range is scaled by {shared:.2} in every frame"),
            );
        }
    }
    let more = found.len().saturating_sub(6);
    problems.extend(found.into_iter().take(6));
    if more > 0 {
        problems.push(format!("and {more} more frame problems"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{ConvertParams, WorkingFrame, convert};
    use crate::settings::{AlphaOutput, InputFormat, WorkingEncoding};

    const HQ: Codec = Codec::ProRes(ProResProfile::Hq);
    const H264: Codec = Codec::H264 {
        bitrate_bps: 1_000_000,
    };

    fn value(code: f64) -> f32 {
        ((code - 64.0) / 876.0) as f32
    }

    /// A 48 × 48 region of frame `frame` at (0, 0) whose luma is
    /// `luma(x, y)` in 10-bit codes.
    fn region(frame: u64, luma: impl Fn(u32, u32) -> f64) -> BlockSample {
        let region = BlockRegion {
            x: 0,
            y: 0,
            width: REGION,
            height: REGION,
        };
        let values = (0..REGION)
            .flat_map(|y| (0..REGION).map(move |x| (x, y)))
            .map(|(x, y)| [f64::from(value(luma(x, y))), 0.0, 0.0, 1.0])
            .collect();
        BlockSample::new(frame, region, values)
    }

    fn checked(
        codec: Codec,
        want: &[BlockSample],
        got: &[BlockSample],
    ) -> (Vec<String>, Vec<String>) {
        let (mut problems, mut notes) = (vec![], vec![]);
        check_pixel_blocks(
            codec,
            want,
            got.iter().cloned(),
            &mut PictureReport::default(),
            &mut problems,
            &mut notes,
        );
        (problems, notes)
    }

    /// A frame whose grids are flat at `luma` codes with neutral colour and
    /// opaque alpha, with `cell` codes more luma in grid cell `at`.
    fn signature(luma: f64, at: usize, cell: f64) -> FrameSignature {
        let mut cells = [[0.0; CELLS]; 4];
        cells[0] = [value(luma); CELLS];
        cells[0][at] = value(luma + cell);
        cells[3] = [1.0; CELLS];
        FrameSignature {
            mean: [value(luma), 0.0, 0.0, 1.0],
            cells,
        }
    }

    /// Codec noise of up to `amount` (normalized) on every cell.
    fn noisy(s: &FrameSignature, seed: u64, amount: f32) -> FrameSignature {
        let mut s = s.clone();
        for k in 0..4 {
            for (c, v) in s.cells[k].iter_mut().enumerate() {
                let h = (seed * 131 + k as u64 * 17 + c as u64 * 7919) % 1000;
                *v += amount * (h as f32 / 500.0 - 1.0);
            }
        }
        s
    }

    fn frame_problems(
        codec: Codec,
        want: &[FrameSignature],
        got: &[FrameSignature],
    ) -> Vec<String> {
        let decoded: Vec<_> = got
            .iter()
            .cloned()
            .enumerate()
            .map(|(n, s)| (n as u64, s))
            .collect();
        let mut problems = vec![];
        check_frames(
            codec,
            &|n| want.get(n as usize),
            Some(want.len()),
            &decoded,
            &mut PictureReport::default(),
            &mut problems,
        );
        problems
    }

    #[test]
    fn a_stale_swapped_missing_or_foreign_frame_is_refused() {
        // Something crosses the frame: each frame is bright in its own cell.
        let want: Vec<_> = (0..6).map(|n| signature(400.0, 9 + n, 120.0)).collect();
        let coded: Vec<_> = want
            .iter()
            .enumerate()
            .map(|(n, s)| noisy(s, n as u64, 2e-4))
            .collect();
        assert!(frame_problems(HQ, &want, &coded).is_empty());
        let mut stale = coded.clone();
        stale[3] = coded[2].clone();
        assert_eq!(
            frame_problems(HQ, &want, &stale),
            ["frame 3 shows the picture of frame 2"]
        );
        let mut swapped = coded.clone();
        swapped.swap(1, 2);
        assert_eq!(
            frame_problems(HQ, &want, &swapped),
            [
                "frame 1 shows the picture of frame 2",
                "frame 2 shows the picture of frame 1"
            ]
        );
        let found = frame_problems(HQ, &want, &coded[..5]);
        assert_eq!(found, ["the file decodes to 5 frames, 6 were rendered"]);
        let mut foreign = coded.clone();
        foreign[4] = signature(400.0, 50, 300.0);
        assert!(
            frame_problems(HQ, &want, &foreign)[0]
                .contains("frame 4 is not the picture that was rendered")
        );
        // A slow move the grid barely shows (3 codes in one cell): an
        // encode whose frames are this close to their own still tells them.
        let slow: Vec<_> = (0..6).map(|n| signature(400.0, 9 + n, 24.0)).collect();
        let mut swapped = slow.clone();
        swapped.swap(3, 4);
        assert_eq!(
            frame_problems(HQ, &slow, &swapped),
            [
                "frame 3 shows the picture of frame 4",
                "frame 4 shows the picture of frame 3"
            ]
        );
    }

    /// Frames that differ only in colour: equal luma, the colour halves
    /// changing sides from frame to frame.
    fn colour(n: u64) -> FrameSignature {
        let mut s = signature(500.0, 0, 0.0);
        for c in 0..CELLS {
            let left = c % GRID < GRID / 2;
            s.cells[1][c] = if left == n.is_multiple_of(2) {
                0.1
            } else {
                -0.1
            };
        }
        s
    }

    #[test]
    fn a_frame_that_lost_or_swapped_its_colour_is_refused() {
        let want: Vec<_> = (0..6).map(colour).collect();
        let coded: Vec<_> = want
            .iter()
            .enumerate()
            .map(|(n, s)| noisy(s, n as u64, 5e-4))
            .collect();
        assert!(frame_problems(HQ, &want, &coded).is_empty());
        let mut grey = coded.clone();
        grey[2].cells[1] = [0.0; CELLS];
        assert_eq!(
            frame_problems(HQ, &want, &grey),
            ["frame 2 lost its colour"]
        );
        let mut stale = coded.clone();
        stale[2] = coded[1].clone();
        assert_eq!(
            frame_problems(HQ, &want, &stale),
            ["frame 2 shows the picture of frame 1"]
        );
        // Alpha in other places, at the same average.
        let mut alpha = coded.clone();
        for c in 0..CELLS {
            alpha[3].cells[3][c] = if c < CELLS / 2 { 0.3 } else { 0.9 };
        }
        let mut want_alpha = want.clone();
        for s in &mut want_alpha {
            for c in 0..CELLS {
                s.cells[3][c] = if c % GRID < GRID / 2 { 0.3 } else { 0.9 };
            }
        }
        for (n, s) in alpha.iter_mut().enumerate() {
            if n != 3 {
                s.cells[3] = want_alpha[n].cells[3];
            }
        }
        assert_eq!(
            frame_problems(HQ, &want_alpha, &alpha),
            ["frame 3 is not the picture that was rendered"]
        );
        // Alpha gone opaque in one frame: lost.
        let mut opaque = alpha.clone();
        opaque[3].cells[3] = [1.0; CELLS];
        assert_eq!(
            frame_problems(HQ, &want_alpha, &opaque),
            ["frame 3 lost its alpha"]
        );
    }

    #[test]
    fn a_shift_every_frame_shares_is_refused_and_a_codecs_drift_is_not() {
        let want: Vec<_> = (0..4).map(|n| signature(500.0, n, 30.0)).collect();
        let lifted: Vec<_> = want
            .iter()
            .map(|s| {
                let mut s = s.clone();
                s.cells[0].iter_mut().for_each(|c| *c += 10.0 / 876.0);
                s.mean[0] += 10.0 / 876.0;
                s
            })
            .collect();
        let found = frame_problems(HQ, &want, &lifted);
        assert_eq!(
            found,
            ["luma is +10.0 codes from what was rendered in every frame"]
        );
        // A long-GOP encoder's predicted frames drift, but not all of them
        // the same way: its key frame is exact.
        let value8 = |code: f64| ((code - 16.0) / 219.0) as f32;
        let flat = |code: f64| {
            let mut s = signature(0.0, 0, 0.0);
            s.cells[0] = [value8(code); CELLS];
            s.mean[0] = value8(code);
            s
        };
        let want = vec![flat(120.0); 4];
        let starved = [flat(120.0), flat(121.3), flat(118.9), flat(121.0)];
        let found = frame_problems(H264, &want, &starved);
        assert!(found.is_empty(), "{found:?}");
        let lifted = [flat(122.5), flat(123.0), flat(122.6), flat(122.5)];
        assert_eq!(
            frame_problems(H264, &want, &lifted),
            ["luma is +2.5 codes from what was rendered in every frame"]
        );
    }

    #[test]
    fn a_range_mistake_is_refused() {
        // Luma spread over the grid, in four frames.
        let want: Vec<_> = (0..4)
            .map(|n| {
                let mut s = signature(0.0, 0, 0.0);
                for c in 0..CELLS {
                    s.cells[0][c] = value(100.0 + 12.0 * c as f64 + n as f64);
                }
                s
            })
            .collect();
        // Full range read as video range about mid-grey: a gain of 1.17.
        let stretched: Vec<_> = want
            .iter()
            .map(|s| {
                let mut s = s.clone();
                s.cells[0]
                    .iter_mut()
                    .for_each(|c| *c = 0.5 + (*c - 0.5) * 1.17);
                s
            })
            .collect();
        let found = frame_problems(HQ, &want, &stretched);
        assert!(
            found
                .iter()
                .any(|p| p == "luma's range is scaled by 1.17 in every frame"),
            "{found:?}"
        );
    }

    /// A busy region: luma of ±100 codes about 500 from pixel to pixel,
    /// with a slope that gives each block a low-frequency picture.
    fn busy(x: u32, y: u32) -> f64 {
        500.0
            + 3.0 * f64::from(x % 16)
            + if (x + y).is_multiple_of(2) {
                100.0
            } else {
                -100.0
            }
    }

    #[test]
    fn a_block_flat_or_foreign_among_its_neighbours_is_refused_where_blocks_are_coded_alone() {
        let want = region(1, busy);
        // A codec that kept 70 % of the detail.
        let coded = region(1, |x, y| 500.0 + (busy(x, y) - 500.0) * 0.7);
        assert_eq!(
            checked(
                HQ,
                std::slice::from_ref(&want),
                std::slice::from_ref(&coded)
            ),
            (vec![], vec![])
        );
        // The centre block flattened to its mean.
        let flattened = region(1, |x, y| {
            let centre = (16..32).contains(&x) && (16..32).contains(&y);
            if centre {
                522.5
            } else {
                500.0 + (busy(x, y) - 500.0) * 0.7
            }
        });
        let (problems, _) = checked(
            HQ,
            std::slice::from_ref(&want),
            std::slice::from_ref(&flattened),
        );
        assert!(
            problems[0].contains("frame 1 at (16, 16): a block lost its detail"),
            "{problems:?}"
        );
        // The centre block holding a picture of its own.
        let foreign = region(1, |x, y| {
            let centre = (16..32).contains(&x) && (16..32).contains(&y);
            if centre {
                500.0
                    + if (x / 4 + y / 4) % 3 == 0 {
                        120.0
                    } else {
                        -60.0
                    }
            } else {
                500.0 + (busy(x, y) - 500.0) * 0.7
            }
        });
        let (problems, _) = checked(
            HQ,
            std::slice::from_ref(&want),
            std::slice::from_ref(&foreign),
        );
        assert!(
            problems[0].contains("holds a picture that was not rendered"),
            "{problems:?}"
        );
        // A codec that kept no detail anywhere is not refused for it.
        let starved = region(1, |x, _| 500.0 + 3.0 * f64::from(x % 16));
        assert!(
            checked(HQ, std::slice::from_ref(&want), &[starved])
                .0
                .is_empty()
        );
        // A long-GOP codec's predicted frame: noted, the file kept; its key
        // frame is held as a ProRes frame is.
        let (problems, notes) = checked(
            H264,
            std::slice::from_ref(&want),
            std::slice::from_ref(&flattened),
        );
        assert!(
            problems.is_empty() && notes.len() == 1,
            "{problems:?} {notes:?}"
        );
        let mut key = want.clone();
        key.frame = 0;
        let mut flattened = flattened;
        flattened.frame = 0;
        let (problems, _) = checked(H264, &[key], &[flattened]);
        assert_eq!(problems.len(), 1, "{problems:?}");
    }

    #[test]
    fn a_picture_moved_a_pixel_is_refused() {
        let picture =
            |x: i64, y: i64| 500.0 + 150.0 * (x as f64 * 0.9).sin() + 90.0 * (y as f64 * 0.7).cos();
        let at = |n: u32| move |r: BlockSample| BlockSample { x: n * REGION, ..r };
        let want: Vec<_> = (0..3)
            .map(|n| at(n)(region(1, |x, y| picture(i64::from(x), i64::from(y)))))
            .collect();
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let moved: Vec<_> = (0..3)
                .map(|n| {
                    at(n)(region(1, |x, y| {
                        picture(i64::from(x) - dx, i64::from(y) - dy)
                    }))
                })
                .collect();
            let (problems, _) = checked(HQ, &want, &moved);
            assert!(
                problems
                    .iter()
                    .any(|p| p.contains(&format!("displaced by ({dx}, {dy})"))),
                "{problems:?}"
            );
        }
        // Noise on the picture in place is not a displacement.
        let noisy: Vec<_> = (0..3)
            .map(|n| {
                at(n)(region(1, |x, y| {
                    picture(i64::from(x), i64::from(y))
                        + if (x * 7 + y * 13) % 5 < 2 { 9.0 } else { -6.0 }
                }))
            })
            .collect();
        assert!(checked(HQ, &want, &noisy).0.is_empty());
    }

    #[test]
    fn sampled_frames_are_first_middle_last() {
        assert_eq!(sampled_frames(1), vec![0]);
        assert_eq!(sampled_frames(2), vec![0, 1]);
        assert_eq!(sampled_frames(48), vec![0, 24, 47]);
    }

    fn converted(w: u32, h: u32, format: InputFormat, rgb: [f32; 3]) -> EncoderInput {
        let rgba: Vec<f32> = (0..w * h)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 1.0])
            .collect();
        convert(
            &WorkingFrame {
                width: w,
                height: h,
                rgba: &rgba,
            },
            format,
            &ConvertParams {
                working: WorkingEncoding::Standard,
                alpha: AlphaOutput::None,
                tags: crate::settings::ColorTags::Rec709QuickTime,
                dither: false,
                frame: 0,
            },
        )
        .unwrap()
    }

    #[test]
    fn regions_cover_a_grid_and_a_signature_the_frame() {
        let input = converted(160, 120, InputFormat::V420, [0.5, 0.25, 0.75]);
        let regions = block_means(&input, 7);
        assert_eq!(regions.len(), 9);
        assert!(regions.iter().all(|b| b.x % 2 == 0
            && b.y % 2 == 0
            && b.x + REGION <= 160
            && b.y + REGION <= 120
            && b.width == REGION));
        let first = regions[0].mean;
        assert!(regions.iter().all(|b| b.mean == first && b.frame == 7));
        assert!(
            regions
                .iter()
                .all(|b| b.samples.len() == (REGION * REGION) as usize && b.activity == [0.0; 4])
        );
        // The centre region is centred on the frame's centre block, and
        // the corners' regions hold the corner blocks.
        let hd = block_regions(1280, 720);
        assert_eq!((hd[4].x, hd[4].y), (608, 336));
        assert_eq!((hd[8].x + 2 * BLOCK, hd[8].y + 2 * BLOCK), (1264, 704));
        let signature = frame_signature(&input);
        for (k, want) in first.iter().enumerate() {
            assert!(
                signature.cells[k]
                    .iter()
                    .all(|c| (f64::from(*c) - want).abs() < 1e-6)
            );
            assert!((f64::from(signature.mean[k]) - want).abs() < 1e-6);
        }
        // Tiny frames still sample inside the frame.
        let tiny = converted(4, 2, InputFormat::Y416, [0.0, 0.0, 0.0]);
        assert_eq!(block_means(&tiny, 0).len(), 1);
        assert!(frame_signature(&tiny).mean[3] == 1.0);
    }
}
