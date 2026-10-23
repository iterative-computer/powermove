//! Today's export voices, sample for sample (decisions AUDIO-5 and AUDIO-6).
//!
//! The current desktop exporter mixes with an `OfflineAudioContext` at the
//! context rate (`legacy/core/audio.ts` `renderOffline`): every clip is an
//! `AudioBufferSourceNode` over the asset decoded by `decodeAudioData` at
//! the live context's rate, through a `GainNode` whose gain follows the
//! envelope `applyEnvelope` schedules, into the master `GainNode`. This
//! module reproduces that chain for the voices whose time map is linear
//! (no speed, remap, or conformed rate anywhere on the path), with the
//! arithmetic of the Chromium 152 sources Electron 44 ships:
//!
//! * **Decode resampling** (`AudioBus::CreateBySampleRateConverting`, Blink's
//!   `SincResampler`, `third_party/blink/renderer/platform/audio/
//!   sinc_resampler.cc`): a 32-tap Blackman-windowed sinc with 32 kernel
//!   offsets, the two neighbouring kernels' sums blended linearly, a running
//!   `f64` source index advanced by the rate ratio per output frame and
//!   wrapped every 512-frame block, the scalar (ARM) summation order, and an
//!   output length of `floor(frames / ratio)`. [`Conversion`] keeps the
//!   running index at the start of every block, so any output frame is
//!   computed exactly without resampling from the start.
//! * **The source node** (`audio_buffer_source_handler.cc`,
//!   `audio_scheduled_source_handler.cc`): the offset rounded to a whole
//!   sample (`TimeToSampleFrame`, rounded at 1024× first), the start rounded
//!   up the same way, a start between frames played by linear interpolation
//!   in `f64` from the fractional read position, and the duration counted in
//!   render quanta of 128 frames ([`SourceRead`]).
//! * **The gain envelope** (`envelopePoints`, `gainAt`, `applyEnvelope`;
//!   `audio_param_handler.cc` `ProcessLinearRamp`): points at the clip's
//!   start, its end, one composition frame apart (at least 30 a second) when
//!   any level input is a channel, on keyframes and on the fade corners;
//!   their `gainAt` values as `f32`; `setValueAtTime` at the first and
//!   `linearRampToValueAtTime` to the rest; each frame's value
//!   `v1 + x × (v2 − v1)` with `x = f32((frame / rate − t1) × f32(1 / (t2 −
//!   t1)))` from the event whose time rounded up is at or before the frame;
//!   the parameter's default (1) before the first event ([`Envelope`]).
//!
//! Output frame `f` of today's context is project sample `origin + f`, where
//! the origin is the export range's first sample ([`Range`]); playback
//! renders with the origin at the project start, which is today's export of
//! the whole project.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use pm_eval::{Comp, Evaluator, JsValue, node};
use pm_model::{JsString, Node};

use crate::error::Result;
use crate::graph::{VoiceKind, VoicePlan};

/// Web Audio's render quantum.
const QUANTUM: i64 = 128;
/// Blink's `SincResampler` block (`kBlockSize`).
const BLOCK: f64 = 512.0;
/// Its taps and kernel offsets (`AudioBus::CreateBySampleRateConverting`
/// uses the defaults).
const TAPS: usize = 32;
const OFFSETS: usize = 32;

/// `audio_utilities::TimeToSampleFrame` rounding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    Nearest,
    Up,
}

/// `audio_utilities::TimeToSampleFrame`: the frame at a time, rounded at
/// 1024× the rate first (so `k / rate × rate` gives `k` back).
pub fn time_to_sample_frame(time: f64, rate: f64, rounding: Rounding) -> f64 {
    let frame = (time * rate * 1024.0).round() / 1024.0;
    match rounding {
        Rounding::Nearest => frame.round(),
        Rounding::Up => frame.ceil(),
    }
}

// ---------------------------------------------------------------------------
// Decode resampling
// ---------------------------------------------------------------------------

/// `SincResampler::InitializeKernel` for a rate ratio (source / output).
pub fn blink_kernel(ratio: f64) -> Vec<f32> {
    use std::f64::consts::PI;
    let alpha = 0.16;
    let a0 = 0.5 * (1.0 - alpha);
    let a1 = 0.5;
    let a2 = 0.5 * alpha;
    let mut sinc_scale = if ratio > 1.0 { 1.0 / ratio } else { 1.0 };
    sinc_scale *= 0.9;
    let n = TAPS as i32;
    let half = n / 2;
    let mut kernel = vec![0f32; TAPS * (OFFSETS + 1)];
    for offset in 0..=OFFSETS {
        let subsample = offset as f64 / OFFSETS as f64;
        for i in 0..n {
            let s = sinc_scale * PI * (f64::from(i - half) - subsample);
            let mut sinc = if s == 0.0 { 1.0 } else { libm::sin(s) / s };
            sinc *= sinc_scale;
            let x = (f64::from(i) - subsample) / f64::from(n);
            let window = a0 - a1 * libm::cos(2.0 * PI * x) + a2 * libm::cos(2.0 * PI * 2.0 * x);
            kernel[i as usize + offset * TAPS] = (sinc * window) as f32;
        }
    }
    kernel
}

/// A source's decode resampled as `decodeAudioData` resamples it to the
/// context rate: which source frames and kernels make each output frame.
#[derive(Debug)]
pub struct Conversion {
    /// Source rate / output rate.
    pub ratio: f64,
    /// The output rate.
    pub rate: u32,
    /// Output frames: `floor(source frames / ratio)`.
    pub len: u64,
    kernel: Arc<[f32]>,
    /// Per 512-frame block: its first output frame and the running source
    /// index there (after the wrap).
    blocks: Vec<(u64, f64)>,
}

