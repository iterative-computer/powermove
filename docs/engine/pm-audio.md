# pm-audio: decode, mix, playback clock, export audio

Plan sections 15 and 16, contract 6 (the playback clock), section 25 (audio bars).
Phase 2 tracer bullet: the device clock with its latency as the master clock,
streaming decode, mixing, playback, scrubbing, and the offline render export uses.

| Module | What it does |
|---|---|
| `pm_media::audio` | FFmpeg audio decode to program samples: priming removed, sample-exact positions, sample-accurate seeks, speaker layouts, no length limit |
| `source` | Decoded program samples in 65,536-sample blocks under a byte budget, decoded ahead by worker threads |
| `kernel` | Band-limited interpolation (rubato's windowed-sinc design) for sample-rate conversion, speed, and remaps |
| `graph` | Which clips sound and how level and source time follow the document (the legacy `audioLayers` and `gainAt`) |
| `mixer` | Voices → per-layer strips → master in f32 at `audioSampleRate`, a pure function of the sample range |
| `layout` | Down-mix and up-mix matrices |
| `offline` | Export render: PCM float, 24-bit, 16-bit with TPDF dither; streaming WAV/RF64 writer |
| `meter`, `waveform` | Level taps (peak, RMS) per strip and master; legacy peak envelopes and a min/max pyramid |
| `transport` | Play, pause, seek, loop, frame step, play around, audio scrub |
| `clock` | Clock samples from the device's presentation times (contract 6) |
| `ring`, `playback`, `device` | Render thread → lock-free ring → CoreAudio callback through `cpal` |

## Decode (`pm_media::audio`)

- **Program samples.** Sample 0 is the first program sample. FFmpeg already removes the
  priming its demuxers announce (MP4 edit lists, the LAME header, Ogg pre-skip) and shifts
  frame timestamps to match, so its stream `start_time` is program sample 0. When
  `probe::timing::priming` knows of more (an `iTunSMPB` tag without an edit list, Apple AAC
  without metadata: 2112), the decoder skips the difference too.
- **Positions.** With a timebase at least as fine as a sample (MP4, MOV, Ogg, MP3, WAV,
  FLAC, MPEG-TS) every frame is placed by its timestamp, rounded to the nearest sample;
  gaps become silence and overlaps are dropped beyond a one-sample tolerance. Coarser
  timebases (Matroska and WebM use 1 ms) are counted from the start, and a seek decodes
  forward from the start.
- **Seeks.** `seek(n)` starts a per-codec pre-roll earlier (AAC 4096, MP3 8 × 1152, Opus
  80 ms, Vorbis 8192, AC-3 3072, lossless 0) from a freshly opened decoder and discards
  to `n`. A fresh decoder matters: FFmpeg's AAC decoder keeps its noise-substitution
  generator across a flush, so a flushed decoder answers differently depending on history.
  `seek_anchored` never continues the running decode, so a block's samples depend on the
  file and the block index only.
- **Length.** From the container's duration (Ogg's last granule minus its pre-skip), a
  pure function of the metadata, never refined while decoding.

Measured (`pm-media/tests/audio_decode.rs`): every fixture (`tone.wav`, `.flac`, `.m4a`,
`.mp3`, `.opus`, `h264-aac.mp4`) starts on the first sample of its sine (priming 1024,
1105, 312 handled) with the metadata length equal to the decoded length; seeks equal the
sequential decode exactly for WAV and FLAC (also 44.1 kHz), within 4.3e-5 for AAC tones,
exactly for the MP3 and Opus fixtures, and land on the exact sample for AAC with noise
substitution (whose noise is random by design). A 40-minute AAC renders bit-identical to
FFmpeg's own accurate seek and decode at 39:59.

## Resampling (`kernel`)

The filter is rubato's sinc design: 512 taps under a squared Blackman-Harris window, cutoff
from `rubato::calculate_cutoff`, scaled down (and the filter lengthened) by the read rate
when a voice reads its source faster than one source sample per output sample.

Voices do not run `rubato::Async`. A voice must answer "what is heard at project sample
n" for any n: seeks land on a sample, playback after a seek must equal playback straight
through, remaps jump and run backwards, and the export must equal the preview. `Async`
keeps a running position advanced by the ratio and can neither seek nor reverse. So
voices evaluate the same filter as a pure function of each output sample's source
position:

- **Copy**: same rate, linear map: an integer offset, no filter.
- **Grid**: linear map at any rational ratio: positions fall on `project / gcd` phases
  (160 for 44.1 → 48 kHz); the bank holds exactly those phases and each output is one dot
  product, with exact integer position arithmetic (rubato's `Nearest` interpolation at the
  oversampling factor it recommends for such ratios).
