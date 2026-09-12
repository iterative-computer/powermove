# Electron migration codec fixtures

This directory contains tiny, deterministic media files for codec/container migration probes. Run `./gen.sh` from any working directory to regenerate every fixture and `fixtures.json`. The script uses only generated FFmpeg filter sources; it does not depend on external media.

## Video contract

- 160×90, 30 fps.
- Normally 2.0 seconds: solid RGB red `[255,0,0]` from 0–1 s, then solid RGB green `[0,255,0]` from 1–2 s.
- Verify green at 1.5 s and red at 0.5 s. If a ProRes size cap forces a 1.2 s fixture, `fixtures.json` changes its verification points to 0.9 s and 0.3 s.
- Audio, where present, is a 440 Hz stereo sine at 48 kHz and approximately −12 dBFS peak.
- Alpha fixtures have an opaque colored left half and a fully transparent right half. Sample color in the opaque half (for example x=40, y=45); inspect alpha in the right half (for example x=120, y=45).

`expectedColorAt` records the source-color contract. Lossy YUV codecs and matrix/range round trips can decode one or two integer levels away from the nominal RGB triplet; verification should use a small tolerance.

## Audio size exception

A 2.0 s, 48 kHz, stereo, 16-bit PCM WAV is 384,000 bytes before its header and cannot satisfy the 300 KiB fixture limit. `tone.wav` is therefore 1.5 s (288,000 bytes of samples); all compressed audio fixtures remain 2.0 s. The manifest records the duration.

## Regeneration

```bash
cd apps/desktop/e2e/fixtures
./gen.sh
```

The generator detects the requested FFmpeg encoders from `ffmpeg -encoders`. Missing encoders, failed optional image conversion, or failed HEVC-with-alpha conversion are represented in `fixtures.json` with a non-null `skipped` reason and no output file.

## hevc-alpha.mov (added by the integrator)

`avconvert` rejected `.mov` output in this runtime, but VideoToolbox encodes HEVC-with-alpha directly through ffmpeg — it just refuses a 160×90 session, so this fixture is 320×180:

```sh
ffmpeg -y -i prores4444-alpha.mov -vf scale=320:180 -c:v hevc_videotoolbox -pix_fmt bgra -alpha_quality 0.9 -tag:v hvc1 -an hevc-alpha.mov
```

Verified with `-pix_fmt rgba` readback at t=1.5: left `(80,90)` = `[20,255,7,255]`, right `(240,90)` = `[20,255,7,0]`.