impl Conversion {
    pub fn new(source_frames: u64, source_rate: u32, rate: u32) -> Conversion {
        let ratio = f64::from(source_rate) / f64::from(rate);
        let len = (source_frames as f64 / ratio) as u64;
        let mut blocks = vec![(0u64, 0.0f64)];
        let (mut j, mut v) = (0u64, 0.0f64);
        while j < len {
            while v < BLOCK && j < len {
                v += ratio;
                j += 1;
            }
            if j >= len {
                break;
            }
            v -= BLOCK;
            blocks.push((j, v));
        }
        Conversion {
            ratio,
            rate,
            len,
            kernel: kernel_for(ratio),
            blocks,
        }
    }

    /// Output frame `j`'s block and running source index.
    fn position(&self, j: u64) -> (u64, f64) {
        let m = self.blocks.partition_point(|&(first, _)| first <= j) - 1;
        let (first, mut v) = self.blocks[m];
        for _ in first..j {
            v += self.ratio;
        }
        (m as u64, v)
    }

    /// The source frames output frames `[j0, j1)` read.
    pub fn source_span(&self, j0: u64, j1: u64) -> (i64, i64) {
        if j1 <= j0 {
            return (0, 0);
        }
        let (m0, v0) = self.position(j0);
        let (m1, v1) = self.position(j1 - 1);
        (
            m0 as i64 * BLOCK as i64 + v0 as i64 - (TAPS / 2) as i64,
            m1 as i64 * BLOCK as i64 + v1 as i64 + (TAPS / 2) as i64,
        )
    }

    /// Whether each of output frames `[j0, j0 + out.len())` has all of its
    /// source taps among `valid` (source frames from `src_first`).
    pub fn valid(&self, j0: u64, valid: &[bool], src_first: i64, out: &mut [bool]) {
        if out.is_empty() {
            return;
        }
        let (mut m, mut v) = self.position(j0);
        for o in out.iter_mut() {
            if v >= BLOCK {
                v -= BLOCK;
                m += 1;
            }
            let base = m as i64 * BLOCK as i64 + v as i64 - (TAPS / 2) as i64 - src_first;
            *o = (base..base + TAPS as i64)
                .all(|at| at < 0 || at as usize >= valid.len() || valid[at as usize]);
            v += self.ratio;
        }
    }

    /// Output frames `[j0, j0 + out.len())` from source frames starting at
    /// `src_first` (`src`, zero outside the source as Blink pads it).
    pub fn render(&self, j0: u64, src: &[f32], src_first: i64, out: &mut [f32]) {
        if out.is_empty() {
            return;
        }
        let (mut m, mut v) = self.position(j0);
        for o in out.iter_mut() {
            if v >= BLOCK {
                v -= BLOCK;
                m += 1;
            }
            let i0 = v as u64;
            let remainder = v - i0 as f64;
            let offset_index = remainder * OFFSETS as f64;
            let k = offset_index as usize;
            let k1 = &self.kernel[k * TAPS..(k + 1) * TAPS];
            let k2 = &self.kernel[(k + 1) * TAPS..(k + 2) * TAPS];
            let base = m as i64 * BLOCK as i64 + i0 as i64 - (TAPS / 2) as i64 - src_first;
            let (mut sum1, mut sum2) = (0f32, 0f32);
            for t in 0..TAPS {
                let at = base + t as i64;
                let input = if at >= 0 && (at as usize) < src.len() {
                    src[at as usize]
                } else {
                    0.0
                };
                sum1 += input * k1[t];
                sum2 += input * k2[t];
            }
            let factor = offset_index - k as f64;
            *o = ((1.0 - factor) * f64::from(sum1) + factor * f64::from(sum2)) as f32;
            v += self.ratio;
        }
    }
}

fn kernel_for(ratio: f64) -> Arc<[f32]> {
    static KERNELS: OnceLock<Mutex<HashMap<u64, Arc<[f32]>>>> = OnceLock::new();
    let mut map = KERNELS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    map.entry(ratio.to_bits())
        .or_insert_with(|| blink_kernel(ratio).into())
        .clone()
}

/// The conversion of a source of `frames` frames, shared while any voice
/// holds it (computing a long file's block table walks every output frame).
pub fn conversion(source: u64, frames: u64, source_rate: u32, rate: u32) -> Arc<Conversion> {
    type Key = (u64, u64, u32, u32);
    static CONVERSIONS: OnceLock<Mutex<HashMap<Key, Weak<Conversion>>>> = OnceLock::new();
    let key = (source, frames, source_rate, rate);
    let mut map = CONVERSIONS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(c) = map.get(&key).and_then(Weak::upgrade) {
        return c;
    }
    map.retain(|_, w| w.strong_count() > 0);
    let c = Arc::new(Conversion::new(frames, source_rate, rate));
    map.insert(key, Arc::downgrade(&c));
    c
}

// ---------------------------------------------------------------------------
// The source node
// ---------------------------------------------------------------------------

/// `x` advanced by `k` additions of 1.0 in `f64`, as the source node's
/// running read index advances one frame at a time: exact within a binade,
/// rounded once at each power of two crossed.
fn advance(mut x: f64, mut k: u64) -> f64 {
    while k > 0 {
        if x.fract() == 0.0 || x >= 9_007_199_254_740_992.0 {
            return x + k as f64;
        }
        let next = 2f64.powi(x.log2().floor() as i32 + 1);
        // Steps that stay below the next power of two are exact.
        let inside = (next - x).ceil() as u64 - 1;
        if k <= inside {
            return x + k as f64;
        }
        x += inside as f64;
        x += 1.0;
        k -= inside + 1;
    }
    x
}

