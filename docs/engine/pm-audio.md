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

## Tests

```sh
source engine/env.sh
cargo test -p pm-media --test audio_decode
cargo test -p pm-stretch --release -- --nocapture       # the binding: pitch, level, alignment, determinism
cargo test -p pm-audio -- --test-threads=4
cargo test --release -p pm-audio --test stretch -- --nocapture   # the pitch and quality tables above
cargo test --release -p pm-audio --test playback -- --nocapture  # J/K/L lines, pitch, and placement
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
(12 callbacks of 512 frames, all silent and counted as underruns), then only
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

## A4 validation (2026-10-18)

Round 1 (historical; round 2 below supersedes it). The round 1 A4 implementation passed **30/30 plain and 30/30 stressed playback
suites**, 25 tests each: 1,500 executions, zero failures, none ignored. The
six mapped-clip comparisons in each suite total 360 comparisons of 80,000
frames in both channels (57,600,000 sample values): zero differences, source
starvation, drops, or ring underruns in those comparisons. The callback-paced
A4 probes assert zero ring underruns while a worker remains blocked, exact
ready-voice samples, bit-exact transient recovery, and one notification per
voice episode. The overlapping-episode probe checks all 24,000 audible B
samples while A remains stuck, then checks that A reports its next episode.

Validation uses the pinned Rust 1.96.1 toolchain, `kache`, `nice -n 15`, four
build jobs, and four test threads. The stress runs set
`PM_AUDIO_PLAYBACK_STRESS=1` (two shared CPU contention threads and 8 ms cold
decode delay). Reproduce the repetition commands above with the additional
local environment setting `CARGO_PROFILE_RELEASE_LTO=off`. Thin-LTO test
builds in this environment failed with `failed to get bitcode from object
file for LTO (Can't find section __bitcode)`, including a retry with explicit
embedded bitcode. (The cause was pm-media bundling CrabbyAvif's Rust
staticlib, whose objects carry no bitcode, into its rlib; not kache or this
machine. Fixed since: [release-build.md](release-build.md).) The tests passed
with optimized release code and LTO off;
the repository's release profile was not changed. These counts therefore do
not claim a successful thin-LTO test link. No parity tolerance or corpus case
was changed.

Final targeted checks also pass: 42 library tests (including forward/reverse
short-drop vocoder continuity), 9 decode-readiness tests, 14 mixer tests,
and 6 stretch tests. `cargo check --manifest-path engine/Cargo.toml --release
-p pm-audio --lib` passes with the normal release configuration and test hooks
disabled. Modified Rust files pass `rustfmt --check`.

### A4 review round 2 (2026-10-18)

The independent review found two blocking defects in round 1:

1. The per-voice stall message changed wording, and pm-node's
   `present::audio::tests::recurring_render_errors_are_listed_once`, which
   round 1 never ran, failed (it matches `exceeded 120 ms`). The 120 ms
   message is the A3 wording again (per voice); a reserve-ended wait has its
   own stable message. The pm-node test passes unchanged, and nothing in
   pm-node changed.
2. A cold play or seek dropped its missing voices about one device callback
   after starting, because real callbacks advance `played_frames` through
   underruns and an empty ring had no wait budget. The A3 start hold is back
   (see the policy above), and an all-missing mix mid-run keeps it too.

Non-blocking items addressed: whole-mix failures report per run episode
instead of once per playback; the stale doc comment on
`a_long_decode_stall_is_reported_and_recovers` is rewritten; the clippy
`type_complexity` warning is gone with `has_ready_source`; large device
callbacks keep a wait budget (`fill_target`); the long-gap catch-up cost is
remeasured (table above).

`a_cold_start_holds_by_device_time` and
`a_cold_start_past_the_bound_drops_only_the_missing_voice` are new and fail
on round 1's code (all-cold play from a stop: `dropped_renders` is 1, not 0,
one 64-frame callback before the bound). Their first version was flaky in
the seek cases (6 of 25 runs): with the ring full of the old epoch's chunks
the run renders nothing until a callback retires them, which the probe did
not do. The probe now calls back until the run waits and bounds the hold
start accordingly; 40 repetitions (20 plain, 20 stressed) then passed.

Measured at 7586687b (release, `CARGO_PROFILE_RELEASE_LTO=off`, `nice -n 15`,
four build jobs, four test threads, other lanes building on the same Mac):

- **30/30 plain and 30/30 stressed playback suites**
  (`PM_AUDIO_PLAYBACK_STRESS=1`), 27 tests each: 1,620 executions, zero
  failures, none ignored. 360 mapped-clip comparisons of 80,000 frames:
  zero differing samples, zero source starvation, zero ring underruns.
  Suite times 20.0 to 23.7 s (median 20.2 s).
- Full `cargo test --release -p pm-audio`: 44 library (1 ignored: the
  catch-up measurement), 9 decode-readiness, 2 layouts, 1 legacy-parity,
  14 mixer, 27 playback, 3 resampler, 5 sources, 6 stretch.
- `cargo test --release -p pm-node --lib present::audio`: 1 passed.
- `cargo clippy --release --all-targets` is clean for pm-audio and pm-node.
  `cargo fmt --check` reports only `pm-audio/src/graph.rs`, which this lane
  does not touch (it arrived unformatted from the integration branch).

Remaining: the mixed-readiness start needs the orchestrator's confirmation
(see the A3 review follow-ups); long-gap stretch continuity (over 120 ms of
dropped output) restarts by deterministic pre-roll; 4,096-frame and larger
callbacks leave mid-run misses beside a sounding voice no reserve-bounded
wait; thin-LTO test links still failed as in round 1 (fixed since:
release-build.md).

### A4 review round 3 (2026-10-18)

The independent review found one blocking defect: A4 point 1 failed during a
retune glide. Round 2 decided whether the reserve protects anyone from
`report.voices > report.starved`, but `voices` counts only the current mix
while `starved` adds every outgoing glide's copy of the missing voice. With
voice A ready, voice B missing and one glide in flight, that is 2 against 2,
so B took the full 120 ms hold, the 80 ms ring ran dry and A went silent.
Every live edit and monitor solo change while playing starts a glide, and a
waiting render never advances one, so a fader drag during a decode miss hit
this. The decision now uses ids: some voice in `RenderReport::heard_whole`
(sounded whole, on a heard strip, current or outgoing mix) is not in
`starved_voices`. `dropped_voice_frames` had the same inflation and now counts
each voice once.

New tests:

- `a_decode_miss_during_a_live_edit_keeps_ready_voices_heard` (playback,
  callback-paced): one live edit per 256-frame callback, a gain drag on the
  late voice and, in a second case, a monitor solo toggled between every strip
  and both strips (glides that do not change what is heard), from before the
  miss until after re-entry. It asserts the ring never runs below a callback,
  zero underruns, the drop before 120 ms with the output-buffer message, the
  ready voice bit exact, one contiguous gap, and `dropped_voice_frames` equal
  to the silenced samples. With round 2's `voices > starved` rule it fails:
  the ring runs out at turn 32 with `dropped_renders: 0` and 45 retries
  (`/tmp/ne-w5/audiopolicy-r3/live-edit-on-round2-protects.txt`, outside the
  repo). Before the per-voice count fix it also failed on 14,336 dropped
  frames for 5,120 silenced.
- `heard_whole_voices_count_once_across_glides_and_follow_the_solo`
  (decode_readiness): four glides in flight give `voices` 2 and `starved` 5,
  with `heard_whole` `["good"]` and `starved_voices` `["late"]`; a dropped
  render counts 256 frames once; a soloed-out voice is not heard.
- `stretch::tests::committed_drops_continue_within_the_catch_up_limit`:
  300 ms and 500 ms committed drops (forward and reverse maps) re-enter with
  one vocoder start and equal an uninterrupted run; 1 s and 1.28 s drops (the
  latter ending on a `GRID` boundary) equal a fresh run from the re-entry.
  With the old 120 ms window the 300 ms case fails; without the commit-drop
  reset the 1.28 s case fails.
- `a_stretched_voice_re_enters_a_300_ms_drop_bit_exact` (decode_readiness,
  through the mixer, speed 1.5): a playing stretched voice loses its next
  source block for 300 ms of output, then re-enters equal to a mixer that
  never dropped, with the other stretched voice equal throughout. It fails
  with the old 120 ms window (first differing render at output 53,248).

Non-blocking items addressed: the stretch continuity window is now the
work-based `catch_up_limit` (output samples, documented as render work, not
device time), replacing the hard-coded 120 ms; a soloed-out voice no longer
shortens a soloed voice's wait. The three orchestrator decisions are listed
under the A3 review follow-ups.

Measured at f466b8ed (release, `CARGO_PROFILE_RELEASE_LTO=off`, `nice -n 15`,
four build jobs, four test threads, other lanes building on the same Mac):

- **30/30 plain and 30/30 stressed playback suites**
  (`PM_AUDIO_PLAYBACK_STRESS=1`), 28 tests each: 1,680 executions, zero
  failures, none ignored. 360 mapped-clip comparisons of 80,000 frames:
  zero differing samples, zero source starvation, zero ring underruns.
  Suite times 20.3 to 21.6 s (median 20.5 s). The new live-edit test passed
  in all 60.
- Full `cargo test --release -p pm-audio` (before the mixer-level re-entry
  test): 45 library (1 ignored: the catch-up measurement), 10
  decode-readiness, 2 layouts, 1 legacy-parity, 14 mixer, 28 playback, 3
  resampler, 5 sources, 6 stretch. At f466b8ed the library, decode-readiness
  (11), mixer and stretch suites pass again; playback is the repetition
  above.
- `cargo test --release -p pm-node --lib present::audio`: 1 passed, pm-node
  unchanged.
- `cargo clippy --release -p pm-audio --all-targets` is clean. `cargo fmt
  --check` reports only `pm-audio/src/graph.rs`, inherited from the
  integration branch.
- Catch-up remeasured (same table shape as above): 0.12 s 1.1 / 4.0 ms,
  0.5 s 3.8 / 3.7 ms, 1 s 7.0 / 4.4 ms, 10 s 58.4 / 2.4 ms, 60 s 345.9 /
  3.6 ms (catch-up / fresh start; reads unchanged).

Remaining: the three orchestrator decisions above; stretch continuity past
`catch_up_limit` restarts by deterministic pre-roll; thin-LTO test links
still failed (fixed since: release-build.md).


## A6: uninterrupted clock, adaptive ring, paused preparation (2026-10-18)

A6 implements the engine policies for the three orchestrator decisions
previously listed as pending. The caller integration follow-up below also
delivers ordinary paused app seeks before Play.
The 120 ms cold-start no-loss hold stays. After a run has queued output,
every missing voice drops at the earlier of its own 120 ms bound and the
remaining ring budget. This applies when all voices are missing too: the
mixer commits intentional silence with the correct media tags, so the device
clock and the picture keep moving. Each voice retains its own episode,
recovery point and one diagnostic per episode; exports still wait for input.

The start hold now lasts until the first **device output chunk** is queued,
including time spent priming the 2× shuttle and device-rate converter. A6's
first repetition exposed a −2× cold/warm mismatch: ending the hold after the
first project input segment allowed a second source miss during shuttle
priming to drop with an empty ring. Moving the transition to actual output
fixes it; the existing bit-exact cold/warm comparison now also asserts zero
committed drops. No sample comparison or tolerance changed.

The callback only records its frame count. The render producer grows a
linked SPSC ring by splicing stable, empty nodes at its unfilled sentinel;
queued/partially consumed nodes never move and remain alive until both
sides close. The callback does not allocate, lock or wait. The ring retains
the largest observed callback size, so a first observation or buffer-size
increase adapts on the next fill. The target is at least the configured
80 ms, and otherwise three callbacks plus 40 ms: two callbacks of refill
reserve, the callback already spent waking the producer, and a per-voice
wait. At 48 kHz it is 3,840 frames for 512-frame callbacks, 14,080 for 4,096,
and 26,368 for 8,192 (whole 256-frame chunks). Growth preserves old-epoch
skips, exact partial-frame counts and sample order, including concurrent
producer/consumer traversal. It does not shrink while a viewer lives.

While paused, the render loop queues bounded look-ahead for every voice at
the playhead and work-area boundaries, including the stretch analysis
pre-roll. At most eight new time-map groups are bounded per window per turn.
Up to three starts keep separate, freshly pre-rolled vocoder histories;
preparing the work start cannot change the playhead's phase history. A new
project/evaluator invalidates these histories; a seek retains only relevant
starts. Prepared PCM never reaches the device or meters while paused. A
normal play installs the matching prepared history and replays its initial
chunk exactly as an offline render from that target.

The minimal presenter hooks are in `pm-node/src/present/audio.rs`: feed
project/media revisions to warmed mixers before the first Play, remember
paused seeks before device open, and handle a delivered paused scrub with
Scrub audio off as an exact, silent seek. They use the existing transport
and status interfaces; no protocol or app UI changed.

**Paused caller integration.**
The small follow-up in `pm-node/src/present/render.rs` forwards
paused exact seeks, forwards paused scrubs regardless of the audio-scrub
preference (the audio control handler decides whether to play a grain), and
seeds a newly created audio link with the current playhead. Host-clock test
views still avoid opening audio. A focused hidden-app regression verifies the
prepared sample positions after exact seeks, scrub-off moves, and an exact
seek before audible media/device open. Scrub-off emits no grain levels;
scrub-on emits a tone grain and returns to the requested stopped position.
The existing broader waveform/scrub test retains all its assertions.

A6 point 4 accepts deterministic pre-roll after long stretch gaps. Valid
phase history continues up to `catch_up_limit` (27,904 output samples,
0.5813 s at 48 kHz); longer committed gaps restart at the deterministic grid
and match a fresh render from re-entry. Recovering the old phase beyond that
requires analysing every missed sample: the earlier measurements above show
351 ms for a 60 s catch-up versus 3.5 ms for a fresh start. The long gap is
already audible, and the bounded restart protects the other voices.

New playback regressions cover an all-missing mid-run mix with zero ring
underruns and a continuously correct clock; 4,096-/8,192-frame callbacks
with transient recovery inside each voice's budget and prolonged drops
beside a ready voice or with every voice missing; and warm / initially cold
paused preparation at the playhead, a work start and a paused seek. Every
prepared start must queue a complete 256- or 512-frame buffer within its
first device callback (5.333 or 10.667 ms) and equal offline samples exactly
(128 frames before the buffer-size review follow-up below).
A ring test also grows while full and partially consumed, then skips an old
epoch; the concurrent order test grows twice while both threads run. `graph.rs` is
formatted as requested.

Prepared starts are best effort under cache pressure, pending media, or an
immediate play after an edit/seek; genuinely cold starts keep the no-loss
hold. Paused source windows now also look behind the playhead for reverse
runs. The project/device converter is constructed on the idle render thread;
the first audible pause also allocates and exercises the 2× master's FFT and
work buffers with discarded silence, then resets its stream bookkeeping.
A unit test proves that warming preserves exact forward/reverse PCM and tags.
This warms resources, not a signal-dependent speculative stream: the master
output-seek and converter histories still start on the real input at Play.
Caching complete streams at every direction/rate would also need separate
transport, voice histories, converted PCM and meter state. That larger change
is deferred; the one-callback prepared-start gate covers
normal forward play, the default app transport. Acoustic loopback and
listening are not claimed by the scheduled-presentation measurement.

### A6 initial play-start measurement (superseded by interactive-buffer follow-up)

The synthetic 10 s stereo WAV was captured through hidden, isolated real
app launches under `pm-electron-lock`, with 30 ready plays/seeks per engine
(alternating 0 and 3 s, 250 ms paused preparation, one initial unmeasured
play to resume the device). Scheduled output presentation, in milliseconds
from `PM.play`, was measured as follows:

- Today's WebAudio app: p50 **30.234**, p95 **32.031**, p99 **32.238**,
  range 27.432–32.238 ms.
- Native app: p50 **30.831**, p95 **38.781**, p99 **38.955**,
  range 28.621–38.955 ms. The native tail is **6.750 ms slower at p95**;
  this initial result failed the latency target; the follow-up below closes it.

The legacy capture maps the source's scheduled context time through
`AudioContext.getOutputTimestamp()`. The native capture maps the first
program sample on the device clock through the renderer's measured host
offset (at most 50 µs error in the captured native run). The native report
confirms one 48 kHz device, 15.625 ms reported output latency, zero
`hostClockRuns`, zero source-starved renders and no audio problems. Ring
underruns in that report include continuously running callbacks while
paused and are not a mid-run dropout measurement. Both apps report zero
page errors. This is scheduled-presentation evidence, not acoustic loopback.
The direct prepared-start gate separately measures command → first queued
buffer within one callback; it excludes host transport and output latency.

`engine/crates/pm-audio/tools/play-start.ts` reproduces the app capture. It
uses the corpus Node bridge because Bun's direct Electron inspector driver
does not resolve on this machine. Build `pm-wasm`, the native addon, and the
app first; keep generated media and output JSON outside the repository:

