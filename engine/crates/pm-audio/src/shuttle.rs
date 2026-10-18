//! The 2× shuttle (J/K/L, section 15): the mix at twice the speed with its
//! pitch preserved, through one time stretch on the master.
//!
//! The render thread mixes the samples a 2× run plays, in play order
//! (reversed for −2×), and hands them to [`Shuttle::push`] as a stream; the
//! stretch turns every two stream samples into one output frame. Output
//! frame k is aligned with stream sample 2k
//! ([`pm_stretch::Stretcher::output_seek`] at the start of each epoch, and
//! input fed the library's latencies ahead), so each output frame is tagged
//! with the media of the stream sample it plays: the clock line stays
//! exact through loop wraps (a wrap is a jump in the stream's media, where
//! output pieces split).
//!
//! The stretch needs its input ahead of its output, so the first output of
//! a run comes once the stream holds the seek length (about 0.25 s of
//! media, mixed in a few milliseconds); the end of a run flushes the
//! stretch's tail.

use std::collections::VecDeque;

use pm_protocol::clock::TransportState;
use pm_protocol::common::TickRange;
use pm_stretch::Stretcher;

use crate::error::{AudioError, Result};

/// Stream samples per output frame.
pub const SPEED: u64 = 2;

/// A contiguous run of the stream: consecutive stream samples playing
/// consecutive media samples in one direction.
#[derive(Clone, Copy, Debug)]
struct Run {
    /// Stream index of its first sample.
    offset: u64,
    len: u64,
    /// Media sample of its first stream sample.
    media: i64,
    /// +1 forward, −1 reverse.
    dir: i64,
    state: TransportState,
    loop_range: Option<TickRange>,
}

/// One piece of output, linear in media time.
#[derive(Debug)]
pub struct Piece {
    /// Media sample of its first frame.
    pub start: i64,
    /// Project samples per frame (±2).
    pub rate: i64,
    pub state: TransportState,
    pub loop_range: Option<TickRange>,
    /// Planar frames.
    pub planar: Vec<Vec<f32>>,
}

pub struct Shuttle {
    stretcher: Stretcher,
    channels: usize,
    epoch: Option<u32>,
    started: bool,
    /// Stream samples received and not yet fed (planar).
    pending: Vec<Vec<f32>>,
    /// Stream index of `pending[0]`.
    pending_at: u64,
    /// Stream samples received.
    received: u64,
    /// Output frames produced.
    produced: u64,
    runs: VecDeque<Run>,
    scratch: Vec<Vec<f32>>,
}

fn err(e: pm_stretch::StretchError) -> AudioError {
    AudioError::Device(e.to_string())
}

impl Shuttle {
    pub fn new(channels: usize, rate: u32) -> Result<Shuttle> {
        Ok(Shuttle {
            stretcher: Stretcher::new(channels, rate).map_err(err)?,
            channels,
            epoch: None,
            started: false,
            pending: vec![Vec::new(); channels],
            pending_at: 0,
            received: 0,
            produced: 0,
            runs: VecDeque::new(),
            scratch: vec![Vec::new(); channels],
        })
    }

    /// Touch the master FFT/work buffers while paused, then restore a fresh
    /// deterministic stream. Real input's output-seek still belongs to its
    /// run; silence here must never become that run's pre-roll or output.
    pub(crate) fn warm(&mut self) -> Result<()> {
        let frames = self
            .stretcher
            .output_seek_length(SPEED as f64)
            .max(self.lead() as usize + 2 * crate::ring::CHUNK);
        let silence = vec![vec![0.0; frames]; self.channels];
        let mut output = Vec::new();
        let result = self.push(
            &silence,
            0,
            1,
            0,
            TransportState::Shuttling,
            None,
            &mut output,
        );
        self.reset();
        result
    }

    /// The epoch of the stream in progress.
    pub fn epoch(&self) -> Option<u32> {
        self.epoch
    }

    /// Drops the stream (a new epoch).
    pub fn reset(&mut self) {
        self.epoch = None;
        self.started = false;
        for p in &mut self.pending {
            p.clear();
        }
        self.pending_at = 0;
        self.received = 0;
        self.produced = 0;
        self.runs.clear();
    }

    /// Stream samples needed ahead of output frame `k`.
    fn lead(&self) -> u64 {
        (self.stretcher.input_latency() + SPEED as usize * self.stretcher.output_latency()) as u64
    }