/// How today's `AudioBufferSourceNode.start(at, offset, duration)` plays a
/// buffer of `len` frames at the context rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceRead {
    /// The context frame of the first output.
    pub start_frame: i64,
    /// Output frames.
    pub frames: i64,
    /// The read position of the first output (fractional for a start
    /// between frames).
    pub read: f64,
    pub len: u64,
    /// The buffer's end stopped it (rather than the duration).
    pub buffer_end: bool,
}

impl SourceRead {
    pub fn new(at: f64, offset: f64, duration: f64, len: u64, rate: f64) -> SourceRead {
        let buffer_duration = len as f64 / rate;
        let offset = offset.clamp(0.0, buffer_duration);
        let read0 = time_to_sample_frame(offset, rate, Rounding::Nearest);
        let start_frame = time_to_sample_frame(at.max(0.0), rate, Rounding::Up);
        let start_offset = at.max(0.0) * rate - start_frame;
        let end = buffer_duration * rate;
        let max_frames = duration.max(0.0) * rate;
        let start_frame = start_frame as i64;
        let mut read = read0;
        let mut played = 0.0f64;
        if start_offset < 0.0 {
            // The first quantum skips the start's fraction of a frame.
            let skipped = (start_offset * 1.0).abs();
            read += skipped;
            played += skipped;
        }
        // The duration, counted per render quantum; from a whole start
        // every quantum adds whole frames, so one ceiling counts them.
        let mut total = 0i64;
        let mut available = QUANTUM - start_frame.rem_euclid(QUANTUM);
        if played == 0.0 {
            if max_frames > 0.0 {
                total = max_frames.ceil() as i64;
            }
            available = 0;
        }
        while available > 0 {
            let left = max_frames - played;
            if left <= 0.0 {
                break;
            }
            let until = left.ceil();
            if until < available as f64 {
                total += until as i64;
                break;
            }
            total += available;
            played += available as f64;
            available = QUANTUM;
        }
        // The end of the buffer: frame k > 0 plays while its read position
        // is before the end; the first while it is inside the buffer.
        let fast = read.fract() == 0.0 && end.fract() == 0.0;
        let first_plays = if fast {
            read < end
        } else {
            (read as u64) < len
        };
        let fails = |k: i64| {
            let x = advance(read, k as u64);
            x >= end || x as u64 >= len
        };
        let to_end = if !first_plays {
            0
        } else {
            let mut k = ((end - read).ceil().min(len as f64 - read.floor()) as i64).max(1);
            while k > 1 && fails(k - 1) {
                k -= 1;
            }
            while !fails(k) {
                k += 1;
            }
            k
        };
        let buffer_end = to_end <= total;
        let total = total.min(to_end);
        SourceRead {
            start_frame,
            frames: total,
            read,
            len,
            buffer_end,
        }
    }

    /// The first context frame after the source node's output is
    /// disconnected from the graph: it finishes in the render quantum where
    /// it stops (a duration that ends on a quantum's end stops in the next
    /// one), and the context disables a finished node's output after that
    /// quantum. Until then its channel count widens the bus it feeds.
    pub fn disconnected(&self) -> i64 {
        let end = self.start_frame + self.frames;
        let finish = if self.buffer_end {
            (end - 1).max(self.start_frame).div_euclid(QUANTUM)
        } else {
            end.div_euclid(QUANTUM)
        };
        (finish + 1) * QUANTUM
    }

    /// The read position of output `k`.
    pub fn position(&self, k: i64) -> f64 {
        advance(self.read, k as u64)
    }

    /// `ProcessInterpolatedPath`'s sample at read position `x` from buffer
    /// frames starting at `first` (`f64` blend, then `f32`).
    pub fn sample(&self, x: f64, buf: &[f32], first: i64) -> f32 {
        let at = |i: u64| {
            let k = i as i64 - first;
            if k >= 0 && (k as usize) < buf.len() {
                f64::from(buf[k as usize])
            } else {
                0.0
            }
        };
        let ri = x as u64;
        let factor = x - ri as f64;
        let mut ri2 = ri + 1;
        if ri2 >= self.len {
            ri2 = ri;
        }
        let s = if ri == ri2 && ri >= 1 {
            let (s1, s2) = (at(ri - 1), at(ri));
            s2 + (s2 - s1) * factor
        } else {
            let (s1, s2) = (at(ri), at(ri2));
            (1.0 - factor) * s1 + factor * s2
        };
        s as f32
    }
}

// ---------------------------------------------------------------------------
// The voice
// ---------------------------------------------------------------------------

/// Today's export range: context frame 0 is project sample `origin`, at
/// `t0` seconds; clips are cut at `t1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub t0: f64,
    pub t1: f64,
    pub origin: i64,
    /// Playback: today connects a clip when it comes within a second of the
    /// playhead (`LIVE_LOOKAHEAD`); an export connects every clip first.
    pub live: bool,
}

/// Today's playback lookahead (`LIVE_LOOKAHEAD` in `legacy/core/audio.ts`).
pub const LIVE_LOOKAHEAD: f64 = 1.0;

