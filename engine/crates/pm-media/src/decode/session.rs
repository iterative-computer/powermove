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
}

pub struct Session {
    index: Arc<FrameIndex>,
    reader: PacketReader,
    decoder: Decoder,
    /// Sample index by composition time (decoders echo it back).
    by_cts: HashMap<i64, usize>,
    /// The sync sample the current run started at.
    run_start: Option<usize>,
    /// The next decode-order sample the run feeds.
    next: usize,
    drained: bool,
    /// Decoded frames by sample.
    window: BTreeMap<usize, DecodedFrame>,
    window_limit: usize,
    color: Option<ColorDescription>,
    pub stats: SessionStats,
}

/// Frames a session keeps besides the one it returns.
pub const DEFAULT_WINDOW: usize = 16;
/// Samples fed past the target, waiting for a decoder with reorder delay to
/// output it, before draining instead.
const MAX_LOOKAHEAD: usize = 64;

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
        Session {
            index,
            reader,
            decoder,
            by_cts,
            run_start: None,
            next: 0,
            drained: false,
            window: BTreeMap::new(),
            window_limit: DEFAULT_WINDOW,
            color,
            stats: SessionStats::default(),
        }
    }

    pub fn set_window(&mut self, frames: usize) {
        self.window_limit = frames;
        self.trim(None);
    }

    pub fn decoder_info(&self) -> DecoderInfo {
        self.decoder.info()
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
        if s >= self.index.samples.len() {
            return Err(MediaError::invalid(format!("sample {s} out of range")));
        }
        self.stats.requests += 1;
        if let Some(f) = self.window.get(&s) {
            self.stats.window_hits += 1;
            return Ok(f.clone());
        }
        let sync = self.index.sync_for(s);
        let continuous = !self.drained
            && self
                .run_start
                .is_some_and(|start| sync >= start && sync < self.next)
            && s >= self.next;
        if continuous {
            self.stats.continued += 1;
        } else {
            self.stats.seeks += 1;
            self.decoder.reset()?;
            self.window.clear();
            self.run_start = Some(sync);
            self.next = sync;
            self.drained = false;
        }
        // Feed through s.
        let mut out = Vec::new();
        let upto = s;
        self.feed_until(upto, &mut out)?;
        self.absorb(&mut out)?;
        if !self.window.contains_key(&s) && self.decoder.syncs() {
            self.decoder.sync(&mut out)?;
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
                self.feed_until(next, &mut out)?;
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
        Ok(frame)
    }

    /// Feeds decode-order samples from `self.next` through `upto`, skipping
    /// the leading pictures of the run's sync sample.
    fn feed_until(&mut self, upto: usize, out: &mut Vec<DecodedFrame>) -> Result<()> {
        if self.next > upto {
            return Ok(());
        }
        let start = self.run_start.expect("a run is open");
        let start_cts = self.index.samples[start].cts;
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
        let pkts = self.reader.read(&self.index, &which)?;
        for p in &pkts {
            self.decoder.send(p, out)?;
            self.stats.samples_fed += 1;
        }
        self.next = upto + 1;
        Ok(())
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
        while self.window.len() > self.window_limit.max(1) {
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

    /// Frames currently held by the window.
    pub fn window_frames(&self) -> impl Iterator<Item = (usize, &DecodedFrame)> {
        self.window.iter().map(|(k, v)| (*k, v))
    }
}
