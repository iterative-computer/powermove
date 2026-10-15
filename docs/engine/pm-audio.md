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
  gaps become silence and overlaps (a frame stamped before the end of the previous one)
  drop the samples the timeline already has, so everything after sits on its timestamp;
  within one sample, a frame continues the line (container rounding).
- **Coarse timebases.** Matroska and WebM (1 ms) cannot place a frame to the sample, so
  their samples are counted. A demux-only scan (FFmpeg's parsers, or the codec's fixed
  frame size) counts the samples of every packet exactly as a decode from the start does
  and keeps anchors: packets with a unique timestamp at least 4,096 samples apart, whose
  program sample is known. A seek starts a fresh decoder exactly at the last anchor at
  least the pre-roll before its target and counts from there, so it costs the same
  anywhere in the file. The scan runs once per source, only as far as seeks need, and
  the anchors are a function of the file alone. Where no packet length is known (ALAC
  in Matroska has no parser), a seek decodes from the start, and a seek forward
  continues the running decode, so reading in order costs the stream once.
- **Seeks.** `seek(n)` starts a per-codec pre-roll earlier (AAC 4096, MP3 8 × 1152, Opus
  500 ms, Vorbis 8192, AC-3 3072, lossless 0) from a freshly opened decoder and discards
  to `n`. A fresh decoder matters: FFmpeg's AAC decoder keeps its noise-substitution
  generator across a flush, so a flushed decoder answers differently depending on history.
  Opus asks for 80 ms (RFC 7845), but CELT predicts band energies from the previous frame
  and converges slowly from a cold start (measured on tones: 0.11 off after 80 ms, 1e-3
  after 250 ms, 3e-7 after 500 ms), and a block boundary would step by the difference.
  `seek_anchored` continues the running decode only when it began where a fresh seek
  would begin (the same anchor, or the start of the stream), so a block's samples depend
  on the file and the block index only.
- **Length.** From the container's duration (Ogg's last granule minus its pre-skip), a
  pure function of the metadata, never refined while decoding. Matroska and WebM round
  their duration to the millisecond and count Opus's codec delay in it, so their Opus
  length runs over the decoded length by a few hundred samples (measured: +384 at 10 s,
  30 s, and 30.5 s; +370 at 10.0123 s; none for a live WebM, which has no duration). Past
  the decoded end the source reads as silence, so the overrun never cuts audio; a
  sample-exact length would need the demux-only scan of the whole file at open.

Measured (`pm-media/tests/audio_decode.rs`): every fixture (`tone.wav`, `.flac`, `.m4a`,
`.mp3`, `.opus`, `h264-aac.mp4`) starts on the first sample of its sine (priming 1024,
1105, 312 handled) with the metadata length equal to the decoded length; seeks equal the
sequential decode exactly for WAV and FLAC (also 44.1 kHz), within 4.3e-5 for AAC tones,
exactly for the MP3 and Opus fixtures, and land on the exact sample for AAC with noise
substitution (whose noise is random by design). A 40-minute AAC renders bit-identical to
FFmpeg's own accurate seek and decode at 39:59. Overlapping and gapped MPEG-TS timestamps
keep every sample on its timestamp (`gaps_are_silence_and_overlaps_are_dropped`).

Coarse timebases (`coarse_timebase_seeks_start_near_their_target`, 40 s in Matroska and
WebM): seeks equal the sequential decode exactly for FLAC, PCM, ALAC, Vorbis, and AAC
without noise substitution; within 2e-7 for Opus, 4e-5 for AC-3, E-AC-3 and MP2, and
7.2e-3 for AAC with noise substitution (its noise generator runs across frames). A seek
decodes 4 to 32 packets wherever it lands (ALAC: from the start), and blocks read in
order cost the stream once plus a pre-roll per block.

