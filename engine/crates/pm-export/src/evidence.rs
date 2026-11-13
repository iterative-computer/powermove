//! Picture evidence (section 17, "Verification"): what an export keeps of
//! the frames it hands the encoder, and how the decoded file is held to it.
//!
//! Two things are kept. Of every frame, a coarse signature: the frame's
//! channel means and an 8 × 8 grid of luma means ([`FrameSignature`]). Of
//! the first, middle and last frame, the samples of 16 × 16 blocks at nine
//! places ([`BlockSample`]).
//!
//! The file is decoded and held to both:
//!
//! * every frame's means stay within a signed tolerance that does not grow
//!   with texture, and the part of that difference every frame shares (a
//!   level, range or matrix mistake is in all of them) within a tighter
//!   one;
//! * every frame's grid stays its own, over the grid and in each cell (a
//!   dropped, repeated, swapped or foreign frame shows in it), and for the
//!   codecs that code each frame on its own no neighbour's grid is clearly
//!   nearer;
//! * a block's mean stays within the flat tolerance plus a fraction of the
//!   block's own activity (a codec at its data-rate limit codes a busy
//!   block coarsely and its mean moves with the detail it drops), where the
//!   activity is the spread of the middle four fifths of its samples, so a
//!   few outliers buy no room;
//! * a block's low-frequency picture (4 × 4 cell means, mean-centred)
//!   correlates with the reference's and keeps its contrast, where that
//!   picture carries the block; and a block does not go flat where the
//!   frame's other blocks keep their fine detail: a flattened, stale or
//!   foreign block fails where its mean alone would pass;
//! * the sampled blocks register where they are: moving the reference a
//!   pixel does not bring the squared difference clearly down.
//!
//! What this cannot tell: a change smaller than the tolerances below (they
//! are set from what the encoders do to heavy noise at their data-rate
//! limits, `tests/detailed_delivery.rs`), a repeated or swapped frame whose
//! neighbours' grids differ by less than that, damage smaller than a grid
//! cell's samples resolve (about 64 pixels across in HD) away from the
//! sampled blocks, and fine detail a starved codec has dropped everywhere.

use crate::convert::{EncoderInput, sample_ycbcra};
use crate::settings::Codec;

/// Block side for pixel checks.
pub const BLOCK: u32 = 16;

/// Side of a cell of a block's low-frequency fingerprint.
const CELL: u32 = 4;

/// A frame signature's luma grid is this many cells each way.
pub const GRID: usize = 8;

/// Samples taken across each grid cell, each way.
const CELL_SAMPLES: usize = 32;

/// A 16 × 16 block of the picture: normalized Y′, Cb, Cr, A.
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

/// One of the verification blocks in full-frame, top-down coordinates.
/// WebCodecs can convert just these pixels without changing the sampled
/// locations used by the decoded-file check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlockRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The frames whose blocks are checked: first, middle, last.
pub fn sampled_frames(frames: u64) -> Vec<u64> {
    let mut v = vec![0, frames / 2, frames.saturating_sub(1)];
    v.dedup();
    v
}