```sh
source engine/env.sh
nice -n 15 node engine/crates/pm-wasm/scripts/build.mjs
CARGO_PROFILE_RELEASE_LTO=off nice -n 15 node apps/desktop/scripts/build-engine-addon.mjs
nice -n 15 bun run --cwd apps/desktop build
"$PM_FFMPEG_DIR/bin/ffmpeg" -hide_banner -loglevel error -y -f lavfi \
  -i 'aevalsrc=0.000001*sin(2*PI*997*t):s=48000:d=10' -ac 2 \
  -c:a pcm_f32le /tmp/pm-audio-start.wav
~/Library/Caches/powermove-engine/bin/pm-electron-lock nice -n 15 \
  bun engine/crates/pm-audio/tools/play-start.ts \
  /tmp/pm-audio-start.wav /tmp/pm-audio-start.json 30
```

### A6 initial validation

Pinned Rust 1.96.1, `kache`, four build jobs, niceness 15 and four test
threads: **30/30 plain and 30/30 stressed playback suites**, 31 tests each,
**1,860 executions**, zero failures and none ignored. Stress uses the
existing `PM_AUDIO_PLAYBACK_STRESS=1` profile (two shared CPU contention
threads and 8 ms cold-decode delay). Suite times were 21.065–21.260 s,
median 21.158 s. All 360 mapped-clip comparisons of 80,000 frames in both
channels were bit-exact, with zero source starvation or ring underruns in
those comparisons. Across 360 warm/paused-prepared starts, first-buffer
queue latency was p50 0.0845 ms, p95 0.361 ms, maximum **0.467 ms**, against
the 10.667 ms callback bound. The callback-paced all-missing and large-buffer
probes each assert zero mid-run ring underruns, exact ready samples and
sample-accurate re-entry.

Targeted checks pass: 46 library tests (one existing ignored catch-up
measurement), 11 decode-readiness, 14 mixer and 6 stretch tests; pm-node's
`present::audio::tests::recurring_render_errors_are_listed_once`; strict
`cargo clippy --release -p pm-audio -p pm-node --all-targets -- -D warnings`;
`cargo fmt -p pm-audio --check`; and production `cargo check --release
-p pm-node --lib` with test hooks disabled and the default release profile.
The app, pm-wasm and native addon builds pass; the paired hidden app capture
has zero page errors, zero source-starved renders and no audio problems.

As in A4, thin-LTO test linking failed with `Can't find section
__bitcode` (pm-media's bundled CrabbyAvif archive; fixed since, see
release-build.md). Tests, clippy and the measured addon use the local environment
setting `CARGO_PROFILE_RELEASE_LTO=off`; repository release settings are
unchanged. The repetition gate executes the built playback test binary
with the same four-thread arguments and FFmpeg library path supplied by
Cargo, avoiding Cargo's build lock during the app/addon builds. To repeat
with Cargo, use the existing 30-iteration commands above and the LTO-off
environment setting. No audio tolerance, corpus case or existing comparison
was relaxed. The follow-up below supersedes the initial caller and latency gaps.

### A6 interactive-buffer follow-up (buffer size superseded)

The 128-frame request below was wrong: it is WebAudio's render quantum,
not the device buffer today's app runs at. The review follow-up after this
section restores today's 256-frame output buffer and re-measures. The
latency attribution here still explains why an explicit size is needed.

The 512-frame inherited HAL default caused the slower start tail. Tracing
30 starts separated the app request, audio control arrival/application,
first output ready to queue, callback entry, and the HAL output timestamp.
App → audio control took 0.885–1.484 ms; control scheduling took 0.001–0.006
ms; preparing the first output took 0.014–0.021 ms. Waiting for the callback
then took 0.185–10.446 ms, followed by 11.602–11.650 ms of HAL scheduling
lead and the unchanged 15.625 ms device/stream latency. These were warm
starts, so opening the device was outside the measured path. Source/DSP
preparation and the bridge hop did not explain the additional tail.

`OutputStream` now requests 128 frames, clamped to the device's supported
buffer range; unknown-size devices retain their default. Devices whose
minimum is 4,096/8,192 frames retain that minimum and the producer's adaptive
reserve/growth behavior. Opening the stream can change HAL properties, so
latency, safety offset and buffer metadata are read again after opening.
The audio callback's observed frame count, rather than the request, proves
the size used in the measurement. No clock offset or reported output
latency was reduced to improve the numbers.

The final paired remeasurement of the verified binary, using 30 captures per
engine, 250 ms paused preparation, alternating 0/3 s, unmeasured initial
resume, hidden isolated app and scheduled-presentation method, measured:

| Engine | p50 (ms) | p95 (ms) | p99 (ms) |
| --- | ---: | ---: | ---: |
| Today's app | 29.929 | 32.128 | 32.433 |
| Native, 128-frame callback | 21.322 | 22.745 | 22.860 |

All three native percentiles are below today's. The native app used a
48 kHz device clock, reported the same 15.625 ms output latency, and had
zero host-clock runs, source-starved renders, audio problems or page errors.
All 30 traces match their app request on the host clock: app → audio control
0.824–1.211 ms, control scheduling 0.001–0.008 ms, first-output preparation
0.015–0.019 ms, callback wait 0.111–2.451 ms, and HAL lead 3.661–3.676 ms.
This remains scheduled device presentation evidence, not acoustic loopback.
The harness uses the packaged `native/pm_bridge.node` copy installed by the
addon build, so final captures can run after Rust target cleanup too.

`AudioStatus.startTiming` exposes the first-buffer trace and
`preparedStarts` exposes ready paused positions in project samples. Paused
audio status refreshes at the existing 33 ms reporting interval so it can
observe asynchronous preparation and device errors without a Play control.
The callback uses only atomics and reads the host clock once on the first
buffer of an epoch. Presenter and audio epoch counters are independent;
the capture tool associates a trace with its app request on the shared
host clock, rather than comparing the two counters.

The final follow-up playback gate repeats the updated binary **30/30 plain
and 30/30 stressed**, 31 tests per suite: **1,860 executions**, zero failures
or ignored tests. Suite times were 21.072–21.319 s (median 21.158 s). All
360 mapped comparisons of 80,000 stereo frames remained bit-exact with
zero source-starved renders. The 720 prepared starts split evenly between
callback sizes:

| Callback | Starts | Queue p50 / p95 / p99 (ms) | Maximum (ms) | One callback (ms) |
| --- | ---: | --- | ---: | ---: |
| 128 frames | 360 | 0.043 / 0.061 / 0.093 | 0.160 | 2.667 |
| 512 frames | 360 | 0.073 / 0.323 / 0.351 | 0.393 | 10.667 |

Both warm and initially cold-but-paused-prefetched voices pass; these tests
retain exact offline PCM, no source hold/drop, the work-area target and
paused-seek assertions. The 4,096-/8,192-frame and all-missing clock tests
continue unchanged. The local thin-LTO linking limitation described above
still applies; no repository profile or audio tolerance was changed.

The final targeted Rust checks pass: 49 library tests (one existing ignored
catch-up measurement), 11 decode-readiness, 14 mixer, 6 stretch tests, and the
presenter's recurring-audio-error diagnostic test. Strict release clippy for
`pm-audio`/`pm-node` all targets and the default production-profile `pm-node` library check with test hooks disabled pass.
The hidden transport app run passes J/K/L, play-around, and the new paused
seek/scrub regression (including ordinary `PM.setTime` calls). Its fourth,
unchanged waveform/meter test fails before reaching scrub assertions:
`peaksSource` never becomes `engine`, so the expected 150 engine buckets are
reported as zero. Diagnostics show the asset already has 150 legacy buckets
but the renderer's native peak-request/answer sets are empty. This broader
waveform synchronization gate remains open; its assertion was not removed,
skipped, or relaxed. No waveform routing/synchronization code was changed
for A6.

### A6 review follow-up: today's output buffer, overloads, idle cost

**Buffer size.** Today's app runs its `AudioContext` (latency hint
`interactive`) on Chromium's macOS output, whose
`AudioManagerMac::ChooseBufferSize` asks for 256 frames up to 96 kHz and
512 above, clamped to the device's I/O range. Every legacy capture reports
`AudioContext.baseLatency` 5.333 ms, 256 frames at 48 kHz. The previous
follow-up requested 128 frames, which halved the real-time deadline
(2.667 ms against 5.333 ms) and doubled callback wakeups (375 against 187.5
per second), a cost and underrun risk today's app does not have.
`OutputStream` now requests exactly Chromium's size, 256 frames, or 512
frames above 96 kHz. Both are clamped to the device range; unknown-size
devices keep their default. The real device (MacBook Pro speakers, 48 kHz)
runs 187.5 callbacks a second in every native measurement below, the same
rate as today's app.