    /// Appends stream samples (planar, in play order) that play media from
    /// `media` in direction `dir`, and returns the output pieces that are
    /// complete.
    #[allow(clippy::too_many_arguments)]
    pub fn push(
        &mut self,
        planar: &[Vec<f32>],
        media: i64,
        dir: i64,
        epoch: u32,
        state: TransportState,
        loop_range: Option<TickRange>,
        out: &mut Vec<Piece>,
    ) -> Result<()> {
        if self.epoch != Some(epoch) {
            self.reset();
            self.epoch = Some(epoch);
        }
        let n = planar.first().map_or(0, Vec::len) as u64;
        if n == 0 {
            return Ok(());
        }
        match self.runs.back_mut() {
            Some(r)
                if r.dir == dir
                    && r.media + dir * r.len as i64 == media
                    && r.state == state
                    && r.loop_range == loop_range =>
            {
                r.len += n;
            }
            _ => self.runs.push_back(Run {
                offset: self.received,
                len: n,
                media,
                dir,
                state,
                loop_range,
            }),
        }
        for (p, c) in self.pending.iter_mut().zip(planar) {
            p.extend_from_slice(c);
        }
        self.received += n;
        self.drain(false, out)
    }

    /// The run ended: produces the rest of the stream.
    pub fn finish(&mut self, out: &mut Vec<Piece>) -> Result<()> {
        if self.epoch.is_none() {
            return Ok(());
        }
        self.drain(true, out)?;
        self.reset();
        Ok(())
    }

    fn take(&mut self, n: usize) -> usize {
        let n = n.min(self.pending[0].len());
        for (s, p) in self.scratch.iter_mut().zip(&mut self.pending) {
            s.clear();
            s.extend(p.drain(..n));
        }
        self.pending_at += n as u64;
        n
    }