/// The deduplicated 3 × 3 grid, in the same order as the block samples.
pub(crate) fn block_regions(width: u32, height: u32) -> Vec<BlockRegion> {
    let size_x = BLOCK.min(width);
    let size_y = BLOCK.min(height);
    let place = |extent: u32, size: u32, i: u32| -> u32 {
        let c = (extent - size) * i / 2;
        (c / BLOCK * BLOCK).min(extent - size)
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

/// The blocks of a 3 × 3 grid (block corners aligned to 16 pixels).
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
/// frame with other levels.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameSignature {
    /// The means of Y′, Cb, Cr and A over the sampled pixels.
    pub mean: [f32; 4],
    /// Luma means of a [`GRID`] × [`GRID`] grid over the frame, row by row.
    pub cells: [f32; GRID * GRID],
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
    let mut cells = [0.0f32; GRID * GRID];
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
            cells[j * GRID + i] = if n == 0 {
                0.0
            } else {
                (sum[0] / n as f64) as f32
            };
            for k in 0..4 {
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
    /// The blocks of the sampled frames.
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
    /// The mean's difference beyond the flat tolerance, as a fraction of
    /// the same channel's activity (the largest of the channels).
    pub excess: f64,
    /// The lowest correlation of a channel's low-frequency picture with the
    /// reference's (channels with enough contrast to judge).
    pub correlation: Option<f64>,
    /// The fraction of the block's luma activity the file keeps (blocks
    /// busy enough to judge).
    pub detail: Option<f64>,
}

/// What the picture checks measured over the whole file.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PictureReport {
    /// Frames whose signature was held to the encoder input's.
    pub frames_checked: u64,
    /// The largest signed difference of a frame's channel mean, in codes.
    pub frame_bias_cv: f64,
    /// The largest signed difference of a channel's mean that every frame
    /// shares, in codes.
    pub level_cv: f64,
    /// The largest difference (RMS over the grid) of a frame's luma grid
    /// from its own reference, in codes.
    pub frame_distance_cv: f64,
    /// The largest difference of one grid cell from its reference, in codes.
    pub frame_cell_cv: f64,
    /// The smallest ratio of the sampled blocks' squared luma difference
    /// with the reference moved one pixel to the difference in place
    /// (`None`: the blocks are too close to the reference to tell).
    pub registration: Option<f64>,
}

/// What a lossy codec may move a block's mean by for each code value of the
/// block's own activity, on top of [`pixel_tolerance`]. A codec at its
/// data-rate limit codes a busy block coarsely, and its mean moves with the
/// detail it drops: on HD noise of ±0.3 every ProRes flavour moved block
/// means beyond the flat tolerance, by up to 0.05 of the block's activity
/// (4444; `tests/detailed_delivery.rs`), and a fixed tolerance refused the
/// export. A flat block has no activity and keeps the tight tolerance. The
/// room is for one block's local loss only: the frame's own means are held
/// to [`bias_tolerance`], whatever its texture.
pub const ACTIVITY_TOLERANCE: f64 = 0.11;

/// Block-mean tolerance in code values at the codec's depth for a flat
/// block: lossless PNG is exact; ProRes is within 4 CV; the long-GOP codecs
/// within 6 CV at 8 bits (24 at 10 bits).
pub fn pixel_tolerance(codec: Codec) -> f64 {
    match codec {
        Codec::Png { .. } => 0.0,
        Codec::ProRes(_) => 4.0,
        Codec::Hevc { ten_bit: true, .. } => 24.0,
        Codec::H264 { .. } | Codec::Hevc { .. } | Codec::Vp9 { .. } | Codec::Vp8 { .. } => 6.0,
    }
}

/// How far a frame's channel mean may sit from the encoder input's,
/// signed, in code values at the codec's depth. It does not grow with
/// texture: coding loss is local and averages out over a frame, a wrong
/// level does not. On HD noise of ±0.3 the ProRes flavours moved a frame's
/// means by up to 0.93 codes (1.52 of 12 bits), and the frames the
/// long-GOP encoders predict at a starved rate by 1.27 codes of 8 bits and
/// 6.77 of 10 (`tests/detailed_delivery.rs`).
pub fn bias_tolerance(codec: Codec) -> f64 {
    match codec {
        Codec::Png { .. } => 0.0,
        Codec::ProRes(p) if p.is_4444() => 4.0,
        Codec::ProRes(_) => 2.0,
        Codec::Hevc { ten_bit: true, .. } => 12.0,
        Codec::H264 { .. } | Codec::Hevc { .. } | Codec::Vp9 { .. } | Codec::Vp8 { .. } => 3.0,
    }
}

/// How far every frame's channel mean may sit from the encoder input's the
/// same way: the difference of the least magnitude among the frames, when
/// all of them share its sign. A wrong level, range or matrix is in every
/// frame, the frames an encoder codes best included, so this is held
/// tighter than one frame's mean: on the same noise it was within 0.24
/// codes for every encoder, and H.264 at 3 Mb/s put 0.48 of its 8-bit
/// codes on every frame of a grainy picture.
pub fn level_tolerance(codec: Codec) -> f64 {
    match codec {
        Codec::Png { .. } => 0.0,
        Codec::ProRes(p) if p.is_4444() => 2.0,
        Codec::ProRes(_) => 1.0,
        Codec::Hevc { ten_bit: true, .. } => 6.0,
        Codec::H264 { .. } | Codec::Hevc { .. } | Codec::Vp9 { .. } | Codec::Vp8 { .. } => 1.5,
    }
}

/// How far a frame's luma grid may sit from its own reference (RMS over
/// the grid), in code values at the codec's depth; one cell may sit
/// [`CELL_TOLERANCE`] times as far. The grid is taken from 1,024 samples a
/// cell, so fine detail a codec drops leaves a thirty-second of its
/// strength in a cell: 3.4 codes on HD noise of ±0.3 for ProRes Proxy, 7.8
/// of 12 bits for 4444, 1.3 of 8 bits and 7.4 of 10 for the long-GOP
/// encoders (`tests/detailed_delivery.rs`).
pub fn frame_tolerance(codec: Codec) -> f64 {
    match codec {
        Codec::Png { .. } => 0.0,
        Codec::ProRes(p) if p.is_4444() => 20.0,
        Codec::ProRes(_) => 8.0,
        Codec::Hevc { ten_bit: true, .. } => 16.0,
        Codec::H264 { .. } | Codec::Hevc { .. } | Codec::Vp9 { .. } | Codec::Vp8 { .. } => 3.0,
    }
}

/// One grid cell may sit this many times [`frame_tolerance`] from its
/// reference.
pub const CELL_TOLERANCE: f64 = 3.0;

/// A channel's low-frequency picture is judged when its contrast (RMS of
/// the cell means about their mean) is at least this many times the
/// codec's flat tolerance, and at least [`FINGERPRINT_SHARE`] of the
/// block's activity: a block of fine noise has a low-frequency part too, a
/// quarter of its strength, which a codec at its limit does not keep.
const FINGERPRINT_CONTRAST: f64 = 2.0;
const FINGERPRINT_SHARE: f64 = 0.5;

/// The least correlation of a judged channel's low-frequency picture with
/// the reference's.
const FINGERPRINT_CORRELATION: f64 = 0.8;

/// The decoded low-frequency contrast stays within this factor of the
/// reference's, either way.
const FINGERPRINT_GAIN: f64 = 2.0;

/// A block's luma activity is judged from this many times the flat
/// tolerance.
const DETAIL_ACTIVITY: f64 = 4.0;

/// A block has gone flat when it keeps less than [`DETAIL_FLAT`] of its
/// luma activity while the frame's other judged blocks keep
/// [`DETAIL_KEPT`] of theirs at the median (a codec that is not starved).
const DETAIL_FLAT: f64 = 0.1;
const DETAIL_KEPT: f64 = 0.5;

/// Moving the reference one pixel must not bring the squared difference
/// under this fraction of the difference in place (a picture in place
/// gives 1 or more).
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

/// Compares sampled decoded frames with the encoder input's blocks.
pub fn check_pixels(
    codec: Codec,
    blocks: &[BlockSample],
    decoded: &[(u64, EncoderInput)],
    picture: &mut PictureReport,
    problems: &mut Vec<String>,
) -> Vec<PixelCheck> {
    let sampled = decoded
        .iter()
        .flat_map(|(frame, input)| block_means(input, *frame));
    check_pixel_blocks(codec, blocks, sampled, picture, problems)
}

/// One sampled frame's blocks together.
#[derive(Default)]
struct Pooled {
    /// Squared luma difference in place, and with the reference moved one
    /// pixel left, right, up, down.
    squares: [f64; 5],
    samples: usize,
    /// The fraction of its luma activity each judged block keeps.
    kept: Vec<(String, f64)>,
}

const MOVES: [(i32, i32); 5] = [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)];