**Overloads.** cpal 0.18 reports CoreAudio's `kAudioDeviceProcessorOverload`
as an `Xrun` stream error, which used to set the stream's diagnostic
"failed" flag. Overloads now leave the stream running (as WebAudio's does)
and are counted: `PlaybackStats::device_overloads` and
`device_callbacks`, and `AudioStatus.deviceOverloads` and
`deviceCallbacks` in the presenter report. Ring underruns still count
every callback while paused, because the ring is empty by design. Only
deltas over a playing window measure dropouts.

**Play-start, re-measured.** The measurement used the final binary at
256 frames, a hidden isolated app under the Electron lock, 30 paired
captures per engine, and the same scheduled-presentation method as
before. Values are in ms from `PM.play`:

| Seek → Play | Engine | p50 | p95 | p99 | max |
| --- | --- | ---: | ---: | ---: | ---: |
| 250 ms paused | Today's app | 29.412 | 32.027 | 32.315 | 32.315 |
| 250 ms paused | Native, 256 frames | 26.358 | 28.441 | 28.495 | 28.495 |
| same task (0 ms) | Today's app | 29.407 | 32.114 | 32.614 | 32.614 |
| same task (0 ms) | Native, 256 frames | 25.798 | 28.032 | 28.366 | 28.366 |

An earlier 256-frame capture, made before the preparation-order change,
agreed within 0.8 ms at every percentile (native p95 28.336 and 28.502).

`PM_AUDIO_START_WAIT_MS=0` seeks and plays in the same renderer task, so
paused preparation gets no time; the single-voice project's start still
leads today at every percentile. The native captures report a 256-frame
first callback, zero host-clock runs, zero device overloads and starved
renders, and no audio problems. Both engines report zero page errors.
Host-offset error was at most 50 µs (250 ms) and 150 µs (same task).
Legacy `baseLatency` was 5.333 ms in every sample.

**Device cost against today.** `engine/crates/pm-audio/tools/audio-load.ts`
measures both engines in the hidden app over 15 s windows: paused with the
device open, playing, and playing beside four busy loops (all at nice 15).
Each window records the Electron process tree's CPU time, top's energy
impact and coreaudiod's CPU. Two paired runs after the idle fix below:

| Window | Today's tree CPU / energy | Native tree CPU / energy | coreaudiod today / native |
| --- | --- | --- | --- |
| Paused | 6.6%, 5.3% / 6.5, 5.1 | 4.9%, 4.6% / 3.9, 4.1 | 3.1-3.5% / 3.3-3.5% |
| Playing | 50.0%, 51.3% / 47.4, 48.8 | 51.1%, 51.7% / 46.8, 47.7 | 3.8% / 4.1-4.2% |
| Playing under load | 33.2%, 34.0% / 31.4, 32.7 | 45.2%, 38.3% / 42.9, 36.2 | 3.0-3.1% / 3.1-3.6% |

Whole-tree CPU includes each engine's video path. Under load it reflects
the CPU share a niced app wins against the loops, so it is not an audio
cost. Today's Chromium exposes `AudioContext.playbackStats`. It counted zero
underrun events in every window, including under load. In every window,
native counted 187.5 callbacks a second, zero device overloads, and zero
ring underruns or starved renders while playing. top's idle-wakeup column
did not produce interval values usable as rates and is not reported.
Paused, today's AudioContext keeps rendering (it is never suspended), and
so does the native stream. The audio path alone is measured by
`examples/paused_cost.rs` on the real device. It uses a silent
-120 dBFS source, 10 s windows and process CPU from `getrusage`:

| Project | Paused CPU | Playing CPU | Overloads | Playing ring underruns |
| --- | ---: | ---: | ---: | ---: |
| 1 voice | 0.10-0.25% | 0.44-0.84% | 0 | 0 |
| 16 voices | 0.13-0.25% | 1.4-4.1% | 0 | 0 |
| 16 voices at 1.5x, looping | 0.13-0.30% | 25-32% | 0 | 0-7 (see below) |

Before this follow-up, the paused render loop re-bounded every voice's
spans and readiness for three starts on each 20 ms turn. That cost 0.8-2.2%
CPU idle with 16 stretched voices, against 0.19-0.22% on the base branch.
Fully prepared, unchanged starts are now re-checked once a second
(`PAUSED_REFRESH`), so evicted blocks are still queued again. The
presenter's paused 33 ms status wake is part of the 1-voice paused figure.

**Pre-existing loop-wrap underruns.** With 16 stretched voices looping
[4 s, 6 s), ring underruns occur at loop wraps: 0-7 counted 256-frame
callbacks over 10 s in six runs. The base branch (before A6, 512-frame default) also
counts 1-5 at 512 frames. A straight play-through of the same project
counts 0. A wrap re-enters the loop start with a fresh stretch pre-roll for
every voice in one render, and 16 voices nearly fill the 80 ms reserve. A6
did not introduce this, and its prepared paused histories are used only by
Play. Keeping a prepared loop-start history while playing is the likely
fix. It is listed as an open gap.

**Preemptible preparation.** Each paused start renders every voice, about
2.4 ms per stretched voice for a fresh pre-roll. The render loop now checks
for a control between starts, and between 8-group bounding steps, and
handles it first. The first start whose look-ahead is not yet bounded (the
playhead first) gets the turn's whole bounding budget: six steps, the same
total the pass used to spread over every start's windows. Reverse
look-behind windows follow once every look-ahead is bounded. A first
attempt bounded every start's look-ahead before any look-behind, and that
was slower for the playhead (0.91-0.99 s). The final order replaced it.
`play_preempts_paused_preparation_after_an_edit` edits 16 stretched voices
and sends Play at 48 phases, 2 ms apart, of the restarted pass:

| | Play acknowledged p95 / max (ms) | Playhead prepared | All three starts |
| --- | --- | ---: | ---: |
| No preemption, original order | 8.6-9.2 / 9.6-10.3 | not measured | 2.10-2.25 s |
| Preemption, original order | 2.3-4.0 / 3.3-4.3 | 0.65-0.73 s | 1.93-2.10 s |
| Final: preemption, playhead budget first | 1.5-1.8 / 1.9-2.0 | 0.49-0.52 s | 1.23-1.30 s |

The last row includes one run under the stress profile. A Play that lands
before the playhead is prepared is a fresh start. Its first 512-frame
buffer is queued 27.7-29.0 ms (p50), 43.3-48.4 ms (p95) and 49.8-51.6 ms
(max) after Play. For such a heavy project, that adds about the same to the
app-level start within half a second of an edit. That is the existing A3
start hold, not a dropout. Today's app has no equivalent wait, so it stays
on the orchestrator decision list above (item 1).

**Presenter.** A paused exact seek can create a view's audio link before
the view's first document arrives. `warm_audio` returns once any link
exists, so the device used to open cold at the first Play. `audio_for` now
sends `AudioCtl::Warm` whenever it creates a link. Paused exact seeks reach
audio only for views the engine draws, the same `Route::Engine` check that
scrubs and Play use. `present/audio.rs` is rustfmt-clean.

**Validation of the final binary.** These ran with pinned Rust, `kache`,
four build jobs, niceness 15, four test threads and the local
`CARGO_PROFILE_RELEASE_LTO=off` setting described above. The playback
suite passed **30/30 plain and 30/30 stressed**, 32 tests each (the new
preemption test included): 1,920 executions with zero failures or ignored
tests. Suite times were 24.10-25.23 s. All pm-audio targets pass: 49
library tests (one existing ignored measurement), 11 decode-readiness, 2
layouts, 1 legacy parity, 14 mixer, 3 resampler, 5 sources and 6 stretch.
pm-node passes its 37 `present::` unit tests and the present and scrub
integration tests. Strict release clippy for `pm-audio`/`pm-node` all
targets, `cargo fmt -p pm-audio --check`, a rustfmt-clean `present/`, and
the production `cargo check --release -p pm-node --lib` all pass. The hidden
transport app run passes J/K/L, play around and the paused seek/scrub
regression. The waveform/meter case still fails at the same assertion
(`peaksSource` never `engine`, 150 expected, 0 received), unchanged from
before this follow-up.

**What remains.** The loop-wrap underruns above, the A3 start hold after
an edit or cold seek, reverse and 2x master output-seek and converter
histories that still prime on Play, and the waveform/meter app case
(native peaks provenance, outside this lane). Bluetooth and USB output
devices were not attached to this machine. The buffer request matches
Chromium's for them too, but no measurement on them is claimed.
`RenderReport.heard_whole` is kept: it is a public diagnostic with its own
decode-readiness test.

## A7/A8: prepared loop histories and edit-to-play (2026-10-18)

> **Reading order.** The subsections from here through "A8 round 2" are the
> historical record of rounds 1 and 2. Their statements that A8 is still a
> blocker, that late voices omit a counted prefix, and that validation is
> incomplete are superseded: round 3 made every ordinary start complete and
> simultaneous, and round 4 (at the end of this section) qualifies it on
> distinct voices and closes the review's follow-ups. Current behavior and
> evidence are in "A8 round 3" and "A8 round 4".

A7 keeps the loop/work-area history when Play consumes the playhead's
prepared start. During playback the producer replenishes one prepared voice
between ring fills. A forward wrap installs that start's ready histories;
continuous entry from before the work area keeps its uninterrupted phase
history. Prepared histories and source windows remain bounded to three
positions. Preparation never advances meters, retune envelopes, or the
current vocoders. The first input chunk, including its deterministic
analysis pre-roll, is prepared before the optional long look-ahead.

A static level/fade edit retains a voice's prepared history only when its
source and complete map key agree. That key now includes each level's span,
on state, group on states, and audio-muted input as well as its existing
source, trim, speed, remap, conform, and rate inputs. Animated levels and
nonconstant maps invalidate prepared histories on a revision: a map
expression may depend on another edited property. Partially prepared starts
install only their ready voices; other voices take the existing no-loss
start hold. No missing voice is faded in late to improve these measurements.

The app-level trace exposed two additional causes of missed preparation:

- A callback publishes the first clock line before its device presentation.
  Pause in that interval now stays at the epoch's target instead of
  extrapolating backwards before it (or around the loop).
- Audio can stop a fraction of a frame past the displayed frame. Paused
  preparation warms that composition frame first, then retains the exact
  sample position for raw seeks and the work-area boundaries.

The minimal presenter change is `pm-node/src/present/audio.rs`: Play sends
SetLoop, Seek, and Play as one ordered batch through `Playback::commands`.
Previously each synchronous command could trigger another paused bounding
step before the next command arrived. The batch preserves command order,
error reporting, start timing, and the existing levels watch. A failed
command returns its error and leaves earlier commands applied.

Regressions compare both channels exactly: repeated prepared loop starts
against a fresh offline render; level edits against the edited offline mix;
map/span edits and animated maps invalidating histories; a pause before the
first device presentation; preparation of both displayed and raw seek
positions; and uninterrupted stretched PCM through the first entry into a
loop. The 16-voice level-edit regression requires the first 512-frame buffer
within one callback and asserts no source starve or committed drop. The
existing loaded remap comparisons retain their zero-underrun assertions and
bit equality.

The paired app harness is `engine/crates/pm-audio/tools/playback-gates.ts`.
It opens hidden isolated apps under `pm-electron-lock`, using the same
10 s stereo WAV, sixteen speed-1.5 voices, and work area [4 s, 6 s) for
both engines. It measures 60 s of continuous looping after a 1 s settling
period, then 30 edit-and-Play requests in the same renderer task. Before a
level/speed edit it seeks to 4 s and allows 250 ms for preparation;
`PM_AUDIO_START_WAIT_MS=0` alternates previously unprepared 2 s / 3 s
same-task seek/edit/Play requests.
`PM_AUDIO_LOAD_THREADS=4` runs four joined busy-loop processes at the
harness's inherited niceness throughout the stressed capture. Output is
scheduled device presentation through WebAudio's output timestamp or the
native device clock and measured host offset, without acoustic loopback.
Native traces also record the pre-edit prepared positions and first-buffer
timing. Generated media, app profiles and raw captures stay outside Git.

Reproduce with the A6 app/addon build commands above, then:

```sh
L=~/Library/Caches/powermove-engine/bin/pm-electron-lock
$L nice -n 15 bun engine/crates/pm-audio/tools/playback-gates.ts \
  /tmp/pm-audio-start.wav /tmp/pm-audio-a7-plain.json 30
PM_AUDIO_LOAD_THREADS=4 $L nice -n 15 bun \
  engine/crates/pm-audio/tools/playback-gates.ts \
  /tmp/pm-audio-start.wav /tmp/pm-audio-a7-stress.json 30
PM_AUDIO_START_EDIT=speed PM_AUDIO_LOOP_SECONDS=0 $L nice -n 15 bun \
  engine/crates/pm-audio/tools/playback-gates.ts \
  /tmp/pm-audio-start.wav /tmp/pm-audio-a8-speed.json 30
PM_AUDIO_START_WAIT_MS=0 PM_AUDIO_LOOP_SECONDS=0 $L nice -n 15 bun \
  engine/crates/pm-audio/tools/playback-gates.ts \
  /tmp/pm-audio-start.wav /tmp/pm-audio-a8-immediate.json 30
```

### Paired app latency: delivered and remaining (historical, round 1–2)

The final speaker captures used a 48 kHz device, 256-frame native callbacks,
15.625 ms native device/stream latency, and WebAudio `baseLatency` 5.333 ms.
Each row has 30 requests per engine. Times below are ms from `PM.play` to
scheduled presentation of the first program sample:

| Scenario | Engine | p50 | p95 | max |
| --- | --- | ---: | ---: | ---: |
| Level edit, playhead prepared before the edit | Today's app | 30.920 | 32.518 | 32.529 |
| Level edit, playhead prepared before the edit | Native | 28.547 | 31.604 | 31.784 |
| Speed edit invalidating every history | Today's app | 29.633 | 32.331 | 32.434 |
| Speed edit invalidating every history | Native | 143.900 | 146.738 | 146.750 |
| Same-task unprepared seek, level edit, Play | Today's app | 31.011 | 32.410 | 32.515 |
| Same-task unprepared seek, level edit, Play | Native | 171.279 | 186.590 | 203.590 |

The level-edit case meets all three comparison points. **A8 is still a
blocker for invalidated or unprepared starts.** The native first-buffer
trace measures up to 112.988 ms from audio-control application to queueing
after a speed edit, and 113.346–169.195 ms for the immediate unprepared
seeks. These captures have consistent 15.625 ms native clock/status latency
in all 30 samples and zero page errors; this failure is independent of the
Bluetooth metadata discrepancy below. Cold starts preserve every initial
sample through the no-loss hold. Meeting today's latency still requires
moving the serial per-voice map/pre-roll work off the producer or reusing
its exact evaluated maps; faster dispatch and retained static histories do
not close the cold case.

One earlier stressed app capture switched output configuration between the
paired launches: WebAudio reported 168 ms output latency, native audio
status reported 160 ms, but the renderer's clock carried 15.625 ms. The
native status retains the device descriptor from opening, while playback
watches the current default output, so this is evidence of inconsistent
metadata during output changes, not proof of which latency is acoustically
correct. Its raw renderer-clock comparison (172.912/175.511/176.004 ms today,
37.753/42.897/43.611 ms native) is **not a latency pass**. Reconstructing the
native first-buffer presentation using the reported status latency instead
gives 182.079/187.272/187.986 ms. That alternative is an inference, also not
a same-device pass. Both paths recorded zero ring underruns for their 60 s
loop windows; native also recorded zero decode starvation and device
overloads. A repeat with consistent output metadata is reported separately.

### A7 real-device loop gate and repeated stressed latency (historical, round 1–2)

The final plain capture and the repeated stressed capture each played
sixteen 1.5x voices over [4 s, 6 s) for 60 s: **zero loop-window ring
underruns in both today's app and the native engine**. Native also counted
zero source-starved renders and zero device overloads in both windows. The
stressed capture used four busy-loop processes. The repeat had consistent
15.625 ms native clock/status latency in every sample; WebAudio reported
24 ms output latency and a 5.333 ms base latency, as in the plain capture.

| Level edit under stress | p50 (ms) | p95 (ms) | max (ms) |
| --- | ---: | ---: | ---: |
| Today's app | 30.034 | 32.632 | 32.635 |
| Native | 28.530 | 32.250 | 32.938 |

P50 and p95 meet today's latency. Maximum latency is **0.303 ms slower**,
so this row also misses the brief's strict every-number requirement. It is
not rounded into a pass. Combined with the cold-start rows above, A8 remains
open despite the improvement for already prepared static level edits.

### Final Rust validation and qualification (historical, round 1–2)

The final binary passed **30/30 plain and 30/30 stressed playback suites**,
35 tests each: 2,100 executions, zero failures and none ignored. Stress is
`PM_AUDIO_PLAYBACK_STRESS=1` (two shared CPU contention threads and 8 ms cold
decode delay). Suite times were 23.04–23.45 s plain and 23.09–23.42 s
stressed. The 360 mapped comparisons of 80,000 stereo frames remained exact
(57,600,000 sample values), with zero source starvation and ring underruns
in those comparisons. The original simulated-device patience and all
comparison tolerances are unchanged.

| Sixteen-voice level edit → first 512-frame buffer | Starts | p50 (ms) | p95 (ms) | max (ms) |
| --- | ---: | ---: | ---: | ---: |
| Plain | 120 | 0.719 | 1.274 | 1.683 |
| Stressed | 120 | 0.700 | 0.839 | 2.701 |

Every start met the one-callback 10.667 ms bound and the edited offline PCM
comparison. Before those final repetitions, one 35-test suite during
concurrent builds failed the loaded-remap zero-underrun assertion: one ring
underrun, zero source-starved renders, and zero differences in its 80,000
compared frames. The final consecutive runs pass the same assertion; the
earlier scheduling failure remains evidence for review, not an allowed
difference.

Targeted checks pass: 52 library tests (one existing ignored catch-up
measurement), 11 decode-readiness, 14 mixer, 6 stretch and all 37 presenter
unit tests. Strict release clippy for all pm-audio/pm-node targets, pm-audio
formatting, and the normal production-profile pm-node library check with
test hooks disabled pass. App, pm-wasm and native addon builds pass.
Validation used pinned Rust 1.96.1, kache, four build jobs, niceness 15 and
four test threads, with the existing local `CARGO_PROFILE_RELEASE_LTO=off`
setting for test/addon links. The normal production check uses the default
release configuration; no successful thin-LTO test link is claimed. The
30-iteration reproduction commands in the A4/A6 sections still apply.

### A8 round 2: parallel preparation and measured late joins (historical, round 1–2)

The serial cold path was CPU-bound. In an opt-in timing capture of sixteen
stretched voices at 48 kHz, the eleven actual speed changes (the first of
twelve samples did not change speed) averaged approximately:

| Stage | Wall time (ms) |
| --- | ---: |
| Control building / delivery before application | 12.3 |
| Source bounds, including their map evaluations | 36.0 |
| Vocoder history map evaluations | 25.1 |
| Signalsmith construction/reset, output seek and processing | 45.1 |
| Cached source reads and copies | 0.3 |
| Explicit no-loss source wait | 0.0 |
| Other render work, allocation, scheduling and timer granularity | 2.8 |
| Queue to scheduled device output | 25.1 |

The applied-control-to-queue interval averaged 109.4 ms. These are timed
wall scopes, not sampled CPU counters; scheduling inside a scope is charged
to that scope. Reads were already cached in this edit scenario. The
instrumented end-to-end p50 was 147.0 ms, close to the earlier uninstrumented
143.9 ms capture. The source hold was not the cause of this regression.
`PM_AUDIO_PROFILE=1` enables the counters in `StartTiming.preparation`;
normal playback leaves timing disabled and the callback adds no profiling
work.

`preparation.rs` owns a process-wide worker pool capped at
`max(1, available_parallelism - 2)` (fourteen on the measured machine).
Workers use reduced thread priority: utility QoS with relative priority -5
on macOS, and niceness 15 on Linux. Project edits and seeks submit work at
control application, before Play's render turn. Batched transport commands
submit only their final target; intermediate Pause/SetLoop positions do not
launch obsolete histories. Playhead starts and the nearest sounding voices
come first. Loop/work-area replenishment uses at most one background job
at a time, preserving A7's render reserve. Stale document/seek generations
cancel at map/read boundaries, and completed jobs must still match their
original history allocation before installation.

Each job receives an evaluator from the configured factory. The native
presenter's one-line `set_standard_evaluator` opt-in identifies its standard
evaluator with the expression engine and no custom hooks. Constant maps
compile the original per-ancestor arithmetic, active/span tests, fades and
source-end calculation; deriving a single intercept would change rounding.
Those constant points also remove repeated content resolution from playback
and source bounding. Expressions retain full evaluation. Custom evaluator
factories retain their hooks and invalidate prepared histories on every
project revision. The ordinary audible render still reports failed
preparation's decode/evaluation errors.

The deterministic prefix is unchanged. Signalsmith reports 2,880 input and
2,880 output latency samples; the existing deterministic prefix is 11,520
samples at 48 kHz. Trying 0, 5,760, 8,640 and 11,504 samples at 0.75x and
1.5x, from 2, 3 and 4 seconds, changed every one of the 8,192 compared
stereo values in each of 24 probes. Even the nearest shorter control-aligned
candidate changes the existing offline PCM. The ordinary mapped playback
comparisons continue against the unchanged offline algorithm.

Parallel preparation with complete first buffers alone still measured
35.464/37.962/37.962 ms for speed edits and 38.162/46.062/46.062 ms for
unprepared seeks (p50/p95/max in twelve-start diagnostic captures). That
miss enabled the requested A4 alternative: an actually ready audible voice
starts the run, and late voices omit a counted prefix and join at the
current project sample. Workers catch up through real input from the
original start, retaining its offline vocoder phase. During this short
join period the ring keeps two callbacks ahead instead of the whole normal
look-ahead. A paused partial start discards its consumed prepared entry;
an inactive/out-of-span prepared voice cannot release other cold voices.
`StartTiming.lateVoiceFrames` records the omitted frames per voice, and the
existing starvation/drop counters record this loss. Prepared starts retain
complete PCM and have no such omission. Exports continue to wait.

The added sixteen-voice cold test verifies every stereo sample: each voice
is absent only for its recorded prefix and then equals the unchanged
offline render from the original start. It preserves the one-callback
first-buffer bound. Existing mapped comparisons and their patience,
zero-starvation/underrun assertions and tolerances are unchanged.

A subsequent prefetch fix queues all source blocks for the bounded canonical
prefix before a preparation worker reads its first block. Constant maps
use two exact endpoint evaluations and a conservative reader/latency pad;
dynamic maps reuse the bounded span traversal. It does not require decoding
an entire file. A gated AAC-source regression observed only one requested
block before the fix and two concurrent requests after it. Its extended
wall deadline isolates this queue observation; existing playback patience,
PCM comparisons and underrun assertions remain unchanged. Completed tasks
also pin their history allocation with a strong reference until accepted,
preventing a recycled allocation address from accepting obsolete work.

#### Latest completed paired captures (historical, round 1–2)

Thirty starts per engine, sixteen stretched voices, 48 kHz, 256-frame
callbacks. Native status and device-clock latency agreed at 15.625 ms;
WebAudio output latency was 24 ms. These are scheduled device presentation
times, not an acoustic loopback. Level-edit rows additionally looped
[4 s, 6 s) for 60 seconds per engine; stress used four busy-loop processes.
Each start continued for 120 ms after first onset to observe late joins.
Unprepared cases alternated seeks to 2 and 3 seconds with seek/edit/Play in
one renderer task; neither target was prepared before the edit.

| Case | Today p50/p95/max (ms) | Native p50/p95/max (ms) | Every percentile passes |
| --- | ---: | ---: | --- |
| Prepared level edit, plain | 28.938/31.735/31.821 | 24.969/28.193/28.509 | Yes |
| Speed edit, plain | 30.324/32.425/32.621 | 30.163/32.651/35.038 | No |
| Unprepared seek/edit, plain | 30.010/32.415/32.515 | 29.453/31.390/31.859 | Yes |
| Prepared level edit, stressed | 30.137/32.130/32.719 | 25.825/28.560/28.682 | Yes |

The latest speed-edit p95 misses today by
0.225667 ms and its maximum by
2.416667 ms. These are raw failures, with no
rounding or noise exemption. Both completed 60-second loop pairs have zero
underruns in both engines, and native has zero loop-window source
starvation and overloads. A8 is still open.

| Cold case | Longest omitted voice prefix p50/p95/max (ms) | All voices at output p50/p95/max (ms) | Today node-start spread p50/p95/max (ms) |
| --- | ---: | ---: | ---: |
| Speed edit, plain | 32.000/48.000/48.000 | 62.270/77.988/79.424 | 0.000/0.000/0.000 |
| Unprepared seek/edit, plain | 26.667/32.000/32.000 | 55.742/61.482/61.492 | 0.000/0.000/0.000 |

All-voices output is first scheduled output plus the longest counted
omission. This exposes the partial-mix cost beyond first onset; it is not
a measurement of subjective audibility. Today scheduled all sixteen nodes.
The plain prepared baseline did show a 5.333 ms maximum node-start spread;
it is not assumed to be zero in every capture. Prepared native starts
omitted no frames.

#### Three paired stressed repeats and all-ready controls (historical, round 1–2)

The following completed repeats precede the source-prefetch follow-up.
Each row contains thirty starts per engine plus a 60-second loop per
engine. They are evidence for the pool implementation, not final-revision
qualification. Every loop had zero underruns; native had zero loop-window
source starvation and overloads.

| Earlier stressed repeat | Today p50/p95/max (ms) | Native p50/p95/max (ms) |
| --- | ---: | ---: |
| 1 | 29.933/32.325/32.433 | 25.370/28.084/28.333 |
| 2 | 30.128/32.629/32.671 | 26.549/28.524/28.699 |
| 3 | 30.035/32.336/32.770 | 25.127/28.048/28.405 |

The maxima ranges do not overlap: 32.433–32.770 ms today and
28.333–28.699 ms native. The revised engine was faster in all
three repeats. This does not establish that the earlier 0.303 ms native
excess was noise; no noise exemption is claimed. Only the first repeat
completed after the prefetch follow-up. The remaining two final repeats
and final stressed cold cases were stopped before completion.

Earlier optimized all-ready controls temporarily removed only the
first-ready exit from the join wait. Their source and addons were restored
byte-for-byte afterward. They retained complete audio and still missed
today, justifying the requested A4 experiment:

| Earlier all-ready control | Today p50/p95/max (ms) | Native p50/p95/max (ms) |
| --- | ---: | ---: |
| Speed edit | 30.236/32.234/32.427 | 32.639/35.330/35.540 |
| Unprepared seek/edit | 30.119/32.321/32.427 | 33.138/35.496/35.590 |

The all-ready control was not repeated after the source-prefetch fix. A4's
up-to-48 ms omitted prefix in the latest speed capture remains a measured
loss relative to today's complete start, even where first onset passes.

#### Validation status: incomplete repetition gate (historical, round 1–2)

The last full playback smoke run passed all 37 tests. Targeted checks
passed 56 library tests (one existing ignored measurement), 11 decode
readiness tests, 14 mixer tests, 6 stretch tests and 37 presenter tests.
The smoke and direct tests precede the final conservative reader-tap bound
adjustment; the final revision rebuilt its test executable and production
addon, and passed strict release clippy for pm-audio/pm-node all targets.
The earlier default production-profile pm-node check without test hooks
passed. Test/addon links used the existing local
`CARGO_PROFILE_RELEASE_LTO=off` workaround; a new default thin-LTO runtime
link is not claimed.

The repetition history is retained rather than combined into a pass:

- An initial interrupted campaign had passing runs, then cold/warm WAV
  fixture paths were isolated to prevent concurrent tests sharing a file.
- The next campaign passed eight plain and eight stressed suites, then
  plain run nine failed the existing loaded-remap zero-underrun assertion
  once. Its source-starvation count was zero and its 80,000-frame PCM
  comparison was exact. This led to the concurrent-prefix-queue fix and
  its deterministic gated-source regression; the failed log is preserved.
- The final-revision 30/30 plain and 30/30 stressed campaign had **zero
  completed runs** when the mandatory under-40-GiB disk guard stopped work.
  The final all-ready diagnostic had not started. The build target was
  deleted, but free disk remained below the guard. No final repetition
  pass is claimed.

Existing mapped comparisons remain against the unchanged offline PCM;
no tolerance, source-starvation assertion or ring-underrun assertion was
relaxed. Completing all final captures, the all-ready control and sixty
repetition suites remains required before sign-off. Only one additional
line changed pm-node's presenter in this round: its evaluator-factory
opt-in. No render.rs change was needed.

### A8 round 3: complete and simultaneous starts

The round-2 ordinary-start omission path is superseded. Play now waits
until every stretched voice has complete first-output PCM. The mix is
published in one interleaved buffer at one project sample. A CPU preparation
job cannot release a partial mix. Only source input still missing after
120 ms of device callbacks can use the A3 per-voice safety net; exports
continue to wait for input.

The start's independent workers still begin on Project/Seek, before Play.
Repeated voices can now share an immutable prefix of their stretched source
signal. Reuse requires the same source identity, exact voice bounds and
bit-identical compiled static map at every ancestor, including active state
and conform factor. The standard evaluator opts in; hooks, expressions and
animated maps keep their configured evaluation. Each voice still applies
its own gain, fades, layout matrix and strip in the original order.

The prefix contains 4,096 frames at 48 kHz (85.333 ms), scales with project
rate, and rounds up to a ring chunk. A representative job prepares it first.
Dependent jobs then prime independent vocoder histories while complete PCM
is already available for every voice. The producer uses that PCM until
the independent history can continue. Failed input reads retry from the
original start's phase. The canonical 11,520-sample pre-roll at 48 kHz is
unchanged; the existing short-prefix experiment and exact mapped
comparisons remain in place.

The process-wide pool remains capped at max(1, cores - 2), fourteen workers
on the measured machine. Foreground preparation requests user-interactive
QoS; boundary replenishment keeps one utility job at a time. The start's
control batch and producer request the same foreground class through scoped
guards, then restore the previous effective class. The guards stay on their
creating thread, and a nested-scope regression verifies restoration.

A live gain edit preserves the immutable prefix and original phase across
retune glides, including a canonical-grid boundary. Cancelled histories
return to the audible mix instead of receiving new detached jobs.
Replacing an evaluator clears the previous factory's histories and PCM.
Deterministic regressions cover both cases, different gain/fade/map
identities, independent-history handoff and two grouped decoder-stalled
voices recovering after the A3 bound.

The native I/O request is now 64 frames through 48 kHz, 256 above 48 through
96 kHz, and 512 above 96 kHz, clamped to the device's supported range.
Unknown ranges keep the device default. This reduces callback phase and
scheduling lead: the callback copies premixed PCM and publishes the clock,
while DSP and allocation remain on the producer/workers. Today's measured
WebAudio base latency is 5.333 ms, equivalent to 256 frames at 48 kHz.
The smaller native callback has a tighter real-time deadline; the paired
stressed loop captures below qualify this choice on the measured device.

StartTiming records each voice's first project sample, spread, summed
omitted prefix, A3 safety-net use and callbacks that ran out of program PCM
after first onset. Captures assert sixteen voices at the target sample and
zero for every loss/spread counter. The existing cumulative ring counter
still includes intentionally empty paused/start-wait callbacks; the A7
loop gate measures its change inside the steady playing window.
prefillReadyUs additionally records worker completion before the
producer's readyUs, separating compute from publication handoff.

#### All-ready scope profile

The baseline profile enabled the all-ready policy before adding shared
prefill. In eleven actual speed changes, the producer waited 7.260 ms.
Summed worker DSP took 63.668 ms; map evaluation took 0.127 ms and cached
reads 0.707 ms. A second split-scope capture measured DSP initialization
4.292 ms, output-seek priming 6.107 ms and processing 44.033 ms. Ready-to-queue
was only 0.091 ms in the baseline. These timed wall scopes identified
processing as the critical preparation cost.

The final opt-in captures separate these stages as follows (mean ms):

| Stage | Final speed edit | Final unprepared seek/edit |
| --- | ---: | ---: |
| Frontend/IPC/graph before audio command | 1.755 | 0.810 |
| Audio command channel delivery | 0.053 | 0.111 |
| Applied command to worker prefill ready | 3.697 | 3.646 |
| Worker ready to producer continuation | 0.010 | 0.077 |
| Producer join/poll scope | 3.695 | 3.674 |
| First mix | 0.036 | 0.036 |
| Ready to queue | 0.096 | 0.387 |
| Queue to callback | 0.622 | 0.506 |
| Callback to HAL presentation host time | 2.340 | 2.340 |
| Explicit source hold | 0.000 | 0.000 |
| Accepted worker DSP before first queue, summed | 6.709 | 31.146 |
| Accepted DSP initialization, summed | 0.323 | 1.587 |
| Accepted DSP seek, summed | 0.639 | 2.815 |
| Accepted DSP processing, summed | 5.744 | 26.735 |
| Accepted map evaluations, summed | 0.002 | 0.001 |
| Accepted source reads, summed | 0.050 | 0.213 |

Worker sums overlap the producer join. With prefill, only jobs accepted
before the first queue appear in that snapshot; the remaining independent
jobs finish afterward. The table does not claim their total CPU work was
eliminated. Cached prefill completion can precede Play, so handoff intervals
clamp their origin to command application. Speed scope averages exclude
the first unchanged-speed sample (eleven actual changes); the seek profile
has twelve samples. Normal qualification captures leave profiling off.

Smaller foreground pools did not improve the measured wait: 8/10/12-worker
sweeps averaged 7.023/6.986/6.159 ms, so the fourteen-worker cap remains.
The start-priority guards and shorter I/O phase complement the prefill;
the final captures measure their combined behavior.

#### Final paired qualification

Each row has thirty starts per engine on the same sixteen-voice project.
The plain and three stressed level-edit rows each include a 60-second
[4 s, 6 s) loop per engine. Stress uses four busy-loop processes. Cold
seek/edit cases alternate 2 s and 3 s, neither in the pre-edit prepared
start list; seek/edit/Play occur in one renderer task. Prepared cases
allow 250 ms before the edit. Starts continue for 120 ms after first clock
observation, covering prefill handoff. Each speed sequence retains its
first unchanged-speed sample and twenty-nine actual changes.

The clock reports scheduled device presentation, using the same method
for both engines. This is not an acoustic loopback. All native samples
use 48 kHz, 64-frame callbacks and 15.625 ms reported device latency;
clock and status metadata agree. Today's base latency is 5.333 ms
(256 frames) and output latency is 24 ms. The two all-ready control rows
use the same shipping policy and binary, with every completeness assertion
enabled.

| Case | Today p50/p95/max (ms) | Native p50/p95/max (ms) | Raw misses |
| --- | ---: | ---: | --- |
| Prepared level edit, plain | 29.703/32.426/32.432 | 19.680/20.575/20.619 | None |
| Speed edit, plain | 31.027/32.717/32.736 | 24.026/24.785/24.891 | None |
| Unprepared seek/edit, plain | 29.823/32.411/32.604 | 24.040/24.710/25.803 | None |
| Prepared level edit, stress 1 | 29.236/32.237/32.404 | 20.093/20.702/20.740 | None |
| Prepared level edit, stress 2 | 30.137/32.436/32.529 | 19.973/20.685/20.738 | None |
| Prepared level edit, stress 3 | 30.531/32.533/32.630 | 19.981/20.608/21.061 | None |
| Speed edit, stressed | 29.037/32.534/32.636 | 23.806/24.828/24.921 | None |
| Unprepared seek/edit, stressed | 29.322/32.527/32.660 | 23.765/24.862/25.965 | None |
| All-ready speed control | 30.333/31.821/32.216 | 23.998/24.769/24.810 | None |
| All-ready seek control | 30.125/32.509/32.617 | 23.950/24.706/26.830 | None |

All ten final paired cases meet today at every requested percentile.
Every final native start has sixteen voices at its exact target sample,
zero start spread, zero omitted frames, zero A3 safety-net use and zero
program underruns. All four paired loop windows have zero underruns in
both engines, with zero native source starvation and device overloads.

| Stressed repeat | Today p50/p95/max (ms) | Native p50/p95/max (ms) |
| --- | ---: | ---: |
| 1 | 29.236/32.237/32.404 | 20.093/20.702/20.740 |
| 2 | 30.137/32.436/32.529 | 19.973/20.685/20.738 |
| 3 | 30.531/32.533/32.630 | 19.981/20.608/21.061 |

Observed maximum ranges: **32.404–32.630 ms today**, **20.738–21.061 ms native**. They do not overlap. The three distributions are reported without a noise exemption for earlier misses.

The prepared level-edit capture also records transport stages without DSP
profiling overhead (thirty plain starts, p50/p95/max in ms). Its immutable
prefill is already ready before Play; zero-valued disabled DSP scopes are
not interpreted as zero CPU work.

| Prepared level-edit stage | p50/p95/max (ms) |
| --- | ---: |
| Frontend/IPC/graph before audio command | 1.090/1.192/1.217 |
| Audio command delivery | 0.093/0.114/0.116 |
| Applied command to all-ready | 0.004/0.005/0.005 |
| Ready to queue | 0.199/0.228/0.517 |
| Queue to callback | 0.356/1.196/1.302 |
| Callback to HAL presentation host time | 2.337/2.343/2.343 |

#### Retained failed qualification attempts

An intermediate 256-frame/2,048-frame-prefill speed pair measured
28.950/32.006/33.410 ms native against 28.818/31.915/32.202 ms today.
Its raw p50/p95/max misses were +0.131667/+0.090667/+1.207667 ms.
After prefill review fixes, the first 128-frame qualification pass completed
four pairs before the speed maximum still failed. Those data remain
separate from the final revision:

| Earlier 128-frame case | Today p50/p95/max (ms) | Native p50/p95/max (ms) | Raw misses |
| --- | ---: | ---: | --- |
| Prepared level edit | 31.033/32.533/32.812 | 22.309/23.143/23.241 | None |
| Speed edit | 29.419/31.609/32.421 | 26.209/28.651/32.587 | max +0.165667 ms |
| Unprepared seek/edit | 29.417/31.826/32.327 | 25.785/26.976/28.437 | None |
| Stressed prepared edit | 30.826/32.033/32.535 | 21.652/22.892/23.160 | None |

The earlier maximum's trace contained 6.336 ms of delivery/build work,
6.557 ms from applied command to readiness, 0.103 ms ready-to-queue,
0.302 ms queue-to-callback, 3.664 ms callback host lead and 15.625 ms
device latency. It had zero omission/spread and no host-offset error.
The qualification stopped at the next source guard before another pair
or any repetition suite. The final source adds scoped start priorities,
an independent prefill-ready timestamp and the 64-frame request; all final
captures and repetitions were restarted. Earlier failed numbers remain
raw failures rather than being reclassified as noise.

#### Final repeated correctness and delivery checks

The frozen final playback executable passed **30/30 plain and 30/30 stressed
suites**, 38 tests each (2,280 executions). Stress includes the shared CPU
load and 8 ms cold-decode delay. The original mapped comparisons retain
their assertions and patience: 360 comparisons of 80,000 stereo frames,
57,600,000 sample values, exact PCM and zero mapped source starvation or
ring underruns. The cold sixteen-voice comparison also checks complete
output through its independent-history handoff.

The earlier round-2 plain-run-nine loaded-remap underrun was not reproduced
in these sixty final runs. Its failed log remains preserved. The bounded
prefix-input queue regression and source-prefetch fix remain in place;
no tolerance, mapped case or underrun assertion was relaxed.

| First 512 frames, sixteen voices | p50/p95/max (ms) |
| --- | ---: |
| plain, prepared level edit, 120 starts | 0.343/5.522/10.062 |
| plain, cold seek/speed edit, 120 starts | 4.640/8.287/11.810 |
| stress, prepared level edit, 120 starts | 0.330/3.944/5.703 |
| stress, cold seek/speed edit, 120 starts | 4.495/9.064/14.046 |

The elapsed-time observer also reports values above the 10.667 ms first-buffer
target, despite every existing queue/PCM assertion passing. The polling loop
checks queue readiness, tests the deadline separately and prints elapsed
time afterward. These are not an atomic publication timestamp: the raw data
cannot distinguish a late queue from a delayed observer. No test bound is
relaxed, and these exceedances are retained without a noise exemption. They
are separate from the paired app's scheduled-presentation gates above.

| Suite observation | Raw elapsed (ms) | Excess above 10.667 ms |
| --- | ---: | ---: |
| plain-22, cold=true, line 24 | 11.810 | +1.143333 |
| stress-15, cold=true, line 18 | 14.039 | +3.372333 |
| stress-15, cold=true, line 20 | 13.831 | +3.164333 |
| stress-15, cold=true, line 24 | 12.020 | +1.353333 |
| stress-23, cold=true, line 18 | 14.046 | +3.379333 |
| stress-5, cold=true, line 24 | 11.410 | +0.743333 |

Final targeted checks pass: 61 library tests (one existing ignored catch-up
measurement), 11 decode-readiness, 14 mixer, 6 stretch and 37 presenter
tests. Strict release clippy for pm-audio/pm-node all targets, formatting,
the default production-profile pm-node library check without test hooks
and the production native addon build pass. Test/addon links use the
existing local CARGO_PROFILE_RELEASE_LTO=off workaround; no fresh default
thin-LTO runtime link is claimed.

The only additional pm-node source change in round 3 is the four-line
control-batch priority guard in present/audio.rs. Over the whole branch,
`pm-node/src/present/audio.rs` changes in three commits, 31 insertions and
19 deletions: 8345dba5 (SetLoop/Seek/Play as one ordered command batch),
21fe9c6c (the `set_standard_evaluator` opt-in) and d2ccfadb (the start
priority guard). Round 4 changes nothing in pm-node. No render.rs change is
needed. The capture/repetition manifest verifies unchanged native addons
and playback executable throughout the final campaign. Raw evidence stays
outside the repository.

### A8 round 4: distinct voices and review follow-ups (2026-10-18)

The round-3 qualification used sixteen bit-identical voices, the best case
for the shared immutable head: one representative job served all sixteen.
Round 4 qualifies starts where no voice can share a head, and fixes the
review's correctness follow-ups. Commits: e92a267e (engine), 30b16308
(playback tests), ad3baecb (harness), 943b81fd, 987ef700 and fae25da8
(test synchronization and the first-buffer check). No pm-node, protocol or
render.rs change.

**Changes.**

- *Distinct-voice harness mode.* `PM_AUDIO_START_DISTINCT=trim` trims voice
  *i* by *i* × 61.3 ms, so every voice has its own stretched signal.
  Native samples must report `prefillReadyUs` 0 (no shared head) in
  addition to the existing completeness assertions. Reproduce with the
  commands above plus `PM_AUDIO_START_DISTINCT=trim`.
- *Buffered start lead.* A start's own job now renders 1,024 frames
  (rate-scaled, a whole number of ring chunks) of the voice's continuous
  vocoder output before reporting ready, kept in the history's output
  buffer. Without it, sixteen distinct voices whose second vocoder step fell
  inside the first 512 frames (for example 2 s at 1.6x) made the producer
  run sixteen vocoder steps serially between the first and second ring
  chunk: about 4 ms with only one 256-frame chunk queued. The lead is the
  same deterministic stream, so PCM is unchanged; the existing bit-exact
  comparisons run through it.
- *Per-voice A3 safety net.* When the A3 bound fires, only voices whose own
  first-chunk input is still missing are marked late. A healthy voice whose
  CPU job is still queued stays on its live history, which the ordinary
  render primes by the canonical pre-roll: complete from its first sample.
- *Bounded wait at an exhausted shared head.* Jobs carry a claim. When a
  shared head ends before a voice's independent history, the producer claims
  every such history no worker has started and primes it itself from the
  head's original start (`prepare_join`), bit exact. Only a history a worker
  is already running is awaited, so the wait is bounded by running work, not
  by a saturated pool's queue. Paused synchronous preparation uses the same
  claim, so no history is primed twice.
- *Producer publication times.* `StartTiming.publications` records the
  cumulative frames and host time of the run's first 16 ring pushes, taken
  by the producer after each push. The headless check now waits for the
  buffer and reads the producer's own publication time, so a descheduled
  observer no longer decides the result. Prepared starts (no vocoder work
  after Play) must publish 512 frames within one callback by that clock.
  Cold starts report it per start; see the repeated suites below for why
  it is not asserted, and the decision list.

**New regressions.** The first three fail with their fix removed (checked
by mutation); the map and distinct-voice tests guard existing behavior:
`the_a3_safety_net_omits_only_voices_whose_own_input_is_missing` (a
genuinely undecoded source beside a shared head and a CPU-pending healthy
voice; first 8,192 frames bit exact against the mix without the late voice),
`an_exhausted_head_never_waits_for_an_unstarted_history` (two unstarted
histories claimed, the running one awaited, 8,192 frames past the head bit
exact), `a_start_job_buffers_its_opening_output` (the lead is read with
panicking map/read callbacks at three targets),
`compiled_constant_maps_match_through_precomps_and_embedded_video` (one- and
two-level linear-parent precompositions and a video's embedded audio; every
`Point` field and map point bit-identical to the full evaluator), and the
playback tests `edit_then_play_reuses_sixteen_distinct_stretched_voices`
and `cold_starts_of_sixteen_distinct_voices_are_complete_and_simultaneous`
(sixteen voices at the target, zero spread, omission, A3 use and program
underruns, no shared head, and all 8,192 stereo frames equal to the sum of
per-voice offline renders).

#### Paired qualification, round 4 revision

Same method as round 3: thirty starts per engine per row, scheduled device
presentation (no acoustic loopback), hidden isolated apps under
`pm-electron-lock`, four busy-loop processes for stress. Revision
ad3baecb; the playback executable and both addons were hashed
before and after the campaign; engine source and addons are unchanged
through fae25da8. Every native sample used 48 kHz, 64-frame
callbacks and 15.625 ms device latency; today's base latency was 5.333 ms and
output latency 24 ms.

Identical voices (round-3 project, re-measured on this revision):

| Case | Today p50/p95/max (ms) | Native p50/p95/max (ms) | Raw misses |
| --- | ---: | ---: | --- |
| Prepared level edit, plain | 30.429/32.720/32.825 | 19.875/20.563/20.698 | None |
| Speed edit, plain | 30.334/31.737/32.424 | 23.932/24.863/24.933 | None |
| Unprepared seek/edit, plain | 30.020/32.520/32.624 | 23.817/24.605/25.091 | None |
| Prepared level edit, stress 1 | 29.234/31.634/32.434 | 19.844/20.446/20.613 | None |
| Prepared level edit, stress 2 | 29.933/32.536/32.649 | 19.645/20.501/20.577 | None |
| Prepared level edit, stress 3 | 30.634/32.126/32.231 | 19.817/20.758/20.789 | None |
| Speed edit, stressed | 29.936/32.336/32.436 | 24.171/25.010/25.246 | None |
| Unprepared seek/edit, stressed | 29.432/32.509/32.616 | 24.007/25.055/25.449 | None |
| All-ready speed control | 29.318/32.329/32.616 | 24.072/24.883/24.899 | None |
| All-ready seek control | 30.229/32.033/32.428 | 23.737/25.076/25.280 | None |

Sixteen distinct voices (no shared head in any native sample):

| Case | Today p50/p95/max (ms) | Native p50/p95/max (ms) | Raw misses |
| --- | ---: | ---: | --- |
| Prepared level edit, plain | 29.418/32.750/32.814 | 19.726/20.568/20.583 | None |
| Speed edit, plain | 30.516/32.406/32.606 | 27.531/28.173/28.833 | None |
| Unprepared seek/edit, plain | 30.017/32.416/32.614 | 27.509/29.437/29.934 | None |
| Speed edit, stress 1 | 30.532/32.337/32.628 | 27.342/27.813/27.826 | None |
| Speed edit, stress 2 | 29.837/31.734/32.137 | 27.468/27.903/27.932 | None |
| Speed edit, stress 3 | 30.336/32.237/32.737 | 27.053/28.945/29.555 | None |
| Unprepared seek/edit, stress 1 | 29.524/32.396/32.610 | 27.465/29.433/30.578 | None |
| Unprepared seek/edit, stress 2 | 29.424/31.928/32.520 | 27.479/29.221/29.292 | None |
| Unprepared seek/edit, stress 3 | 30.511/32.418/32.430 | 27.553/28.095/28.252 | None |
| Prepared level edit, stressed, 60 s loop | 30.836/32.622/32.624 | 19.787/20.671/20.699 | None |

All twenty paired rows meet today at p50, p95 and max.
Identical rows: 300 native starts, 300 with a shared head.
Distinct rows: 300 native starts, 0 with a shared head. In
all 600 native starts: voices at the target sample
[16], total omitted prefix 0 frames,
maximum start spread 0 frames, A3 safety-net
uses 0, program underruns 0,
callback sizes [64] frames.

The distinct project is the costly case: every voice runs the canonical
pre-roll from its grid start (11,520 samples plus up to one 16,384-sample
grid step). The margin to today is smaller than with identical voices but
positive at every percentile; the worst distinct maximum is
30.578 ms against today's
32.137–32.814 ms
range of maxima.

| 60 s loop window [4 s, 6 s) | Today underruns | Native underruns | Native starved renders | Native overloads |
| --- | ---: | ---: | ---: | ---: |
| final-plain | 0 | 0 | 0 | 0 |
| final-stress-1 | 0 | 0 | 0 | 0 |
| final-stress-2 | 0 | 0 | 0 | 0 |
| final-stress-3 | 0 | 0 | 0 | 0 |
| distinct-plain | 0 | 0 | 0 | 0 |
| distinct-loop-stress | 2 | 0 | 0 | 0 |

Today's WebAudio path counted 2
underrun events in the stressed distinct loop window; native counted none in
any window. This is reported as measured, not as a claim about today's
audible behavior.

Opt-in profiles (native only, twelve starts each, `PM_AUDIO_PROFILE=1`;
speed means exclude the first, unchanged-speed sample; summed worker scopes
overlap the producer wait):

| Mean ms | Distinct speed edit (n=11) | Distinct unprepared seek/edit (n=12) |
| --- | ---: | ---: |
| Producer wait for complete first output | 6.688 | 3.998 |
| Summed worker DSP | 57.270 | 30.840 |
|   initialization | 4.609 | 2.502 |
|   output seek | 5.953 | 3.128 |
|   processing | 46.690 | 25.201 |
| Summed map evaluation | 0.004 | 0.046 |
| Summed source reads | 0.405 | 0.208 |
| First mix | 0.044 | 0.041 |

Sixteen distinct jobs take about two job lengths: a headless probe with 14,
16 and 18 workers measured the same ~7 ms from Play to all-ready. That is
consistent with jobs beyond the twelve performance cores queueing or running
on the slower efficiency cores, but per-core placement was not profiled. The
pool size is unchanged.

#### Repeated suites, round 4 revision

The first campaign on ad3baecb failed its first stressed suite:
`a_decode_wait_below_the_bound_is_bit_exact` asserted
`blocked_decoders() > 0` at the turn its first starved render was counted,
but the render counts the miss before the decode worker it queued reaches
the test gate. 943b81fd waits for the worker to reach the gate (wall time
only; no device callback runs, so no buffered device frames are spent) and
changes nothing else; that failed log is kept beside the campaign. The
engine source and addons of 943b81fd are those of the paired captures.

On 943b81fd the first plain suite then failed the producer-timed bound
for a cold identical start (published 10.823 ms; applied-to-ready 9.611 ms
against about 3.7 ms unloaded) while the distinct cold test ran beside it on
the same pool and other lanes' foreground work kept the load average near
11. A locked six-suite check with the four sixteen-voice tests serialized
still had one cold identical start at 11.146 ms (ready-to-published
4.192 ms, the producer descheduled; load average about 13, other lanes'
node, dsymutil and corpus processes at niceness 0). 987ef700 therefore
serializes the four sixteen-voice tests (one shared pool) and asserts the
producer-timed bound only for prepared starts. Those logs are kept with the
campaign.

On 987ef700, after twelve plain and eleven stressed suites passed, stressed
suite 12 counted one ring underrun in the loaded time-mapped comparison
(`a remapped soundtrack: device ring underruns`, a 40 ms real-time device;
load average about 15). That assertion failed under machine load in rounds
1 and 2 too, but round 4 doubled the sixteen-voice tests whose bursts of
user-interactive pool jobs compete with another test's producer in the same
process, which no app does during its own playback. fae25da8 makes the
start tests hold a write lock that the two time-mapped comparisons share;
no assertion, patience or comparison changed.

On fae25da8 the frozen executable (40 tests: the 38 of round 3 plus the two
distinct tests) passed **30/30 plain and 30/30 stressed
suites** (2,400 executions) under `pm-electron-lock`,
with `PM_AUDIO_PLAYBACK_STRESS=1` for stress, `nice -n 15` and four test
threads. The mapped comparisons, patience, zero-underrun assertions and
bit-exact checks are unchanged.

Outside the lock, while another lane's Electron app and builds ran (load
average about 15), 5 of 10 stressed suites of the ad3baecb executable failed:
seven sixteen-voice starts published after the bound (11.301–25.475 ms; the
prepared ones too), and the existing real-time mapped comparisons counted one
or two ring underruns, and one forced-miss test missed its miss. Six stressed
suites of 943b81fd under the lock right afterwards all passed. The
producer-timed bound is therefore a real-time check: it holds on this
machine without a concurrent app, and fails, like the existing real-time
tests, when the test process (niceness 15) competes with that much
foreground load.