Long coarse sources (`pm-audio/tests/sources.rs`
`long_coarse_timebase_sources_decode_in_linear_time`, 10 minutes, release build, the
lane's `SourceCache`; the earlier code decoded every block from sample 0):

| Source | Block 0 | Block 100 | Block 200 | Block 400 | All 440 in order | Cold block 437 |
| --- | --- | --- | --- | --- | --- | --- |
| WebM/Opus, before | 1.5 ms | 92 ms | 185 ms | 365 ms | | |
| WebM/Opus, now | 3.2 ms | 3.8 ms | 4.0 ms | 3.8 ms | 1.7 s | 20 ms (96 packets) |
| MKV/FLAC, before | | | | 206 ms | | |
| MKV/FLAC, now | 2.1 ms | 1.9 ms | 2.0 ms | 2.1 ms | 0.9 s | 21 ms (64 packets) |

The "before" rows are the review's measurement of the earlier code on the same kind of
file (its whole-file cost grew as N²/2 blocks, about 1.5 minutes for this one). Every
block in order now decodes 96 to 99 packets (Opus) or 64 (FLAC), the same at the end
as at the start. A cold block near the end includes the first demux-only scan of the
file (about 16 ms for 10 minutes); later seeks reuse the anchors.

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
9. In-points of resampled and time-mapped clips are exact (the Grid and Free readers
   place them to the sub-sample). Sources already at the project rate are read by the
   Copy reader, which rounds the in-point to a whole sample as today's export does.
   Today's export plays each clip from a whole sample of its 48 kHz decode
   (`AudioBufferSourceNode.start` rounds the offset), so a trim that is not a whole
   number of samples plays up to half a sample early or late: on the real project
   0.4, −0.2, and −0.4 samples for its three clips, measured as a phase slope of
   0.0525 rad at 1 kHz (−26 dB residual at 1 kHz, −10 dB at 6 kHz). With the in-points
   rounded the same way the residual falls from −42.7 to −71.6 dBFS (−94 dB at 1 kHz).
10. Switches (`on` of layers and groups) cut on their sample (through the 16-sample
    control interpolation); today's envelope samples them one composition frame apart,
    so a switch becomes a one-frame linear ramp: over the frame after a switch off and
    the frame before a switch on (`mute-groups`: −11 to −15 dBFS residual in those
    frames, bit-identical everywhere else, guarded by the parity test).
11. Resampling is flat to 0.45 fs of the source and rejects images by more than 120 dB.
    Chromium's decode resampler (today's path for sources not at 48 kHz) rolls off from
    about 17 kHz and leaves images near −72 dBFS between 21 and 24 kHz on the real
    project; a 44.1 kHz click differs from today's in its last 3 kHz only (−58 dBFS).
12. The bus stays stereo. Today's master `GainNode` takes the widest input's channel
    count, so while a 5.1 source plays, every mono source is up-mixed to the center and
    folds back to L and R at √½ (−3 dB); without one, mono plays at unity in both
    channels. pm-audio plays mono at unity always (`layouts`: with the mono layer's
    level times √½, as today's bus plays it, ours matches today's to −96.6 dBFS, guarded
    by the parity test; the `mono` project confirms unity today for plain, center-masked,
    unmasked, 22.05 kHz WAV, AAC, Opus, and FLAC mono).

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
- **Read-ahead.** `Mixer::source_spans` bounds the source samples a range reads, for
  the pre-roll wait and for the 3 s look-ahead queued to the decode workers. Linear
  voices map the range and pad by their reader's filter. Time-mapped voices (speed,
  remaps, animated trims) are bounded per control interval from the same control points
  the render evaluates, padded by the filter the render picks for that interval, and the
  bounds are kept per group of 256 control points (4,096 samples) until the graph
  changes. Evaluating them costs about as much as the render's own evaluation of those
  points (table below), so the render loop bounds at most 8 new groups a turn (about
  6 ms) after filling the ring and finishes a long look-ahead over the next turns.
  (Before this, time-mapped voices were never read ahead and started silent: 4 starved
  renders and every sample of a speed-1.5 AAC clip differing from the offline render.)