/// Web Audio's up-mix of `from` channels into a bus of `to` (Chromium's
/// `AudioBus::SumFromByUpMixing`, speakers interpretation): one row per bus
/// channel, each a copy of at most one source channel.
pub fn up_mix(from: usize, to: usize) -> Vec<Vec<f32>> {
    let mut m = vec![vec![0f32; from]; to];
    let mut copy = |dst: usize, src: usize| m[dst][src] = 1.0;
    match (from, to) {
        (f, t) if f == t => (0..f).for_each(|c| copy(c, c)),
        (1, 2) | (1, 4) => {
            copy(0, 0);
            copy(1, 0);
        }
        (1, 6) => copy(2, 0),
        (2, 4) | (2, 6) => {
            copy(0, 0);
            copy(1, 1);
        }
        (4, 6) => {
            copy(0, 0);
            copy(1, 1);
            copy(4, 2);
            copy(5, 3);
        }
        (f, t) => (0..f.min(t)).for_each(|c| copy(c, c)),
    }
    m
}

/// The stereo fold of a bus of `n` channels named by `speakers` (the
/// widest clip's layout) for the counts Web Audio would fold discretely
/// (decision LAYOUT-1): ITU-R BS.775 / SMPTE coefficients by speaker
/// (`layout::matrix`), and an equal-power sum of every channel into both
/// sides when the channels have no speaker meaning. Rows L and R.
pub fn stereo_fold(speakers: &[pm_media::audio::Speaker], n: usize) -> Vec<Vec<f32>> {
    use pm_media::audio::Speaker;
    let named = speakers.len() == n && speakers.iter().any(|s| !matches!(s, Speaker::Other(_)));
    if named {
        return crate::layout::matrix(speakers, &crate::layout::stereo());
    }
    let w = (1.0 / n.max(1) as f32).sqrt();
    vec![vec![w; n], vec![w; n]]
}

/// Adds `bus` folded through `m` (rows L, R) into `out` frames `[0, n)`.
pub fn matrix_fold(bus: &[Vec<f32>], m: &[Vec<f32>], out: &mut [Vec<f32>], n: usize) {
    for (o, row) in out.iter_mut().zip(m) {
        for (c, &g) in row.iter().enumerate() {
            if g == 0.0 {
                continue;
            }
            for i in 0..n {
                o[i] += bus[c][i] * g;
            }
        }
    }
}

/// Folds a bus of `bus.len()` channels to stereo as the destination does
/// (`AudioBus::SumFromByDownMixing`, in its operation order), adding into
/// `out` frames `[at, at + n)` from bus frames `[0, n)`.
pub fn down_mix(bus: &[Vec<f32>], out: &mut [Vec<f32>], at: usize, n: usize) {
    let half_sqrt = 0.5f32.sqrt();
    for i in 0..n {
        let (l, r) = match bus.len() {
            1 => (bus[0][i], bus[0][i]),
            4 => (
                bus[2][i].mul_add(0.5, bus[0][i].mul_add(0.5, 0.0)),
                bus[3][i].mul_add(0.5, bus[1][i].mul_add(0.5, 0.0)),
            ),
            6 => (
                bus[4][i].mul_add(half_sqrt, bus[2][i].mul_add(half_sqrt, 0.0 + bus[0][i])),
                bus[5][i].mul_add(half_sqrt, bus[2][i].mul_add(half_sqrt, 0.0 + bus[1][i])),
            ),
            _ => (bus[0][i], bus.get(1).map_or(0.0, |c| c[i])),
        };
        out[0][at + i] += l;
        out[1][at + i] += r;
    }
}

impl Default for Range {
    fn default() -> Self {
        Range {
            t0: 0.0,
            t1: f64::INFINITY,
            origin: 0,
            live: false,
        }
    }
}

/// One precomposition on a voice's path, as today's walk records it.
#[derive(Clone, Debug)]
struct ChainLevel {
    comp: usize,
    layer: usize,
    /// The offset of the composition holding the precomposition.
    offset: f64,
    /// Its composition's own level (`comp.audioGain`, read with `Number`).
    nested: f64,
}

/// The parts of a voice today's envelope reads.
#[derive(Clone, Debug)]
pub(crate) struct Shape {
    kind: VoiceKind,
    chain: Vec<ChainLevel>,
    comp: usize,
    layer: usize,
    groups: Vec<usize>,
    /// The offset of the clip's composition (`naturalStart − from`).
    comp_offset: f64,
    /// The clip's start in master time (`offset + max(0, from)`).
    natural_start: f64,
    from: f64,
    dur: f64,
    window: (f64, f64),
    /// The clip's trim (`max(0, finite(trim))`).
    trim: f64,
    stepping: bool,
    /// A precomposition on the path has an animated `trim`: the shape
    /// depends on the time it is read at ([`Shape::new`]).
    pub(crate) precomp_trim_animated: bool,
    step: f64,
    keys: Vec<f64>,
    /// The evaluator is the standard one (no hooks override channels), so a
    /// keyframed level field without an expression evaluates as `evP` does
    /// from its track prepared once ([`Shape::field`]).
    pub(crate) standard: bool,
    /// The prepared tracks of `gain`, `fadeIn` and `fadeOut` (`None`: not a
    /// plain keyframed channel), prepared on first use.
    tracks: [std::sync::OnceLock<Option<Arc<pm_eval::keyframes::PreparedTrack>>>; 3],
}

fn is_property(n: Option<&Node>) -> bool {
    node::is_property(n)
}

fn number(n: Option<&Node>, fallback: f64) -> f64 {
    let v = node::to_number(n);
    if v.is_finite() { v } else { fallback }
}

fn finite(v: &JsValue, fallback: f64) -> f64 {
    let n = v.to_number();
    if n.is_finite() { n } else { fallback }
}

/// `levelOf`.
fn level_of(v: &JsValue) -> f64 {
    if matches!(v, JsValue::Undefined | JsValue::Null) {
        1.0
    } else {
        finite(v, 1.0).clamp(0.0, 4.0)
    }
}