First 512 frames published, producer timestamps (p50/p95/max ms over all
starts in the sixty suites; the observer column is the latest elapsed time
at which the test thread saw the buffer):

| Suite, start | Starts | Published p50/p95/max | Observed max | Published over 10.667 ms |
| --- | ---: | ---: | ---: | ---: |
| plain, prepared level edit, identical | 120 | 0.373/2.383/4.443 | 16.283 | 0 |
| plain, cold seek/speed edit, identical | 120 | 4.186/6.489/7.986 | 20.500 | 0 |
| plain, prepared level edit, distinct | 120 | 0.267/0.381/0.543 | 5.471 | 0 |
| plain, cold seek/speed edit, distinct | 120 | 8.350/16.384/23.131 | 23.143 | 35 |
| stress, prepared level edit, identical | 120 | 0.367/2.424/5.320 | 15.411 | 0 |
| stress, cold seek/speed edit, identical | 120 | 4.195/7.193/8.026 | 21.276 | 0 |
| stress, prepared level edit, distinct | 120 | 0.269/0.494/0.610 | 3.718 | 0 |
| stress, cold seek/speed edit, distinct | 120 | 8.829/22.698/36.952 | 39.085 | 40 |

Prepared starts are asserted against the one-callback bound on the
producer's timestamp. Cold starts (identical and distinct) are reported
only: their latency is the paired app gate above. The round-3 observer readings above 10.667 ms (up to
14.046 ms) are consistent with observer delay: in these suites the observer
saw buffers up to the observed maxima above while the producer had published
them earlier.

