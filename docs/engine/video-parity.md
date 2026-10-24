# Video reading

How a decoded video frame becomes working colour, on every surface:
playback, pause, scrub, thumbnails and export. Quality bar: QUALITY-1 and
QUALITY-2: colour follows the standards the stream's reading names, and
the proof is closeness to a high-precision reference; where today's viewer
departs from the standard, the standard wins. Cleanups against today are rows in
[quality-ledger.md](quality-ledger.md).

## The path

1. **Decode.** One backend policy (`pm_media::decode::backends`):
   VideoToolbox first for H.264, HEVC, ProRes and hardware AV1, in the
   stream's own range; FFmpeg or dav1d only when VideoToolbox refuses the
   stream or cannot produce its format. Display and export frames come from
   the same decoder kind; ProRes decodes to 16-bit `v216` for export and
   10-bit `x422` for display (amendment 6), which differ by nothing a
   10-bit source holds.
2. **Import.** Planes reach the GPU as they are (CoreVideo IOSurfaces, or
   the decoder's own pool memory) and pm-gpu normalizes them to f32 Y′CbCr
   with the format's range constants (`pm_protocol::color::normalization`).
3. **Reading.** `pm_media::probe::color::video_reading` is the one table
   for untagged and partly tagged video, shared with the source monitor:
   tags win field by field; matrix BT.601 for at most 576 lines and under
   1280 wide, else BT.709; transfer the sRGB curve; primaries SMPTE 170M
   at the same SD sizes, else Rec.709 (owner decision VIDEO-SD-1); range
   limited. Interpret Footage overrides, then the host's per-file
   override, replace it. Image-sequence renditions keep their authored
   BT.601 matrix.
4. **Convert.** The source pass (`shaders/source.wgsl`) reconstructs chroma
   at its siting (`pm_color::chroma`: Catmull-Rom across, linear down for
   4:2:0; linear for 4:2:2), applies the matrix, and in float converts
   primaries (BT.601, BT.2020, Display P3) to Rec.709 in linear light.
   Premultiplied sources convert straight colour. A reading that changes colour (a gamut
   conversion, a tone map) clips the signal to 0..1 before linearising it,
   as a display does.
5. **HDR** (owner decision HDR-1). PQ and HLG are decoded per BT.2100
   (HLG through the OOTF for a 1000 cd/m² display, BT.2408's route to PQ
   light) and tone mapped with the BT.2390 EETF, per component in the PQ
   domain, from the content peak (mastering display, else 1000 cd/m²) to
   SDR white at 203 cd/m² (`pm_color::tonemap`). HDR classes the gate
   does not cover (metadata, Dolby Vision, other codecs) route to the
   legacy renderer unless interpreted.
6. **Store.** The converted frame is stored at the project's working
   format, and never below half float for 10-bit or HDR sources.

## Invariants

- One reading: no surface, sampling mode or layer level changes a frame's
  colour. The converted-frame cache key holds the interpretation, never
  the surface.
- Preview = export: the same composition time draws identical working
  values when playing, paused and exported (`tests/video_reading.rs`).
- Depth is honoured: storage follows the project, deep sources never pass
  through 8 bits.
- Opening a project never reads a whole video: the probe/index cache keys a
  file by its stamp (device, inode, size, modification and change time)
  and its sampled fingerprint (`pm_media::probe::cache`).

## Proof

`tests/video_reading.rs` renders each clip of `tests/fixtures/video`
(one per behaviour: matrices, ranges, coded padding, 10-bit, alpha, wide
gamut SDR, PQ/HLG, ProRes) at 8, 16 and 32 bits, and compares it with an
f64 conversion of the decoded Y′CbCr and with today's paused and playing
captures. The engine must be within its stated bound at each depth and
at least as close as both captures, overall and on flat-chroma pixels.
The shift from today's paused composition is reported for the ledger.

## Performance

`examples/video_bench.rs` times one clip (cold and prepared first frame,
scrub P50/P95, paced playback drops and CPU, export) for a paired,
alternating comparison with today's viewer. Results stay outside Git.
