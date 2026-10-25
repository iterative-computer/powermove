# pm-audio: decode, mix, playback clock, export audio

Plan sections 15 and 16, contract 6 (the playback clock), section 25 (audio bars).
Phase 2 tracer bullet: the device clock with its latency as the master clock,
streaming decode, mixing, playback, scrubbing, and the offline render export uses.
Phase 3b: pitch preserved through Signalsmith Stretch (our binding, `pm-stretch`) or
following speed per clip, conformed frame rates, every channel layout, J/K/L at 1× and
2× forward and reverse, play around, audio scrub, and the engine's meters, solo, and
waveforms for today's audio panels.

| Module | What it does |
|---|---|
| `pm_media::audio` | FFmpeg audio decode to program samples: priming removed, sample-exact positions, sample-accurate seeks, speaker layouts, no length limit |
| `source` | Decoded program samples in 65,536-sample blocks under a byte budget, decoded ahead by worker threads |
| `kernel` | Band-limited interpolation (rubato's windowed-sinc design) for sample-rate conversion, speed, and remaps |
| `graph` | Which clips sound and how level and source time follow the document (the legacy `audioLayers` and `gainAt`) |
| `conform` | A clip's `pitch` choice and a conformed frame rate (Interpret Footage `frameRate`) |
| `stretch` | Pitch-preserving voices: time-stretch runs with deterministic pre-roll (`pm-stretch`, Signalsmith Stretch) |
| `shuttle` | The 2× J/K/L stretch of the master, with its media bookkeeping |
| `mixer` | Voices → per-layer strips → master in f32 at `audioSampleRate`, a pure function of the sample range |
| `layout` | Down-mix and up-mix matrices |
| `offline` | Export render: PCM float, 24-bit, 16-bit with TPDF dither; streaming WAV/RF64 writer |
| `meter`, `waveform` | Level taps (peak, RMS) per strip and master; legacy peak envelopes and a min/max pyramid |
| `transport` | Play (J/K/L at 1× and 2×, forward and reverse), pause, seek, loop, frame step, play around, audio scrub |
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
  least the lead (the pre-roll, and at least the codec delay) before its target and
  counts from there, so it costs the same
  anywhere in the file. The scan runs once per source, only as far as seeks need, and
  the anchors are a function of the file alone. Where no packet length is known (ALAC
  in Matroska has no parser), a seek decodes from the start, and a seek forward
  continues the running decode, so reading in order costs the stream once.
- **Anchored first frames.** A fresh decoder drops its codec delay (`initial_padding`,
  Matroska's `CodecDelay`: Opus 312, AC-3 256, MP2 481, HE-AAC 4,224) from the head of
  its output and moves the frame's timestamp by as much. When the delay is shorter than
  the anchor packet, the first frame is the anchor packet's last samples; HE-AAC's runs
  past its 2,048-sample packets, so its first frame is the third packet's and starts
  4,224 samples into the anchor (checked against the moved timestamp). Until this was
  handled, every anchored seek on HE-AAC (M4A and Matroska) landed one frame early
  (measured 0.18 off a tone; now within 2.8e-4 on both channels, SBR's noise). The codec
  delay (4,224) is longer than AAC's pre-roll (4,096), so the anchor is the last one at
  least the longer of the two before the target: with the pre-roll alone, 128 of every
  4,096 targets put their first sample after the target (36 of 1,171 targets swept by
  7 from 958,464 on the `afconvert` file, for example 960,389) and decoded the stream
  from the start, 475 packets instead of 5
  (`he_aac_seeks_inside_the_codec_delay_window`: every target now 3 to 5 packets).
- **Seeks that land after their target** (a demuxer index coarser than its packets, a
  delay the lead does not cover) start one anchor (or, placed by timestamps, one lead)
  earlier, up to twice, before decoding from the start. They depend on the target, not
  the stream, so they never count against its anchors
  (`a_seek_that_lands_after_its_target_starts_an_anchor_earlier`).
- **Anchor failures.** An anchor that cannot be found again, or whose first frame does
  not fit it, comes from the demuxer or the codec and usually repeats for the stream's
  other anchors. Each such seek decodes from the start; after 3 in a row the stream
  counts from the start for good, and reads in order continue that decode
  (`failed_anchors_fall_back_after_a_few`: 30 blocks with every anchor moved off its
  packet decode the stream once plus 6 blocks). A seek that places its anchor resets the
  count, so one stray anchor costs one decode from the start and nothing after
  (`a_stray_failed_anchor_does_not_stick`). A read error in the demux-only scan ends the
  scan there; seeks resolve against the anchors found before it, and the decode reports
  the error if it reaches that spot.
- **HE-AAC v2 (Parametric Stereo).** A fresh decoder applies no stereo until a frame
  carries a PS header: FFmpeg copies the mono core to both channels until then, and once
  PS runs it changes the left channel too, so both channels are off by about the
  signal's level. Apple's encoder repeats the header about every 1.8 s. Measured on
  `afconvert -d aacp` encodes (32, 44.1 and 48 kHz; stereo, swept, gated, mono and noise
  content; 24 to 48 kbit/s; a fresh start every 0.25 s over 60 s), a fresh decoder was
  off by up to 0.3 on a 0.3 tone for as long as 1.86 s (1.66 s at 32 kHz), then within
  the sequential decode's SBR noise (4e-5 to 6e-5 on tones). With AAC's 4,096-sample
  pre-roll nearly every 65,536-sample block of such a source was off by 0.3 for its
  first 1.2 to 1.9 s. Streams FFmpeg reports as `HE-AACv2` (the codec configuration, or
  the probe's decode for implicit signalling in ADTS) pre-roll 4 s
  (`PS_PREROLL_SECONDS`, about twice the longest wait measured), and every block, read
  in any order from a fresh decoder, is within 6.5e-5 of the sequential decode on both
  channels in M4A, Matroska, MP4 and ADTS at all three rates
  (`he_aac_v2_blocks_match_the_sequential_decode`; `pm-audio/tests/sources.rs`
  `he_aac_v2_blocks_match_the_one_pass_decode`: 88 cold blocks in reverse and strided
  order through `SourceCache`, within 5.1e-5, at most 4.5 ms a block). A block costs
  about 4 s of decode more (95 to 128 packets at 32 to 48 kHz instead of 4 to 6), which
  FFmpeg decodes at over 1,000× real time. An encoder that repeats the header less often
  than every 4 s would still differ for the rest of its wait; only Apple's encoder was
  available to measure (our LGPL build has no PS encoder).
- **Seeks.** `seek(n)` starts a per-codec pre-roll earlier (AAC 4096, HE-AAC v2 4 s,
  MP3 8 × 1152, Opus 500 ms, Vorbis 8192, AC-3 3072, lossless 0; at least the codec
  delay) from a freshly opened decoder and discards
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
7.2e-3 for AAC with noise substitution (its noise generator runs across frames),
2.8e-4 for HE-AAC and 3.7e-5 for HE-AAC v2 (from macOS `afconvert`, in M4A and remuxed to
Matroska), every channel compared, and the same for 65,536-sample blocks read in order
and in reverse. A seek decodes 3 to 32 packets wherever it lands (HE-AAC v2: 95 to 97,
its 4 s pre-roll; ALAC: from the start). Lossless codecs have no
pre-roll, so a seek up to 2 s ahead of the running decode decodes forward instead (FLAC
and PCM: 42 packets for the seek to 1 s + 123 after the one at 0.5 s), and blocks read in
order cost the stream once plus a pre-roll per block.

Long coarse sources (`pm-audio/tests/sources.rs`
`long_coarse_timebase_sources_decode_in_linear_time`, 10 minutes, release build, the
lane's `SourceCache`; the earlier code decoded every block from sample 0). Measured
2026-10-14 on Jude's Mac under `nice -n 15` with other lanes building (load average
about 8); wall times move with load (an earlier run under heavier load measured
3.2 to 4.0 ms per Opus block and 1.7 s in total), the packet counts do not:

| Source | Block 0 | Block 100 | Block 200 | Block 400 | All 440 in order | Cold block 437 |
| --- | --- | --- | --- | --- | --- | --- |
| WebM/Opus, before | 1.5 ms | 92 ms | 185 ms | 365 ms | | |
| WebM/Opus, now | 2.1 ms | 2.8 ms | 3.1 ms | 2.7 ms | 1.27 s | 13 ms (96 packets) |
| MKV/FLAC, before | | | | 206 ms | | |
| MKV/FLAC, now | 1.4 ms | 1.4 ms | 1.3 ms | 1.3 ms | 0.60 s | 16 ms (64 packets) |

The "before" rows are the review's measurement of the earlier code on the same kind of
file (its whole-file cost grew as N²/2 blocks, about 1.5 minutes for this one). Every
block in order now decodes 96 to 99 packets (Opus) or 64 (FLAC), the same at the end
as at the start. A cold block near the end includes the first demux-only scan of the
file (10 to 15 ms for 10 minutes); later seeks reuse the anchors.

FFmpeg's HE-AAC decoder runs 962 samples (481 core samples) later than Apple's: the
`afconvert` HE-AAC test file decodes through pm-media and through FFmpeg's own command
line 962 samples behind its source, and through `afconvert` on the sample. The priming
(4,224, from the edit list or `CodecDelay`) is honored; the extra delay is the decoder's.
It is not compensated here; whether legacy (Chromium's decode) shows the same offset is
unmeasured. A legacy parity capture with HE-AAC and HE-AAC v2 sources would settle both this
offset and the v2 pre-roll against Chromium's one-pass decode.

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
mixes unknown layouts "discretely", dropping 7.1's center and surrounds). Streams whose
channels have no speaker meaning (ambisonics, unknown orders) fold as Web Audio folds
every stream, by channel count (1, 2, 4, 6 by its speaker rules, other counts keep the
first two channels as left and right); a channel without a meaning inside a named layout
folds as a center. Every one of the 40 layouts our FFmpeg build names (mono to 22.2,
binaural, downmix) decodes with its speakers and folds with each channel heard on its
side at the matrix's gain and nothing but LFE dropped (`tests/layouts.rs`). A voice keeps
its source's channels up to its strip, so a wider bus receives them by name.

Automation: gain is applied per sample. Animated inputs (levels, `on` switches, fades,
remaps, speed) are evaluated through `pm_eval` at control points every 16 samples
(3 kHz at 48 kHz), aligned to absolute sample indexes, and interpolated linearly between
them; fades are computed per sample from the clip's local time. Linear time maps place
every sample exactly. Today's engine schedules gain ramps between points one composition
frame apart.

**Differences from the legacy engine.** Decisions AUDIO-5 and AUDIO-6 (2026-10-22)
require today's mix sample for sample; the earlier list of "allowed differences" no
longer stands. What remains, with its status:

1. Speed and time remapping are followed (appendix A item 10: today ignores speed, so
   audio drifts from retimed video, and remapped clips are silent). Kept by decision
   AUDIO-4 (a bug fix, on the beta list). Such voices (a speed other than 1, a remap or
   `sourceTime` anywhere on the path: `VoicePlan::retimed`) play through the engine's
   own readers and stretch, not today's model.

Matched since 2026-10-22: a negative layer start is clamped to 0 as `audioLayers` does
(the clip plays from 0, shifted, with its local time from there); an animated `trim`
on an audio layer is read once where the range or the run starts (`plan()`);
a video's embedded trim is frozen at the audio index build playhead, with the
original video as its evaluation scope; a conformed frame rate (Interpret Footage) is ignored, as today plays a
video's soundtrack at its own rate and pitch. A precomposition's animated `trim`:
today's `audioLayers` reads it with the precomposition at `PM.time`, the playhead
when it last built its audio index (captures `precomp-trim` with the playhead at 0 and
`precomp-trim-playhead` at 1.2 s differ by −16 dBFS from each other). The mixer reads
it at `Mixer::set_today_playhead` when given, else at the range's (export) or the
run's start; with the capture's playhead both projects are bit-identical offline.

Playback follows today's index lifetime (review round 9; before it, every run start
re-read the trim). Today caches `audioLayers` until a `layers` or `project` event or a
`retune` invalidates it, and rebuilds it at the next `audioLayers()` call: while
playing, the edit's own `sync(PM.time)`; paused, the next Play's `sync`, at its start.
Seeks, loop wraps and later Plays keep it. The producer keeps that playhead
(`RenderThread::today_index`): each new project invalidates it (set to the playhead
heard then when playing, else cleared), a run start with none sets it to the run's
start, and seeks, wraps and later runs keep it
(`a_precomposition_animated_trim_reads_at_todays_index_build`). Embedded video trims follow the same lifetime
(`embedded_video_animated_trim_reads_at_todays_index_build`). Exports carry the
app's cached index playhead in `ExportSettings.audioIndexPlayhead` (ticks),
independently of the export range. Headless clients may omit it to use the range
start. The first-open warm-up and event-specific invalidation gaps remain below.

**Beta list (bug fixes against today, orchestrator decisions):**

- AUDIO-4: audio follows clip speed and time remapping (today plays it at 1×, out of
  sync with the retimed picture).
- LAYOUT-1: files of 3, 5, 7, 8 or more channels fold to stereo by their layout
  (BS.775) instead of keeping only their first two channels; channels without a
  speaker meaning sum with equal power. 1, 2, 4 and 6 channel folds, the wide-bus
  up-mix, and mono beside a quad at −6 dB stay today's.
- LONG-1 (approved, beta list): the engine plays long files through bounded
  sample blocks; exceeding today's decode limit never refuses an engine stream.

| LONG-1, same file | Today | Engine |
|---|---|---|
| Decoded buffer fits 128 MiB | Plays the decoded buffer | Plays; inactive sample blocks obey the same 128 MiB budget |
| Decoded buffer exceeds 128 MiB (about 5.8 min stereo at 48 kHz), any codec including MP3, WAV and FLAC | Silence and toast: “This audio file is too long to decode safely” | Plays through the bounded block cache; other inactive samples are evicted |


The covered cases below match today: automation stepping and ramps, the
decode resampler, in-points and starts between samples, switches, channel folds and
the wide bus, the playback alignment, and MP3, FLAC and PCM decoding.

## Today's export model (`today`, decisions AUDIO-5 and AUDIO-6)

A voice whose time map is linear (no speed, remap, or conformed rate on its path, no
negative start, a static trim) plays through `pm_audio::today`, a port of the chain
today's exporter builds in an `OfflineAudioContext` (`legacy/core/audio.ts`
`renderOffline`), with the arithmetic of the Chromium 152 sources Electron 44 ships:

- **Decode resampling.** `decodeAudioData` resamples a file not at the context rate
  with Blink's `SincResampler` (`AudioBus::CreateBySampleRateConverting`): 32 taps of
  a Blackman-windowed sinc at 0.9 of the lower Nyquist, 32 kernel offsets blended
  linearly, a running `f64` source index wrapped every 512 frames, the scalar
  summation order, `floor(frames / ratio)` frames. `Conversion` keeps the running
  index at every block start, so any frame is computed exactly without resampling
  from the file's start; the graph prepares it for the project rate and for 48 kHz.
- **The source node.** `start(at, offset, duration)`: the offset rounded to a frame
  (`TimeToSampleFrame`, at 1024× first), the start rounded up, a start between frames
  played by linear interpolation in `f64` from its fraction, the duration counted in
  128-frame render quanta.
- **The gain envelope.** `envelopePoints`: the clip's start and end, points one
  composition frame apart (at least 30 a second) when any level input is a channel,
  keyframes, fade corners; `gainAt` at each as `f32`; `setValueAtTime` then
  `linearRampToValueAtTime`, each frame `v1 + x × (v2 − v1)` with Chromium's `f32`
  `x`, from the event whose time rounded up is at or before the frame.
- **The bus.** Today's master takes the widest connected clip's channel count from
  the start of the render until a quantum after that clip's source node finishes
  (measured: the `wide-*` captures), up-mixes every clip into it by Web Audio's
  channel-count rules (`AudioBus::SumFromByUpMixing`), applies the master there and
  folds to stereo at the destination (`SumFromByDownMixing`, with the fused
  multiply-adds Chromium's NEON `Vsma` compiles to). So beside a 5.1 file, mono plays
  through the center at −3 dB; beside a quad, at −6 dB (today's, kept). Web Audio
  folds every other count discretely, keeping only the first two channels, so a 7.1
  file loses its center and surrounds today. Decision LAYOUT-1 (a bug fix, on the beta
  list): those buses fold by the widest clip's layout with ITU-R BS.775 / SMPTE
  coefficients (`today::stereo_fold`, `layout::matrix`), and channels without a
  speaker meaning sum into both sides with equal-power weights (1/√n). The up-mix
  into such a bus stays Web Audio's (discrete: mono beside 7.1 lands in its front
  left, as today).
- **Ranges.** An export's context starts at its range (`Mixer::set_today_range`). In
  playback, each producer run (Play, a seek, a loop wrap) starts today's model at the
  playhead, as today's `sync(time, true)` schedules every clip and envelope from it
  (`Mixer::set_today_playback`; clips connect a second ahead, `LIVE_LOOKAHEAD`).
  Decode reads are bounded independently of the run, with two frames of margin, so
  ahead-of-time decoding serves any start.

The order in which today's bus sums its clips is a pointer hash set's
(`AudioSummingJunction::outputs_`). Where three or more clips feed one bus channel at
once, today's own captures differ from each other by an ulp: two captures of
`corpus-media-audio` measure −149.2 dBFS apart. That spread is the parity ceiling for
such projects; the engine sums in clip order.

### Decoders

Chromium 152 does not decode every format with FFmpeg (`media/filters/
audio_file_reader.cc`): MP3 and FLAC go to Symphonia (`kSymphoniaMp3Decoding`,
`kSymphoniaAudioDecoding`, on by default on macOS); AAC, Opus and Vorbis to the FFmpeg
fork Chromium ships (revision `2b68d2b`; Opus through its libopus wrapper, float
output). Decoded floats are clipped to [−1, 1] (NaN to 0).

| Codec | Engine | Against today's `decodeAudioData` |
|---|---|---|
| PCM, FLAC | our FFmpeg | identical |
| MP3 | Symphonia 0.6.0 (Chromium's pin, `src/mp3.rs`) | identical (a fixture and a real 87 s file, all 127 blocks) |
| AAC (M4A, MP4, MOV, ADTS), Opus (Ogg, WebM), Vorbis | Electron's own `libffmpeg.dylib` (`src/chromium.rs`) | identical (9 files, SHA-256 of every sample) |

`src/chromium.rs` uses the very library today runs. The Electron Framework links it,
so it is already mapped into the engine's utility process in development
(`node_modules/.../Electron.app`) and in the packaged app
(`Powermove.app/Contents/Frameworks/Electron Framework.framework/Versions/A/Libraries/
libffmpeg.dylib`): the decoder finds it among the loaded images (suffix
`/Libraries/libffmpeg.dylib`; `POWERMOVE_CHROMIUM_FFMPEG` overrides), opens it with
`RTLD_NOW | RTLD_LOCAL` and resolves every function by `dlsym` on that handle, so no
symbol binds by name and our LGPL FFmpeg never clashes with it. It is used only when
its libavcodec/libavformat/libavutil majors equal our bindings' (63/63/61 today), so
our struct layouts describe its objects. Electron's libffmpeg contains Chromium's
libopus; no separate libopus revision is needed. Outside Electron (the CLI, a test
without the checkout's Electron) the source falls back to our FFmpeg (−93 dBFS for AAC
and Opus), and says so: `pm-cli render` prints a warning naming the codecs and reports
`"audioDecoder": "powermove-ffmpeg"` with the notice in its JSON (`"today"` when
`POWERMOVE_CHROMIUM_FFMPEG` points it at Electron's library).

Chromium's rules: the first audio stream; other streams' packets skipped; a custom
I/O context (Chromium's `InMemoryUrlProtocol`; here it reads the file, so the demuxer
sees the same bytes and sizes without the file held in memory); float output requested;
FFmpeg's skip side data dropping priming and end padding (the fork's own handling, what
today hears); the decoded length is today's buffer length, which also sets the clip's
audible end (packet durations less skip side data do not reproduce it: measured off by
152 to 696 frames on the test files).

**Random access uses one sequential Chromium decode per file.** AAC's SBR
noise index and Opus's CELT prediction carry state across the stream, so a
packet seek with pre-roll cannot reproduce today's samples exactly. The shared
`chromium::Whole` now spools those samples to an anonymous temporary PCM file.
It keeps no whole-file sample allocation in RAM. Each block read seeks in that
spool; the source block cache retains decoded samples under today's **128 MiB**
`audio` budget, evicting least-recently-used blocks which no render has pinned.
In-flight blocks may briefly exceed the budget, as today's active voices can.
The host lease's explicit memory-pressure hook releases inactive sample blocks;
live playback protects pinned blocks and blocks read, checked, or asked for
again (look-ahead and prepared starts) within the last 1.5 s (round 11).

Opening a restored source only probes metadata. Its sequential decode begins
on the first sample read, with at most two background decodes unless a reader
is waiting. The source registry holds weak references: dropping the last mix
or pending read closes its decoder and removes its anonymous spool. A waiting
read holds the source until it completes. Temporary-file failures are named
decode errors. Disk space and file I/O replace the former unbounded resident
whole buffers; a five-minute stereo file uses about 115 MB of temporary disk.

`tests/chromium.rs` checks bit-identical random reads, lazy opens, and the total
memory across ten independent 45-second AAC files (more than 128 MiB decoded),
including LRU eviction, memory pressure with a pinned block, and last-reader
cleanup. `SourceCache::stats().bytes` accounts for resident sample blocks;
`chromium::resident_bytes()` is zero because the shared PCM is on disk.

Files today cannot play keep Powermove's random-access decoder: today refuses a decoded
buffer past its `audio` budget (`PM.Memory`, 128 MiB: "too long to decode safely"), so
such a file plays nothing today and has no samples to match. `chromium::plays_today`
estimates today's buffer (container frames at the lower of the file's rate and
44.1 kHz, × channels × 4) so a file today plays is never sent to the fallback; those
over it decode through `pm_media::audio` with block-cache memory (the plan's "no length
limit"; samples about −93 dBFS from what Chromium's FFmpeg would give; LONG-1 on the
approved beta list above). A stream without a container length can also spool without a resident-memory length limit.

Playback opens a source with the container's length until the whole decode ends; the
next mix build takes today's length (up to about 700 frames at a clip's end in
between). An export's cache waits for it (`SourceCache::set_blocking_lengths`), on the
job's thread. Verified: `tests/chromium.rs` (fixtures, last block first; the
whole-decode test above; a seven-minute file on the random-access decoder), outside the
repository odd lengths (ADTS mono 2.71 s, M4A 44.1 kHz 3.317 s, MOV 6.1 s, Ogg and
WebM Opus) against captures from the app, and the export encoding parity e2e with
`PM_PARITY_FIXTURE=tone.m4a` and `tone.opus` in the app (all four formats identical).

Today's summing bus copies the first playing clip into a silent bus
(`AudioChannel::SumFrom`), so its signed zeros survive: the mixer starts its sums at
−0, mixes zero-gain stretches of playing clips (±0 by the sample's sign), and makes
+0 the frames where no clip's source node plays (`Mixer::signed_zeros`). The parity
test compares bits for every identical project.

### Measured (2026-10-22, `tests/legacy_parity.rs`; rerun on `c035ad187`, review round 9)

Round 9 rerun, with Electron 44's `libffmpeg` (`POWERMOVE_CHROMIUM_FFMPEG` set to an
absolute path; a relative one does not resolve from the test's directory and silently
falls back): capture `audio-20261023-035108-f3369e00`, 22 of 24 projects −240 dBFS with
bit-equal samples, corpus-media-audio −148.3 (ceiling −145), wide-71 only inside its
LAYOUT-1 window; the older capture with the real project −240. `tests/chromium.rs` 4 of
4.

Capture `audio-20261023-015202-824ea344` (built legacy app, same machine) and the
earlier capture with the real project, residual rms against today's float mix:

| Project | Residual |
|---|---|
| clicks-30, clicks-2997, clicks-44k, gain-fades, precomp-nesting, solo, mute-groups | −240 (identical) |
| app-channels, eased-24, eased-60, fractional, rates, layouts, wide-51 | −240 (identical) |
| real project (87.4 s, a 44.1 kHz MP3 in three clips, a keyframed fade) | −240 (identical; was −71.7) |
| wide-quad | −168.6 (summation order, below today's own −149) |
| wide-71 | −240 outside its 7.1 clip; inside, LAYOUT-1's fold by design |
| mono (seven sources, two lossy), embedded (AAC soundtrack) | −240 (identical, with today's FFmpeg) |
| corpus-media-audio (six clips at once) | −149.3 (today's own summation order; two captures of today: −149.2) |

Export encoding parity e2e (`e2e/export-encoding-parity.spec.ts`, rebuilt app):
`basic` and `automation` identical in all four formats (decoded audio, the WAV and f32
captures, AAC and Opus stream bytes). `advanced` differs only by its nested clip at
speed 1.2 (AUDIO-4). The nested-composition silence (a precomposition's `trim`
channel read as a number, NaN through the mix) is fixed with a regression test
(`tests/mixer.rs`).

Export compatibility (2026-10-23): `Mixer::new_legacy_export` now reproduces
the offline exporter's source timing even for speed 1.2. Audio clip speed
and sourceTime are ignored there; explicitly remapped precompositions and
embedded soundtracks are omitted as in today's audio walk. Normal playback
retains retiming. Sample-bit regressions cover nested trims, fades and gain
automation, unchanged playback speed, and remapped-precomposition omission.
The rebuilt app's strict media comparisons then matched all four formats
for basic, automation, advanced and 1080p projects, plus advanced AAC and
Opus sources. The separate WebM throughput gate is recorded in `export.md`.

### Allocations in the real-time render (glitch-safety rule)

The producer's render must not allocate once warm. `tests/render_allocations.rs`
installs a counting allocator and counts the rendering thread's heap allocations over
200 warm renders of a mix with every kind of voice (keyframed and faded 48 kHz,
a 44.1 kHz clip in a precomposition, a speed 1.5 stretch, a 5.1 clip): 316,504
(about 1,580 a render, deterministic). Nearly all come from channel evaluation in
`pm_eval` (`ev_p`'s key strings and keyframe curves, `Comp::in_root` for every
composition in `MixGraph::resolve`), the rest from the mixer's per-render views and
the stretch's. The test is a ratchet (the ceiling only goes down; target 0) until the
work lands: evaluation moves off the render (control points and envelope values
computed ahead, on the preparation side) or becomes allocation-free, and the views
become preallocated. Not done in this round.

## Pitch (`conform`, `stretch`, `pm-stretch`)

Plan section 16: "pitch preserved by default (Signalsmith Stretch, our own binding) or
following speed per clip (`pitch`); deterministic pre-roll."

**The binding.** `pm-stretch` vendors Signalsmith Stretch 1.4.0 and Signalsmith Linear
0.6.4 (both MIT, on the section 24 `cargo-deny` list; local cooperative setup
changes and upstream hashes are documented in
`engine/crates/pm-stretch/vendor/README.md`) behind a C shim that catches every C++
exception. It uses the library's portable FFT with fast math off, and seeds (and reseeds
on reset) the random engine the library uses below 0.5×, so the same calls give the same
bytes on every machine. Default preset: 120 ms analysis blocks every 30 ms.

**The clip's choice** (`d.pitch`, additive, section 6): `"follow"` lets the pitch follow
the speed (the band-limited resampler, as tape would); anything else, absent included,
preserves it. The inspector shows "Preserve pitch" on video clips with sound while the new
engine runs; `"follow"` stamps the project `requiresEngine: "native"` (engine-stamp.ts).
Audio layers keep the field through validation (it matters inside time-remapped
precompositions).

**Which voices stretch.** A time-mapped voice (speed, remaps, animated trims along its
path, a conformed rate) whose clip preserves pitch. Constant read rates within
`unity_band` of 1 (2 / interval samples: ±0.14% at 48 kHz) resample instead: there the
library does not re-analyse the previous block and its output follows the speed anyway
(measured: 997 Hz at 1.001× came out at 997.5 Hz, residual −36 dB), while the resampler
moves the pitch by at most 2.4 cents, below hearing, at −140 dB. Constant rates beyond
32× or below 1/64 resample too.

**Runs.** The voice's map is sampled at the mixer's control points (every 16 samples). A
control interval reads forward, backward, or nothing: it is silent when the voice is off
or out of span, below `HOLD_RATE` (1/64: a held remap is as silent as a held tape), or
above `CUT_RATE` (32: a remap jump is a cut). A run covers consecutive intervals reading in
one direction and starts with `output_seek` on the source at its first position, which
aligns the first output with that sample; each call is fed input up to the map's position
one output latency ahead plus the input latency, extrapolated along the run's last
interval past its end. Backward runs feed the source reversed (pitch-preserved reverse
playback inside a remap). The stretch's input is the source at the project rate through
the exact-phase reader.

**Deterministic pre-roll.** A phase vocoder's output depends on everything it heard, so
runs start only at points that are a function of the map and of where rendering starts:
a render that continues the previous one (or reads inside what it produced) continues the
run; any other render starts at the latest run start at or before it, but no earlier than
`preroll` (two analysis blocks) before the 16,384-sample `GRID` boundary at or before it,
and discards the pre-roll. The stretcher is driven in calls of exactly one analysis
interval aligned to the run's start, so its analysis positions do not depend on callers'
block sizes. Hence playback from a seek equals the offline render from the same sample
(the playback tests above pass unchanged with stretched voices, also at speed 1.5, 0.75,
and under a remap), and a render is the same in any block size. A render that played on
from an earlier start differs from a fresh start at the same sample only in the phases the
vocoder chose (log-spectral distance 0 to 1.8 dB below). A stretch in progress carries
over a live edit when the voice's map is unchanged (`VoicePlan::map_key`, a digest of every
input of the map), and a retune glide's outgoing mix reads the same run's output, so a
fader drag never restarts a stretch.

**Conformed frame rates** ("Frame-rate override keeps audio in sync"). An asset whose
`interpret` sets `frameRate`/`frameRateRational` (the formats lane's Interpret Footage
shape) shows presented frame n at n / conformed; its soundtrack follows the same map,
`conformed / native` times as fast (native: the probe's nominal rate, kept per file),
with the clip's pitch choice. 23.976 → 24 (1.001×) lies in the unity band and resamples
(+1.7 cents); 23.976 → 25 stretches.

Measured 2026-10-16 (`tests/stretch.rs`, release, this Mac under other lanes' builds):

| Speed | Pitch | Frequency (1 kHz tone) | Level | THD+N (residual) |
|---|---|---|---|---|
| 0.5 | preserve | 1000.000 Hz | −0.45 dB | −46.1 dB |
| 0.75 | preserve | 1000.000 Hz | −0.00 dB | −57.6 dB |
| 1.25 | preserve | 1000.000 Hz | −0.00 dB | −65.6 dB |
| 1.5 | preserve | 1000.000 Hz | −0.00 dB | −59.3 dB |
| 2 | preserve | 1000.000 Hz | −0.00 dB | −49.8 dB |
| 0.5 to 2 | follow | 500 to 2000 Hz exactly | 0.00 dB | −138.5 to −142.4 dB |

Placement (energy centroid of 3 kHz bursts, through the mixer): 0.5× within 0.006 ms,
0.8× within 0.32 ms, 1.6× within 0.05 ms of where the map puts them; conformed footage
(23.976 → 24 and → 25, preserve and follow) within 0.11 ms, the bed at 1000.00 Hz
(preserve) and 1042.71 Hz (follow at 25/23.976). A first render 150 s into a stretched
clip costs 3.3 ms (one grid step and the pre-roll).

Offline mixes with stretch against references (`offline_stretch_mixes_against_references`:
a 12 s music-like signal of chords with vibrato, bass, and noise percussion; log-spectral
distance of aligned 2048-sample frames, bins floored 80 dB under each frame's peak; phases
differ between methods, so waveforms are not compared):

| Speed | vs the library's one-shot stretch (L / R) | vs FFmpeg `atempo` (L / R) | one-shot vs `atempo` | fresh start at 3 s vs the run from 0 |
|---|---|---|---|---|
| 0.5 | 1.35 / 1.31 dB | 6.20 / 5.88 dB | 6.21 dB | 1.34 dB |
| 0.8 | 0.95 / 0.92 dB | 4.00 / 3.83 dB | 4.00 dB | 1.08 dB |
| 1.25 | 1.03 / 1.02 dB | 4.40 / 4.21 dB | 3.97 dB | 1.18 dB |
| 2.0 | 1.33 / 1.30 dB | 6.02 / 5.65 dB | 6.03 dB | 1.80 dB |

Our streaming mix stays within 1 to 1.4 dB of the library's best case (its one-shot
`exact()` of the whole clip: no streaming, no pre-roll, no restarts), and differs from
WSOLA (`atempo`, a different method) exactly as much as the library's own one-shot does,
so the streaming, the pre-roll, and the alignment add nothing measurable beyond the
method itself. No listening test was run; the numbers are as measured.

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
  epochs, and the render thread requests the blocks of the first 250 ms before it queues
  the first chunk. A start waits for every voice's pre-roll for at most 400 ms of wall
  time and 120 ms of device time (the A3 hold): nothing of the run is queued yet, so the
  wait is start latency, not a gap. Mid-run missing-input episodes have at most 120 ms of
  device time and end earlier when the ring would otherwise run dry under a voice that is
  sounding. Within a voice's own available budget, what plays equals the offline render
  bit for bit; see the A4 policy below for committed drops.
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
- **Audio scrub** (contract 6 open question 2, decided): rides on `Shuttling` samples. A
  scrub seek or frame step while paused, with audio scrub on, plays a grain of one
  composition frame or 60 ms, whichever is longer, with 4 ms fades. The presenter sends
  every paused scrub seek and step to the audio when the session's switch is on
  (Settings › General › Engine › Scrub audio, `PM.setAudioScrub`; off by default, as
  today's engine plays nothing while scrubbing); the viewer's line does not follow grains.
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
- **J/K/L** (section 15): `Play` at 1×, 2×, −1×, and −2× (`JKL_RATES`); faster shuttles
  (after cutover) are refused with `Unsupported`. A segment's frames are output frames
  covering `|rate|` project samples each; every ring chunk carries its rate, and the clock
  advances the media by `rate` frames per device frame (exact in ticks). Reverse runs mix
  the samples they play forward and play them backwards (reverse audio, at the pitch the
  mix has), stop after sample 0, and inside the loop wrap from its start to its last
  sample; a reverse run from past the loop end plays down into it first. Reverse renders
  each chunk on its own, so stretched voices resample there (their pitch follows speed
  while shuttling backwards; `RenderOptions::no_stretch`). 2× (both ways) runs the mix
  through one master time stretch (`shuttle`): output frame k plays stream sample 2k
  (`output_seek` at the start, input fed the latencies ahead), so each output piece is
  tagged with the media it plays and the line stays exact through loop wraps; the first
  2× output comes once the stream holds the seek length (about 0.25 s of media, mixed in
  a few milliseconds). A new rate while playing starts a new epoch from what is heard.
- **Play around** (`PlayAround { pre_roll, post_roll }`): a 1× run from `pre_roll` before
  the playhead to `post_roll` after, then the clock stops back on the playhead (the
  renderer asks for 2 s each way, the Final Cut Pro and Premiere default). Without an
  output device the presenter runs it on the host clock.

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

## The audio panels (levels, solo, waveforms)

Appendix C: "Timeline waveforms; mixer meters, solo, retune | WebAudio | Engine". Contract
6's `AudioMessage` (one session-bound `audio` event, RFC 06) carries them while "Use New
Engine" is on:

- **Meters.** While the device plays, the viewer's audio thread sends the levels of what
  is heard (`Playback::meters`, the 2048-sample peak and RMS window the legacy taps
  measure, per strip and the master) about 30 times a second, and silence once when it
  stops. While the engine runs the transport, `PM.Audio.meters` reads them
  (`engine-connection/audio.ts`), so the mixer panel's meters, ballistics, and clip
  indicators work unchanged; levels older than 250 ms read as silence.
- **Solo.** `PM.Audio.setMonitorSolo` reaches the legacy mix and the engine's
  (`Playback::set_monitor_solo`, glided like any live edit); a session that opens later
  receives it.
- **Retune.** Edits during playback glide into the new mix over 12 ms (above).
- **Waveforms.** For every asset with sound, the renderer asks the engine for its
  envelope (`peaksWanted`); a worker thread computes `peakEnvelope`'s 100 buckets a
  second from a streaming decode of any length (today's whole-file decode stops at its
  128 MiB budget, so long files get no waveform) and keeps the last 64 per file, size,
  and modification time; the answer fills `asset.peaks`, which the timeline, the assets
  panel, and `api.media.audio.drawWaveform` draw as before.

## Export (`offline`)

`offline::render(mixer, first, end, block, sink)` renders project samples `[first, end)`
(`sample_range` converts a tick range: round(ticks × sr / TICKS), so a range starting on a
sample has round(duration × sr) samples) and hands interleaved f32 to a sink, decoding
missing blocks on the calling thread. `pm_export::Pipeline::push_audio` takes that f32;
`offline::encode` and `WavWriter` produce PCM: 32-bit float, 24-bit (rounded, no dither:
its quantization sits at −144 dBFS), 16-bit with TPDF dither (±1 LSB triangular from a
counter-based generator keyed by sample index and channel, so chunked and whole renders
write the same bytes). `WavWriter` writes WAVE_FORMAT_EXTENSIBLE and switches to RF64 past
4 GiB. Its channel mask names the mix's speakers (`with_speakers`) or, given only a
channel count (`new`), FFmpeg's default layout for it (`av_channel_layout_default`, the
layout pm-media reports for unspecified sources): 0x4, 0x3, 0xB (2.1), 0x107, 0x37 (5.0),
0x3F (5.1), 0x70F, 0x63F for 1 to 8 channels, the masks our FFmpeg build's WAV muxer
writes for `-ac N`. AAC through AudioToolbox is the Export lane's encoder.

## Performance (`examples/mix_bench`, release, this Mac, 20 s renders)

Measured 2026-10-13 with other lanes building (numbers vary about 1.5× with load):

| Voices | Cost per voice-second of audio | Real time, voices as listed |
|---|---|---|
| 48 kHz sources (copy), 16 voices | 0.18 ms | 356× |
| 44.1 kHz sources (exact-phase sinc, NEON), 16 voices | 3.8 ms | 16× |
| Keyframed gain (control points), 16 voices | 3.1 ms | 20× |
| Variable speed (free-position sinc), 4 voices | 15.4 ms | 16× |
| Constant speed 1.5, 4 voices | 11.7 ms | 21× |

Pitch preserved (2026-10-16, load average 7 to 9 from other lanes), against the same
voices with `pitch: "follow"` in the same run:

| Voices (stereo, 48 kHz) | Pitch follows (free-position sinc) | Pitch preserved (stretch) |
|---|---|---|
| Variable speed 0.5 → 1, 4 voices | 15.0 ms per voice-second (16.6×) | 18.5 ms (13.5×) |
| Constant speed 1.5, 4 voices | 11.9 ms (21.0×) | 8.6 ms (29.0×) |

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

The source monitor's decoded-PCM probe,
`cargo build --manifest-path engine/Cargo.toml --release -p pm-node --example source_monitor_pcm`, renders
`<source-read-path> <output.f32le> <rate>` (the frozen browser rendition when an
asset has one) through the same isolated unity-gain source document as live
`SourceAudio`, then the real `MixGraph` and `Mixer` offline. Output is
headerless interleaved stereo little-endian float32; stderr reports the frame
count. `e2e/engine-source-media.spec.ts` compares it with Chromium's
`decodeAudioData` of the same bytes.

## Tests

```sh
source engine/env.sh
cargo test -p pm-media --test audio_decode
cargo test -p pm-stretch --release -- --nocapture       # the binding: pitch, level, alignment, determinism
cargo test -p pm-audio -- --test-threads=4
cargo test --release -p pm-audio --test stretch -- --nocapture   # the pitch and quality tables above
cargo test --release -p pm-audio --test playback -- --test-threads=1   # timing suite: one test at a time
PM_AUDIO_PLAYBACK_STRESS=1 cargo test --release -p pm-audio --test playback -- --test-threads=1
cargo test --release -p pm-audio --test layouts                  # every FFmpeg layout through the mix
cargo run --release -p pm-audio --example mix_bench
pm-electron-lock cargo run --release -p pm-audio --example device_probe   # opens the output device, silent
```

- `tests/playback.rs` adds J/K/L: `reverse_playback_plays_backwards_on_its_clock` (−1×,
  sample by sample, the line within 1.2 µs of what is heard, stops on sample 0),
  `jkl_rates_keep_pitch_and_stay_on_their_line` (2×, −2×, and −1× on 48 and 44.1 kHz
  devices: a 1 kHz bed stays at 999.9 to 1000.0 Hz, bursts land within 0.02 ms of the
  line's instant for their media), `a_rate_change_continues_from_what_is_heard` (1× → −1×
  starts on the heard sample, within 0.1 ms).
- `pm-node`: the presenter's host-clock play around (`present/clock.rs`).
- Renderer: `engine-connection/engine-connection.test.ts` (J/K/L, play around, scrub
  switch, lines at any rate), `engine-connection/audio.test.ts` (meters, solo, peaks,
  routing `PM.Audio`), the keymap tests (J/K/L ahead of the After Effects keys, declining
  while paused or on the legacy engine), `shared/engine-stamp.test.ts` (clip pitch).
- e2e, serially under the Electron lock: `e2e/native-engine-transport.spec.ts` (J/K/L at
  1× and 2× both ways with A/V sync per run, play around, audio scrub on scrubs and
  steps, engine meters, solo, and waveforms):

  ```sh
  pm-electron-lock bunx playwright test e2e/native-engine-transport.spec.ts --workers=1
  ```

  Measured 2026-10-16 (two runs, this Mac, built-in output, other lanes building): all
  three pass. Each J/K/L run, 0.7 s of page samples after the key: playhead rate (least
  squares) 2.0002 and 1.9998 at 2×, −1.0002 and −0.9997 at −1×, −1.9999 and −2.0011 at −2×,
  1.0000 at 1×; A/V sync by frame-callback timing (the audio clock's line at each timed
  frame's display instant against the frame's interval, section 15's ±10 ms): 0 of 21 to
  43 frames outside in every run, worst 2.4 ms outside (−2×). Every run played on the
  device clock (`hostClockRuns` 0). Play around from 3 s ran from 1 s past 4.2 s and
  returned to 3 s; scrub and step grains were heard (master peak above 0.05 within
  0.1 s of their position) and nothing was heard with Scrub audio off; the master meter
  read the tone while playing and silence under a monitor solo of another strip;
  `asset.peaks` came from the engine (150 buckets for the 1.5 s tone).

## Decode starvation recovery (wave 5, 2026-10-18)

A nonblocking source read must never become input to stateful DSP until every
sample has decoded. Previously `StretchRun::stream` fed a cache miss's zeroes
into Signalsmith and `render` cleared its incomplete flag at the end of that
call. The next call could report complete while its phase history still came
from missing input. The deterministic regression
`stretch::tests::missing_input_never_poison_stretch_history` fails on the old
implementation at sample 48,256 after a miss in `output_seek`: that call reports
complete but differs from the fully decoded run. A miss during later analysis
has the same defect.

`StretchRun` now suspends before `output_seek` or `process` consumes incomplete
input. It returns silence with an incomplete result, retains its valid history,
and catches up through the missing output range after decoding completes. It
preserves that history throughout a transient wait. Forward and reverse runs,
initial pre-roll, and later analysis reads are covered; every complete chunk
recovered within the wait bound is compared bit for bit in both channels with
the uninterrupted offline run. The A3 policy below describes committed drops
when the bound is exceeded.

The rest of the path follows the same rule:

- Within the wait bound, `Playback::fill` counts a missing segment and retries
  it before advancing to another segment. It queues no incomplete PCM and never
  feeds undecoded silence into the master 2x shuttle or device-rate converter.
  The A4 policy below ends source waits before the ring's refill reserve is spent
  while another voice is sounding.
  A new pre-roll or stop discards the pending segment and every stretch's
  starved request. Over the bound, only intentional per-voice silence commits.
- A starved `StretchRun` remembers the starved request's output range. Only a
  request inside that range or starting where it ended (a retry, or the
  continuation rule) resumes the run; any other request (a seek, a play around,
  a switch back from reverse or `no_stretch`) starts fresh by the pre-roll
  rules. Before review round 2 a bare `pending` flag let any later request
  continue: one render 60 s past a starve read 4.4 M source samples (a fresh
  start reads 44 k) and differed from the offline render at every sample.
  `stretch::tests::a_seek_after_a_starve_starts_fresh` (forward 60 s, just past,
  a grid step, backward; with and without `abandon_retry`) and the playback
  test `a_seek_after_a_starve_plays_as_rendered_offline` both fail on the old
  flag.
- Retune glides include outgoing voices' starvation and errors in the returned
  report and advance their envelope only after a complete render or an A3
  committed drop. A shared stretch can be retried at the same range without
  changing the envelope.
- Read-ahead bounds include zero-gain portions of a stretched voice: a fresh
  vocoder still needs that input as its pre-roll history. The readiness regression
  fills exactly the advertised spans before rendering just after a muted region.
- Copy, exact-phase and free-position readers have no persistent sample history;
  their missing reads already return incomplete. The retry at playback also
  protects the downstream converter and shuttle when those readers miss.
  Constant and animated speeds, reverse remaps, trims, preserve/follow pitch,
  conformed footage, and existing stretch determinism tests are exercised.

**Current policy: A6 below supersedes the A4 exceptions in this historical
account.** Every mid-run miss now uses the remaining-ring deadline, even
when every voice is missing; callback-sized ring growth restores per-voice
budgets, and paused source/DSP preparation avoids ordinary start holds.

**Bounded-wait policy (A4, historical; superseded by A6 below).** Each voice
owns a missing-input episode, ending when that voice renders completely or
leaves the audible mix. Its hard bound is 120 ms of device time from the
episode's start (the A3 hold). The ring's usable audio shortens it only while
it protects a voice that is sounding (A4 point 1: "so ready voices never go
silent because another voice is waiting"):

- Mid-run, with at least one heard voice rendering whole, the deadline is
  the earlier of the hard bound and the usable audio left in the ring. It can
  shorten as the reserve shrinks. "Heard" and "whole" are decided by voice id
  (`RenderReport::heard_whole` against `starved_voices`), across the current
  mix and every outgoing retune mix, on strips the monitor solo lets through.
  Review round 3 found that round 2 compared counts instead
  (`voices > starved`): `voices` counts only the current mix, while
  `starved` adds every outgoing glide's copy of the missing voice, so during
  a live edit (a fader drag or a solo toggle keeps one to four glides alive,
  and a waiting render never advances them) one missing voice beside a ready
  one looked like an all-missing mix, took the full 120 ms hold, and the
  80 ms ring ran dry under the ready voice. A voice the monitor solo
  silences no longer counts as protected either.
- With every heard voice missing, dropping silences nothing that would
  otherwise play, so the voice keeps the full hold and stays bit exact if
  its block arrives within 120 ms, as under A3. The ring can underrun for
  the rest of that hold (about 40 to 60 ms with the default ring), which
  stalls the device clock and the picture following it; dropping at
  ring-empty would sound the same and keep the clock running. That trade is
  listed for the orchestrator below.
- At a play or seek (review round 2 of A4), every missing voice keeps the full
  hold, counted from the pre-roll's device frame, until the run's first
  segment commits. Nothing of the run is queued yet, so the hold delays the
  start (the clock stays at the target) instead of opening a gap, and a
  briefly cold voice, alone or beside a ready one, starts bit exact. This is
  the base branch's A3 start behavior.
- A drop is sticky for its episode, so a deadline restored to the hard bound
  (another voice also going missing) never resumes holding a voice already
  committed as silence.

`Ring::played_frames` counts callback frames, including underruns,
independently of project rate, shuttle speed, and elapsed wall time.

- The ring exposes a producer-safe frame count from cumulative written and
  retired frame counters, with an epoch base excluding old queued audio.
  Partial chunks and old-epoch skips retire their exact remaining frames.
  No producer reads consumer-owned slots or its partial-chunk offset.
- Playback keeps two callback buffers as a refill reserve (the largest
  callback observed, at least 256 frames). It ends a source wait before
  consuming that reserve, leaving time to wake the render thread, mix, and
  feed the master converter/shuttle. With the default 80 ms ring the usable
  wait is therefore shorter than 120 ms. This prevents source stalls from
  emptying ready audio; unrelated OS scheduling stalls remain separately
  counted ring underruns.
- The fill target is the configured 80 ms, and at least half of that beyond
  the refill reserve (`fill_target`), bounded by the ring. With 512-frame
  callbacks the target is unchanged; 2,048-frame callbacks (Bluetooth or
  AirPlay sized) reserve 4,096 frames, more than the 3,840-frame default
  target, which left a transient miss no wait at all. The ring then keeps
  23 chunks (5,888 frames, 37 ms of wait). Callbacks of 4,096 frames and more
  exceed the 160 ms ring (sized at start, before callbacks are seen) and
  still leave no reserve-bounded wait; mid-run misses beside a sounding voice
  then drop at once, while starts and all-missing mixes keep the hold.
- Review round 1 of A4 ended the start pre-roll as soon as one voice was
  ready or the first callback ran, and gave an empty ring no budget. A real
  device's callbacks run continuously and advance `played_frames` through
  underruns, so every cold start or seek dropped its missing voices about one
  callback (10.7 ms at 512 frames) after starting, while the base branch held
  them bit exact for up to 120 ms. That poll (`Mixer::has_ready_source`) is
  removed; the pre-roll again waits for every voice
  (`Mixer::is_ready`), as on the base branch.
- `RenderOptions::drop_voices` names only expired voices. A later missing
  voice gets its own remaining-ring budget even while another is dropping.
  `RenderReport::waiting_voices` and `starved_voices` list each id once over
  current and outgoing retune versions (the `starved` count does not, and
  playback no longer decides anything from it); PCM, meters, and retune
  progress commit only when none still need a wait.
  Ordinary export uses `wait: true` and never drops.
- Copy readers keep every available sample. Sinc readers silence only output
  samples whose filter support includes missing input. Validity comes from
  samples actually copied into scratch, avoiding decode-arrival / eviction
  races. Re-entry reads the current project sample, without shifting the
  transport or the other voices.
- A missing stretched voice returns a whole silent segment without feeding
  undecoded PCM to Signalsmith. Its valid vocoder state survives committed
  silent segments while catching up costs no more than a fresh start's worst
  case (`stretch::catch_up_limit`: one `GRID` step plus the pre-roll, 27,904
  output samples or 0.58 s at 48 kHz; review round 3, replacing round 2's
  fixed 120 ms). The unit is render work in project output samples, not
  device time: a catch-up analyses every skipped interval, so its cost is the
  output distance whatever the shuttle speed. Recovery renders and discards
  the missing output before returning current samples: re-entry within the
  limit is bit-identical to uninterrupted DSP in both forward and reverse
  source maps. Other voices' vocoders continue normally. Ordinary incomplete
  retries do not spend this allowance.
- A drop committed before a run ever started (its first input missing) has
  no history to keep, so the next request starts fresh at its own sample.
  A voice that comes back after a longer drop then equals a render started
  where it comes back (a seek there), also when that is a `GRID` boundary;
  keeping the failed start instead would begin one grid step early there.
- Longer stretch outages restart by deterministic bounded pre-roll. Preserving
  exact phase state then requires processing every skipped analysis interval
  with its real input, whose cost grows with the outage. Remeasured in review
  round 2 (`stretch::tests::measure_gap_catch_up`, ignored by default; release,
  LTO off, `nice -n 15`, other lanes building; stereo, 48 kHz, speed 1.5):

  | Gap | Catch-up reads | Catch-up time | Fresh-start reads | Fresh-start time |
  | --- | --- | --- | --- | --- |
  | 0.12 s | 8,640 | 0.9 ms | 33,120 | 3.9 ms |
  | 0.5 s | 36,720 | 3.3 ms | 35,280 | 3.2 ms |
  | 1 s | 73,440 | 6.2 ms | 48,240 | 4.0 ms |
  | 10 s | 721,440 | 59.0 ms | 30,960 | 2.5 ms |
  | 60 s | 4,320,000 | 351.1 ms | 43,920 | 3.5 ms |

  Catch-up work is linear in the gap (72,000 reads a second at speed 1.5),
  a fresh start constant. Both are spent at re-entry on the shared render
  thread, where a 10 s catch-up (59 ms) would nearly empty the 80 ms ring
  under the ready voices. The two cost the same near 0.5 s at speed 1.5.
  Round 3 continues exactly up to `catch_up_limit` (0.58 s of output at
  48 kHz), where the catch-up equals the most a fresh start ever renders, so
  the worst-case re-entry cost is unchanged. Long-gap continuity beyond that
  needs a separate, bounded background catch-up.
- `starved_renders` counts missing segments once, `starve_retries` their
  incomplete retries, `dropped_renders` committed missing segments, and
  `dropped_voice_frames` audible project frames silenced, summed over
  voices, each voice once (its largest drop over the current and outgoing
  retune versions, `RenderReport::dropped_by_voice`; round 2 summed every
  version, so a drop during a glide counted 14,336 frames for 5,120 silenced). Retry attempts advance neither meters nor
  retune envelopes; committed drops advance both exactly once.
- A voice emits one decode-wait diagnostic when its first drop commits.
  Draining `Playback::take_errors` never repeats an ongoing episode. After
  that voice recovers, a later episode reports again, independently of other
  voices. The message names the bound that ended the wait, with stable text
  (pm-node deduplicates its status list by string):
  `Audio decode exceeded 120 ms; missing voice plays as silence: <id>` (the
  A3 wording per voice, which pm-node's
  `present::audio::tests::recurring_render_errors_are_listed_once` matches
  unchanged) or `Audio decode fell behind the output buffer; missing voice
  plays as silence so the others keep playing: <id>`.
- Per-voice decode and evaluation errors deduplicate while they recur on
  consecutive renders and re-arm after a render without them or a restart.
  Whole-mix failures (a failed render, the master converter, the 2x shuttle)
  used the playback-lifetime set until review round 2; they now report once
  per run (a play, seek or stop starts a new episode), and a failed render
  reports again after a successful one (`ErrorLog`,
  `playback::tests::whole_mix_failures_are_reported_once_per_episode`).
  pm-node forwards all of these through its existing bounded status list; no
  pm-node, protocol, modal, or bridge-owned file changes.
- Playback always reads sources nonblocking. Starting playback ensures at
  least one decode worker even for an initially offline-only cache. Export
  waits for all source data regardless of decode duration.

`a_decode_wait_below_the_bound_is_bit_exact` and
`a_long_decode_stall_is_reported_and_recovers` start with ready audio, then
hold the next source block at an explicit worker gate, at 48 and 44.1 kHz
(including a cache initially having zero workers). Real-paced callbacks do
not wait for queue readiness. They assert zero ring underruns, bit-exact
ready audio, sample-accurate missing-voice recovery, device-clock progression,
and dropped-frame accounting. The transient case holds callbacks for 130 ms
of wall time and releases the source within its device reserve: no sample
is dropped. The stuck case crosses the reserve deadline before 120 ms and
keeps the ready voice playing until release.

`a_cold_start_holds_by_device_time` and
`a_cold_start_past_the_bound_drops_only_the_missing_voice` (review round 2)
drive a cold start with real-device semantics: 64-frame callbacks keep
running against the empty ring without waiting for it. They cover a play
from a stop and a seek during an established run, the cold voice alone and
beside a ready voice, at 48 and 44.1 kHz (the latter with a zero-worker
cache). One callback before the bound, after 130 ms more of wall time,
nothing has dropped or been reported; released then, every voice plays bit
exact from the target with the device clock on its line. Held to the bound,
only the missing voice drops, with one `exceeded 120 ms` message, and
re-enters sample-accurately. Both tests fail on review round 1's code
(`dropped_renders` is already non-zero one callback before the bound).

`voices_own_their_waits_and_notifications` starts with A cold beside a
ready voice: callbacks hold the start for exactly 120 ms of device time
(12 callbacks of 512 frames, all silent; since review round 9 not counted as
ring underruns, because nothing of the run has been heard yet), then only
A drops with the `exceeded 120 ms` message. A stays stuck while B becomes
briefly late; B recovers within its own ring budget and every one of its
24,000 audible samples is bit-exact. A recovers and then stalls on its next
source block mid-run, reporting a second time (the output-buffer message,
since the ready voice is sounding); there is one message per episode and no
ring underrun after the start hold.
`stretch::tests::short_committed_drops_reenter_with_uninterrupted_phase_history`
compares both channels to an uninterrupted run across a short committed
outage with forward and reverse maps, asserting a single vocoder start.
The existing long-drop, retune, meter, and offline-wait regressions remain.

The reported wave-5 playback A1 pattern (79,744 of 80,000 different, decoder
starvation zero) also has a distinct possible cause: the first 256-frame ring chunk plays, the second
half of the device callback underruns, and subsequent media is displaced by 256
samples. `a_partial_callback_underrun_is_reported_and_recovers_in_order`
reproduces exactly that mismatch count without any decoder, and checks that the
ring counts the underrun and resumes the correct source order. The mapped
playback comparisons now print and assert both counters, compare both channels,
and retain the original 40 ms simulated-device patience and bit equality. The
original failure did not record its ring-underrun counter, so this reproduction
is evidence for the signature, not proof of that original run's cause.

`time_mapped_clips_play_as_rendered_offline_under_load` adds 8 ms per cold decode
block and two shared CPU contention threads. `PM_AUDIO_PLAYBACK_STRESS=1` applies
that load and delay to every simulated-device test. The contention threads stop
and join when their last device/test owner drops. The forced-starvation playback
regression uses 20 ms cold decode delay and no pre-roll wait; a cold run must count
source starvation and produce exactly the same recovered stereo samples as a
warm run at 1x/48 kHz, 2x/44.1 kHz and -2x/44.1 kHz.

Review round 2 found that regression flaky (5/6 plain, 5/6 stressed at load
about 48): its stall on purpose outlasted the 40 ms simulated device's
patience, the ring underran, and the capture was displaced by 256 frames, the
A1 signature again. Tests of rendered content (that one and the seek after a
starve) use a patient simulated device that waits up to 5 s
for a whole buffer of the current epoch (`DeviceSide::queued_frames`; a chunk
count would also count a seek's dropped chunks and short chunks) and assert it
never gave up. The comparisons are unchanged (bit equality). Real-time tests,
including the two time-mapped comparisons, keep the 40 ms device and assert
`underruns == 0`, so on a heavily loaded machine they can still fail, and when
they do the failure names the ring, not the decoder. The artificial decode
delay compiles only with the `test-hooks` feature, which the crate's tests turn
on through a dev-dependency on the crate itself.

Reproduce the repetition gate with `source engine/env.sh`, then 30 iterations of
`nice -n 15 cargo test --manifest-path engine/Cargo.toml --release -p pm-audio
--test playback -- --test-threads=4 --nocapture`, and another 30 iterations with
`PM_AUDIO_PLAYBACK_STRESS=1`. Direct regressions and existing quality checks:
`nice -n 15 cargo test --manifest-path engine/Cargo.toml --release -p pm-audio
--lib --test decode_readiness --test mixer --test stretch -- --test-threads=4`.

Measured in round 1: 30/30 plain and 30/30 stressed suites of 20 tests. The
independent review could not reproduce that (5/6 and 5/6, the forced-starvation
test above), so those counts are superseded.

Historical audiofix measurement, before A3 (2026-10-18, this Mac, load average 20 to 41 from
other lanes): **30/30 plain suites and 30/30 stressed suites**, 22 tests each
(1,320 playback test executions, no failures, none ignored), with `nice -n 15`
and four test threads. The two time-mapped tests compared three clips over
80,000 frames each in every suite: 360 comparisons, 0 differing samples, 0
starved renders and 0 ring underruns in every one. The seek-after-starve test
matched the offline render at the seek target in all 60 suites (0 of 48,000).
The reviewer's
probe (one starved render, then a render 60 s forward, public `StretchRun` API
only) now reads 43,920 source samples in 3.7 ms and differs from the fresh render
in 0 of 512 samples (before: 4,368,240 samples, 408 ms, 512 of 512). Direct
suites: 41 library, 4 decode-readiness, 14 mixer, 6 stretch, 5 sources,
3 resampler, 2 layouts and 1 legacy-parity test pass. No audio tolerance or
corpus coverage was changed; the real-time tests keep the 40 ms device.

**Measured after A3 (2026-10-18).** With `nice -n 15` and four test
threads: **30/30 plain and 30/30 stressed playback suites**, 23 tests each
(1,380 test executions, no failures, none ignored). The six mapped-clip
comparisons per suite total 360 comparisons of 80,000 frames in both channels:
zero differing samples, source starvation, dropped segments, or ring underruns
in those comparisons. Each suite also passes the callback-driven below-bound
and stuck-decoder tests at 48 and 44.1 kHz, including the initially zero-worker
cache, sample-accurate re-entry, device-clock progression, and one error per
ongoing stall. The existing DSP recovery and seek-after-starve comparisons
remain bit-exact.

Direct checks pass: 41 library tests, 8 decode-readiness tests, 14 mixer tests,
and 6 stretch tests (9 decode-readiness and 24 playback tests after the
review round below). `cargo check --manifest-path engine/Cargo.toml --release
-p pm-node` passes with the test hooks disabled. Modified pm-audio files pass
`rustfmt --check`; staged gitleaks scans and the pre-commit hook are required
for both implementation and documentation commits. The repetition command
above is unchanged, with `PM_AUDIO_PLAYBACK_STRESS=1` for the stress runs.

**Review round (A3 diagnostics, historical; A4 above replaces lifetime
message deduplication with per-episode deduplication).** The independent review found that the
first forwarding flooded `AudioStatus.problems`: `take_errors` emptied its
list, so the next segment that met the same memoized decode error queued it
again, and `Control::report` appended it every 33 ms with no duplicate check.
Reverting both fixes reproduces it: the new pm-node test
`present::audio::tests::recurring_render_errors_are_listed_once` then fails
with all 16 slots holding the same `bad:` decode message and the earlier
media-resolve problem evicted, and the pm-audio test
`recurring_errors_are_taken_once` takes the decode message 46 times in half
a second of device time. With the fixes, over 0.5 s of device time polled on
every 256-frame buffer (more often than the real 33 ms), with one voice's
blocks failing to decode (the `set_decode_failing` test hook) and another's
decoder held past the bound: exactly one decode message, exactly one A3
message naming `late` once, the earlier problem still first, the good voice
still audible, and several dropped segments. `decode_readiness` adds
`a_memoized_decode_failure_reads_as_the_first`.

Measured after the review round (2026-10-18, `nice -n 15`, four test
threads): **30/30 plain and 30/30 stressed playback suites**
(`PM_AUDIO_PLAYBACK_STRESS=1`), 24 tests each, 0 failed, none ignored. The
full `cargo test --release -p pm-audio` passes (41 library, 9
decode-readiness, 2 layouts, 1 legacy-parity, 14 mixer, 24 playback, 3
resampler, 5 sources, 6 stretch); the pm-node test passes; `cargo clippy
--release --all-targets` is clean for pm-audio and pm-node; and `cargo
check --release -p pm-node` (no test hooks) passes.

**A3 review follow-ups.** A4 above resolves the source-wait whole-mix gap,
shared voice budgets, short-gap vocoder restarts, and lifetime diagnostic
suppression. Remaining limits:

- A5's app status/performance surfaces belong to the consumers lane; this
  crate emits per-episode diagnostics and cumulative counters through the
  existing presenter API, without claiming that UI integration here.
- Stretched voices still drop whole segments. Their valid phase history
  continues through gaps up to `catch_up_limit` (0.58 s of output at
  48 kHz); longer gaps use deterministic bounded pre-roll, for the measured
  catch-up reason above.
- With playback's single started worker (a cache created with zero workers)
  stuck on one source, other sources' uncached blocks can starve too. Already
  decoded voices continue, and each missing voice has its own episode.
- Orchestrator decisions pending (today's app decodes whole buffers for
  WebAudio and never waits, so none of these can match it exactly):
  1. A start with one voice ready and another cold. This lane keeps the base
     branch's A3 behavior (hold every voice up to 120 ms of device time, then
     play all of them bit exact, or drop only the still-missing one), which
     adds up to 120 ms of start latency whenever a voice is cold. Review
     round 1 started the ready voice at once and the cold one silent until
     it decoded, a literal reading of A4 point 1 that turns start latency
     into a dropout of the cold voice's first milliseconds on such seeks.
     The hold is the one with no lost audio.
  2. Mid-run, every heard voice missing. The voice keeps the full A3 hold
     rather than A4 point 1's literal min(120 ms, ring left): the ring then
     underruns for roughly 40 to 60 ms and the device clock (and the picture
     on it) stalls, where a drop at ring-empty would be equally silent and
     keep the clock running, at the cost of the voice's bit exactness when
     its block lands within the hold.
  3. Device callbacks of 4,096 frames or more. The refill reserve is larger
     than the 160 ms ring (sized before any callback is seen), so a mid-run
     miss beside a heard voice drops at once with no wait budget, which A3
     never did. Sizing the ring from the first observed callback (or
     re-sizing it on a device change) would restore a budget.

**Remaining evidence gap for wave-5 A1.** The original intermittent failure did
not recur with both counters recorded in the 120 audiofix suites or the
60 A3 suites. Its exact mismatch
signature is reproduced by a counted ring underrun, and the review's failing
runs of the forced-starvation test showed that same 256-frame displacement from
a ring underrun on a loaded machine; the independent decoder/DSP corruption is
fixed and covered by regressions that fail on the old code. The original
failure's attribution stays unconfirmed until a failure records both counters.
The time-mapped tests are real-time tests (40 ms device, `underruns == 0`), so a
heavily loaded machine can still fail them; such a failure now names the ring.

## Review round 11 (2026-10-23)

The round-10 review found one blocker; round 11 also fixes two of its
non-blocking notes. Code revisions `f5fc6053c`, `857d7c357` and `bdee98f31`; e2e
regression `e3830de02`.

| Item | Defect | Fix | Regression evidence |
|---|---|---|---|
| PLAY-1 open-on-Play (blocker) | A Play or PlayAround that the ingress forwarded before the warm-up opened the device opened a fresh transport at 0. The seeks before it had only moved `paused_at`, so the audio and the picture following its clock started at 0. | When `AudioCtl::Transport(Play/PlayAround)` opens the playback, it sends `SetLoop`, `Seek{paused_at}` and the start as one batch, like the other opening paths. The device opener is injectable (headless in tests). | `forwarded_play_that_opens_the_device_starts_at_the_playhead` (clock and heard samples, Play and PlayAround) fails without the fix at `media_time: 0`. The e2e test `a Play forwarded before the warm-up opened the device starts at the playhead` failed in the app without the fix (clock started at 0.000 s) and passes with it. |
| PLAY-1 deferred replay | A forwarded Play that the render thread deferred because the edit's route was pending was replayed with `audio_forwarded=None`. That sent `AudioCtl::Play` a second time and restarted what was being heard. | The view keeps the forwarded marker with the deferred command and replays with it. The route-pending stop clears the marker, because it pauses the audio itself; an explicit Pause clears it too. The audio run keeps going, and the picture joins its clock once the route is decided. | `deferred_forwarded_play_replays_without_restarting_the_audio`: zero `AudioCtl::Play` for a forwarded deferral and one for a non-forwarded deferral. It fails without the fix. |
| Memory pressure while playing | Playback pressure kept only blocks read within 250 ms. Blocks the look-ahead had decoded but the render had not yet read were evicted, so the render starved when it reached them. This happened in 3 of 3 runs (`starved_renders: 1`). | `prefetch` marks a decoded block it asks for again as wanted (`read_at`; the LRU order is unchanged). Playback pressure keeps blocks read, checked or wanted within 1.5 s. That covers the look-ahead cadence (a quarter second of media at J/K/L rates of 1x or more) and the 1 s paused refresh. Today never evicts active voices' assets. | `memory_pressure_during_playback_keeps_it_whole` sends a grant while paused on a prepared start and then every 3 callbacks for 150 callbacks. The output is bit-identical to offline with zero underruns and zero starved renders, in 3 plain and 3 stressed runs. The new unit test `playback_pressure_keeps_blocks_the_look_ahead_still_wants` also passes. |

Validation on the final code (load average 17 to 24 on the shared Mac):

- pm-node presenter tests: 105 of 105. pm-audio lib: 139 passed, 1 ignored.
  `tests/chromium.rs`: 5 of 5. `tests/sources.rs`: 7 of 7. Clippy is clean for
  pm-audio and for pm-node (test-hooks), with all targets.
- Playback suite, release, single-threaded: 44 of 45, plain and stressed. In both
  modes the one failure is AUDIO-2,
  `cold_starts_of_sixteen_distinct_voices_are_complete_and_simultaneous`:
  - plain: 10.92, 10.97 and 11.51 ms
  - stressed: 11.35, 11.58 and 12.23 ms
  - bound: 10.67 ms
  - all failing trials: 0 omitted prefix, 0 start spread, startup arm `Armed`

  With `--test-threads=4` the suite also fails the 16-voice scheduling asserts
  with idle-lease `Conflict`. That happens when concurrent in-process Playbacks
  have different callback periods; one app device has a single period. This is
  a test-parallelism artifact, not a regression.
- Electron, `e2e/native-engine-transport.spec.ts` under the lock with
  `--workers=1`: 5 of 5.

Not rerun in round 11: the paired A8 campaign and the 60 s A7 loop rows. Round 11
does not change the warm-start path the campaign measures. A Play on an
already-open device sends the same `[SetLoop, Play]` batch as before. The
pressure change affects only explicit host memory-pressure grants. The A8 and
AUDIO-2 qualification remains with the orchestrator's quiet window, as listed
under Gaps.

## Current status (review round 10, 2026-10-23)

The round-9 review found three blockers: unbounded resident sequential decodes,
embedded video trims reread at each run, and understated A8 results. The previous
campaign is preserved in [pm-audio-history.md](pm-audio-history.md): five scenarios
missed, including the keyed cold seek (43.40 vs 33.41 ms max); the keyed edit reached
44.90 vs 32.70 ms p95 (+12.2 ms).

### Changes and regression evidence

| Review item | Current behavior | Evidence |
|---|---|---|
| Total decode memory | Lazy metadata opens; exact Chromium PCM spooled to anonymous temporary files; resident sample blocks use a 128 MiB LRU; explicit host pressure evicts inactive blocks while pinned/recent inputs survive; the last mix/read closes its spool | Ten independent AAC files exceeding the total budget, LRU and pressure checks in `tests/chromium.rs`; `rebuilding_a_mix_releases_decoders_but_keeps_unchanged_sample_identity` |
| Embedded animated trim | Frozen at the same `today_index` as precomposition trim; subsequent Plays, seeks and loop wraps keep it | `embedded_video_animated_trim_reads_at_todays_index_build`, including edits paused and playing |
| Export index playhead | `ExportSettings.audioIndexPlayhead` carries the app's cached index time in ticks; the mixer receives it separately from the export range; headless callers may omit it to use range start | Renderer index-lifetime and main settings tests; the complete export pipeline equals an explicitly flattened trim and differs without the field |
| PLAY-1 | A warmed audio link receives transport and ordered document updates directly from presenter ingress, before the render mirror and surface replan | FIFO/revision-barrier and duplicate-Play test; presenter pool-order test; paired measurements below |
| A8-WORKER | Nonaudible preparation uses twice the callback period, between 10.67 and 40 ms, with a matching parked-worker policy; audible workers use the callback policy | Actual Mach readback and producer period/computation/deadline invariants for 64- and 512-frame callbacks |

The memory-pressure hook follows explicit host memory-pressure grants. Ordinary
playback leases and surface-pool replans do not flush audio. Live pressure preserves
pinned blocks and inputs read, checked, or asked for again within 1.5 s (round 11
widened this from 250 ms reads only; see below); the ordinary LRU budget remains
128 MiB. Exact PCM spools trade resident
whole-file memory for temporary disk space (about 115 MB for five minutes of stereo
AAC), released with their final reader.

The direct audio path retains FIFO ordering. While a new link is catching up with
older document commands, a revision barrier keeps later transport on the ordered
fallback path. Media replacement, composition changes and stepping also synchronize
through that path. Video still configures its surface generation before its own
transport command, then follows the audio clock's new epoch, captured before the
fast dispatch. Audio owns time; video catches up to that line.

`ingressRequestMs` measures PM.play → presenter audio request. The separate
`producerRequestMs` includes the audio control thread's document/mix preparation;
the endpoint timestamp does not rename or conceal that preparation. End-to-end
latency remains PM.play → the scheduled program sample at device output.

### Qualification

Code revision `5c0a905c3` (the subsequent commits change documentation only).
Paired hidden Electron apps, 30 requested starts per mode per row, one plain and
three repeats with four busy threads, MacBook Pro speakers at 48 kHz. Latency is
PM.play → the scheduled program sample at device output, without acoustic
loopback. Engine device latency is 15.625 ms; today's is about 24 ms. The measured
rows span 11:18:05–12:18:21 UTC on 2026-10-23.

One 3,590-second reservation was created and deleted before its original expiry;
it was never renewed. The corrected candidate completed 12 rows exclusively.
The deadline interrupted the next keyed-loop row; its partial evidence is preserved, and both
modes were rerun under the normal lock. The remaining 20 rows are marked shared.
No compilation or headless suite from this lane overlapped these measurements.
Host one-minute load at row start ranged from 5.8 to 35.5;
this is shared-machine evidence, not the campaign's final quiet-window qualification.

| Scenario | Run | Today p50 / p95 / max (ms) | Engine p50 / p95 / max (ms) | ≤ today | Play → audio ingress p50 / p95 / max (ms) | Engine loop U/S/O | Lock |
|---|---|---|---|---|---|---|---|
| A7 60 s loop, gain edit | plain | 30.62 / 32.43 / 32.61 | 19.35 / 19.76 / 19.89 | yes | 0.154 / 0.234 / 0.242 | 0 / 0 / 0 | exclusive |
| A7 60 s loop, gain edit | 4 threads, 1 | 30.03 / 32.51 / 32.51 | 19.84 / 20.73 / 21.13 | yes | 0.252 / 0.347 / 0.349 | 0 / 0 / 0 | exclusive |
| A7 60 s loop, gain edit | 4 threads, 2 | 30.71 / 32.61 / 32.70 | 19.97 / 28.84 / 30.18 | yes | 0.344 / 0.431 / 0.694 | 0 / 0 / 0 | shared |
| A7 60 s loop, gain edit | 4 threads, 3 | 30.11 / 32.52 / 32.60 | — (gate failed) | **no** | — | 0 / 0 / 0 | shared |
| 16 voices: speed edit | plain | 29.23 / 32.61 / 32.63 | 25.14 / 26.10 / 26.35 | yes | 0.185 / 0.281 / 0.308 | — | exclusive |
| 16 voices: speed edit | 4 threads, 1 | 30.32 / 32.33 / 32.43 | 27.37 / 28.69 / 30.03 | yes | 0.235 / 0.302 / 0.347 | — | exclusive |
| 16 voices: speed edit | 4 threads, 2 | 29.61 / 32.21 / 32.41 | 29.53 / 31.89 / 39.94 | **no** | 0.304 / 0.562 / 0.925 | — | shared |
| 16 voices: speed edit | 4 threads, 3 | 30.01 / 32.41 / 32.68 | — (gate failed) | **no** | 0.230 / 0.230 / 0.230 | — | shared |
| 16 distinct trims: speed edit | plain | 29.72 / 31.92 / 32.11 | 27.23 / 28.96 / 33.32 | **no** | 0.230 / 0.323 / 0.325 | — | exclusive |
| 16 distinct trims: speed edit | 4 threads, 1 | 29.91 / 32.02 / 32.21 | 28.79 / 29.88 / 30.20 | yes | 0.255 / 0.305 / 0.320 | — | exclusive |
| 16 distinct trims: speed edit | 4 threads, 2 | 30.21 / 32.61 / 32.62 | 31.71 / 36.53 / 45.34 | **no** | 0.353 / 0.440 / 0.457 | — | shared |
| 16 distinct trims: speed edit | 4 threads, 3 | — | — (gate failed) | **no** | 0.496 / 0.496 / 0.496 | — | shared |
| Cold seek (2 / 3 s) | plain | 30.31 / 32.21 / 32.51 | 24.22 / 25.36 / 25.50 | yes | 0.331 / 0.502 / 0.589 | — | exclusive |
| Cold seek (2 / 3 s) | 4 threads, 1 | 29.81 / 32.60 / 32.60 | 26.26 / 27.80 / 31.46 | yes | 0.337 / 0.444 / 0.458 | — | exclusive |
| Cold seek (2 / 3 s) | 4 threads, 2 | 30.00 / 32.20 / 32.69 | — (gate failed) | **no** | 0.149 / 0.460 / 0.576 | — | shared |
| Cold seek (2 / 3 s) | 4 threads, 3 | 29.00 / 32.28 / 32.50 | — (gate failed) | **no** | 0.233 / 0.347 / 0.347 | — | shared |
| Keyed 290 s: 60 s loop, key edit | plain | 30.01 / 32.90 / 33.02 | 30.25 / 35.13 / 35.21 | **no** | 0.225 / 0.283 / 0.296 | 0 / 0 / 0 | exclusive |
| Keyed 290 s: 60 s loop, key edit | 4 threads, 1 | 31.29 / 33.30 / 33.50 | 39.80 / 95.25 / 98.22 | **no** | 0.333 / 0.456 / 0.473 | 0 / 0 / 0 | shared |
| Keyed 290 s: 60 s loop, key edit | 4 threads, 2 | 31.50 / 33.61 / 33.80 | 38.35 / 56.31 / 83.29 | **no** | 0.376 / 0.534 / 0.548 | 0 / 0 / 0 | shared |
| Keyed 290 s: 60 s loop, key edit | 4 threads, 3 | 31.12 / 33.70 / 33.71 | 39.41 / 44.29 / 48.22 | **no** | 0.354 / 0.482 / 1.048 | 0 / 0 / 0 | shared |
| Keyed 290 s: cold seek (230 / 200 s) | plain | 30.22 / 33.10 / 33.30 | 20.11 / 20.76 / 51.23 | **no** | 0.266 / 0.375 / 0.399 | — | exclusive |
| Keyed 290 s: cold seek (230 / 200 s) | 4 threads, 1 | 31.30 / 33.50 / 33.59 | 20.83 / 26.23 / 26.35 | yes | 0.340 / 0.557 / 1.428 | — | shared |
| Keyed 290 s: cold seek (230 / 200 s) | 4 threads, 2 | 31.78 / 33.69 / 34.20 | 20.66 / 24.14 / 28.75 | yes | 0.302 / 0.589 / 1.220 | — | shared |
| Keyed 290 s: cold seek (230 / 200 s) | 4 threads, 3 | 31.09 / 34.29 / 34.30 | 20.79 / 27.51 / 27.68 | yes | 0.312 / 0.465 / 0.781 | — | shared |
| AAC video: cold seek (240 / 200 s) | plain | 29.49 / 32.40 / 32.50 | 19.18 / 19.55 / 19.99 | yes | 0.239 / 0.335 / 0.354 | — | exclusive |
| AAC video: cold seek (240 / 200 s) | 4 threads, 1 | 29.59 / 31.79 / 32.29 | 19.11 / 21.72 / 24.68 | yes | 0.291 / 0.766 / 1.066 | — | shared |
| AAC video: cold seek (240 / 200 s) | 4 threads, 2 | 29.09 / 32.10 / 32.10 | 19.38 / 20.53 / 22.14 | yes | 0.296 / 0.472 / 0.958 | — | shared |
| AAC video: cold seek (240 / 200 s) | 4 threads, 3 | 29.78 / 32.48 / 32.49 | 19.16 / 25.82 / 29.57 | yes | 0.216 / 0.456 / 0.518 | — | shared |
| AAC video: gain edit at 240 s | plain | 30.22 / 32.52 / 32.62 | 19.06 / 19.67 / 19.84 | yes | 0.208 / 0.327 / 0.338 | — | exclusive |
| AAC video: gain edit at 240 s | 4 threads, 1 | 28.91 / 31.71 / 32.10 | — (gate failed) | **no** | 0.267 / 0.267 / 0.267 | — | shared |
| AAC video: gain edit at 240 s | 4 threads, 2 | 29.00 / 32.21 / 32.40 | 19.46 / 19.89 / 19.97 | yes | 0.315 / 0.536 / 0.566 | — | shared |
| AAC video: gain edit at 240 s | 4 threads, 3 | 30.40 / 32.49 / 32.50 | 19.45 / 20.56 / 24.01 | yes | 0.303 / 0.411 / 0.627 | — | shared |

“≤ today” requires p50, p95 and maximum all no worse than today, plus every
correctness and scheduling gate passing. U/S/O means engine loop underruns,
starved renders and device overloads. Gate-aborted rows are failures, not passes.

- **A8 remains unqualified:** 18/32 rows beat today; 8 completed rows miss latency
  and 6 rows abort at a gate. These include 4 startup-arm-timeout
  rows and 3 CPU-budget-overrun rows (categories may overlap). The table
  includes every final-candidate attempt.
- **Loop delivery:** all eight 60-second loops had zero underruns, starved renders
  and device overloads; today had zero underrun events. The third A7 stress row
  still fails its startup-arm guard and never reaches its timed-start samples.
- **Start correctness:** 811 traced engine starts, 0 omitted prefix frames,
  0 frames of start spread, 0 safety-net fires and
  0 program underruns. Loop counters and failed gates are shown separately.
- **PLAY-1:** ingress p50/p95/max is 0.279/0.458/1.428 ms;
  24 of 811 starts exceed 0.5 ms. The p95 and maximum are both
  reported; the dispatch target is not claimed universally met. Producer-request
  p50/p95/max is 0.525/14.877/169.443 ms,
  including document/mix preparation after the fast request.

The rejected provisional candidate (`8ea4815ff`) is kept outside the qualifying
table: 313 traced starts, seven gate-failure rows, 256 omitted prefix frames, one
safety-net fire and 113 program underruns in the start snapshots. Its hook
incorrectly flushed warm inputs on ordinary lease and surface-pool changes.
`5c0a905c3` routes only explicit memory pressure and protects recently read inputs;
the new lease-reason and hot-input tests cover that correction.

### AUDIO-2 after the final fixes

The final release playback binary was rebuilt after the code fixes. These runs
were isolated from this lane's Electron measurements.
Fifteen targeted commands per mode ran four cold distinct-voice trials each.
The stressed headless mode uses the existing two native contention threads; the
Electron campaign above uses four busy threads.
The atomic first-publication timestamp still enforces 512/48,000 s = 10.666667 ms;
the observer timestamp is diagnostic. These are isolated shared-host runs.

| Mode | Atomic trials | p50 / p95 / max (ms) | Trials over 10.666667 ms | Failed commands | Full playback suite |
|---|---|---|---|---|---|
| plain | 60 | 10.527 / 11.665 / 57.749 | 24 | 13 / 15 | 43/44; AUDIO-2 failed |
| stress | 60 | 10.645 / 12.604 / 22.015 | 30 | 14 / 15 | 44/44 passed |

The producer's playing period and deadlines remain unchanged, verified by actual
Mach readback tests. The preparation policy is longer; the 10.67 ms gate has not
been relaxed. Final A8/AUDIO-2 numbers belong to the orchestrator's quiet window.

Validation also passed: 138 audio unit tests (one ignored), five Chromium decode/cache
tests, 99 presenter tests, four Electron A/V transport tests, 113 protocol tests
(one ignored), 64 focused desktop tests, both index-playhead playback regressions,
the complete export-index pipeline regression and 18 remaining audio integration
tests. Clippy for pm-audio/pm-node/pm-export tests passes with warnings denied.
The release native addons and desktop app build. Svelte reports zero errors and
one existing warning; Node TypeScript passes with `composite=false`, while the
standard script retains two existing TS6307 file-list errors from the
canonical-json test's compositions/registry imports.

Raw evidence, the rejected candidate and the deadline-interrupted attempt remain
outside Git under `/tmp/ne-w5/audioa7-r10`.


## Gaps

- **J/K/L keys** need Jude's sign-off. Powermove's keymap is After Effects's, where J and K
  step between timeline events and L reveals audio levels, so J, K, and L shuttle only
  while the new engine plays (ahead of those bindings, declining otherwise); Control-J
  starts backward playback from a pause and Control-Shift-Space plays around. The
  commands are in the palette too. With the legacy engine they decline and nothing
  changes.
- **Reverse shuttle and stretched clips.** Backward J/K/L renders each chunk on its own,
  so clips that preserve pitch resample there (their pitch follows their speed while
  shuttling backwards). Forward shuttles and every export keep the stretch.
- **Conformed frame rates** follow the formats lane's `interpret` shape; the picture's
  conform is that lane's (`feat/ne-formats`, not merged here), so until both are merged
  only the soundtrack follows an override.
- **Listening.** Stretch quality is measured against the library's one-shot stretch,
  FFmpeg's `atempo`, and tones; no listening test was run.
- **Waveform peaks** are computed on request by a streaming decode and kept per process
  (64 files), not stored with the probe in the media index or app cache (section 14);
  each engine launch decodes again.
- **`cargo-deny`** is not installed on this Mac; `pm-stretch` adds only the `cc` build
  dependency (already in the lockfile) and MIT sources (vendor README).
- **Today's first-open index playhead.** Exports now receive the cached app index
  playhead; playback still builds its own index. A project's first open builds
  today's index through the context warm-up at that moment's playhead (the engine builds
  it at the first Play), and a document change today announces with neither a `layers`
  nor a `project` event invalidates the engine's index but not today's. The exact fix is
  an app-side signal: the renderer tracks today's invalidations and sends the build
  playhead with Play and with the export settings.
- **Today's decoder fallback is silent in the app.** If Electron's `libffmpeg` fails to
  load (an ABI major mismatch after an Electron upgrade, a missing symbol) or cannot open a
  container, AAC/Opus/Vorbis decode with Powermove's FFmpeg (about −93 dBFS from today)
  and only `pm-cli` says so. Surfacing it in the audio status problems and a CI check that
  the packaged library loads are open.
- **Real-time allocations are not zero.** `tests/render_allocations.rs` counts 316,504
  heap allocations in 200 warm renders (a ratchet; target 0), mostly channel evaluation in
  `pm_eval` and `MixGraph::resolve` per render. Orchestrator decision; follow-up lane.
- **The headless distinct cold-start bound (AUDIO-2, 10.67 ms)** remains enforced.
  The longer preparation and parked-worker policy is implemented; final measured
  results and any deadline misses are recorded above. Shared-machine results do
  not replace the campaign's final quiet-window qualification.

- **Keyed level channels** prepare their track once per voice and document revision
  (about 0.1 ms for 290 keys); an edit of one key re-prepares every voice whose channel
  changed. Sharing prepared tracks across revisions for unchanged channels, and moving
  the preparation off the producer, are open.