Targeted checks on this revision: 65 library tests (one existing ignored
measurement), 11 decode-readiness, 14 mixer, 6 stretch, 5 sources, 2
layouts, 1 legacy-parity, 3 resampler and 37 presenter tests pass; strict
release clippy for pm-audio/pm-node all targets, formatting, the
default-profile pm-node check and the production addon build pass (test and
addon links use the existing local `CARGO_PROFILE_RELEASE_LTO=off`
workaround).

#### Remaining and orchestrator decisions

- **64-frame native I/O request (decision).** Round 3 reversed the earlier
  256-frame parity choice. This revision re-measured it: every paired native
  start used 64-frame callbacks and all six 60 s loop windows (four
  identical, two distinct, four of them stressed) had zero native underruns
  and overloads. Energy use, Bluetooth/USB/aggregate devices and loads such
  as concurrent GPU render or video decode remain unmeasured. It needs
  explicit approval.
- **Headless first-buffer bound (decision).** The check now uses the
  producer's publication time. It is enforced for prepared starts; for cold
  starts it is reported, because on this shared machine a niceness-15 test
  process beside other lanes' foreground work measured cold identical
  starts at 10.823 and 11.146 ms (locked) and up to 19.592 ms (unlocked),
  where the round-3 observer-based check would usually have passed by being
  late itself. In the final sixty suites no cold identical start exceeded it
  (240 starts, maximum 8.026 ms); cold distinct starts exceeded it in 75 of
  240 (maximum 36.952 ms under stress). Enforcing it for cold starts needs a
  quiet machine or a different bound; the orchestrator should choose.
- **Preparation pool load.** Up to fourteen workers at user-interactive QoS
  during edits and seeks; a continuous speed or map drag resubmits sixteen
  jobs per edit, and Signalsmith processing between map/read callbacks runs
  to completion after cancellation. Effect on UI and render responsiveness
  (D6 layer drag, S1 pan) is unmeasured.
- **Trim drags on stretched clips.** The map key now includes span and on
  states, so a live trim drag or on-toggle replaces that voice's history
  and costs a fresh canonical pre-roll on the producer per edit (where the
  base kept the run). Unmeasured.
- **Cross-platform priority.** Foreground QoS elevation is macOS-only. Linux
  workers stay at niceness 15 (an unprivileged thread cannot lower it again
  per job) and Windows has no priority handling. Start latency there is
  unmeasured (plan section 23).
- **Slow-history wait.** The producer no longer waits for unstarted jobs at
  an exhausted head, but a history a worker is running is still awaited
  without its own deadline. Its length is that job's CPU work (reads are
  nonblocking), so it is bounded in practice, not by a timer. No test runs a
  descheduled worker.

Raw captures, profiles, suite logs and the artifact manifest are outside Git
in `/tmp/ne-w5/audioa7-r4`.

### A8 round 5: installed histories leave their start (2026-10-18)

