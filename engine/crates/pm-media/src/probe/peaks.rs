//! Waveform peaks (section 13 probe item 7): minimum and maximum per bucket
//! of samples, per channel, from a full decode of an audio stream through
//! FFmpeg (section 13: FFmpeg decodes all audio). Priming samples are skipped
//! so bucket 0 starts at the first program sample.

use crate::error::{MediaError, Result};
use crate::ff::{Input, check};
use crate::sys::ffmpeg as ff;
use serde::Serialize;
use std::path::Path;
use std::ptr;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peaks {
    pub sample_rate: i64,
    pub channels: usize,
    pub samples_per_peak: usize,
    /// Priming samples skipped before bucket 0.
    pub skipped: i64,
    /// Program samples per channel that were decoded.
    pub samples: u64,
    /// `[min, max]` per bucket, channel-interleaved: bucket b of channel c is
    /// at `b * channels + c`.
    pub data: Vec<[f32; 2]>,
}

struct Acc {
    channels: usize,
    per_peak: usize,
    skip: i64,
    filled: usize,
    cur: Vec<[f32; 2]>,
    out: Vec<[f32; 2]>,
    samples: u64,
}

impl Acc {
    fn push(&mut self, frame: &[f32]) {
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        for (c, &v) in frame.iter().enumerate() {
            let e = &mut self.cur[c];
            e[0] = e[0].min(v);
            e[1] = e[1].max(v);
        }
        self.filled += 1;
        self.samples += 1;
        if self.filled == self.per_peak {
            self.flush();
        }
    }
    fn flush(&mut self) {
        if self.filled > 0 {
            self.out.extend_from_slice(&self.cur);
            self.cur.iter_mut().for_each(|e| *e = [f32::INFINITY, f32::NEG_INFINITY]);
            self.filled = 0;
        }
    }
}

/// Decodes audio stream `stream` (the best audio stream when None) and
/// returns its peaks.
pub fn peaks(path: &Path, stream: Option<usize>, samples_per_peak: usize, skip_priming: i64) -> Result<Peaks> {
    if samples_per_peak == 0 {
        return Err(MediaError::invalid("samples per peak must be positive"));
    }
    let input = Input::open(path)?;
    let s = match stream {
        Some(s) => s,
        None => input.best_audio().ok_or_else(|| MediaError::invalid("no audio stream"))?,
    };
    if input.media_type(s) != ff::AVMEDIA_TYPE_AUDIO {
        return Err(MediaError::invalid(format!("stream {s} is not audio")));
    }
    unsafe {
        let par = input.par(s);
        let codec = ff::avcodec_find_decoder((*par).codec_id);
        if codec.is_null() {
            return Err(MediaError::unsupported(format!("no FFmpeg decoder for {}", input.codec_name(s))));
        }
        let mut ctx = ff::avcodec_alloc_context3(codec);
        let guard = CtxGuard(&mut ctx);
        check(ff::avcodec_parameters_to_context(*guard.0, par), "audio parameters")?;
        (**guard.0).pkt_timebase = (*input.st(s)).time_base;
        check(ff::avcodec_open2(*guard.0, codec, ptr::null_mut()), "open audio decoder")?;
        let channels = (*par).ch_layout.nb_channels.max(1) as usize;
        let mut acc = Acc {
            channels,
            per_peak: samples_per_peak,
            skip: skip_priming.max(0),
            filled: 0,
            cur: vec![[f32::INFINITY, f32::NEG_INFINITY]; channels],
            out: Vec::new(),
            samples: 0,
        };
        let mut pkt = ff::av_packet_alloc();
        let mut frame = ff::av_frame_alloc();
        let mut result = Ok(());
        let mut buf = vec![0f32; channels];
        loop {
            let r = ff::av_read_frame(input.fmt, pkt);
            let eof = r == ff::AVERROR_EOF;
            if r < 0 && !eof {
                result = check(r, "read audio").map(|_| ());
                break;
            }
            if !eof && (*pkt).stream_index as usize != s {
                ff::av_packet_unref(pkt);
                continue;
            }
            let sr = ff::avcodec_send_packet(*guard.0, if eof { ptr::null() } else { pkt });
            ff::av_packet_unref(pkt);
            if sr < 0 && sr != ff::AVERROR_INVALIDDATA && sr != ff::AVERROR_EOF {
                result = check(sr, "send audio").map(|_| ());
                break;
            }
            loop {
                let rr = ff::avcodec_receive_frame(*guard.0, frame);
                if rr == ff::averror(libc::EAGAIN) || rr == ff::AVERROR_EOF {
                    break;
                }
                if rr == ff::AVERROR_INVALIDDATA {
                    continue;
                }
                if let Err(e) = check(rr, "receive audio") {
                    result = Err(e);
                    break;
                }
                let n = (*frame).nb_samples as usize;
                let fmt = (*frame).format;
                let ch = ((*frame).ch_layout.nb_channels as usize).min(channels);
                let planar = ff::av_sample_fmt_is_planar(fmt) != 0;
                let bytes = ff::av_get_bytes_per_sample(fmt) as usize;
                for i in 0..n {
                    for (c, slot) in buf.iter_mut().enumerate().take(ch) {
                        let p = if planar {
                            (*(*frame).extended_data.add(c)).add(i * bytes)
                        } else {
                            (*(*frame).extended_data).add((i * ch + c) * bytes)
                        };
                        *slot = sample(p, fmt);
                    }
                    acc.push(&buf[..channels]);
                }
                ff::av_frame_unref(frame);
            }
            if result.is_err() || eof {
                break;
            }
        }
        ff::av_packet_free(&mut pkt);
        ff::av_frame_free(&mut frame);
        result?;
        acc.flush();
        Ok(Peaks {
            sample_rate: (*par).sample_rate as i64,
            channels: acc.channels,
            samples_per_peak,
            skipped: skip_priming.max(0) - acc.skip,
            samples: acc.samples,
            data: acc.out,
        })
    }
}

struct CtxGuard(*mut *mut ff::AVCodecContext);

impl Drop for CtxGuard {
    fn drop(&mut self) {
        unsafe { ff::avcodec_free_context(self.0) }
    }
}

unsafe fn sample(p: *const u8, fmt: ff::AVSampleFormat) -> f32 {
    unsafe {
        match fmt {
            ff::AV_SAMPLE_FMT_U8 | ff::AV_SAMPLE_FMT_U8P => (*p as f32 - 128.0) / 128.0,
            ff::AV_SAMPLE_FMT_S16 | ff::AV_SAMPLE_FMT_S16P => (p as *const i16).read_unaligned() as f32 / 32768.0,
            ff::AV_SAMPLE_FMT_S32 | ff::AV_SAMPLE_FMT_S32P => (p as *const i32).read_unaligned() as f32 / 2147483648.0,
            ff::AV_SAMPLE_FMT_S64 | ff::AV_SAMPLE_FMT_S64P => ((p as *const i64).read_unaligned() as f64 / 9.223372036854776e18) as f32,
            ff::AV_SAMPLE_FMT_FLT | ff::AV_SAMPLE_FMT_FLTP => (p as *const f32).read_unaligned(),
            ff::AV_SAMPLE_FMT_DBL | ff::AV_SAMPLE_FMT_DBLP => (p as *const f64).read_unaligned() as f32,
            _ => 0.0,
        }
    }
}
