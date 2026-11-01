use pm_promedia::{codec::*, media::*};
use std::{collections::VecDeque, io};
struct Decode {
    info: MediaInfo,
    samples: VecDeque<Sample>,
    fail: bool,
}
impl Decoder for Decode {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn next_sample(&mut self) -> io::Result<Option<Sample>> {
        if self.fail {
            Err(io::Error::other("decode"))
        } else {
            Ok(self.samples.pop_front())
        }
    }
}
#[derive(Default)]
struct Encode {
    samples: Vec<Sample>,
    finished: bool,
    fail: bool,
}
impl Encoder for Encode {
    fn write(&mut self, s: &Sample) -> io::Result<()> {
        if self.fail {
            return Err(io::Error::other("encode"));
        }
        self.samples.push(s.clone());
        Ok(())
    }
    fn finish(&mut self) -> io::Result<()> {
        self.finished = true;
        Ok(())
    }
}
fn decoder() -> Decode {
    Decode {
        info: MediaInfo {
            width: 1,
            height: 1,
            frame_rate: Rational {
                numerator: 24,
                denominator: 1,
            },
            time_base: Rational {
                numerator: 1,
                denominator: 24,
            },
            start_tick: 0,
            duration_ticks: 2,
            alpha: true,
            audio_channels: 2,
            audio_sample_rate: 48000,
            colour: "bt709/srgb/full".into(),
        },
        samples: VecDeque::from([
            Sample::Video {
                tick: 0,
                duration: 1,
                width: 1,
                height: 1,
                rgba: vec![[0.25, 0.5, 1., 0.5]],
            },
            Sample::Audio {
                tick: 0,
                channels: 2,
                sample_rate: 48000,
                interleaved: vec![0.5, -0.5],
            },
            Sample::Video {
                tick: 1,
                duration: 1,
                width: 1,
                height: 1,
                rgba: vec![[1., 0., 0., 1.]],
            },
        ]),
        fail: false,
    }
}
#[test]
fn linux_codec_double_preserves_timestamps_alpha_audio_and_order() {
    let mut d = decoder();
    let expected = d.samples.iter().cloned().collect::<Vec<_>>();
    let mut e = Encode::default();
    assert_eq!(
        transcode(&mut d, &mut e, |_| Control::Continue).unwrap(),
        TranscodeOutcome::Complete { samples: 3 }
    );
    assert_eq!(e.samples, expected);
    assert!(e.finished);
}
#[test]
fn pause_cancel_never_finalize_partial_output() {
    for pause in [true, false] {
        let mut d = decoder();
        let mut e = Encode::default();
        let out = transcode(&mut d, &mut e, |n| {
            if n == 1 {
                if pause {
                    Control::Pause
                } else {
                    Control::Cancel
                }
            } else {
                Control::Continue
            }
        })
        .unwrap();
        assert_eq!(
            out,
            if pause {
                TranscodeOutcome::Paused { samples: 1 }
            } else {
                TranscodeOutcome::Cancelled { samples: 1 }
            }
        );
        assert_eq!(e.samples.len(), 1);
        assert!(!e.finished);
    }
}
#[test]
fn control_is_checked_again_after_decoder_returns() {
    let mut d = decoder();
    let mut e = Encode::default();
    let mut checks = 0;
    let out = transcode(&mut d, &mut e, |_| {
        checks += 1;
        if checks == 2 {
            Control::Cancel
        } else {
            Control::Continue
        }
    })
    .unwrap();
    assert_eq!(out, TranscodeOutcome::Cancelled { samples: 0 });
    assert!(e.samples.is_empty());
    assert!(!e.finished);
}
#[test]
fn decode_and_encode_failures_do_not_finalize() {
    for decode_fail in [true, false] {
        let mut d = decoder();
        d.fail = decode_fail;
        let mut e = Encode {
            fail: !decode_fail,
            ..Encode::default()
        };
        assert!(transcode(&mut d, &mut e, |_| Control::Continue).is_err());
        assert!(!e.finished);
    }
}
#[test]
fn malformed_decoded_samples_refuse() {
    for s in [
        Sample::Video {
            tick: 0,
            duration: 1,
            width: 2,
            height: 1,
            rgba: vec![[0.; 4]],
        },
        Sample::Audio {
            tick: 0,
            channels: 2,
            sample_rate: 48000,
            interleaved: vec![0.],
        },
        Sample::Video {
            tick: 0,
            duration: 1,
            width: 1,
            height: 1,
            rgba: vec![[f32::NAN; 4]],
        },
    ] {
        assert!(s.validate().is_err());
    }
}

#[test]
fn decoder_cannot_finalize_empty_video_or_audio_only_stream() {
    for audio_only in [false, true] {
        let mut d = decoder();
        d.samples
            .retain(|s| audio_only && matches!(s, Sample::Audio { .. }));
        let mut e = Encode::default();
        assert!(transcode(&mut d, &mut e, |_| Control::Continue).is_err());
        assert!(!e.finished);
    }
}
#[test]
fn decoded_samples_must_match_declared_source_metadata() {
    for audio in [false, true] {
        let mut d = decoder();
        d.samples = VecDeque::from([if audio {
            Sample::Audio {
                tick: 0,
                channels: 1,
                sample_rate: 44100,
                interleaved: vec![0.],
            }
        } else {
            Sample::Video {
                tick: 0,
                duration: 1,
                width: 2,
                height: 1,
                rgba: vec![[0.; 4]; 2],
            }
        }]);
        let mut e = Encode::default();
        assert!(transcode(&mut d, &mut e, |_| Control::Continue).is_err());
        assert!(e.samples.is_empty());
        assert!(!e.finished);
    }
}