- **Free**: speed, variable speed, time remaps: 256 phases with cubic interpolation
  between them (rubato's `Cubic`), positions from the time map. The filter for a control
  interval is chosen from that interval's read rate, never from a render's block, so the
  result does not depend on where a render starts.

rubato itself (`Async`, sinc, cubic, 256 phases, the same 512-tap design) converts the
master to the device rate when the device does not run at the project rate.

Measured (`tests/resampler.rs`; passband deviation up to 0.45·fs of the lower rate;
stopband: everything that aliases or images into the passband):

| Path | Passband max deviation | Noise and distortion | Stopband |
|---|---|---|---|
| Grid 44.1 → 48 kHz | 0.0000 dB | −140.2 dB | −153.5 dB |
| Grid 48 → 44.1 kHz | 0.0000 dB | −140.0 dB | n/a (nothing folds into the passband) |
| Grid 96 → 48 kHz | 0.0000 dB | −137.5 dB | −137.0 dB |
| Grid 22.05 → 48 kHz | 0.0000 dB | −139.9 dB | −153.3 dB |
| Grid 32 → 48 kHz | 0.0000 dB | −139.4 dB | −144.4 dB |
| Free, irrational rate 0.986 | 0.0000 dB | −139.3 dB | n/a |
| Free, speed 1.5 / 2 | 0.0000 dB | −140.6 / −141.6 dB | −135.8 / −139.5 dB |
| rubato 48 → 44.1 / 44.1 → 48 kHz | 0.0000 / 0.0001 dB | −136.3 / −133.4 dB | −129.1 / −154.4 dB |

## The mix (`graph`, `mixer`)

The graph walk is the legacy `audioLayers`: audio layers with media; video layers with
`embeddedAudio`; precompositions to depth 8, one voice per path (a composition used twice
plays twice, ids `pre/inner`); per-composition solo (a layer is skipped when its
composition has a soloed layer and neither it nor an enclosing group is soloed); layers
and groups switched off; `audioMuted` on videos and precompositions. Each voice feeds the
strip of its top-level layer.

Levels follow `gainAt`: `gain` (audio layers, clamped 0 to 4) or `audioGain` (embedded
audio, null is unity), times each enclosing precomposition's `audioGain` and its
composition's `audioGain`; linear fades over the clip's audible length (the layer, cut
where the source ends); zero while the layer or an enclosing group is switched off. The
open composition's `audioGain` is the master fader. Channel layouts fold into the bus by
Web Audio's speaker rules where Web Audio has them (mono copies to both sides, quad
½(L + Ls), 5.1 L + √½(C + Ls) with LFE dropped) and by ITU-R BS.775 otherwise (Web Audio
mixes unknown layouts "discretely", dropping 7.1's center and surrounds).

Automation: gain is applied per sample. Animated inputs (levels, `on` switches, fades,
remaps, speed) are evaluated through `pm_eval` at control points every 16 samples
(3 kHz at 48 kHz), aligned to absolute sample indexes, and interpolated linearly between
them; fades are computed per sample from the clip's local time. Linear time maps place
every sample exactly. Today's engine schedules gain ramps between points one composition
frame apart.

**Allowed differences from the legacy engine** (each a fix):

1. Speed and time remapping are followed (appendix A item 10: today speed is ignored and
   remapped clips are silent). Source time is `pm_eval`'s `sourceTime` at every level, so
   audio stays with the picture; pitch follows speed (pitch preservation is Phase 3b).
2. A negative layer start plays from the right source time (the legacy walk clamps `from`
   to 0, shifting the clip against its picture).
3. Animated `audioMuted`, precomposition `on`, and groups around a precomposition are
   evaluated over time, not only when static.
4. Animated `trim` follows `sourceTime` over time (the legacy mixer reads it once, at the
   playhead or at the range start).
5. Keyframed levels follow their curve at 3 kHz instead of straight segments one frame
   apart.
6. The export has round(duration × sr) samples (section 2); today's has
   ceil(duration × 48000).
7. The mix runs at `audioSampleRate` (default 48 kHz); today's export always mixes at
   48 kHz after decoding at the live AudioContext's rate.
8. Layouts Web Audio mixes discretely fold by BS.775 (above).

## Playback (`transport`, `clock`, `ring`, `playback`, `device`)

```text
control ──commands──▶ render thread ──chunks──▶ ring ──▶ CoreAudio callback
                       (transport, mixer,                    │
                        rate conversion)                     │
consumers ◀───────────── ClockSlot ◀── clock samples ◀───────┘
```

- **Master clock.** The callback knows the media time of the first frame of each buffer
  (chunk tags in the ring) and its presentation time: the callback's host time (CoreAudio's
  output time stamp, `mach_absolute_time`, the `HostMicros` domain) plus the device latency
  and output stream latency, the sum Chromium's AUHAL output uses for its playout time. The
  safety offset is the HAL's scheduling margin before the output time and is not added.
  cpal's own `playback` estimate adds the I/O buffer as well, which would put the clock a
  buffer late (about 10 ms at 512 frames); `examples/device_probe` reports both.