/// Compares already extracted decoded blocks with the encoder input's.
pub(crate) fn check_pixel_blocks(
    codec: Codec,
    blocks: &[BlockSample],
    decoded: impl IntoIterator<Item = BlockSample>,
    picture: &mut PictureReport,
    problems: &mut Vec<String>,
) -> Vec<PixelCheck> {
    let mut pixels = vec![];
    let lossless = matches!(codec, Codec::Png { .. });
    let flat = pixel_tolerance(codec);
    let mut frames: std::collections::BTreeMap<u64, Pooled> = Default::default();
    for got in decoded {
        let Some(want) = blocks
            .iter()
            .find(|b| b.frame == got.frame && b.x == got.x && b.y == got.y)
        else {
            continue;
        };
        let scale = |k: usize| channel_scale(codec, k);
        let error = |k: usize| (got.mean[k] - want.mean[k]).abs() * scale(k);
        let activity = |k: usize| want.activity[k] * scale(k);
        // Rounding of the block mean itself (lossless outputs compare exactly).
        let allowed = |k: usize| {
            let busy = if lossless {
                0.0
            } else {
                ACTIVITY_TOLERANCE * activity(k)
            };
            flat + busy + 1e-9
        };
        let at = format!("frame {} block ({}, {})", got.frame, got.x, got.y);
        if let Some(k) = (0..4).find(|&k| error(k) > allowed(k)) {
            problems.push(format!(
                "{at} differs by {:.2} CV where {:.2} is allowed ({:?} vs {:?})",
                error(k),
                allowed(k),
                got.mean,
                want.mean
            ));
        }
        let (mut correlation, mut detail) = (None::<f64>, None::<f64>);
        let comparable = !lossless
            && !want.samples.is_empty()
            && (got.width, got.height, got.samples.len())
                == (want.width, want.height, want.samples.len());
        if comparable {
            // The low-frequency picture, where it carries the block.
            for k in 0..4 {
                let (Some(r), Some(d)) = (want.cells(k), got.cells(k)) else {
                    continue;
                };
                let rms =
                    |v: &[f64]| (v.iter().map(|c| c * c).sum::<f64>() / v.len() as f64).sqrt();
                let (rr, dd) = (rms(&r), rms(&d));
                if rr * scale(k) < FINGERPRINT_CONTRAST * flat
                    || rr < FINGERPRINT_SHARE * want.activity[k]
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
                if c < FINGERPRINT_CORRELATION
                    || dd * FINGERPRINT_GAIN < rr
                    || dd > rr * FINGERPRINT_GAIN
                {
                    problems.push(format!(
                        "{at} channel {k} is not the picture that was encoded: correlation {c:.2}, contrast {:.1} CV where {:.1} was encoded",
                        dd * scale(k),
                        rr * scale(k)
                    ));
                }
            }
            let pooled = frames.entry(got.frame).or_default();
            // The fine detail of the luma.
            if activity(0) >= DETAIL_ACTIVITY * flat {
                let kept = got.activity[0] / want.activity[0];
                detail = Some(kept);
                pooled.kept.push((at.clone(), kept));
            }
            if want.width > 2 && want.height > 2 {
                for (m, (dx, dy)) in MOVES.iter().enumerate() {
                    for y in 1..want.height - 1 {
                        for x in 1..want.width - 1 {
                            let reference =
                                want.at(0, (x as i32 - dx) as u32, (y as i32 - dy) as u32);
                            let d = (got.at(0, x, y) - reference) * scale(0);
                            pooled.squares[m] += d * d;
                        }
                    }
                }
                pooled.samples += ((want.width - 2) * (want.height - 2)) as usize;
            }
        }
        let excess = (0..4)
            .filter(|&k| activity(k) > 0.5)
            .map(|k| (error(k) - flat).max(0.0) / activity(k))
            .fold(0.0, f64::max);
        pixels.push(PixelCheck {
            frame: got.frame,
            x: got.x,
            y: got.y,
            expected: want.mean,
            decoded: got.mean,
            error_cv: (0..4).map(error).fold(0.0, f64::max),
            activity_cv: (0..4).map(activity).fold(0.0, f64::max),
            excess,
            correlation,
            detail,
        });
    }
    for (frame, pooled) in &frames {
        // A block that went flat where the frame's others kept their detail.
        for (at, kept) in &pooled.kept {
            let mut others: Vec<f64> = pooled
                .kept
                .iter()
                .filter(|(other, _)| other != at)
                .map(|(_, kept)| *kept)
                .collect();
            others.sort_by(f64::total_cmp);
            if *kept < DETAIL_FLAT && others.len() >= 2 && others[others.len() / 2] >= DETAIL_KEPT {
                problems.push(format!(
                    "{at} lost its detail: it keeps {:.0} % of its luma activity where the frame's other blocks keep {:.0} %",
                    kept * 100.0,
                    others[others.len() / 2] * 100.0
                ));
            }
        }
        // Registration is judged where the blocks differ from the
        // reference by more than a code.
        if pooled.samples > 0 && pooled.squares[0] > pooled.samples as f64 {
            let (m, moved) = (1..5)
                .map(|m| (m, pooled.squares[m]))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .expect("four moves");
            let ratio = moved / pooled.squares[0];
            picture.registration = Some(picture.registration.map_or(ratio, |v| v.min(ratio)));
            if ratio < REGISTRATION {
                problems.push(format!(
                    "frame {frame}: the picture is displaced by ({}, {}) pixels (its blocks differ by {:.1} CV in place and {:.1} CV from the reference moved there)",
                    MOVES[m].0,
                    MOVES[m].1,
                    (pooled.squares[0] / pooled.samples as f64).sqrt(),
                    (moved / pooled.samples as f64).sqrt()
                ));
            }
        }
    }
    if pixels.len() != blocks.len() {
        problems.push(format!(
            "checked {} of {} pixel blocks",
            pixels.len(),
            blocks.len()
        ));
    }
    pixels
}

