//! A decode session: one decoder on one stream, serving frames by sample
//! with exact random access (section 2, Seeking bar).
//!
//! A request for sample `s` either continues the current run (when the run
//! already passed the random-access point `s` needs and has not passed `s`)
//! or seeks: the decoder is reset (an end-of-sequence sample for
//! VideoToolbox, a flush for software decoders) and fed from the sync sample
//! [`FrameIndex::sync_for`] picks, skipping that sync sample's leading
//! pictures. Decoded frames are kept in a small window for nearby requests
//! (forward and reverse stepping); the service's frame cache holds more.

use super::skip::Disposable;
use super::{Decoder, DecoderInfo};
use crate::error::{MediaError, Result};
use crate::ff::PacketReader;
use crate::frame::DecodedFrame;
use crate::index::FrameIndex;
use pm_protocol::color::ColorDescription;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Counters for diagnosis and tests.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    pub requests: usize,
    pub window_hits: usize,
    pub seeks: usize,
    pub continued: usize,
    pub samples_fed: usize,
    pub leading_skipped: usize,
    pub frames_out: usize,
    pub drains: usize,
    /// Unreferenced pictures before the target left out of a seek
    /// ([`super::skip`]).
    pub disposable_skipped: usize,
    /// Requests given up because a newer one superseded them.
    pub stopped: usize,
    /// Time spent reading samples from the file, in µs.
    pub read_us: u64,
    /// Time spent submitting samples to the decoder, in µs.
    pub send_us: u64,
    /// Time spent waiting for the decoder's output, in µs.
    pub wait_us: u64,
}

pub struct Session {
    index: Arc<FrameIndex>,
    reader: PacketReader,
    decoder: Decoder,
    /// Sample index by composition time (decoders echo it back).
    by_cts: HashMap<i64, usize>,
    /// Presentation position of each sample (`usize::MAX`: not presented).
    shown_at: Vec<usize>,
    /// The sync sample the current run started at.
    run_start: Option<usize>,
    /// The next decode-order sample the run feeds.
    next: usize,
    drained: bool,
    /// Decoded frames by sample.
    window: BTreeMap<usize, DecodedFrame>,
    window_limit: usize,
    window_bytes: usize,
    /// The sample of the newest request (PM_FIX_REORDER).
    last_target: Option<usize>,
    /// Keep reorder frames while a drag scrubs (PM_FIX_VIDEO2).
    reorder: bool,
    color: Option<ColorDescription>,
    /// Classifies unreferenced pictures a seek may leave out.
    skip: Option<Disposable>,
    /// Presented frames just before a target that are decoded and kept even
    /// when unreferenced (a reverse scrub steps into them next): those
    /// within `keep_before` frames whose distance to the target is a
    /// multiple of `keep_stride`.
    keep_before: usize,
    keep_stride: usize,
    /// This session replaced one whose decoder failed and has not decoded
    /// a frame since: if it fails too, the sample is damaged, and it is
    /// not replaced again ([`super::service::SessionGuard`]).
    pub(super) replacement: bool,
    pub stats: SessionStats,
}

/// Where a request stands, for a caller deciding whether to stop it
/// ([`Session::sample_with`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    /// The run's sync sample.
    pub run_start: usize,
    /// The next decode-order sample the run feeds.
    pub next: usize,
    /// The requested sample.
    pub target: usize,
}

/// Frames a session keeps besides the one it returns.
pub const DEFAULT_WINDOW: usize = 16;
/// Samples fed past the target, waiting for a decoder with reorder delay to
/// output it, before draining instead.
const MAX_LOOKAHEAD: usize = 64;
/// Samples read from the file per decoder submission batch, so reading
/// overlaps the decoder's work and decoded frames are trimmed as they come.
const FEED_CHUNK: usize = 8;