The round-4 review found that the producer claim path could put jobs back
on histories that were already playing. The trace: `use_prepared` installed
a partly ready start (a voice behind a complete shared head, its job still
queued, got the entry's run and a `Prefill` sharing the entry's completion
flag) and marked the entry installed. At the head's end the producer
claimed the queued history; the worker then found its job claimed, and
`Preparation::accept` returned early, so the voice stayed not ready.
`prepare_boundaries` kept the installed, incomplete entry and scheduled a
new job on the same live run with a fresh claim and the shared completion
flag cleared. The producer then waited on that queued job again (exactly
the saturated-pool wait the claim exists to avoid), the job rewound the
live run, and at the next wrap of a loop start `use_prepared` could hand
the advanced run back as prepared, so the producer re-primed it on the wrap
(the A7 mechanism). It is reachable in the measured scenario: the harness
sets the work area to [4 s, 6 s] and the speed and prepared-edit rows play
from 4 s. PCM stayed bit exact (the reviewer's probe); the defect was the
real-time stall and the duplicate producer work.

The same ownership gap had two more paths: paused or boundary preparation
(`prepare_start`) could claim a queued job of an installed start and run it
on the live run without ever setting its completion flag (the producer
would then wait at the head's end forever), and evicting or invalidating an
installed entry dropped the only path that set the flag.

**Fix (961230f6).** Ownership now transfers with the history:

- `use_prepared` releases every history it installs (ready, or behind a
  complete head with its job queued or running) from the prepared entry
  (`Prepared::release`). The entry keeps only histories no voice plays, so
  boundary scheduling, paused preparation and the next loop wrap prepare
  fresh histories and never touch a live run. The `installed` state and its
  retention rule are gone.
- Each job owns its completion flag and sets it when its history is
  released, whether it ran to ready, stopped on a missing block, or failed.
  A head's consumer no longer depends on the job's result reaching an entry.
  Each submission gets a fresh flag, so a superseded job still exiting can
  never mark a newer job finished.
- The A3 late path (`begin_prepared`) releases a head-installed history the
  same way (the slot is marked served, so no join job is scheduled on it),
  and the next pre-roll discards a superseded late start's entry.

Nothing in the render path, the stretch, the pre-roll or the ring changed.
No pm-node, protocol or render.rs change in this round (the StartTiming
fields below are serialized through the existing diagnostics report).

**Regressions (all fail with the fix removed, checked by mutation):**
`claimed_installed_histories_are_never_rescheduled` drives the reviewer's
trace through `Preparation`: partial install, claim at the head's end, the
workers' claimed reports through `accept`, then `retain_prepared` and
`schedule` at the same start. It asserts that the four rescheduled jobs run
on histories no voice plays, the producer never waits again, 16,384 frames
past the head stay bit exact while the jobs run on the pool, and the next
wrap installs fresh histories with no producer pre-roll (`fresh_starts` 1)
and 8,192 bit-exact frames. `a_dropped_start_entry_never_strands_an_installed_history`
invalidates the entry while a worker runs a history and checks the job's
own completion releases the head's consumer.
`paused_preparation_never_runs_a_queued_job_on_a_live_history` runs paused
preparation after a partial install and checks the producer can still claim
all three queued histories. The A3 test now also checks the late path
schedules no job on a head-installed voice. The reviewer's probe, run
against the fix, reports 4 rescheduled jobs, 0 on live runs, producer not
awaiting, and 0 mismatched chunks. One existing test,
`immutable_heads_start_complete_then_handoff_to_independent_histories`,
simulated a job that stops on an evicted block by reporting its result
without running it; since the job now owns its completion flag, the
simulation sets that flag as `run` does. Its assertions are unchanged.

**Pool QoS (433bf464).** Investigating a stressed paired failure (below)
showed a second, pre-existing start hazard. Idle preparation workers waited
at utility QoS and raised themselves only after dequeuing a playhead job.
Under default-class load a woken utility thread was starved for longer than
a start's 85 ms shared head, so the producer reached the head's end with
most dependent histories unstarted, claimed them all, and primed them
serially in one render; the ring ran dry. Workers now wait at
user-interactive QoS; only boundary refill lowers itself to utility QoS for
the length of its own job. `idle_workers_wait_at_user_interactive_qos`
reads an idle worker's class from outside the pool and fails with the old
policy. Linux and Windows are unchanged.

**Start diagnostics (a3ed6997).** `StartTiming` now also reports
`historyWaitUs`/`historyWaits` (segment holds while an installed head waited
on a worker's history), `historyClaims` (histories the producer primed
itself), `maxRenderUs` (the run's longest segment render) and
`minQueuedFrames` (the ring's low-water mark when the producer returned).
They are diagnostics only; no gate reads them.

#### Round 5 measurements

**The output device changed during this round.** Jude's default output was
AirPods Pro (Bluetooth) for most of the session: both engines ran
256-frame callbacks; native reported 160 ms device latency, today 5.333 ms
base plus 168 ms output latency. The test signal peaks at -123 dBFS, so
the captures were inaudible. Later rows ran on the MacBook Pro speakers
(64-frame native callbacks), as in rounds 3 and 4. Each JSON sample
records its device.

*Interrupted campaign on 961230f6 (AirPods, paired, 30 starts per row):*

| Case | Today p50/p95/max (ms) | Native p50/p95/max (ms) | Raw misses |
| --- | ---: | ---: | --- |
| Prepared level edit, plain, 60 s loop | 173.595/175.312/175.388 | 168.606/171.672/171.972 | None |
| Speed edit, plain | 173.606/175.607/175.797 | 172.966/176.066/176.099 | p95 +0.459, max +0.302 |
| Unprepared seek/edit, plain | 173.489/175.794/175.874 | 173.172/175.405/175.438 | None |
| Prepared level edit, stress 1, 60 s loop | 172.911/175.510/175.895 | 169.899/171.499/171.832 | None |
| Prepared level edit, stress 2, 60 s loop | 173.212/175.911/175.992 | 168.368/171.601/171.701 | None |
| Prepared level edit, stress 3, 60 s loop | 173.593/175.566/175.882 | 169.690/171.790/172.090 | None |
| Speed edit, stressed | 173.201/175.687/175.909 | 174.397/178.460/179.397 | p50 +1.196, p95 +2.773, max +3.488 |
| Unprepared seek/edit, stressed | failed (below) | failed (below) | — |

All four 60 s loop windows: 0 underruns today, 0 native underruns, starved
renders and overloads. Every completed native start was complete and
simultaneous (16 voices, zero spread, omission and safety-net use).

**A8 on Bluetooth: speed edits miss today.** Native compute is unchanged
from round 4 (Play to all-ready on the plain speed edit: 3.657 ms p50 in
round 4, 3.645 ms in round 5). The device decides the margin. Per start, native
= frontend (about 1.7 ms) + canonical pre-roll (3.6–3.7 ms, more under
stress) + producer queue (0.1 ms) + wait for the next callback (0 to one
period) + callback-to-presentation lead (one period) + device latency.
With 64-frame callbacks a period is 1.333 ms and native's device latency is
13.7 ms below today's (15.625 against 24 + 5.333), which hides the
pre-roll. On AirPods a period is 5.333 ms and the device-latency advantage
is only 8 ms (160 against 168 + 5.333), so the pre-roll plus callback phase
shows at p95 and max. Breakdown p50/p95/max (ms), speed edit, plain: wait
for callback 2.576/5.141/5.159 on AirPods against 0.598/1.314/1.319 on the
speakers; lead 5.335 against 2.341. The baseline binary (41b499d7, round 4)
misses the same way on AirPods: stressed unprepared seek/edit 175.747/
180.880/182.480 ms native against 172.879/175.487/175.576 ms today. This
confirms the review's note: **the A8 pass depends on a short native
callback**, and it does not hold on a 256-frame device for edits that need
a fresh pre-roll on the Play path. Prepared level edits still pass there.
This needs an orchestrator decision; it is not fixable inside the bit-exact
pre-roll.

**Stressed seek: program underruns, pre-existing, located and reduced.**
The stressed unprepared seek/edit row on 961230f6 failed twice on AirPods
(one start with 5 and one with 3 program underruns; a first attempt had
failed on today's side with "WebAudio output timestamp unavailable"). A
native-only copy of the harness that records instead of throwing then ran
alternating rows (40 starts each, unprepared seek/edit, AirPods unless
noted):

| Binary, stress threads | Starts | Starts with program underruns | Starts with claims (histories) |
| --- | ---: | ---: | --- |
| 41b499d7 (round 4), 4 | 150 | 1 | n/a |
| 961230f6, 4 | 120, plus two paired attempts stopped at their first violation | 0, plus those 2 | n/a |
| a3ed6997 (diagnostics), 4 | 120 | 0 | 0 (0) |
| a3ed6997, 12 | 120 | 14 | 30 (251) |
| a3ed6997, 12, speakers | 120 | 7 | 28 (149) |
| 433bf464 (QoS), 12, speakers, alternating with the row above | 120 | 1 | 0 (0) |
| 433bf464, 12, speakers | 120 | 0 | 0 (0) |

Every violating start before the QoS change had 8–15 claimed histories;
where the trace had recorded it before the harness read it, the start's
longest render was 68–111 ms: the claim burst described above. After it, no
start claimed a history; the one remaining violation had no claim and a
46.9 ms render with the whole harness at niceness 15 beside twelve busy
threads at the same niceness (a descheduled producer). The round-4 binary
shows the same failure, so it predates this round; the round-5 history fix
does not cause it.

**Not completed: the final qualification of the final revision.** After
merging feat/native-engine (d5a59079, 132 commits, none in pm-audio,
pm-stretch or present/audio.rs), the final campaign passed the targeted
tests (69 library with one existing ignored measurement, 11
decode-readiness, 14 mixer, 6 stretch, 5 sources, 2 layouts, 1
legacy-parity, 3 resampler) and the 59 presenter tests, then stopped at the
40 GB disk guard (32 GB free, falling to 25 GB as other lanes built). Strict
clippy, formatting and the library tests also pass on 433bf464. Not run on
the final revision: the twenty paired rows, the six loop windows, the
profiles and the sixty playback suites. Rerun with
`/tmp/ne-w5/audioa7-r6/campaign.sh` once at least 40 GB is free.

**Correction to the round-4 report.** Its deliverables line said no
assertion was changed. 987ef700 did change one: cold starts are no longer
held to the one-callback first-buffer bound (10.667 ms) and are only
reported, as the round-4 section above states.

Evidence outside Git: `/tmp/ne-w5/audioa7-r5` (interrupted campaign,
A/B and diagnostic rows, breakdown scripts) and `/tmp/ne-w5/audioa7-r6`
(final-revision verify logs).

#### Remaining and orchestrator decisions, round 5

- **Short native callback (decision; A8 depends on it).** On 64-frame
  devices A8 passed in rounds 3–4. On a 256-frame Bluetooth device
  (AirPods Pro), speed edits miss today at p95/max plain and at every
  percentile stressed, in this round's and round 4's binaries. Options are
  the orchestrator's: accept per-device behavior, or find a non-bit-exact
  or speculative way to take the pre-roll off the Play path (both outside
  the current rules).
- **Final qualification pending** (disk guard; see above).
- **Claim burst remains possible.** With the QoS change the pool starts
  dependents promptly; if it is genuinely saturated, the producer still
  primes every unstarted history serially at the head's end. Bounding that
  (priming at most what the ring can cover) is not done.
- **Producer priority.** The producer is not a real-time thread; today's
  WebAudio renders on Chromium's real-time audio thread. Under equal-
  niceness load the producer can be descheduled (one 46.9 ms render at
  twelve busy threads). Unchanged.
- Earlier items stand: trim drags on stretched clips (map key), pool load
  during drags (D6/S1), Linux and Windows priorities, the running-history
  wait without a timer, and the headless cold-start bound.


### A7/A8: real-time scheduling and enforced cold-start timing (2026-10-19)

AUDIO-1 accepts the recorded AirPods Pro / 256-frame latency difference
(0.3–3.5 ms on about 175 ms). The earlier raw figures remain in the round-5
table. That decision does not accept dropped audio. Ordinary starts still
require all voices, complete and simultaneous, with no omitted prefix.

The producer and every preparation job request a callback-matched Mach time
constraint. Device playback explicitly constructs a schedule that attempts
the retained CoreAudio output-device workgroup
(`kAudioDevicePropertyIOThreadOSWorkgroup`). A device without a usable group,
and `AudioSchedule::new(..., None)` used outside headless playback, keep the
Mach-only fallback. Headless playback explicitly calls
`AudioSchedule::headless(...)`, which creates a dedicated AudioWorkInterval
with the public `AudioWorkIntervalCreate` API. Its producer and workers join
after installing the callback-matched Mach policy. A missing device ID alone
does not request a custom interval.

The headless producer alone owns interval cycles. An edit starts detached
preparation immediately; the first Play/Seek cycle is anchored to the
original request timestamp, including after producer guard re-entry. A
partial first callback keeps that cycle across retries, and publication
completes successive 512-frame output cycles. Finishing ring refill closes a
cycle before idle or metadata work; guard teardown closes an active cycle
before leaving the group. An expired or failed first-cycle start is recorded
for its epoch and is not retried with a newer timestamp. If the producer
cannot install the unique master, it leaves the custom group, restores its
prior scheduling policy, and reports an unavailable schedule; a worker's
membership cannot masquerade as a producer master. Failed creation, joins,
expired requests and lifecycle operations remain observable and fail
ordinary-start gates. Deliberate A3 decoder holds can finish a cycle after its
declared deadline; late finishes
are reported separately, while the atomic first-buffer assertion keeps its
original deadline. Workgroup membership is attempted only after real-time policy
succeeds. The per-thread guard leaves the group and restores the prior real-time
policy, or standard scheduling with an explicit idle Mach base priority of
at least 47 on these owned audio threads. Legacy Mach scheduling permanently
opts a pthread out of QoS, so restoring a cached QoS label is insufficient.
Restoration checks return codes and kernel priority; a bounded feedback
adjustment handles the task's niceness offset. Dedicated-thread and leased
preparation-worker tests verify the actual kernel policy and idle priority. The producer reserves real-time CPU for preroll and ring refill, and releases
it before full-ring boundary scheduling and read-ahead metadata, as well as
document/control work, paused preparation and device-property polling.
The optional master shuttle is warmed before receiving project controls. Edits still launch
preparation before Play. The producer retains its real-time guard across
control waits for at most 20 ms after a paused edit while urgent preparation is pending,
and while playing unless
boundary or look-ahead metadata is actually due. A complete batch becomes
visible under one queue lock before workers wake. Boundary and secondary-start jobs use the same
real-time schedule, with one nonurgent background replenishment job at a time. Installing a
head promotes its queued independent histories to urgent work and wakes
the pool, bypassing that serial background limit.

Preparation workers reserve 95% of a callback period and have a full-period
constraint. The producer retains that callback period with an 80% computation
allowance and an earlier 85% constraint, so a new control can take precedence
over older preparation jobs. Both are preemptible; exact role parameters are
read back from the kernel. The producer's earlier deadline is a candidate
until the measurements below qualify it. Cooperative checkpoints precede vocoder initialization, seek and processing,
voice mixing and bounded map/read-ahead groups. The existing Signalsmith
internal-step hook also checkpoints inside seek, process and flush without
changing DSP arithmetic or process-call boundaries. Scheduled shuttle calls
use it too. A cheap wall-clock gate limits expensive thread-CPU samples to
at most one per 100 µs (or one-sixteenth of a shorter callback period);
forced outer checkpoints still sample immediately. They leave up to 500 µs
(minimum of that and half the computation reservation) for the next unit,
then sleep to the next period when the CPU budget is spent. Known indivisible
stretcher construction/reset units require a 1 ms admission reserve. A new Play
does not reset or forgive CPU already charged in the same period. CPU debt is
retained across job guards and viewers sharing a worker, while CPU spent
outside those guards is excluded. Unknown callback size uses the 512-frame
headless period; an actual 64-frame callback keeps its 1.333 ms period. Per-context
telemetry records maximum CPU between sampled checkpoints (including the
coalescing window, rather than one vendor step), maximum period CPU,
and computation/constraint CPU exceedances against each role's actual limits; blocked wall time is not
reported as CPU service. Per-start counters expose throttle sleeps and
total sleep time. Scheduling entry counters include preparation before
Play; CPU maxima aggregate producer and workers rather than identifying a producer
per-period distribution. CPU maxima and restoration failures cover the current device context
and restart when that context changes. Actual audio delivery is checked separately by
program-underrun and A7 ring counters.

Public Mach latency and throughput tier-0 hints supplement the real-time
policy on owned audio threads. Entry reads back both hints, the requested
time-constraint parameters, and the effective non-timeshare mode. The hints
do not guarantee P-core placement. Their prior values are restored on exit;
readback, hint and restoration failures remain separate zero gates. Kernel
fail-safe demotion after entry is not continuously polled.

The schedule also captures the SDK's `os_workgroup_max_parallel_threads`
recommendation once at creation and exposes it as
`scheduling_workgroup_recommended_threads` in start telemetry. Zero means
no group or no recommendation. This advisory value is not used as the pool
cap and does not establish affinity or current available CPU capacity.

The producer's serial history-claim budget is zero. Independent vocoder
histories stay on preparation workers; the old synchronous paused/boundary
preparation helper is removed too. The producer only marks decoded plain
voices ready. On a heterogeneous Mac reporting named performance and efficiency clusters,
the preparation cap is the larger of eight workers and the performance-core
count minus two, within the total-core-count-minus-two ceiling (minimum one).
This host has 12 performance and 4 efficiency cores, so the candidate uses
10 workers. The tested 14-worker control and bounded launch-gate candidate
both missed all 32 strict cold-start deadlines; the extra concurrency and
launch gate are not retained as latency fixes. Neither worker cap provides
core affinity. Other hardware remains unqualified. Complete voice batches enter the pool under one queue lock
before workers wake, preventing early real-time workers from preempting
the producer while it is still submitting the remaining voices. Rewritten
regressions exercise the current worker-only handoff, released live-history
ownership and fresh loop histories through the real preparation pool.
Each control carries an atomic pending ticket through the existing FIFO and
any deferred slot until producer dispatch. Detached preparation workers alone
install a thread-local view of that counter. Existing forced and wall-gated
DSP checkpoints block once for 50 µs if a control is pending, allowing the
producer to run even when older real-time jobs have earlier deadlines.
The producer never installs this worker gate. Failed sends, shutdown and
queued-control destruction release tickets; no history is cancelled from the
sender thread. Per-worker opt-in profiles distinguish these control waits
from CPU-budget throttle sleeps. Aggregate control-yield count and wait time
appear in capture telemetry as both a per-start delta from its scheduling
baseline and explicit current-context totals. The context totals can include
preparation before Play and must not be attributed solely to that start.
Opt-in producer profiles record whether a real-time guard was already held
at each control receipt as a zero-duration event. The worker profile's
`host_claim_us` is captured at actual queue dequeue, before closure entry
(`host_start_us`) and its worker control yield; cancelled jobs also emit a
record. Profile-only worker lines are formatted outside any real-time guard
and sent with `try_send` to a process-wide ordinary stderr writer.
Its 2,048-entry channel never waits for capacity on a preparation worker;
full or disconnected sends increment a visible dropped-event counter. An
ordinary shutdown caller can request a flush barrier that waits at most
100 ms and records failure separately. The barrier drains records already
queued; a worker finishing afterward can enqueue a later record. This changes
diagnostic delivery, not DSP or the timing bound. V17 runtime probing is
pending, so no latency gain or complete profile capture is claimed.
Forced checkpoints each perform their own bounded
50 µs wait while controls remain pending. A queued control now interrupts
both preroll waits and is dispatched before any audio is filled. Same-epoch
retries preserve the original device-frame origin and phase wall deadlines,
so a nonrestart control cannot renew the 120 ms A3 hold. A new transport epoch
gets a new origin. The existing Seek→Pause regression also forces a blocked
decoder, asserting prompt acknowledgement, no queued PCM and the seek target. Canonical pre-roll, source evaluation,
per-voice processing order and offline PCM comparisons are unchanged.

The headless first-buffer test again enforces 512 frames within 10.667 ms
after Play for both prepared and cold starts, identical and distinct. Each
ring slot publishes PCM with Release ordering and records an atomic host
timestamp afterward. The consumer acquires that same publication and
records its observation time. The earlier available time is an upper bound
on visibility: neither a pre-publication timestamp nor a delayed observer
can turn late output into a pass. The old post-push trace remains diagnostic;
it no longer determines the deadline.

Final qualification must be recorded below against the active output device;
no qualification is implied by the scheduling code or compiler checks alone.
The AirPods latency acceptance does not apply to new program underruns.

Detailed profile DSP scopes include any cooperative throttle sleeps. Cost
comparisons use per-job CPU and throttle times rather than treating those
wall-clock scopes as pure DSP computation.

Runtime captures use the pinned Rust 1.96.1 toolchain at niceness 15 with
four build workers, release optimization, default thin LTO, debug level zero
and symbol stripping disabled. Disabling stripping avoids the pinned toolchain's
[Mach-O string-pool alignment bug](https://github.com/rust-lang/rust/issues/157750);
the native addons must be loaded and checked before final measurements.
A minimal adjacent `pm-media/build.rs` change links the native CrabbyAVIF
archive without bundling it into the Rust rlib. Bundling put native Rust
objects without LLVM bitcode inside that rlib and broke default thin-LTO
links. The codec archive and decoder implementation are unchanged; the
final verification includes the affected image tests and addon load.

#### Cold-start scheduling experiments (qualification pending)

These are preflight experiments, not final campaign results. The strict
atomic first-512 bound is 10.666667 ms. Each row ran eight fresh stressed
processes with four cold distinct-voice starts each, at niceness 15, with
two headless busy/yield threads. The device campaign separately uses four
busy threads. Percentiles use nearest rank; every raw miss is retained.

| Candidate | Deadline misses / 32 | p50 / p95 / max (ms) |
| --- | ---: | ---: |
| P-core-aware cap, 10 workers | 20 | 11.595 / 13.089 / 13.126 |
| Earlier producer constraint, 10 workers | 18 | 11.455 / 12.329 / 12.888 |
| Pending-control yields and paused urgency, 10 workers | 13 | 10.142 / 15.763 / 28.716 |
| Same control handoff, 14 workers | 32 | 14.962 / 17.856 / 18.145 |
| Bounded launch gate, 14 workers | 32 | 15.753 / 19.001 / 25.649 |
| Custom headless work interval, 10 workers | 13 | 10.122 / 12.678 / 13.288 |
| Interleaved control, 10 workers | 14 | 10.504 / 13.098 / 45.321 |
| Interleaved candidate, 8 workers | 27 | 12.180 / 14.263 / 14.659 |

All completed cold-start trials in these rows compared the first 8,192
frames against independently rendered offline voices, summed in voice
order, with exact PCM and zero omitted prefix, start spread, A3 use and
program underruns. Deadline misses still fail the tests. The 14-worker,
launch-gate and 8-worker candidates are rejected. The two worker-count
variants ran interleaved under one shared lease, with the 10-worker variant
first in every pair; that order limits causal comparisons. The final device
campaign counterbalances the engine order across paired rows.

The control-handoff change reduced request-to-apply time to 0.049–0.250 ms
in its measured cold starts. Remaining costs are chiefly canonical DSP
preparation and worker wake/claim handoff. Typical profiled jobs use about
3.6–4.5 ms of CPU; some take 5.3–5.5 ms. In a representative 10-worker
capture, the first ten claims occurred 0.64–1.44 ms after submission and
the second wave at 4.9–5.8 ms. These timings include contention and scheduling;
they do not isolate OS wake latency. The 45.321 ms outlier was profiled,
with claims continuing to 40 ms while job CPU stayed about 4.3–4.7 ms and
no budget throttle occurred in those jobs. Worker stderr serialization was
a possible contributor; the bounded asynchronous writer is awaiting runtime
validation. No outlier is classified as noise.

One custom-interval library preflight failed because an existing worker
priority test expected the Mach-only diagnostic label after the test began
using a custom interval. The assertion now expects the actual custom group;
the later 8-worker library run passed 86 tests with one existing ignored
measurement. The earlier producer/preroll control regression runs passed
plain and stressed, including Pause during a blocked seek, a hold governed
by device time, and the A3 missing-decoder-only safety net. Those runs do
not qualify the current source revision.

An isolated compiler scheduling experiment retained the same baseline ISA,
strict arithmetic and process boundaries, and changed only the CPU tuning
from the default to Apple M4. Its 201,600 output floats per variant were
identical over 1,680 callbacks, but 48 interleaved canonical-job CPU samples
were not faster: default p50/p95/max 2.84046/2.86537/2.87550 ms, tuned
2.84383/2.86821/2.87879 ms. That compiler change is rejected.

The atomic deadline remains a blocker. No final paired rows, six loop
windows, or 30 plain plus 30 stressed repetition suites have yet run on the
current revision. Earlier round-5 statements about ordinary producer
priority, report-only cold timing and unbounded serial history claims are
historical; the implementation above supersedes them, while qualification
remains pending.

### R7 worker wake policy and cold-start preflight (not qualification)

The preparation pool now has an explicit idle scheduling lease. The caller
arms workers before returning a newly constructed playback; Play does not wait
for an arm command to reach the producer. A parked worker holds a verified
Mach-only reservation matching the output callback. It drops that guard before
queue locking, readiness evaluation, job dispatch, or diagnostics, and joins
the actual playback workgroup and reservation for DSP work. Idle contexts do
not retain the output workgroup. A revoked lease shares a tombstone with its
clones. Different simultaneous callback periods select ordinary idle waiting;
actual preparation still uses each job's own callback-matched policy. This
fallback has no multi-viewer cold-start latency qualification.

A wake generation and waiting flag use a sequentially consistent handshake:
a publisher either sees and wakes a parked worker, or the worker observes the
new generation and retries its queue scan. Wake diagnostics expose successful
idle entries, wake count, guard-drop time, queue-lock wait, and wake-to-claim.
The current lease and construction arm result are exported alongside process
pool counters. Counter deltas since Play are separate from lifetime peaks;
in particular, an idle context's period CPU peak includes work inherited from
the preceding job in the same physical thread's reservation window.

A physical thread keeps CPU debt when changing jobs, playback contexts, or
idle contexts with the same policy. Overrun flags belong to that reservation
window, so inherited debt is not charged as a new idle overrun. When the
numeric policy changes, the worker waits for the previous genuine period to
end before installing a fresh reservation. This wait holds no queue mutex,
TLS borrow, or real-time guard. Scheduling metrics retain those waits. The
first headless interval also retains the actual SDK anchor, start, and deadline
ticks; reentry is tied to the original Play request, not a renewed deadline.

V19 kept ten preparation workers, the canonical 11,520-frame pre-roll, and the
unchanged strict atomic first-512 bound of 10.666667 ms. Its eight stressed
cold-distinct preflight processes produced 32 complete bit-exact comparisons:
30 met the deadline and two missed at 10.946 and 12.228 ms. The distribution
was 8.644/10.946/12.228 ms (p50/p95/max). All 32 had zero omitted prefix,
zero start spread, zero A3 fires, and zero program underruns. The profiled
10.946 ms miss had all first ten claims within 64 microseconds; its last two
voices claimed about 5.7 ms later and each incurred roughly 0.5 ms of CPU-budget
throttle. Aggregate preparation CPU was 77.09 ms. These are raw misses.

The V19 library preflight passed 100 tests, failed one new policy-observation
test, and retained one existing ignored test. That new test incorrectly
assumed a real-time thread had no readable priority; the supported extended
query reported priority 97. The correction verifies exact Mach parameters and
the effective scheduling mode separately. Ten targeted seek, Pause, decoder,
and A3 regressions passed (five plain and five stressed). The complete playback
preflight passed 40/40 plain and 39/40 stressed; the stressed cold-distinct
case had two further deadline misses, 11.952 and 11.578 ms. Their complete
PCM, omission, spread, A3, and underrun checks still passed.

Timing suite processes now run one test at a time, within the brief's limit
of four, because different simulated device periods share the pool. All 40
public tests remain and all assertions are retained. Library checks still use
up to four test threads. Device pairs retain four busy stress threads. Earlier
four-test-thread failures remain evidence; a serial preflight is not the final
30 plain plus 30 stressed campaign.

Earlier logger-only V17 and idle-reservation V18 probes also failed the
same strict deadline. V17 passed 2/32 trials, with p50/p95/max
11.867/14.178/14.651 ms; V18 passed 12/32 at 11.023/12.519/13.069 ms.
V17's library passed 87 tests with one existing ignore. V18 passed 93,
failed three new scheduling/wake tests, and retained that ignore. Two failures
came from invalid 100 ms synthetic reservation expectations; the third
exposed a park-token handshake defect that was fixed before V19. Both V17
and V18 full preflights passed 37/40 plain and 37/40 stressed, with two
critical-vector indexing panics and the cold deadline failure in each mode.
The vector remap defect was fixed before V19. These failures remain recorded.

### R7 fair preparation and ownership preflight

A prepared voice now has one owned task through initialization, deterministic
join, optional continuous lead, and immutable head publication. The task keeps
its evaluator, retiming memo, constant map, scratch, and vocoder history when it
moves between workers. A CPU yield is distinct from missing decoded input and
from completion. Yielding happens only after a complete native process call
(or completed initialization), after its output and phase state have committed;
it never changes native call boundaries, canonical pre-roll, map arithmetic,
or output length. A request that has reached its end finishes copying and
pruning before it can yield.

A yielded task releases its history mutex and real-time guard, then returns to
the ordinary queue at the tail of its priority band. It remains pending, so no
duplicate job may claim that history. Only terminal completion sends Done and
sets finished; only a complete head becomes visible. Cancellation of a queued
continuation releases its dependent gate and completion ownership once. The
thread's CPU ledger survives both task migration and policy reentry. There is
no new CPU allowance per slice.

Per-voice profiling sums scopes, thread CPU, entry time, control waits, and
throttle across workers, and records first claim, intervening queue wait,
slices, requeues, and maximum slice CPU. Pool counters expose claims, requeues,
completions, wakes, and idle entries; these are process-wide and the exported
per-Play view subtracts counters, while maxima remain lifetime observations.
A completion only wakes sleepers when ready work exists; a requeue wakes one.

The exactness regressions compare every f32 bit and the native read-call
sequence at five speeds in both directions, plus a resumable join and actual
missing-input retry. Mixer checks compare through the head and 8,192 further
frames against both the unsliced path and a fresh offline render. Queue tests
use the production loop for FIFO rotation and the scan-to-park race. The
64-frame policy check still requires exactly seven logical jobs; its entry
count includes every slice. Construction arming keeps an acknowledgment for
the lease epoch; incompatible-period transitions are counted and gated for
each start. Actual output periods, not project sample rates, select the lease.

V20's eight stressed cold-start processes passed all 32 strict first-buffer
assertions: p50/p95/max 8.405/9.563/9.733 ms. V21 passed all 32 at
8.871/9.823/9.826 ms. Every trial compared complete PCM bit for bit, with
zero omitted prefix, start spread, A3 fires, or program underruns. V20's
library passed 106 tests; V21 passed 110, each with one existing ignored
measurement. Each candidate passed ten targeted regressions and one complete
40-test plain and stressed playback suite. These candidates predate the
latest integration merge and are preflight evidence, not qualification.

In V20's 16 fully logged cold preparation cohorts, all 16 voices completed
in 304 slices and 288 requeues per cohort, with zero budget throttles. First
claim to last completion was 6.842/8.379/9.285 ms (min/p50/max); summed
thread CPU was 60.059/71.292/77.015 ms and summed policy-entry time was
2.620/3.197/3.622 ms. Entry totals span parallel workers; they are not wall
time. Profile scopes overlap and include checkpoint waits. The raw scope,
queue, slice, entry and wake measurements are retained outside Git.

Independent review found two ownership races introduced or exposed by
slicing. A project edit can cancel a worker while it waits for a history
mutex, so cancellation is checked again after locking. Solo can transfer
history ownership without changing the project generation. A worker now
checks the finished ownership flag before initialization and after locking,
then retires silently without rewriting the head, completion flag, or live
history. Tests force both lock-wait interleavings and compare subsequent PCM
with a fresh offline render. Queued and mid-slice Solo retirement, dynamic
speed evaluation, missing-input recovery, and two mixer-level loop passes
also have exactness regressions.

The final merged revision and full device/repetition campaign remain
unqualified at this point. No historical miss is classified as noise.

### R7 callback preparation follow-up (qualification pending)

V22, after the integration merge, passed 32/32 stressed cold first-512
preflights at 7.351/7.957/8.899 ms (p50/p95/max). Each trial retained complete,
bit-exact PCM, 16 simultaneous voices, zero omitted prefix, zero A3 fires,
and zero program underruns. Its library passed 113 tests with one existing
ignore; ten targeted regressions and one 40-test plain and stressed playback
preflight passed. These results did not qualify the device path.

The subsequent partial device campaign used the built-in output at 48 kHz
with 64-frame callbacks and four busy stress threads. The following raw A8
rows remained blockers. Each entry is p50/p95/max in milliseconds, with 30
starts per engine. The third shared seek repeat passed; it does not erase
the other two misses.

| Scenario | Today | Native |
| --- | --- | --- |
| Shared unprepared seek, stressed repeat 1 | 29.512 / 32.507 / 32.511 | 32.249 / 35.243 / 36.027 |
| Shared unprepared seek, stressed repeat 2 | 29.218 / 32.408 / 32.513 | 33.311 / 35.451 / 35.653 |
| Shared unprepared seek, stressed repeat 3 | 29.627 / 32.212 / 32.295 | 28.631 / 30.344 / 30.614 |
| Distinct speed edit, plain | 29.931 / 32.321 / 32.530 | 30.817 / 31.349 / 31.372 |
| Distinct unprepared seek, plain | 30.222 / 32.414 / 32.611 | 30.879 / 32.112 / 32.200 |
| Distinct speed edit, stressed repeat 1 | 30.436 / 32.329 / 32.429 | 30.648 / 31.267 / 31.305 |

A separate stressed speed attempt recorded one computation-budget overrun:
maximum guarded period CPU was 1.309376 ms, and maximum sampled work-unit CPU
was 0.873375 ms. There was no constraint overrun, omitted audio, start spread,
or program underrun. The later retry passed; the first failure remains. The
old record did not identify the offending phase or its time within the
120 ms observation window. It cannot be attributed to first-buffer work.
Five completed 60-second loop pairs, including distinct voices, had zero
underruns in either engine and zero native starvation or device overloads.
The blocked revision's 60 repetition suites were not started. Three earlier
setup attempts also failed before playback measurement; fresh native, Wasm,
and desktop artifacts subsequently connected and played. Their precise API
failure was not isolated, and they do not count as passing captures.

Matched first-head profiles then captured two plain and two stressed native
rows, 12 starts each, on the same 64-frame output. The representative job used
19–21 slices. Across rows, median queue time was 4.026–4.373 ms, thread CPU
3.391–3.595 ms, policy entry 0.175–0.188 ms, and budget sleep 1.668–2.454 ms.
Cached input reads took about 0.027–0.029 ms; no inline decoder CPU was
needed for these already decoded sources. DSP scopes include checkpoint
sleeps and overlap the CPU and wait measures. Job logging followed head
publication by a median 0.024 ms and as much as 1.572 ms, so these full-job
totals are not all necessarily on the first-buffer critical path. They
identify queue handoff as a candidate cost, rather than proving an avoidable
four-millisecond delay.

The revised executor keeps a sole ready foreground priority-zero job on its
current worker for one more complete native DSP slice. It checks the ordinary
queue again before every continuation. An equally urgent ready competitor
puts the retained job at the FIFO tail. Each slice still releases its history
mutex and real-time guard, then reenters and verifies the actual scheduling
policy. The physical thread's CPU debt remains; retaining a worker can also
retain its throttle, so the latency benefit requires measurement. Profiling
separates CPU yields from actual queue reentries and records worker handoffs
and local continuations. HEAD remains 4096 frames, LEAD 1024, and canonical
pre-roll/grid remain 11,520/16,384. No voice is omitted to improve latency.

Pure control-point scans and resampling loops now reach cooperative checks
every 32 points and 64 output samples respectively, preserving arithmetic and
native call boundaries. A fixed atomic record preserves the first observed
computation-budget overrun: host time, role, phase, unit/prior/period CPU,
wall elapsed, and exact limits. Publication is Release/Acquire and performs
no allocation, lock, or logging on the audio thread. This is a schedule-lifetime
record, not a per-Play counter; a simultaneous observer can briefly see an
overrun count before its record is published. Phase changes occur after
sampling the preceding unit; internal DSP checks keep the current label.
Unlabeled work reports unknown. Small gain/mixing tails may remain within
the preceding voice phase until the next checkpoint; phase names are not
exclusive function-level CPU attribution.

V24 then completed four stressed paired preflights, with four busy threads
and 30 starts per engine on the built-in 48 kHz, 64-frame output. Shared
voices passed; distinct voices still missed the stated latency target.

| Scenario | Today p50/p95/max ms | V24 p50/p95/max ms |
| --- | --- | --- |
| Shared unprepared seek | 29.829 / 32.413 / 32.630 | 29.006 / 30.284 / 30.928 |
| Shared speed edit | 30.830 / 32.436 / 32.729 | 26.947 / 27.965 / 28.052 |
| Distinct unprepared seek | 30.432 / 31.923 / 32.528 | 30.590 / 32.116 / 32.296 |
| Distinct speed edit | 30.428 / 32.431 / 32.635 | 30.844 / 31.308 / 31.343 |

The distinct seek p50/p95 misses, and distinct speed p50 miss, remain
blockers. No miss is classified as noise. Across those 120 native starts,
12 shared-head profile starts and 24 distinct profile starts, the observed
program-underrun, omitted-prefix, start-spread, A3, producer history-claim,
and computation/constraint-overrun counters were zero. Application-lifetime
underrun counters were nonzero and remain in the raw reports; zero program
counters do not establish a zero application-lifetime count. These captures
do not replace the full repetition or 60-second loop campaign.

The shared profile showed that retaining a worker helps only a sole ready
head. One of 12 heads had 20 local continuations and 0.019 ms of queue time;
the other 11 competed with ready priority-zero peers and requeued 17–19
times. The median queue time remained 4.034 ms. The queue delay is largely
waiting for other urgent jobs, rather than a measured four milliseconds
inside the queue mutex.

Distinct profiling matched all 12 seek cohorts and 11 of 12 speed cohorts;
one speed start was already ready and had no contemporaneous cohort.
Seek applied-to-ready was 10.711/11.417 ms (p50/p95), with 56.485 ms summed
CPU across the 16 jobs. The last completing job's medians were 3.974 ms CPU,
3.495 ms queue, 3.027 ms budget sleep and 0.171 ms policy entry. Speed
applied-to-ready was 12.957/15.516 ms, with 78.968 ms summed CPU; the last
job's medians were 4.905 ms CPU, 4.962 ms queue, 3.851 ms budget sleep and
0.211 ms entry. Sleep overlaps entry/work wall time. Last job logs preceded
ready by 2–7 microseconds; they are correlated completion observations, not
the publication event itself. These profiled native rows were unpaired
and do not establish a paired latency pass.

A 512-frame output lead was investigated and rejected as the next fix.
The native interval is 1,440 frames at 48 kHz: the measured 2/3/4-second
anchors need the same native process calls for a 512- and 1,024-frame lead.
Other phases could lose useful reserve before a serial producer fill.
HEAD 4096, LEAD 1024 and canonical pre-roll/grid remain unchanged.

The instrumentation patch is measurement only: fixed atomic maxima and an observation
count record thread-CPU spans after admitted DSP checkpoints, including
the fraction of each span's actual computation budget. Such spans can
include intervening work; they are upper bounds rather than isolated vendor
call timers. No reservation, callback limit, CPU debt, or PCM change has been
made. Build, strict Clippy, addons and nine targeted pure regressions passed;
independent source review found no P1/P2 blockers. The values are
schedule-lifetime snapshots across producer
and worker roles; their separate maxima may come from different events.
Finite observed spans cannot establish a hard bound for a smaller reserve.

Its device capture then completed 55 of 84 planned native starts. The speed
row aborted on two worker computation-budget overruns. The first record
identified `stretch-init`: 1.276500 ms of unit CPU plus 0.004250 ms already
spent, against a 1.266666 ms computation allowance and 1.333333 ms constraint.
That record was already present before the first measured Play. Constraint
and program-underrun counters were zero; the strict computation gate still
failed. A larger reserve cannot make this indivisible unit fit its allowance.
Initialization needs finer checkpoints rather than a relaxed CPU limit.
Observed mixed-role DSP-span maxima were 0.132–0.227 ms across these rows;
they do not establish an initialization bound. All 55 observed starts kept
16 complete voices together, without an omitted prefix or A3 fallback.

The unprofiled seek row retained a 1,401.582 ms first-start outlier. Of that,
1,362.958 ms was between the UI request timestamp and the engine request;
apply-to-ready took 19.364 ms. The cause of the delivery delay was not
isolated. These native-only captures have no paired latency verdict, and
neither the outlier nor the overruns are classified as noise. Nonzero
application-lifetime underrun counters remain recorded separately.

The earlier V24 immutable headless preflight also completed: 119 library
tests passed, one was ignored, ten start regressions passed, and 32 cold
atomic first-512-frame observations had a 9.286 ms maximum, below the
10.666667 ms bound. Those results do not qualify the newer instrumentation
or the final revision.

The next candidate adds opt-in initialization checkpoints through FFT
twiddle construction, Kaiser and reconstruction loops, STFT reset, and
between allocation stages. The ordinary constructor/reset remains the
offline reference. The callback-enabled path also skips an initial ACG
window and reset that the immediately following Kaiser setup and final
reset overwrite before use. Final dimensions, buffers, seed and arithmetic
order remain unchanged; direct seeded PCM comparisons cover repeated
resets, 44.1/48 kHz and the random-phase slow-stretch path. Native reset
constructs its replacement before swapping, preserving live state on an
allocation failure. Allocations themselves remain indivisible and require
the strict actual-device CPU gate.

Preparation turns allow two resumable fresh/produce operations, with the
quota carried across stage and join-range transitions. A class-changing
operation can process an outgoing run and seek the incoming one; this is
not a two-FFI-call bound. Independent test counters check actual DSP calls
alongside existing exact PCM and history-handoff comparisons. The FIFO
handoff, continuous CPU checkpoints and physical debt remain intact, as do
the 500-microsecond ordinary reserve, initialization reserve and actual
callback limits.

V26 passed its build and strict Clippy checks, 119 audio library tests (one
ignored), two stretch library tests and five binding tests. Its runtime
preflight failed: 29 of 32 cold first-512 observations exceeded the enforced
10.666667 ms bound; p50/p95/max were 13.120/17.236/20.460 ms. On the built-in
48 kHz, 64-frame output, paired stressed distinct seek measured
42.129/44.837/44.967 ms against Today 29.194/32.203/32.495 ms; distinct speed
measured 39.905/42.853/44.058 ms against 29.973/32.012/32.307 ms. All 60
native starts in those two rows retained complete, simultaneous voices and
zero program-underrun and computation/constraint-overrun counters. The
latency and headless-bound failures still block qualification.

Independent review found setup checks in two shared per-block helpers had
prevented vectorization and bulk stores, affecting processing as well as
initialization. The remaining diagnostic runs were canceled and all
completed failures retained. V27 removes those checks and dispatches STFT
reset once between cooperative setup and an unhooked template path, keeping
ordinary processing clears and arithmetic available to the optimizer. A
stereo seeded repeated-reset comparison supplements the mono coverage.
V27 audio/stretch compilation, strict Clippy, addons and matching app/Wasm
builds passed. Disassembly restores the same helper mnemonic sequences as
the earlier vectorized object: 54 NEON instructions in window-product
arithmetic and six bulk-clear calls in output movement. The ordinary reset
specialization has bulk clears and no per-element indirect callbacks. This
verifies generated code, rather than a runtime or worst-case CPU bound.
V27 runtime qualification is pending. Its first paired attempt retained one
partial 2,063.992 ms native observation, then failed because first-buffer
timing was missing. The selected pre-edit audio report had no device or
project and zero callbacks, so this does not establish device-output timing.
The Today launch separately timed out waiting for its first window. Neither
part supplies a paired latency verdict. One subsequent unloaded startup
probe retained all 16 layers and opened the device within its 300 ms warmup,
without a host-clock or silent run; it did not reproduce or explain the
first failures. Capture setup needs explicit document, session and audio
readiness before the measured edits.
The sampled DSP-span telemetry now includes initialization and destructor
tails; its maxima are not isolated vendor-call times or directly comparable
to V25. Allocation and destruction can still be indivisible.

Full final device and repetition qualification remains pending. The
historical AirPods 256-frame allowance above remains limited to that
accepted device result and does not excuse the built-in-output misses.

V27's later reserved headless preflight passed 121 library tests (one
ignored), two stretch library tests, six binding tests and ten focused
start regressions. Four of its 32 cold atomic observations still exceeded
10.666667 ms; visible first-buffer p50/p95/max were
7.957/11.713/12.755 ms, with a 13.414 ms observer maximum. A separate seeded
probe compared the earlier vectorized object, V27's ordinary path and its
cooperative path: all 1,167,696 floats across eight mono/stereo, rate,
slow-stretch and repeated-reset cases were byte-identical. This supports
PCM preservation on this Mac; final mapped playback comparisons remain
required.

V28 increased the preparation pool from ten to twelve workers on the
16-core Mac (twelve performance and four efficiency cores), still below
total cores minus two. Its reserved preflight passed all 21 processes,
including the same library, binding and focused regressions and all 32
cold atomic observations. First-buffer p50/p95/max were
6.837/8.323/8.466 ms; the observer maximum was also 8.466 ms.

Four paired stressed rows then completed, with four busy threads and 30
starts per engine on MacBook Pro Speakers, 48 kHz and 64 frames. Both
halves of every pair held the exclusive app lease. Recorded one-minute
load means did not change sharply under the preregistered validity rule;
no pair was discarded or retried. Shared rows passed; distinct rows failed.

| Scenario | Today p50/p95/max ms | V28 p50/p95/max ms |
| --- | --- | --- |
| Distinct unprepared seek | 30.128 / 32.207 / 32.317 | 31.189 / 32.886 / 34.154 |
| Distinct speed edit | 30.133 / 32.135 / 32.536 | 32.388 / 34.926 / 35.194 |
| Shared unprepared seek | 30.222 / 32.603 / 32.711 | 29.474 / 31.369 / 31.506 |
| Shared speed edit | 30.034 / 32.528 / 32.635 | 28.749 / 29.790 / 29.834 |

The two distinct rows miss all three required latency statistics. No miss
is classified as noise. Those 120 native starts and 24 additional native
profile starts kept all 16 voices complete and simultaneous, with zero
program underruns, omitted prefix, start spread, A3 fallback, producer
history claims, and computation/constraint overruns. Application-lifetime
underruns are separate counters and are not claimed to be zero.

Profile cohorts matched all twelve seek starts and eleven cold speed
starts; the remaining speed start was already ready. Median apply-to-ready
was 12.170 ms for seek and 10.157 ms for speed. The final initial voice
claim arrived about 1.7 ms after enqueue. The last completing voice used
4.979/4.378 ms CPU, 2.991/2.608 ms queue time and 4.004/3.364 ms budget
sleep, respectively. These scopes overlap and must not be added as
independent wall time. Across the unprofiled rows, median UI-to-engine
request was 1.117 ms for seek and 2.178 ms for speed; request-to-apply was
only 0.022/0.018 ms. Dirty-project graph construction precedes that engine
request timestamp and needs its own control-thread measurement. Full
final qualification remains pending.

The resumed validation replaces test media paths with owned temporary-directory
handles throughout pm-audio and pm-media, including the shared playback
fixture, unit fixtures and examples. Fixture teardown joins decoder workers
before deleting media; panic unwinding releases the directory as well. The
corpus app harness removes profiles when setup or launch fails, and always
removes them after close. The catalog's temporary rewritten module is also
removed immediately after import.

The campaign now gives its children an isolated `TMPDIR` and compares file,
directory and byte counts against its empty starting state, even after a
child failure. Any growth fails the campaign; the outer directory guard then
removes its own tree. The guard's regression checks cover a successful child,
an exception exit and a deliberately leaking child. A failed-extension launch
also returns to an empty temporary tree. This prevents repeated playback
runs from accumulating generated media in the host temporary directory.

Presenter fixtures used by the audio campaign also retain their temporary
Blender area until both the host and worker threads have stopped. The audio
presenter test joins decoder workers during panic unwinding.

The guard uses a short owned directory and sets both `TMPDIR` and
`MAC_CHROMIUM_TMPDIR` for generated test media, harness profiles and
Chromium temporary files without exceeding macOS's socket-path limit.
Frameworks using `NSTemporaryDirectory()` can ignore these variables;
the guard only measures its owned tree. Actual Chromium cleanup remains
subject to the device smoke and final campaign checks. It checks for growth
after each paired app, before reserving, before the suites and at campaign exit.
The report is published as running before launch and must bracket the final
manifest's timestamps to qualify. Signal regressions verify that SIGTERM
and SIGINT join the child and remove the owned tree, including a cooperative
child that joins a grandchild in a separate session. The app and bridge use
one shared close operation for normal close, signals and disconnect; the
outer owners wait for teardown before removing profiles or releasing leases.
The first resumed Electron smoke exposed an empty driver artifacts directory
after a completed Today loop. Driver artifacts now live inside the owned app
profile, so their deletion is awaited with profile teardown. The failed smoke
is retained as a cleanup failure and supplies no native qualification.
An uncatchable process
kill still relies on the reservation's fixed expiry. The exclusive paired
measurement phase releases its reservation before the sixty playback-suite
runs, which each acquire an ordinary lease for their own duration.

The next diagnostic completed seven rows on AirPods Pro, 48 kHz with
256-frame native callbacks, under four stress threads. This candidate is
unqualified. The paired latency results were:

| Case | Today p50/p95/max (ms) | Native p50/p95/max (ms) |
| --- | --- | --- |
| Shared prepared level edit, 60 s loop | 173.286 / 175.887 / 176.096 | 169.881 / 171.948 / 172.448 |
| Distinct unprepared seek | 173.654 / 175.571 / 175.857 | 182.588 / 187.121 / 188.054 |
| Shared unprepared seek | 172.962 / 175.584 / 175.675 | 178.495 / 182.761 / 183.661 |
| Shared speed edit | 172.783 / 175.676 / 175.775 | 178.568 / 180.801 / 186.334 |

The distinct speed row failed final validation after its cold warm Play
waited 120.243 ms and invoked A3, omitting 1,280 frames per voice. Its thirty
later timed starts had zero start-program, real-time, omission, spread and
A3 counters, but that does not excuse the initial dropout. Its retained
partial native samples were 180.755 / 184.555 / 185.211 ms against Today's
173.384 / 175.879 / 175.982 ms; these are raw figures, not a validated row.
One shared-seek
pair was discarded under the fixed sharply unequal load criterion; its raw
results remain retained, along with the replacement pair's latency misses.
These new misses exceed the historical AirPods allowance and are blockers.
Both engines passed the per-app temporary-tree checks, and the failed
campaign's outer guard removed its empty owned tree.

Control profiles matched twelve seek and twelve speed Play receipts.
Median UI-to-control-receipt was 9.605 / 9.014 ms, respectively. Median graph
build-and-send was only 0.160 ms for the ten dirty seek starts; the speed
starts already had their edited projects installed. Separate synchronous
Pause and Exact Seek acknowledgements precede Play, each potentially
crossing a device period. Removing those waits must preserve producer FIFO
order and individual errors. Its effect on output latency needs measurement:
paused preparation can overlap the existing acknowledgement waits.

The unchanged runtime binary also passed one plain and one stressed
40-test headless smoke, with six exact mapped PCM comparisons in each and
zero temporary growth. Cold first-buffer publication maxima were 7.820 /
8.143 ms, below the enforced 10.666667 ms bound. Three observer polls were
later: 10.940 / 11.411 ms plain and 10.700 ms stressed. These remain recorded
separately. The deadline uses the earlier of the producer's post-release
timestamp and the observer's post-acquire timestamp, both valid upper
bounds on actual publication. The final thirty-plus-thirty campaign remains
required.