    fn drain(&mut self, end: bool, out: &mut Vec<Piece>) -> Result<()> {
        let lead = self.lead();
        if !self.started {
            let need = (self.stretcher.output_seek_length(SPEED as f64) as u64).max(1);
            if self.received < need && !end {
                return Ok(());
            }
            let n = self.take(need as usize);
            let views: Vec<&[f32]> = self.scratch.iter().map(|c| &c[..n]).collect();
            self.stretcher.reset().map_err(err)?;
            self.stretcher.output_seek(&views).map_err(err)?;
            self.started = true;
        }
        // Frames whose input is all here: stream index 2k + lead.
        let total = if end {
            self.received.div_ceil(SPEED)
        } else {
            (self.received.saturating_sub(lead)) / SPEED
        };
        while self.produced < total {
            // Pieces split where the run under frame k changes.
            let k = self.produced;
            let run = *self
                .runs
                .iter()
                .find(|r| r.offset + r.len > SPEED * k)
                .or(self.runs.back())
                .expect("a run");
            let run_end = (run.offset + run.len).div_ceil(SPEED);
            let frames = (total.min(run_end).max(k + 1) - k) as usize;
            // Feed the input up to the frames' lead.
            let want = SPEED * (k + frames as u64) + lead;
            let feed = want.saturating_sub(self.pending_at) as usize;
            let fed = self.take(feed);
            let mut planar = vec![vec![0f32; frames]; self.channels];
            {
                let input: Vec<&[f32]> = self.scratch.iter().map(|c| &c[..fed]).collect();
                let mut output: Vec<&mut [f32]> = planar.iter_mut().map(|c| &mut c[..]).collect();
                if fed == 0 {
                    self.stretcher.process(&[], &mut output).map_err(err)?;
                } else {
                    self.stretcher.process(&input, &mut output).map_err(err)?;
                }
            }
            let into = (SPEED * k).saturating_sub(run.offset) as i64;
            out.push(Piece {
                start: run.media + run.dir * into,
                rate: run.dir * SPEED as i64,
                state: run.state,
                loop_range: run.loop_range,
                planar,
            });
            self.produced += frames as u64;
            while self
                .runs
                .front()
                .is_some_and(|r| r.offset + r.len <= SPEED * self.produced && self.runs.len() > 1)
            {
                self.runs.pop_front();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_warmup_preserves_forward_and_reverse_pcm() {
        for direction in [1, -1] {
            let mut cold = Shuttle::new(2, 48_000).unwrap();
            let mut warm = Shuttle::new(2, 48_000).unwrap();
            warm.warm().unwrap();
            let input: Vec<f32> = (0..24_000)
                .map(|i| (i as f32 * 0.071).sin() * 0.25)
                .collect();
            let mut a = Vec::new();
            let mut b = Vec::new();
            for (i, chunk) in input.chunks(512).enumerate() {
                let media = 40_000 + direction * (i * 512) as i64;
                let planar = [chunk.to_vec(), chunk.to_vec()];
                cold.push(
                    &planar,
                    media,
                    direction,
                    8,
                    TransportState::Shuttling,
                    None,
                    &mut a,
                )
                .unwrap();
                warm.push(
                    &planar,
                    media,
                    direction,
                    8,
                    TransportState::Shuttling,
                    None,
                    &mut b,
                )
                .unwrap();
            }
            cold.finish(&mut a).unwrap();
            warm.finish(&mut b).unwrap();
            assert_eq!(a.len(), b.len());
            for (a, b) in a.into_iter().zip(b) {
                assert_eq!(a.start, b.start);
                assert_eq!(a.rate, b.rate);
                assert_eq!(a.planar, b.planar);
            }
        }
    }

    /// A 440 Hz tone through the shuttle keeps its pitch, and the pieces
    /// tag the media they play: frame k plays stream sample 2k.
    #[test]
    fn twice_as_fast_at_the_same_pitch() {
        let rate = 48_000u32;
        let mut sh = Shuttle::new(1, rate).unwrap();
        let tone = |i: i64| (std::f64::consts::TAU * 440.0 * i as f64 / rate as f64).sin() as f32;
        let mut pieces = vec![];
        let mut at = 10_000i64;
        for _ in 0..200 {
            let chunk: Vec<f32> = (at..at + 512).map(tone).collect();
            sh.push(
                &[chunk],
                at,
                1,
                1,
                TransportState::Shuttling,
                None,
                &mut pieces,
            )
            .unwrap();
            at += 512;
        }
        sh.finish(&mut pieces).unwrap();
        let mut frames = 0usize;
        let mut media = 10_000i64;
        let mut out = vec![];
        for p in &pieces {
            assert_eq!(p.start, media, "piece tags follow the stream");
            assert_eq!(p.rate, 2);
            media += 2 * p.planar[0].len() as i64;
            frames += p.planar[0].len();
            out.extend_from_slice(&p.planar[0]);
        }
        assert_eq!(frames, 200 * 256);
        // Zero crossings over 0.5 s of output: 440 Hz.
        let w = &out[12_000..36_000];
        let crossings = w.windows(2).filter(|p| p[0] < 0.0 && p[1] >= 0.0).count();
        assert!((crossings as i64 - 220).abs() <= 1, "{crossings}");
    }

    /// A wrap (a jump in the stream's media) splits pieces, so each is
    /// linear in media time; reverse pieces count down.
    #[test]
    fn pieces_split_at_wraps_and_count_down_in_reverse() {
        let mut sh = Shuttle::new(1, 48_000).unwrap();
        let mut pieces = vec![];
        let chunk = vec![0.1f32; 4_000];
        sh.push(
            std::slice::from_ref(&chunk),
            50_000,
            -1,
            3,
            TransportState::Shuttling,
            None,
            &mut pieces,
        )
        .unwrap();
        // The loop wraps: the stream jumps to 60_000.
        sh.push(
            std::slice::from_ref(&chunk),
            60_000,
            -1,
            3,
            TransportState::Shuttling,
            None,
            &mut pieces,
        )
        .unwrap();
        sh.push(
            &[chunk],
            56_000,
            -1,
            3,
            TransportState::Shuttling,
            None,
            &mut pieces,
        )
        .unwrap();
        sh.finish(&mut pieces).unwrap();
        let mut k = 0u64;
        for p in &pieces {
            let stream = 2 * k;
            let expect = if stream < 4_000 {
                50_000 - stream as i64
            } else {
                60_000 - (stream as i64 - 4_000)
            };
            assert_eq!(p.start, expect, "frame {k}");
            assert_eq!(p.rate, -2);
            k += p.planar[0].len() as u64;
        }
        assert_eq!(k, 6_000);
    }
}