/// A static value as `resolveContent` gives it (a channel's `v`).
fn static_value(n: Option<&Node>) -> JsValue {
    if is_property(n) {
        node::js_value_opt(node::member(n, "v"))
    } else {
        node::js_value_opt(n)
    }
}

/// The composition time `tau` at which `pm_eval` reads layer-local time
/// `tau − from` equal to today's `T − naturalStart` for the flattened clip.
fn local_tau(from: f64, natural: f64, t: f64) -> f64 {
    let local = t - natural;
    let mut tau = from + local;
    for _ in 0..4 {
        let got = tau - from;
        if got == local {
            break;
        }
        tau = if got < local {
            tau.next_up()
        } else {
            tau.next_down()
        };
    }
    tau
}

impl Shape {
    /// Today's walk over the voice's path, or `None` for a voice today's
    /// model does not cover (a time map that is not linear, a negative
    /// layer start, a conformed rate).
    /// `at`: the evaluator and the time a precomposition's animated `trim` is
    /// read at. Today's `audioLayers` reads it once with the precomposition
    /// (`evaluatedValue(PM, layer, layer.d.trim, PM.time)`) when it builds
    /// its index, at the playhead; the engine reads it where the range or
    /// the run starts. Without `at` the static value is used.
    pub(crate) fn new(
        plan: &VoicePlan,
        comps: &[Comp<'_>],
        root: &Node,
        mut at: Option<(&mut Evaluator, f64)>,
    ) -> Option<Shape> {
        // Speed and remaps follow the picture (AUDIO-4). Everything else is
        // today's: a conformed rate is ignored (today plays a video's
        // soundtrack at its own rate), a negative start is clamped to 0,
        // an animated trim is read once (`Clip::new`).
        if plan.retimed {
            return None;
        }
        let (last, chain_levels) = plan.levels.split_last()?;
        let mut offset = 0.0f64;
        let mut window = (0.0f64, f64::INFINITY);
        let mut chain = Vec::new();
        let mut precomp_trim_animated = false;
        for level in chain_levels {
            let layer = comps.get(level.comp)?.layers().get(level.layer)?;
            let from = number(node::member(Some(layer), "from"), 0.0);
            let dur = number(node::member(Some(layer), "dur"), 0.0).max(0.0);
            let natural = offset + from.max(0.0);
            window = (window.0.max(natural), window.1.min(natural + dur));
            let d = node::member(Some(layer), "d");
            let trim_node = node::member(d, "trim");
            let trim = match (&mut at, is_property(trim_node)) {
                (Some((ev, t)), true) => {
                    precomp_trim_animated = true;
                    ev.ev_p(
                        comps.get(level.comp)?,
                        Some(layer),
                        trim_node,
                        *t,
                        &JsString::from("c.trim"),
                    )
                    .ok()?
                }
                (None, true) => {
                    precomp_trim_animated = true;
                    static_value(trim_node)
                }
                _ => static_value(trim_node),
            };
            // `Number(trim || 0)`.
            let trim = if trim.truthy() { trim.to_number() } else { 0.0 };
            let nested_id = node::string(node::member(d, "comp"))?.to_string_lossy();
            let nested = node::member(node::member(Some(root), "comps"), &nested_id)
                .or_else(|| Comp::in_root(root, &nested_id, 0).ok().map(|c| c.node()));
            chain.push(ChainLevel {
                comp: level.comp,
                layer: level.layer,
                offset,
                nested: level_of(&node::js_value_opt(node::member(nested, "audioGain"))),
            });
            offset = natural - trim;
        }
        let comp = comps.get(last.comp)?;
        let layer = comp.layers().get(last.layer)?;
        let from = number(node::member(Some(layer), "from"), 0.0);
        let dur = number(node::member(Some(layer), "dur"), 0.0).max(0.0);
        let natural_start = offset + from.max(0.0);
        window = (
            window.0.max(natural_start),
            window.1.min(natural_start + dur),
        );
        let d = node::member(Some(layer), "d");
        let trim = finite(&static_value(node::member(d, "trim")), 0.0).max(0.0);
        let layers = comp.layers();
        let level_keys: &[&str] = match plan.kind {
            VoiceKind::Audio => &["gain", "fadeIn", "fadeOut"],
            VoiceKind::Embedded => &["audioGain"],
        };
        let stepping = last
            .groups
            .iter()
            .any(|&g| is_property(node::member(layers.get(g), "on")))
            || is_property(node::member(Some(layer), "on"))
            || chain.iter().any(|c| {
                let precomp = comps[c.comp].layers().get(c.layer);
                is_property(node::member(node::member(precomp, "d"), "audioGain"))
            })
            || level_keys.iter().any(|k| is_property(node::member(d, k)));
        let mut keys = Vec::new();
        for k in level_keys {
            for key in node::array(node::member(node::member(d, k), "kf")).unwrap_or(&[]) {
                keys.push(node::to_number(node::member(Some(key), "t")));
            }
        }
        let fps = node::to_number(comps[0].member("fps"));
        let fps = if fps.is_finite() && fps != 0.0 {
            fps
        } else {
            30.0
        };
        Some(Shape {
            kind: plan.kind,
            chain,
            comp: last.comp,
            layer: last.layer,
            groups: last.groups.clone(),
            comp_offset: offset,
            natural_start,
            from,
            dur,
            window,
            trim,
            stepping,
            precomp_trim_animated,
            step: 1.0 / fps.max(30.0),
            keys,
            standard: false,
            tracks: Default::default(),
        })
    }

    /// The decoded buffer frames (at `rate`, `len` of them) that project
    /// samples `[a, b)` can read whatever range today's model starts from,
    /// with two frames of margin for the offset's rounding; `None` when the
    /// clip is silent there.
    pub(crate) fn buffer_span(
        &self,
        a: i64,
        b: i64,
        rate: f64,
        len: u64,
        trim: Option<f64>,
    ) -> Option<(i64, i64)> {
        let trim = trim.unwrap_or(self.trim);
        let layer_start = self.natural_start.max(0.0);
        let audible = self.dur.max(0.0).min((len as f64 / rate - trim).max(0.0));
        let start = layer_start.max(self.window.0);
        let end = (layer_start + self.dur.max(0.0))
            .min(self.window.1)
            .min(layer_start + audible);
        let (a, b) = (
            a.max((start * rate).floor() as i64 - 1),
            b.min((end * rate).ceil() as i64 + 1),
        );
        if b <= a {
            return None;
        }
        let at = |s: i64| ((s as f64 / rate - layer_start + trim) * rate).floor() as i64;
        Some((at(a) - 3, at(b - 1) + 4))
    }

    /// The clip's `trim` member.
    fn trim_node<'c>(&self, comps: &'c [Comp<'_>]) -> Option<&'c Node> {
        let layer = comps[self.comp].layers().get(self.layer)?;
        node::member(node::member(Some(layer), "d"), "trim")
    }

    /// A content field of the clip at master time `t`, as `resolveContent`
    /// on today's flattened clip evaluates it.
    fn field(&self, ev: &mut Evaluator, comps: &[Comp<'_>], key: &str, t: f64) -> Result<JsValue> {
        let comp = &comps[self.comp];
        let layer = &comp.layers()[self.layer];
        let d = node::member(Some(layer), "d");
        let (name, value) = match (self.kind, key) {
            (VoiceKind::Embedded, "gain") => ("audioGain", node::member(d, "audioGain")),
            (VoiceKind::Embedded, "trim") => {
                // `audioLayers` reads a video's trim with the video itself
                // (its own start), at the time it is asked for.
                let v = node::member(d, "trim");
                if !is_property(v) {
                    return Ok(node::js_value_opt(v));
                }
                return Ok(ev.ev_p(comp, Some(layer), v, t, &JsString::from("c.trim"))?);
            }
            (VoiceKind::Embedded, _) => return Ok(JsValue::Number(0.0)),
            _ => (key, node::member(d, key)),
        };
        if is_property(value) {
            let tau = local_tau(self.from, self.natural_start, t);
            if let Some(v) = self.prepared_field(ev, layer, value, key, tau) {
                return Ok(v);
            }
            Ok(ev.ev_p(
                comp,
                Some(layer),
                value,
                tau,
                &JsString::from(format!("c.{name}").as_str()),
            )?)
        } else if self.kind == VoiceKind::Embedded && node::is_nullish(value) {
            Ok(JsValue::Number(1.0))
        } else {
            Ok(node::js_value_opt(value))
        }
    }

    /// `evP(L, prop, T)` for a level field that is a plain keyframed channel
    /// (no expression, a non-empty `kf` array) under the standard evaluator:
    /// `evalKfs(kf, T − L.from)` from the track prepared once per shape,
    /// instead of preparing it again at every point (a long keyed clip's
    /// track costs about 0.1 ms to prepare, its value 20 ns). `None` for
    /// anything else, which `evP` evaluates.
    fn prepared_field(
        &self,
        ev: &mut Evaluator,
        layer: &Node,
        prop: Option<&Node>,
        key: &str,
        tau: f64,
    ) -> Option<JsValue> {
        if !self.standard {
            return None;
        }
        let slot = match key {
            "gain" => 0,
            "fadeIn" => 1,
            "fadeOut" => 2,
            _ => return None,
        };
        let p = prop?;
        let kf = node::member(Some(p), "kf");
        let track = self.tracks[slot].get_or_init(|| {
            let plain = !node::get(p, b"expr").truthy()
                && node::array(kf).is_some_and(|keys| !keys.is_empty());
            plain.then(|| Arc::new(pm_eval::keyframes::PreparedTrack::prepare(kf, ev.springs())))
        });
        let track = track.as_ref()?;
        // `localTime`: `T − L.from`, as `evP` reads it.
        let tl = tau - node::get(layer, b"from").to_js().to_number();
        track.value(kf, tl, false, ev.springs()).ok()
    }

    /// `gainAt(layer, local, audible)`.
    fn gain_at(
        &self,
        ev: &mut Evaluator,
        comps: &[Comp<'_>],
        local: f64,
        audible: f64,
    ) -> Result<f64> {
        let time = self.natural_start + local;
        let comp = &comps[self.comp];
        let layers = comp.layers();
        let layer = &layers[self.layer];
        for &g in &self.groups {
            let group = &layers[g];
            let on = node::member(Some(group), "on");
            let value = if is_property(on) {
                ev.ev_p(
                    comp,
                    Some(group),
                    on,
                    time - self.comp_offset,
                    &JsString::from("l.on"),
                )?
            } else {
                node::js_value_opt(on)
            };
            if !value.truthy() {
                return Ok(0.0);
            }
        }
        let on = node::member(Some(layer), "on");
        let value = if is_property(on) {
            let tau = local_tau(self.from, self.natural_start, time);
            ev.ev_p(comp, Some(layer), on, tau, &JsString::from("l.on"))?
        } else {
            node::js_value_opt(on)
        };
        if value == JsValue::Bool(false) {
            return Ok(0.0);
        }
        let gain = self.field(ev, comps, "gain", time)?;
        let fade_in = finite(&self.field(ev, comps, "fadeIn", time)?, 0.0).max(0.0);
        let fade_out = finite(&self.field(ev, comps, "fadeOut", time)?, 0.0).max(0.0);
        let mut chain = 1.0f64;
        for c in &self.chain {
            let pc = &comps[c.comp];
            let precomp = &pc.layers()[c.layer];
            let level = node::member(node::member(Some(precomp), "d"), "audioGain");
            let value = if is_property(level) {
                ev.ev_p(
                    pc,
                    Some(precomp),
                    level,
                    time - c.offset,
                    &JsString::from("c.audioGain"),
                )?
            } else {
                node::js_value_opt(level)
            };
            chain *= level_of(&value);
            chain *= c.nested;
        }
        let base = finite(&gain, 1.0).clamp(0.0, 4.0) * chain;
        let duration = if audible.is_finite() {
            audible
        } else {
            self.dur
        }
        .max(0.0);
        let local = local.clamp(0.0, duration);
        let entering = if fade_in > 0.0 {
            (local / fade_in).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let leaving = if fade_out > 0.0 {
            ((duration - local) / fade_out).clamp(0.0, 1.0)
        } else {
            1.0
        };
        Ok(base * entering.min(leaving))
    }
}

/// `plan(layer, asset, t0, t1)` and the source node's playback of it.
#[derive(Clone, Debug)]
pub(crate) struct Clip {
    pub read: SourceRead,
    pub envelope: Envelope,
    /// The trim today read for this range (once, at its start).
    pub trim: f64,
}

impl Clip {
    /// Today's clip for `shape` over a buffer of `len` frames at `rate`
    /// within `range`, with its envelope's times (values come later), or
    /// `None` when today plays nothing of it.
    pub(crate) fn new(
        shape: &Shape,
        ev: &mut Evaluator,
        comps: &[Comp<'_>],
        len: u64,
        rate: f64,
        range: Range,
    ) -> Result<Option<Clip>> {
        let layer_start = shape.natural_start.max(0.0);
        let layer_duration = shape.dur.max(0.0);
        let window_start = layer_start.max(shape.window.0);
        let window_end = (layer_start + layer_duration).min(shape.window.1);
        let source_duration = len as f64 / rate;
        // `plan`: the trim as `resolveContent` gives it where the range
        // starts (inside the clip), read once.
        let trim = if is_property(shape.trim_node(comps)) {
            finite(
                &shape.field(ev, comps, "trim", layer_start.max(range.t0))?,
                0.0,
            )
            .max(0.0)
        } else {
            shape.trim
        };
        let audible = layer_duration.min((source_duration - trim).max(0.0));
        let clip_end = window_end.min(layer_start + audible);
        let start = window_start.max(range.t0);
        let end = clip_end.min(range.t1);
        if end - start <= 1e-4 {
            return Ok(None);
        }
        let local_start = start - layer_start;
        let source_offset = trim + local_start;
        if source_duration.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
            || source_offset >= source_duration - 1e-4
        {
            return Ok(None);
        }
        let duration = (end - start).min(source_duration - source_offset);
        if duration <= 1e-4 {
            return Ok(None);
        }
        let at = start - range.t0.max(0.0);
        let read = SourceRead::new(at, source_offset, duration, len, rate);
        let envelope = Envelope::new(shape, ev, comps, local_start, duration, audible, at, rate)?;
        Ok(Some(Clip {
            read,
            envelope,
            trim,
        }))
    }
}

/// The gain parameter's timeline for one clip.
#[derive(Clone, Debug)]
pub(crate) struct Envelope {
    /// Point local times (`envelopePoints`).
    locals: Vec<f64>,
    /// Their `gainAt` values as `f32`, NaN until evaluated.
    values: Vec<f32>,
    /// Event times (context seconds) and their frames rounded up.
    times: Vec<f64>,
    frames: Vec<i64>,
    audible: f64,
    rate: f64,
}

impl Envelope {
    #[allow(clippy::too_many_arguments)]
    fn new(
        shape: &Shape,
        ev: &mut Evaluator,
        comps: &[Comp<'_>],
        local_start: f64,
        duration: f64,
        audible: f64,
        at: f64,
        rate: f64,
    ) -> Result<Envelope> {
        let t = shape.natural_start + local_start;
        let fade_in = finite(&shape.field(ev, comps, "fadeIn", t)?, 0.0).max(0.0);
        let fade_out = finite(&shape.field(ev, comps, "fadeOut", t)?, 0.0).max(0.0);
        let full = if audible.is_finite() {
            audible
        } else {
            shape.dur
        }
        .max(0.0);
        let start = local_start.clamp(0.0, full);
        let end = (start + duration.max(0.0)).clamp(start, full);
        let mut times = vec![start, end];
        if shape.stepping {
            let mut local = start + shape.step;
            while local < end {
                times.push(local);
                local += shape.step;
            }
            times.extend(shape.keys.iter().copied().filter(|&k| k > start && k < end));
        }
        if fade_in > start + 1e-6 && fade_in < end - 1e-6 {
            times.push(fade_in);
        }
        let fade_out_start = full - fade_out;
        if fade_out > 0.0 && fade_out_start > start + 1e-6 && fade_out_start < end - 1e-6 {
            times.push(fade_out_start);
        }
        if fade_in > 0.0 && fade_out > 0.0 && fade_in > fade_out_start {
            let crossing = full * fade_in / (fade_in + fade_out);
            if crossing > start + 1e-6 && crossing < end - 1e-6 {
                times.push(crossing);
            }
        }
        // `[...new Set(times)].sort((a, b) => a - b)`.
        times.sort_by(f64::total_cmp);
        times.dedup_by(|a, b| a == b);
        let locals = times;
        let mut event_times = Vec::with_capacity(locals.len());
        event_times.push(at);
        if duration > 0.002 {
            event_times.extend(locals[1..].iter().map(|&l| at + l - local_start));
        }
        let frames = event_times
            .iter()
            .map(|&t| (t * rate).ceil() as i64)
            .collect();
        Ok(Envelope {
            values: vec![f32::NAN; event_times.len()],
            locals,
            times: event_times,
            frames,
            audible: full,
            rate,
        })
    }

    fn value(
        &mut self,
        i: usize,
        shape: &Shape,
        ev: &mut Evaluator,
        comps: &[Comp<'_>],
    ) -> Result<f32> {
        if self.values[i].is_nan() {
            self.values[i] = shape.gain_at(ev, comps, self.locals[i], self.audible)? as f32;
        }
        Ok(self.values[i])
    }

    /// The gain of context frames `[f0, f0 + out.len())`.
    pub(crate) fn gains(
        &mut self,
        f0: i64,
        out: &mut [f32],
        shape: &Shape,
        ev: &mut Evaluator,
        comps: &[Comp<'_>],
    ) -> Result<()> {
        let last = self.times.len() - 1;
        let mut i = self.frames[1..].partition_point(|&fe| fe <= f0);
        let mut f = f0;
        let mut o = 0;
        while o < out.len() {
            if f < self.frames[0] {
                // Before the first event: the parameter's default.
                let n = ((self.frames[0] - f) as usize).min(out.len() - o);
                out[o..o + n].fill(1.0);
                o += n;
                f += n as i64;
                continue;
            }
            while i < last && self.frames[i + 1] <= f {
                i += 1;
            }
            if i == last {
                let v = self.value(last, shape, ev, comps)?;
                out[o..].fill(v);
                return Ok(());
            }
            let v1 = self.value(i, shape, ev, comps)?;
            let v2 = self.value(i + 1, shape, ev, comps)?;
            let (t1, t2) = (self.times[i], self.times[i + 1]);
            let delta = t2 - t1;
            let k: f32 = if delta <= f64::from(f32::MIN_POSITIVE) {
                0.0
            } else {
                (1.0 / delta) as f32
            };
            let value_delta = v2 - v1;
            let n = ((self.frames[i + 1] - f) as usize).min(out.len() - o);
            for slot in &mut out[o..o + n] {
                let x = ((f as f64 / self.rate - t1) * f64::from(k)) as f32;
                *slot = v1 + x * value_delta;
                f += 1;
            }
            o += n;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_frames_round_through_the_oversampled_grid() {
        // 0.1 s × 48 kHz is 4800 in f64; 1/30 s × 48 kHz rounds back to 1600.
        assert_eq!(time_to_sample_frame(0.1, 48_000.0, Rounding::Up), 4800.0);
        assert_eq!(
            time_to_sample_frame(1.0 / 30.0, 48_000.0, Rounding::Up),
            1600.0
        );
        assert_eq!(
            time_to_sample_frame(1.0000104, 48_000.0, Rounding::Up),
            48001.0
        );
        assert_eq!(
            time_to_sample_frame(0.0104167, 48_000.0, Rounding::Nearest),
            500.0
        );
    }

    #[test]
    fn advancing_by_ones_matches_the_running_index() {
        for start in [0.25, 4800.52, 8191.75, 1.0 / 3.0, 123_456.789, 0.0] {
            let mut x = start;
            for k in 0..20_000u64 {
                assert_eq!(advance(start, k).to_bits(), x.to_bits(), "{start} + {k}");
                x += 1.0;
            }
        }
    }

    #[test]
    fn the_conversion_block_table_matches_a_running_resampler() {
        let c = Conversion::new(44_100, 44_100, 48_000);
        assert_eq!(c.len, 48_000);
        let (mut m, mut v) = (0u64, 0.0f64);
        for j in 0..c.len {
            if v >= BLOCK {
                v -= BLOCK;
                m += 1;
            }
            assert_eq!(c.position(j), (m, v), "frame {j}");
            v += c.ratio;
        }
    }

    #[test]
    fn a_start_between_frames_interpolates_from_its_fraction() {
        // at 1.0000104 s: start frame 48001, 0.4992 of a frame skipped.
        let read = SourceRead::new(1.0000104, 0.5, 0.2, 96_000, 48_000.0);
        assert_eq!(read.start_frame, 48001);
        let skipped = (1.0000104f64 * 48_000.0 - 48001.0).abs();
        assert_eq!(read.read, 24_000.0 + skipped);
        // Duration 0.2 s is 9600 source frames from the fractional start.
        assert_eq!(read.frames, (9600.0 - skipped).ceil() as i64);
        let buf: Vec<f32> = (0..96_000).map(|i| i as f32 * 1e-5).collect();
        let x = read.position(10);
        let want = ((1.0 - x.fract()) * f64::from(buf[24_010]) + x.fract() * f64::from(buf[24_011]))
            as f32;
        assert_eq!(read.sample(x, &buf, 0), want);
    }

    #[test]
    fn whole_starts_copy_and_stop_at_the_duration() {
        let read = SourceRead::new(0.1, 0.05, 0.85, 192_000, 48_000.0);
        assert_eq!((read.start_frame, read.read), (4800, 2400.0));
        assert_eq!(read.frames, 40_800);
        // The buffer's end stops it first.
        let read = SourceRead::new(0.0, 1.0, 5.0, 72_000, 48_000.0);
        assert_eq!(read.frames, 24_000);
    }
}
