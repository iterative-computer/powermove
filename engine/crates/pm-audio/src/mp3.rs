//! MP3 decoded as today decodes it: Chromium 152 (Electron 44) hands MP3 to
//! Symphonia (`media/filters/symphonia_audio_decoder.cc`, enabled by default
//! on macOS through `kSymphoniaMp3Decoding`), not to FFmpeg. The same
//! Symphonia release (0.6.0, Chromium's `third_party/rust` pin) decodes here,
//! so every sample equals today's `decodeAudioData` output.
//!
//! A block decodes from a seek [`PREROLL`] frames ahead of it and discards
//! them: the decoder's state (the bit reservoir, the IMDCT overlap and the
//! synthesis window) depends only on the frames just before, so the block
//! equals the sequential decode from the start (`tests/sources.rs`).

use std::fs::File;
use std::path::{Path, PathBuf};

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::error::{AudioError, Result};

/// MP3 frames of 1152 samples decoded and discarded before a block.
pub const PREROLL: u64 = 1152 * 12;

fn err(what: &str, e: impl std::fmt::Display) -> AudioError {
    AudioError::Media(pm_media::MediaError::decode(format!("{what}: {e}")))
}

pub struct Mp3Decoder {
    path: PathBuf,
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
    track: u32,
    channels: usize,
    /// Program sample of the next decoded frame.
    position: u64,
    /// Decoded frames not yet read, planar.
    pending: Vec<Vec<f32>>,
    offset: usize,
}

fn open_reader(path: &Path) -> Result<Box<dyn FormatReader>> {
    let file = File::open(path).map_err(|e| err("open", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| err("probe", e))
}

impl Mp3Decoder {
    pub fn open(path: &Path, channels: usize) -> Result<Mp3Decoder> {
        let reader = open_reader(path)?;
        let track = reader
            .default_track(TrackType::Audio)
            .ok_or_else(|| err("open", "no audio track"))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| err("open", "no audio parameters"))?
            .clone();
        let id = track.id;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| err("decoder", e))?;
        Ok(Mp3Decoder {
            path: path.to_owned(),
            reader,
            decoder,
            track: id,
            channels,
            position: 0,
            pending: vec![Vec::new(); channels],
            offset: 0,
        })
    }

    /// Positions the decode at program sample `index`, independently of any
    /// earlier decode.
    pub fn seek_anchored(&mut self, index: u64) -> Result<()> {
        let from = index.saturating_sub(PREROLL);
        // A fresh reader and decoder: nothing of an earlier block remains.
        *self = Mp3Decoder::open(&self.path.clone(), self.channels)?;
        if from > 0 {
            let seeked = self
                .reader
                .seek(
                    SeekMode::Accurate,
                    SeekTo::Timestamp {
                        ts: symphonia::core::units::Timestamp::new(from as i64),
                        track_id: self.track,
                    },
                )
                .map_err(|e| err("seek", e))?;
            self.position = seeked.actual_ts.get().max(0) as u64;
        }
        self.skip(index)
    }

    fn skip(&mut self, index: u64) -> Result<()> {
        while self.position < index {
            if self.available() == 0 && !self.fill()? {
                return Ok(());
            }
            let take = (index - self.position).min(self.available() as u64) as usize;
            self.offset += take;
            self.position += take as u64;
        }
        Ok(())
    }

    fn available(&self) -> usize {
        self.pending.first().map_or(0, |c| c.len()) - self.offset
    }

    fn fill(&mut self) -> Result<bool> {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(false),
                Err(symphonia::core::errors::Error::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(false);
                }
                Err(e) => return Err(err("read", e)),
            };
            if packet.track_id != self.track {
                continue;
            }
            let buf = match self.decoder.decode(&packet) {
                Ok(b) => b,
                // As Chromium's glue: a damaged frame is skipped.
                Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                Err(e) => return Err(err("decode", e)),
            };
            let frames = buf.frames();
            let mut inter = vec![0f32; buf.samples_interleaved()];
            buf.copy_to_slice_interleaved(&mut inter);
            let ch = inter.len() / frames.max(1);
            for c in self.pending.iter_mut() {
                c.drain(..self.offset.min(c.len()));
            }
            self.offset = 0;
            for (k, c) in self.pending.iter_mut().enumerate() {
                let src = k.min(ch.saturating_sub(1));
                // decodeAudioData clips floats to [−1, 1] (NaN to 0).
                c.extend((0..frames).map(|i| {
                    let v = inter[i * ch + src];
                    if v.is_nan() { 0.0 } else { v.clamp(-1.0, 1.0) }
                }));
            }
            if frames > 0 {
                return Ok(true);
            }
        }
    }

    /// Up to `frames` samples per channel appended to `out`.
    pub fn read(&mut self, out: &mut [Vec<f32>], frames: usize) -> Result<usize> {
        let mut done = 0;
        while done < frames {
            if self.available() == 0 && !self.fill()? {
                break;
            }
            let take = (frames - done).min(self.available());
            for (dst, src) in out.iter_mut().zip(&self.pending) {
                dst.extend_from_slice(&src[self.offset..self.offset + take]);
            }
            self.offset += take;
            self.position += take as u64;
            done += take;
        }
        Ok(done)
    }
}