- **Publication.** A sample on every new epoch or state, and every 100 ms while running
  (contract 6 asks for 4 a second), so the line re-anchors to the device clock and drift
  never accumulates. Samples are ordered by (epoch, seq); a callback that read a chunk just
  before the transport moved on cannot publish its old line over the new one.
- **Pre-roll.** Play and seek start a new epoch: the callback drops queued chunks of older
  epochs, and the render thread decodes the blocks of the first 250 ms (waiting at most
  400 ms) before it queues the first chunk. What plays equals the offline render from the
  same sample, bit for bit (the mixer is a pure function of the sample range).
- **Stops.** Pause stops where the listener is (the clock's position at the moment of the
  command); the end of a run (composition end, play-around end, a scrub grain) stops once
  the queued audio has played out.
- **Loop.** Chunks end at the loop end and the next starts at the loop start: sample-exact
  wraps; clock samples carry the loop range.
- **Audio scrub** (contract 6 open question 2): rides on `Shuttling` samples. A scrub seek
  or frame step while paused, with audio scrub on, plays a grain of one composition frame
  or 60 ms, whichever is longer, with 4 ms fades.
- **Device changes** (open question 3): playback continues on the new device with a new
  epoch, re-anchored from what is heard. The default output's id and latency are polled
  once a second; a host that owns the device passes its own watch.
- **Device rate.** When the device does not run at the project rate, rubato converts the
  master; chunk tags count device frames, so the clock stays exact in ticks.
- **Rates.** Phase 2 plays at 1× only; `Play` at another rate (J/K/L) is refused with
  `Unsupported` until Phase 3b.

Measured with a simulated device (`tests/playback.rs`): the published line predicts the
presented media to 0.3 µs on a nominal device; at ±150 ppm device drift the final line
alone is off by 400 µs after 2.7 s, while the line as published (re-anchored every
100 ms) stays within 29 µs at 300 ppm. Playback from a seek equals the offline render;
pause, seek, loop wraps, scrub grains, a 44.1 kHz device under a 48 kHz project, and a
device change behave.

## Export (`offline`)

`offline::render(mixer, first, end, block, sink)` renders project samples `[first, end)`
(`sample_range` converts a tick range: round(ticks × sr / TICKS), so a range starting on a
sample has round(duration × sr) samples) and hands interleaved f32 to a sink, decoding
missing blocks on the calling thread. `pm_export::Pipeline::push_audio` takes that f32;
`offline::encode` and `WavWriter` produce PCM: 32-bit float, 24-bit (rounded, no dither:
its quantization sits at −144 dBFS), 16-bit with TPDF dither (±1 LSB triangular from a
counter-based generator keyed by sample index and channel, so chunked and whole renders
write the same bytes). `WavWriter` writes WAVE_FORMAT_EXTENSIBLE and switches to RF64 past
4 GiB. AAC through AudioToolbox is the Export lane's encoder.

## Performance (`examples/mix_bench`, release, this Mac, 20 s renders)

| Voices | Cost per voice-second of audio |
|---|---|
| 48 kHz sources (copy) | 0.27 ms |
| 44.1 kHz sources (exact-phase sinc, NEON) | 6.0 ms |
| Keyframed gain (control points) | 4.8 ms |
| Variable speed (free-position sinc) | 22 ms |
| Constant speed 1.5 | 17 ms |

## Legacy parity

`tools/legacy-mix.ts` captures `PM.Audio.renderOffline(0, dur)` (the path every export with
audio takes) from the built legacy app for the corpus audio project and audio projects
built for this lane (clicks at 30, 29.97, and from 44.1 kHz; gain keyframes, fades, master;
precomposition nesting; solo; mute and groups; layouts and rates; embedded audio), into
`~/Library/Application Support/Powermove Engine Corpus/audio-<stamp>-<commit>/`.
`tests/legacy_parity.rs` renders the same loaded documents with pm-audio and compares.

```sh
bun install && bun run --cwd apps/desktop build
~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-audio/tools/legacy-mix.ts
cargo test -p pm-audio --test legacy_parity -- --nocapture
```

## Tests

```sh
source engine/env.sh
cargo test -p pm-media --test audio_decode
cargo test -p pm-audio -- --test-threads=4
cargo run --release -p pm-audio --example mix_bench
pm-electron-lock cargo run --release -p pm-audio --example device_probe   # opens the output device, silent
```