impl Session {
    pub fn new(
        index: Arc<FrameIndex>,
        reader: PacketReader,
        decoder: Decoder,
        color: Option<ColorDescription>,
    ) -> Session {
        let mut by_cts = HashMap::with_capacity(index.samples.len());
        for (i, s) in index.samples.iter().enumerate() {
            by_cts.entry(s.cts).or_insert(i);
        }
        let mut shown_at = vec![usize::MAX; index.samples.len()];
        for (n, f) in index.frames.iter().enumerate() {
            shown_at[f.sample] = shown_at[f.sample].min(n);
        }
        Session {
            shown_at,
            keep_before: 0,
            keep_stride: 1,
            replacement: false,
            skip: Disposable::for_index(&index),
            index,
            reader,
            decoder,
            by_cts,
            run_start: None,
            next: 0,
            drained: false,
            window: BTreeMap::new(),
            window_limit: DEFAULT_WINDOW,
            window_bytes: 32 << 20,
            last_target: None,
            reorder: false,
            color,
            stats: SessionStats::default(),
        }
    }

    pub fn set_window(&mut self, frames: usize) {
        self.window_limit = frames;
        self.trim(None);
    }

    /// Keep at least the requested frame, even if that one exceeds the budget.
    pub fn set_window_bytes(&mut self, bytes: usize) {
        self.window_bytes = bytes;
        self.trim(None);
    }

    /// Whether seeks leave out unreferenced pictures before their target
    /// (on by default for H.264 and HEVC; [`super::skip`]).
    pub fn set_skip_disposable(&mut self, on: bool) {
        self.skip = if on {
            Disposable::for_index(&self.index)
        } else {
            None
        };
    }

    /// Keeps the presented frames up to `frames` before each target whose
    /// distance to it is a multiple of `stride` (every one for 1),
    /// unreferenced ones included (at most the window).
    pub fn set_keep_before(&mut self, frames: usize, stride: usize) {
        self.keep_before = frames;
        self.keep_stride = stride.max(1);
    }

    pub fn decoder_info(&self) -> DecoderInfo {
        self.decoder.info()
    }

    /// See [`Decoder::invalidate_hardware`].
    #[doc(hidden)]
    pub fn invalidate_hardware(&mut self) -> bool {
        self.decoder.invalidate_hardware()
    }

    /// See [`super::Decoder::release_hardware`].
    pub(super) fn release_hardware(&mut self) {
        self.decoder.release_hardware();
    }

    pub fn index(&self) -> &Arc<FrameIndex> {
        &self.index
    }

    pub fn copies(&self) -> usize {
        self.decoder.copies()
    }

    /// The frame of presented frame `n` (presentation order).
    pub fn presented(&mut self, n: usize) -> Result<DecodedFrame> {
        let s = self
            .index
            .frames
            .get(n)
            .ok_or_else(|| MediaError::invalid(format!("frame {n} out of range")))?
            .sample;
        self.sample(s)
    }

    /// The decoded picture of sample `s`.
    pub fn sample(&mut self, s: usize) -> Result<DecodedFrame> {
        match self.sample_with(s, &mut |_| false)? {
            Some(f) => Ok(f),
            None => unreachable!("never stopped"),
        }
    }

    /// Whether a request for sample `s` continues the current run (no seek):
    /// the run has passed the random-access point `s` needs and not `s`.
    fn continues(&self, s: usize) -> bool {
        if self.window.contains_key(&s) {
            return true;
        }
        let sync = self.index.sync_for(s);
        !self.drained
            && self
                .run_start
                .is_some_and(|start| sync >= start && sync < self.next)
            && s >= self.next
    }

    /// Sample `s` when this session's run has already passed it and does not
    /// keep it in its window: how many samples back from the run's position
    /// it is, and how many samples a seek back to it decodes again. `None`
    /// when continuing (or a forward seek) reaches it.
    pub(crate) fn passed(&self, s: usize) -> Option<(usize, usize)> {
        if self.window.contains_key(&s) || self.run_start.is_none() || s >= self.next {
            return None;
        }
        Some((self.next - s, s + 1 - self.index.sync_for(s)))
    }