- **Stops.** Pause stops where the listener is (the clock's position at the moment of the
  command; until the device has presented a seek's first buffer, the seek target, not
  the old line); the end of a run (composition end, play-around end, a scrub grain)
  stops once the queued audio has played out.
- **Loop.** Chunks end at the loop end and the next starts at the loop start: sample-exact
  wraps. A run from a playhead before the loop plays straight into it and then wraps (as
  legacy `engine.ts`); a run from past the loop end plays to the composition end. The
  loop range travels with the audio: a segment before the loop start ends exactly at it
  (so even a loop shorter than a chunk is entered on its first sample), the transport
  marks each segment that starts inside the loop and wraps at its end, the ring chunk (and the converter's media run) carries
  that range, and the device callback publishes the range of the chunk it hears. So a
  clock sample carries the loop range only while its line is inside the loop; before the
  loop, past it, and when stopped it carries none (contract 6 invariant 1 wraps any line
  that carries a range, so a range on the run before the loop would put the listener
  inside it). The published range is the sample-aligned one the audio wraps at. Samples
  keep their epoch across wraps (invariant 2); entering the loop or a changed range
  publishes a new sample at once. (Before, every sample carried the transport's range:
  from 1.0 s with a loop [2.0, 2.5) every consumer derived a position inside the loop
  while 1.0 s played, the line republished on every callback, and a pause stopped inside
  the loop.)
- **Audio scrub** (contract 6 open question 2): rides on `Shuttling` samples. A scrub seek
  or frame step while paused, with audio scrub on, plays a grain of one composition frame
  or 60 ms, whichever is longer, with 4 ms fades.
- **Device changes** (open question 3): playback continues on the new device with a new
  epoch, re-anchored from what is heard. The default output's id and latency are polled
  once a second (the latency converted at the new device's nominal rate); a host that
  owns the device passes its own watch. This choice awaits Jude's sign-off. Known limit:
  `cpal` opens the default output through an AUHAL that follows the system default and
  resamples internally when the new device's rate differs from the stream's; that
  converter's latency is not in the clock (the stream keeps its original rate).
- **Device rate.** When the device does not run at the project rate, rubato converts the
  master. One epoch is one continuous stream through the converter: a loop wrap continues
  it (the device hears the band-limited concatenation of the passes, as a device at the
  project rate hears them back to back), input reaches the filter in whole 256-frame
  chunks (a partial chunk would insert silence), and the end of a run feeds silence until
  the filter's tail is out, then queues every frame up to the run's end. Device frame j
  sits at stream position (j + delay + 1) × p / d − 256 project frames (rubato's exact
  phase: its reported delay is rounded down, so the stream starts 0.87 project samples
  in at 48 → 44.1 kHz); chunk tags follow that rational map and split where media runs
  change, so the clock stays exact in ticks through wraps. (Before, the converter reset
  at every wrap and dropped its pending frames and filter delay: 297 project samples
  missing from every pass of a 0.25 s loop, and the end of each run cut.)
- **Channels.** A mono device hears the bus folded by the layout rules (stereo as
  ½(L + R)); devices with more channels than the bus get the bus on their first channels.
- **Live edits (retune).** A new document revision or monitor solo while playing glides
  from what played into the new mix over 12 ms (today's `RETUNE_GLIDE`), counted in
  rendered samples so a loop wrap inside the glide continues it. The mixer renders the
  outgoing mix next to the new one and fades linearly, so a level change ramps exactly
  as today's `linearRampToValueAtTime` glide and a structural change (a clip added,
  moved, muted) crossfades instead of clicking. Overlapping glides (a fast fader drag)
  chain, at most four; the DC slope stays within three glide increments a sample. A
  seek or a new run ends glides; paused edits and exports swap at once. An edit is
  heard once the queued audio has played: at most `ahead_ms` (80 ms) plus the device
  latency.
- **Rates.** Phase 2 plays at 1× only; `Play` at another rate (J/K/L) is refused with
  `Unsupported` until Phase 3b.

The simulated device's timeline is synthetic (host times from its frame count), so on a
loaded machine it waits up to 40 ms for the render thread to queue a buffer
(`DeviceSide::queued_chunks`) instead of reading an underrun a real-time device would not
see; an unoptimized build under four building lanes otherwise starved the ring and failed
clock assertions with 160 ms errors that were the underrun, not the clock.

Measured with a simulated device (`tests/playback.rs`): a gain edit while playing ramps
over 576 samples at 48 kHz with no step larger than one glide increment and no new epoch.
The published line predicts the presented media to 0.3 µs on a nominal device; at ±150 ppm
device drift the final line alone is off by 400 µs after 2.7 s, while the line as
published (re-anchored every 100 ms) stays within 29 µs at 300 ppm. Playback from a seek
equals the offline render, also for an AAC source at speed 1.5, at speed 0.75, and under
a time remap (0 starved renders, 0 of 80,000 samples differing). On a 44.1 kHz device
under a 48 kHz project, a 0.25 s loop (a ramp source whose value is its position, read
after dividing out the converter's DC gain of +3.7 ppm): 13 passes begin 11,025 device
frames apart within 0.0006 frames, each pass ends where the next begins within
0.0005 project samples, and the clock line names the heard position within 0.06 samples;
the last 0.5 s of a run lasts exactly 22,050 device frames and reaches the composition's
end. A mono device hears ½(L + R); a pause sent during a seek's pre-roll stops on the
seek target. Pause, seek, scrub grains, and a device change behave. A run from 1.0 s
into a loop [2.0, 2.5): every presented buffer is within 0.7 µs of the published line at
48 kHz (19.4 µs through the 44.1 kHz converter, away from the smeared wraps), with
23 samples in 2.3 s (refreshes plus the loop entry, not one a callback); a pause before
the loop stops where the listener is.

Measured on the real output (`examples/device_probe`, 5 s, 2026-10-13; the default output
was Bluetooth AirPods Pro at 48 kHz, 512-frame buffers): device plus stream latency 7,680
frames (160 ms) folded into the clock, safety offset 0; 9.6 clock samples a second;
consecutive samples re-anchor within 0.33 µs (p50) and 0.67 µs (max) of the previous line;
the device clock runs +0.1 ppm against the host clock; no underruns or starved renders.
The first playing sample presented 176.6 ms after `Play` (160 ms of it the Bluetooth
latency). cpal's own estimate would have put the clock 10.7 ms (the I/O buffer) late.

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

Measured 2026-10-13 with other lanes building (numbers vary about 1.5× with load):

| Voices | Cost per voice-second of audio | Real time, voices as listed |
|---|---|---|
| 48 kHz sources (copy), 16 voices | 0.18 ms | 356× |
| 44.1 kHz sources (exact-phase sinc, NEON), 16 voices | 3.8 ms | 16× |
| Keyframed gain (control points), 16 voices | 3.1 ms | 20× |
| Variable speed (free-position sinc), 4 voices | 15.4 ms | 16× |
| Constant speed 1.5, 4 voices | 11.7 ms | 21× |

Read-ahead bounds of time-mapped voices (`Mixer::source_spans` over 20 s, memo empty;
2026-10-14, other lanes building): 25.2 ms per second of audio for the 4 variable-speed
voices (6.3 ms per voice-second, about 0.5 ms per 4,096-sample group) and 8.4 ms for the 4
constant-speed voices; linear voices cost nothing measurable. The same run mixed the
variable-speed voices at 15.8 ms and the constant-speed ones at 11.6 ms per
voice-second.

## Legacy parity

`tools/legacy-mix.ts` captures `PM.Audio.renderOffline(0, dur)` (the mix every export with
audio takes; `exporter.ts` then writes it as 16-bit WAV for `render-encoder.ts` to mux as
PCM or AAC) from the built legacy app for the corpus audio project, audio projects built
for this lane (clicks at 30, 29.97, and from 44.1 kHz; gain keyframes, fades, master;
precomposition nesting; solo; mute and groups; layouts and rates; mono sources; embedded
audio), and a real project given with `--real <file.pmv>` or `PM_AUDIO_REAL_PROJECT`
(kept outside the repository; its media is extracted under the corpus root), into `~/Library/Application Support/Powermove Engine Corpus/audio-<stamp>-<commit>/`.
`tests/legacy_parity.rs` renders the same loaded documents with pm-audio and compares
against the float mix (before today's 16-bit step). It fails on a length other than
round(duration × sr), a click more than one sample off its frame, or a residual above the
project's ceiling (−85 dBFS; −75 dBFS for keyframed levels; −65 dBFS for real projects
with in-points rounded as today's). Projects dominated by an allowed difference are
guarded where it does not reach, at −85 dBFS: `clicks-44k` below 15 kHz (a 1,023-tap
low-pass on both mixes), `mute-groups` outside the frames today's envelope ramps the
switches over, and `layouts` rendered with the mono layer at today's √½ center fold.
`PM_AUDIO_PARITY_DETAIL=1` prints the worst 100 ms windows and writes `ours.wav` next to
`legacy.wav`.

Measured 2026-10-13, re-run 2026-10-14 after the read-ahead and converter fixes with the
same results (captures `audio-20261014-045249-158f1223` and the `--skip-real` recapture
with the `mono` project; residual against the legacy float mix):

| Project | Samples, ours = round | Residual rms | Clicks from frame start (ours / today) | What differs |
|---|---|---|---|---|
| corpus-media-audio | 96,000 | −93.6 dBFS | | decoder rounding (AAC, MP3, Opus) |
| clicks-30 | 96,000 | −240 (identical) | 0,0,0,0,0,0 / same | |
| clicks-2997 | 105,600 (today 105,601) | −240 (identical) | 0,0,0,0 / same | 6 |
| clicks-44k | 72,000 | −58.0 dBFS (−121.9 below 15 kHz) | 0,0,0 / same | 11 |
| gain-fades | 144,000 | −81.9 dBFS | | 5 |
| precomp-nesting | 144,000 | −88.0 dBFS | | 5 |
| solo | 96,000 | −240 (identical) | | |
| mute-groups | 96,000 | −31.6 dBFS (identical outside the switch frames) | | 10 |
| layouts | 120,000 | −20.1 dBFS (−96.6 with today's mono fold) | | 12 |
| mono (7 sources) | 168,000 | −99.7 dBFS | | |
| embedded | 96,000 | −110.9 dBFS | | |
| real project (87.4 s) | 4,195,200 | −42.7 dBFS; −71.6 with in-points rounded as today's | | 9, 11 |

Every project passes the section 25 audio bar (clicks on their frame's sample, length
round(duration × sr)). The real project's mix (one 44.1 kHz MP3 in three clips with a
keyframed fade) differs only by allowed differences 9 and 11: with whole-sample in-points
the residual is −94 dB at 1 kHz and rises only toward Chromium's roll-off above 14 kHz.

```sh
bun install && bun run --cwd apps/desktop build
~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-audio/tools/legacy-mix.ts [--skip-real]
cargo test --release -p pm-audio --test legacy_parity -- --nocapture   # PM_AUDIO_PARITY=<capture dir> picks one
```

## Tests

```sh
source engine/env.sh
cargo test -p pm-media --test audio_decode
cargo test -p pm-audio -- --test-threads=4
cargo run --release -p pm-audio --example mix_bench
pm-electron-lock cargo run --release -p pm-audio --example device_probe   # opens the output device, silent
```
