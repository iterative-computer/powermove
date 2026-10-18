# Desktop e2e media fixtures

This directory (`apps/desktop/e2e/fixtures/`, formerly `spikes/fixtures/` at the repository root) contains tiny, deterministic media files consumed by the desktop Playwright suite in `apps/desktop/e2e/` for import, export, and codec/container coverage. Run `./gen.sh` from any working directory to regenerate every fixture and `fixtures.json`. The script uses only generated FFmpeg filter sources; it does not depend on external media.

Run the specs that use them from `apps/desktop` (or `bun run --cwd apps/desktop test:e2e -- <spec>` from the repository root):

```sh
bun run test:e2e -- e2e/import-mp4.spec.ts e2e/import-prores.spec.ts
```

## Video contract

- 160×90, 30 fps.
- Normally 2.0 seconds: solid RGB red `[255,0,0]` from 0–1 s, then solid RGB green `[0,255,0]` from 1–2 s.
- Verify green at 1.5 s and red at 0.5 s. If a ProRes size cap forces a 1.2 s fixture, `fixtures.json` changes its verification points to 0.9 s and 0.3 s.
- Audio, where present, is a 440 Hz stereo sine at 48 kHz and approximately −12 dBFS peak.
- Alpha fixtures have an opaque colored left half and a fully transparent right half. Sample color in the opaque half (for example x=40, y=45); inspect alpha in the right half (for example x=120, y=45).

`expectedColorAt` records the source-color contract. Lossy YUV codecs and matrix/range round trips can decode one or two integer levels away from the nominal RGB triplet; verification should use a small tolerance.

## Animation contract

`animated.gif`, `animated-apng.png` (APNG) and `animated.webp` are the same animation in three containers: 160×90, three 100 ms frames of solid red, green, then blue, each with an opaque left half and a fully transparent right half. Importing one produces a 0.3 s clip, so frame *n* is on screen at `t = n / 10`. Chromium decodes all three through `ImageDecoder`, which is how Powermove reads them; FFmpeg cannot demux animated WebP at all.

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

## Still AVIF decoder regressions

`still-avif-{400,422,444}-full.avif` and `still-avif-icc-{p3,adobe}-alpha.avif`
are synthetic 192×128 color/alpha ramps encoded with Pillow 12.3/libavif
1.4.2 (4 threads, speed 8). ICC profiles are the system Display P3 and Adobe
RGB 1998 profiles attached to the synthetic samples. They cover monochrome,
chroma conversion and the premultiplied wide-gamut ICC upload boundary.

`still-red-*-12bpc.avif` are Chromium's synthetic 3×3 decoder test images from
[Chromium 152.0.7977.54](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/third_party/blink/web_tests/images/resources/avif/):
limited-range 420/422/444, alpha, and embedded ICC. Their license follows.
`pm-render/tests/stills.rs` compares each decoded compatibility bitmap's
SHA-256 against a Chromium ImageBitmap upload captured through the app;
the 12-bit RGB source reader retains its original precision. ImageIO rejects
some deep ICC/alpha containers; their level-1 bitmap decodes independently.

```text
// Copyright 2015 The Chromium Authors
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
//    * Redistributions of source code must retain the above copyright
// notice, this list of conditions and the following disclaimer.
//    * Redistributions in binary form must reproduce the above
// copyright notice, this list of conditions and the following disclaimer
// in the documentation and/or other materials provided with the
// distribution.
//    * Neither the name of Google LLC nor the names of its
// contributors may be used to endorse or promote products derived from
// this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
// "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
// LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
// A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
// OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
// LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
// DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
// THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```