    /// As [`sample`](Session::sample), asking `stop` between batches of
    /// samples whether to give up (a newer request superseded this one).
    /// A stopped request returns `None` and leaves the run where it got to:
    /// a later request further along the same run continues from there.
    pub fn sample_with(
        &mut self,
        s: usize,
        stop: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<Option<DecodedFrame>> {
        if s >= self.index.samples.len() {
            return Err(MediaError::invalid(format!("sample {s} out of range")));
        }
        self.stats.requests += 1;
        self.last_target = Some(s);
        if let Some(f) = self.window.get(&s) {
            self.stats.window_hits += 1;
            super::diag::bump(&super::diag::WINDOW_HITS, 1);
            return Ok(Some(f.clone()));
        }
        let sync = self.index.sync_for(s);
        if self.continues(s) {
            self.stats.continued += 1;
            super::diag::bump(&super::diag::CONTINUED, 1);
        } else {
            self.stats.seeks += 1;
            super::diag::bump(&super::diag::SEEKS, 1);
            self.decoder.reset()?;
            self.window.clear();
            self.run_start = Some(sync);
            self.next = sync;
            self.drained = false;
        }
        // Feed through s.
        let mut out = Vec::new();
        if !self.feed_until(s, s, &mut out, stop)? {
            self.stats.stopped += 1;
            return Ok(None);
        }
        self.absorb(&mut out)?;
        if !self.window.contains_key(&s) && self.decoder.syncs() {
            let t = std::time::Instant::now();
            self.decoder.sync(&mut out)?;
            self.stats.wait_us += t.elapsed().as_micros() as u64;
            super::diag::bump(&super::diag::DECODE_WAIT_US, t.elapsed().as_micros() as u64);
            self.absorb(&mut out)?;
        }
        // Decoders with reorder delay output s after later samples.
        let mut extra = 0;
        while !self.window.contains_key(&s) && !self.drained {
            if self.next >= self.index.samples.len() || extra >= MAX_LOOKAHEAD {
                self.decoder.drain(&mut out)?;
                self.stats.drains += 1;
                self.drained = true;
            } else {
                let next = self.next;
                self.feed_until(next, s, &mut out, &mut |_| false)?;
                extra += 1;
            }
            self.absorb(&mut out)?;
        }
        let failures = self.decoder.failures();
        let frame = self.window.get(&s).cloned().ok_or_else(|| {
            let cts = self.index.samples[s].cts;
            match failures.iter().find(|f| f.0 == cts) {
                Some((_, st)) => MediaError::decode(format!(
                    "sample {s} (pts {cts}) failed to decode: status {st}"
                )),
                None => {
                    MediaError::decode(format!("the decoder did not output sample {s} (pts {cts})"))
                }
            }
        })?;
        self.trim(Some(s));
        Ok(Some(frame))
    }

    /// Feeds decode-order samples from `self.next` through `upto`, skipping
    /// the leading pictures of the run's sync sample and, before `target`,
    /// pictures nothing references that show before it. Frames the decoder
    /// outputs on the way go into the window, trimmed around `target` as
    /// they come so a long GOP does not hold every decoded picture.
    fn feed_until(
        &mut self,
        upto: usize,
        target: usize,
        out: &mut Vec<DecodedFrame>,
        stop: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<bool> {
        if self.next > upto {
            return Ok(true);
        }
        let start = self.run_start.expect("a run is open");
        let start_cts = self.index.samples[start].cts;
        let target_cts = self.index.samples[target].cts;
        let gop_end = (start + 1..self.index.samples.len())
            .find(|&k| self.index.samples[k].key)
            .unwrap_or(self.index.samples.len());
        let which: Vec<usize> = (self.next..=upto)
            .filter(|&k| {
                let leading = k > start && k < gop_end && self.index.samples[k].cts < start_cts;
                if leading {
                    self.stats.leading_skipped += 1;
                }
                !leading
            })
            .collect();
        for chunk in which.chunks(FEED_CHUNK) {
            let progress = Progress {
                run_start: start,
                next: self.next,
                target,
            };
            if stop(progress) {
                return Ok(false);
            }
            let t = std::time::Instant::now();
            let pkts = self.reader.read(&self.index, chunk)?;
            self.stats.read_us += t.elapsed().as_micros() as u64;
            let t = std::time::Instant::now();
            for (p, &k) in pkts.iter().zip(chunk) {
                let behind = k != target && self.index.samples[k].cts < target_cts;
                let (shown, at) = (self.shown_at[k], self.shown_at[target]);
                let kept = behind
                    && shown.saturating_add(self.keep_before.min(self.window_limit)) >= at
                    && at.saturating_sub(shown) % self.keep_stride == 0;
                if behind && !kept && self.skip.is_some_and(|d| d.disposable(&p.data)) {
                    self.stats.disposable_skipped += 1;
                    continue;
                }
                self.decoder.send(p, out)?;
                self.stats.samples_fed += 1;
                super::diag::bump(&super::diag::SAMPLES_FED, 1);
            }
            self.stats.send_us += t.elapsed().as_micros() as u64;
            self.next = chunk[chunk.len() - 1] + 1;
            self.absorb(out)?;
            self.trim(Some(target));
        }
        self.next = upto + 1;
        Ok(true)
    }

    fn absorb(&mut self, out: &mut Vec<DecodedFrame>) -> Result<()> {
        for mut f in out.drain(..) {
            let s = *self.by_cts.get(&f.pts).ok_or_else(|| {
                MediaError::decode(format!(
                    "decoder output a frame at pts {} that no sample has",
                    f.pts
                ))
            })?;
            f.color = self.color;
            self.stats.frames_out += 1;
            self.window.insert(s, f);
        }
        Ok(())
    }

    /// Keeps the window within its limit, dropping frames farthest from the
    /// last request.
    fn trim(&mut self, around: Option<usize>) {
        let reorder = self.reorder || super::diag::knob("PM_FIX_REORDER") == Some(1);
        while self.window.len() > 1
            && (self.window.len() > self.window_limit.max(1)
                || self.window.values().map(DecodedFrame::bytes).sum::<usize>() > self.window_bytes)
        {
            if reorder {
                // Drop the farthest frame that a later request could get
                // back without a seek; frames already decoded and shown after
                // the newest request stay (bounded by the reorder depth).
                let c = around.or(self.last_target).unwrap_or(0);
                let Some(drop) = self
                    .window
                    .keys()
                    .copied()
                    .filter(|&k| k != c && !self.pending(k))
                    .max_by_key(|&k| k.abs_diff(c))
                else {
                    break;
                };
                self.window.remove(&drop);
                continue;
            }
            let first = *self.window.keys().next().unwrap();
            let last = *self.window.keys().next_back().unwrap();
            let c = around.unwrap_or(last);
            let drop = if c.abs_diff(first) >= c.abs_diff(last) {
                first
            } else {
                last
            };
            self.window.remove(&drop);
        }
    }

    /// Drops every frame the window holds (a lease shrink). Decoding state
    /// is kept: the next request continues the current run when it can.
    pub fn clear_window(&mut self) {
        if self.reorder || super::diag::knob("PM_FIX_REORDER") == Some(1) {
            let keep: Vec<usize> = self
                .window
                .keys()
                .copied()
                .filter(|&k| self.pending(k))
                .collect();
            self.window.retain(|k, _| keep.contains(k));
            return;
        }
        self.window.clear();
    }

    /// Keep decoded reorder frames (a drag is scrubbing; PM_FIX_VIDEO2).
    pub fn set_reorder(&mut self, on: bool) {
        self.reorder = on;
    }

    /// A decoded frame shown after the newest request whose sample the run
    /// has passed: dropping it means seeking back to its keyframe.
    fn pending(&self, k: usize) -> bool {
        let Some(t) = self.last_target else {
            return false;
        };
        !self.drained
            && k < self.next
            && self.index.samples[k].cts > self.index.samples[t].cts
    }

    /// Frames currently held by the window.
    pub fn window_frames(&self) -> impl Iterator<Item = (usize, &DecodedFrame)> {
        self.window.iter().map(|(k, v)| (*k, v))
    }
}