/// RMS difference of two luma grids, in codes.
fn distance(a: &FrameSignature, b: &FrameSignature, scale: f64) -> f64 {
    let squares: f64 = a
        .cells
        .iter()
        .zip(&b.cells)
        .map(|(p, q)| f64::from(p - q).powi(2))
        .sum();
    (squares / (GRID * GRID) as f64).sqrt() * scale
}

/// The largest difference of one cell of two luma grids, in codes.
fn cell_distance(a: &FrameSignature, b: &FrameSignature, scale: f64) -> f64 {
    a.cells
        .iter()
        .zip(&b.cells)
        .map(|(p, q)| f64::from(p - q).abs())
        .fold(0.0, f64::max)
        * scale
}

/// Frames to either side whose reference is compared with a decoded frame.
const NEIGHBOURS: i64 = 2;

/// Holds decoded frames' signatures to the encoder input's: each frame's
/// levels, the level every frame shares, its own picture, and (codecs that
/// code each frame on its own) that no neighbour's picture is clearly
/// nearer. `expected` gives the signature of an encoded frame; `decoded`
/// holds frames by index; `encoded` is the number of frames encoded when
/// the file was decoded whole.
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
            "the file decodes to {} frames, {encoded} were encoded",
            decoded.len(),
        ));
    }
    let intra = matches!(codec, Codec::ProRes(_));
    let luma = channel_scale(codec, 0);
    let names = ["Y′", "Cb", "Cr", "alpha"];
    let mut found: Vec<String> = vec![];
    // The signed difference of each channel's mean in every frame.
    let mut biases: Vec<[f64; 4]> = vec![];
    for (n, got) in decoded {
        let Some(want) = expected(*n) else {
            continue;
        };
        picture.frames_checked += 1;
        let bias: [f64; 4] = std::array::from_fn(|k| {
            f64::from(got.mean[k] - want.mean[k]) * channel_scale(codec, k)
        });
        biases.push(bias);
        for k in 0..4 {
            if bias[k].abs() > picture.frame_bias_cv.abs() {
                picture.frame_bias_cv = bias[k];
            }
            if bias[k].abs() > bias_tolerance(codec) {
                found.push(format!(
                    "frame {n}: {} is {:+.2} CV from what was encoded, where {:.2} is allowed",
                    names[k],
                    bias[k],
                    bias_tolerance(codec)
                ));
            }
        }
        let own = distance(got, want, luma);
        let cell = cell_distance(got, want, luma);
        picture.frame_distance_cv = picture.frame_distance_cv.max(own);
        picture.frame_cell_cv = picture.frame_cell_cv.max(cell);
        // The reference frame nearby that the decoded frame is nearest to.
        let nearest = (-NEIGHBOURS..=NEIGHBOURS)
            .filter(|d| *d != 0)
            .filter_map(|d| {
                let m = *n as i64 + d;
                expected(u64::try_from(m).ok()?).map(|s| (m, s))
            })
            .map(|(m, s)| (m, distance(got, s, luma)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let limit = frame_tolerance(codec);
        if own > limit || cell > CELL_TOLERANCE * limit {
            found.push(match nearest.filter(|(_, d)| 2.0 * d < own) {
                Some((m, d)) => format!(
                    "frame {n} holds the picture of frame {m} ({d:.2} CV from it, {own:.2} CV from its own)"
                ),
                None => format!(
                    "frame {n} is not the picture that was encoded ({own:.2} CV from it over the frame and {cell:.2} CV in one part, where {limit:.2} and {:.2} are allowed)",
                    CELL_TOLERANCE * limit
                ),
            });
        } else if intra
            && own > limit / 4.0
            && let Some((m, d)) = nearest.filter(|(_, d)| 4.0 * d < own)
        {
            found.push(format!(
                "frame {n} holds the picture of frame {m} ({d:.2} CV from it, {own:.2} CV from its own)"
            ));
        }
    }
    // What every frame shares.
    for k in 0..4 {
        let least = biases
            .iter()
            .map(|b| b[k])
            .min_by(|a, b| a.abs().total_cmp(&b.abs()))
            .unwrap_or(0.0);
        let shared = if biases.iter().all(|b| b[k] * least > 0.0) {
            least
        } else {
            0.0
        };
        if shared.abs() > picture.level_cv.abs() {
            picture.level_cv = shared;
        }
        if shared.abs() > level_tolerance(codec) {
            found.insert(
                0,
                format!(
                    "{} is at least {shared:+.2} CV from what was encoded in every frame, where {:.2} is allowed",
                    names[k],
                    level_tolerance(codec)
                ),
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
    use crate::settings::{AlphaOutput, InputFormat, ProResProfile, WorkingEncoding};

    const HQ: Codec = Codec::ProRes(ProResProfile::Hq);

    /// A 16 × 16 block of frame 0 whose luma is `luma(x, y)` in 10-bit codes.
    fn block(luma: impl Fn(u32, u32) -> f64) -> BlockSample {
        let region = BlockRegion {
            x: 0,
            y: 0,
            width: BLOCK,
            height: BLOCK,
        };
        let values = (0..BLOCK)
            .flat_map(|y| (0..BLOCK).map(move |x| (x, y)))
            .map(|(x, y)| [(luma(x, y) - 64.0) / 876.0, 0.0, 0.0, 1.0])
            .collect();
        BlockSample::new(0, region, values)
    }

    fn problems(codec: Codec, want: &[BlockSample], got: &[BlockSample]) -> Vec<String> {
        let mut problems = vec![];
        check_pixel_blocks(
            codec,
            want,
            got.iter().cloned(),
            &mut PictureReport::default(),
            &mut problems,
        );
        problems
    }

    /// A frame whose channels are flat at `luma` codes, with `cell` codes
    /// more in grid cell `at`.
    fn signature(luma: f64, at: usize, cell: f64) -> FrameSignature {
        let value = |code: f64| ((code - 64.0) / 876.0) as f32;
        let mut cells = [value(luma); GRID * GRID];
        cells[at] = value(luma + cell);
        FrameSignature {
            mean: [value(luma), 0.0, 0.0, 1.0],
            cells,
        }
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
    fn a_flat_block_has_four_codes_and_a_busy_one_room_for_local_loss() {
        let flat = block(|_, _| 500.0);
        assert!(problems(HQ, std::slice::from_ref(&flat), &[block(|_, _| 504.0)]).is_empty());
        assert_eq!(problems(HQ, &[flat], &[block(|_, _| 505.0)]).len(), 1);
        // Equal numbers of two codes 200 apart, in 4 × 4 pieces: activity
        // 100 codes, so 4 + 11 codes of room for the block's mean.
        let busy = |lift: f64| {
            block(move |x, y| if (x / 4 + y / 4).is_multiple_of(2) { 400.0 } else { 600.0 } + lift)
        };
        assert_eq!(busy(0.0).activity[0] * 876.0, 100.0);
        assert!(problems(HQ, &[busy(0.0)], &[busy(10.0)]).is_empty());
        assert!(problems(HQ, &[busy(0.0)], &[busy(16.0)])[0].contains("differs by 16.00 CV"));
        // Lossless stays exact however busy.
        let png = Codec::Png { sixteen_bit: false };
        assert_eq!(problems(png, &[busy(0.0)], &[busy(1.0)]).len(), 1);
    }

    #[test]
    fn one_outlier_buys_a_block_no_room() {
        // 255 samples at code 400 and one at 1000: a standard deviation of
        // 37 codes, and no activity.
        let spiked =
            |lift: f64| block(move |x, y| if (x, y) == (7, 7) { 1000.0 } else { 400.0 } + lift);
        assert_eq!(spiked(0.0).activity[0], 0.0);
        assert!(problems(HQ, &[spiked(0.0)], &[spiked(8.0)])[0].contains("where 4.00 is allowed"));
    }

    #[test]
    fn a_level_shift_under_texture_is_refused_by_the_frames_means() {
        // The block check alone gives ten codes to a busy block; every
        // frame's mean does not.
        let want: Vec<_> = (0..4).map(|n| signature(500.0, n, 30.0)).collect();
        let lifted: Vec<_> = (0..4)
            .map(|n| {
                let mut s = signature(510.0, n, 30.0);
                s.mean[0] = ((510.0 - 64.0) / 876.0) as f32;
                s
            })
            .collect();
        let found = frame_problems(HQ, &want, &lifted);
        assert!(
            found[0].contains("Y′ is at least +10.00 CV from what was encoded in every frame"),
            "{found:?}"
        );
        assert!(frame_problems(HQ, &want, &want).is_empty());
        // A long-GOP encoder's predicted frames may sit further off than
        // its best ones, but not all of them the same way.
        let h264 = Codec::H264 { bitrate_bps: 1 };
        let value = |code: f64| ((code - 16.0) / 219.0) as f32;
        let flat = |code: f64| FrameSignature {
            mean: [value(code), 0.0, 0.0, 1.0],
            cells: [value(code); GRID * GRID],
        };
        let want = vec![flat(120.0); 4];
        let starved = [flat(120.1), flat(121.3), flat(118.9), flat(120.0)];
        assert!(frame_problems(h264, &want, &starved).is_empty());
        let lifted = [flat(122.5), flat(123.0), flat(122.6), flat(122.5)];
        let found = frame_problems(h264, &want, &lifted);
        assert!(
            found[0].contains("in every frame, where 1.50 is allowed"),
            "{found:?}"
        );
    }

    #[test]
    fn a_block_that_is_flat_stale_or_foreign_is_refused() {
        // Low-frequency picture: a ramp across the block, with texture.
        let ramp = |x: u32, y: u32| {
            300.0
                + 20.0 * f64::from(x)
                + if (x + y).is_multiple_of(2) {
                    15.0
                } else {
                    -15.0
                }
        };
        let want = block(ramp);
        assert!(
            problems(
                HQ,
                std::slice::from_ref(&want),
                &[block(|x, y| ramp(x, y) + 2.0)]
            )
            .is_empty()
        );
        // Flattened to its mean: the mean is right.
        let found = problems(HQ, std::slice::from_ref(&want), &[block(|_, _| 450.0)]);
        assert!(
            found[0].contains("is not the picture that was encoded: correlation 0.00"),
            "{found:?}"
        );
        // Another picture of the same mean (the ramp the other way).
        let found = problems(
            HQ,
            std::slice::from_ref(&want),
            &[block(|x, y| ramp(15 - x, y))],
        );
        assert!(found[0].contains("correlation -1.00"), "{found:?}");
        // Half the contrast: a transfer or range mistake that keeps the mean.
        let found = problems(
            HQ,
            &[want],
            &[block(|x, y| 450.0 + (ramp(x, y) - 450.0) * 0.4)],
        );
        assert!(found[0].contains("contrast"), "{found:?}");
    }

    #[test]
    fn a_block_gone_flat_is_refused_where_the_frames_others_keep_their_detail() {
        // Pixel-alternating detail has no low-frequency picture.
        let fine = |n: u32| {
            let mut b = block(|x, y| {
                if (x + y).is_multiple_of(2) {
                    400.0
                } else {
                    600.0
                }
            });
            b.x = n * BLOCK;
            b
        };
        let flat = |n: u32| {
            let mut b = block(|_, _| 500.0);
            b.x = n * BLOCK;
            b
        };
        let want: Vec<_> = (0..5).map(fine).collect();
        assert!(problems(HQ, &want, &want).is_empty());
        let mut got = want.clone();
        got[2] = flat(2);
        let found = problems(HQ, &want, &got);
        assert!(
            found[0].contains("frame 0 block (32, 0) lost its detail"),
            "{found:?}"
        );
        // A codec that kept none of it anywhere is not refused for it.
        let starved: Vec<_> = (0..5).map(flat).collect();
        assert!(problems(HQ, &want, &starved).is_empty());
    }

    #[test]
    fn a_picture_moved_a_pixel_is_refused() {
        let picture =
            |x: i64, y: i64| 500.0 + 150.0 * (x as f64 * 0.9).sin() + 90.0 * (y as f64 * 0.7).cos();
        let want = block(|x, y| picture(i64::from(x), i64::from(y)));
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let moved = block(|x, y| picture(i64::from(x) - dx, i64::from(y) - dy));
            let found = problems(HQ, std::slice::from_ref(&want), &[moved]);
            assert!(
                found
                    .iter()
                    .any(|p| p.contains(&format!("displaced by ({dx}, {dy})"))),
                "{found:?}"
            );
        }
        // Noise on the picture in place is not a displacement.
        let noisy = block(|x, y| {
            picture(i64::from(x), i64::from(y)) + if (x * 7 + y * 13) % 5 < 2 { 9.0 } else { -6.0 }
        });
        assert!(problems(HQ, &[want], &[noisy]).is_empty());
    }

    #[test]
    fn a_stale_swapped_missing_or_foreign_frame_is_refused() {
        // Something crosses the frame: each frame is bright in its own cell.
        let want: Vec<_> = (0..6).map(|n| signature(400.0, 9 + n, 120.0)).collect();
        assert!(frame_problems(HQ, &want, &want).is_empty());
        let mut stale = want.clone();
        stale[3] = want[2].clone();
        assert!(
            frame_problems(HQ, &want, &stale)[0].contains("frame 3 holds the picture of frame 2")
        );
        let mut swapped = want.clone();
        swapped.swap(1, 2);
        let found = frame_problems(HQ, &want, &swapped);
        assert!(
            found[0].contains("frame 1 holds the picture of frame 2"),
            "{found:?}"
        );
        assert!(
            found[1].contains("frame 2 holds the picture of frame 1"),
            "{found:?}"
        );
        let found = frame_problems(HQ, &want, &want[..5]);
        assert!(
            found[0].contains("decodes to 5 frames, 6 were encoded"),
            "{found:?}"
        );
        let mut foreign = want.clone();
        foreign[4] = signature(400.0, 50, 300.0);
        assert!(
            frame_problems(HQ, &want, &foreign)[0]
                .contains("frame 4 is not the picture that was encoded")
        );
        // A slow move the grid barely shows (3 codes in one cell, 0.4 over
        // the grid): an intra codec's frames are still told apart.
        let slow: Vec<_> = (0..6).map(|n| signature(400.0, 9 + n, 24.0)).collect();
        let mut swapped = slow.clone();
        swapped.swap(3, 4);
        let found = frame_problems(HQ, &slow, &swapped);
        assert!(
            found[0].contains("frame 3 holds the picture of frame 4"),
            "{found:?}"
        );
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
    fn blocks_cover_a_grid_and_a_signature_the_frame() {
        let input = converted(100, 48, InputFormat::V420, [0.5, 0.25, 0.75]);
        let blocks = block_means(&input, 7);
        assert_eq!(blocks.len(), 9);
        assert!(
            blocks
                .iter()
                .all(|b| b.x % 16 == 0 && b.y % 16 == 0 && b.x + 16 <= 100 && b.y + 16 <= 48)
        );
        let first = blocks[0].mean;
        assert!(blocks.iter().all(|b| b.mean == first && b.frame == 7));
        assert!(
            blocks
                .iter()
                .all(|b| b.samples.len() == 256 && b.activity == [0.0; 4])
        );
        let signature = frame_signature(&input);
        assert!(
            signature
                .cells
                .iter()
                .all(|c| (f64::from(*c) - first[0]).abs() < 1e-6)
        );
        assert!((0..4).all(|k| (f64::from(signature.mean[k]) - first[k]).abs() < 1e-6));
        // Tiny frames still sample inside the frame.
        let tiny = converted(4, 2, InputFormat::Y416, [0.0, 0.0, 0.0]);
        assert_eq!(block_means(&tiny, 0).len(), 1);
        assert!(frame_signature(&tiny).mean[3] == 1.0);
    }
}
