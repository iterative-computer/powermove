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
0.6.4 (both MIT, on the section 24 `cargo-deny` list; headers unmodified, hashes in
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
