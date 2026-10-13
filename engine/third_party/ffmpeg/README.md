# Powermove's FFmpeg build

A reproducible, LGPL-only FFmpeg for macOS arm64, built from pinned and checksummed
sources. It replaces the bundled `ffmpeg-static` binary, a third-party build whose
configuration includes GPL and nonfree components (plan section 24 item 1, decision 12). The engine
links its dylibs; the `ffmpeg` and `ffprobe` executables come from the same configuration
for transcription, agent media tools, the CLI, and test verification.

The desktop app, the serve and MCP hosts, and the tests run this build; `ffmpeg-static`
is gone. `package.sh` stages it for packaging (below), and
`apps/desktop/src/main/ffmpeg-binary.ts` resolves it.

## Build

```sh
engine/third_party/ffmpeg/build.sh        # clean build, then verify.sh
bun engine/third_party/ffmpeg/smoke.ts    # functional checks against today's call sites
```

A clean build takes 2 min 42 s on an M4 Max (14 jobs, downloads cached). Every run
starts from an empty `work/`; downloads are cached in `downloads/` and re-verified
against their checksums before every extraction. `out/`, `work/`, and `downloads/` are
gitignored.

Builds are bit-for-bit reproducible with the same toolchain: three clean builds
(two in this directory, one with different `PM_FFMPEG_WORK`, `PM_FFMPEG_OUT`, and
`PM_FFMPEG_DOWNLOADS` and fresh downloads) produced identical binaries and identical
`manifest.json` files. A different Xcode or SDK changes the binaries; the manifest
records which was used.

Environment overrides: `PM_FFMPEG_OUT`, `PM_FFMPEG_WORK`, `PM_FFMPEG_DOWNLOADS`,
`PM_FFMPEG_JOBS`, `PM_FFMPEG_PYTHON` (python 3.10 or newer, for meson), and
`PM_FFMPEG_SKIP_VERIFY=1`.

### Build requirements

- macOS on Apple silicon with the Xcode command line tools (clang, the macOS SDK, `lipo`,
  `otool`, `codesign`).
- `cmake`, `ninja`, `pkg-config`, `autoconf`, `automake`, `libtool` (as `glibtoolize`),
  `xz`, and python 3.10 or newer, for example from Homebrew:
  `brew install cmake ninja pkg-config autoconf automake libtool xz python`.
- Meson is not taken from the system: the pinned meson source in `sources.lock` runs from
  the work directory.

The script clears `CFLAGS`, `LDFLAGS`, `PKG_CONFIG_PATH`, `CMAKE_PREFIX_PATH`, and
similar variables, points `PKG_CONFIG_LIBDIR` at its own prefix, and tells CMake to
ignore `/opt/homebrew` and `/usr/local`, so no Homebrew library can be linked by
accident. `verify.sh` checks that afterwards with `otool -L`.

## What is built

| Component | Version | License | Why |
|---|---|---|---|
| FFmpeg | 9.0.2 | LGPL-2.1-or-later | |
| zlib | 1.3.2 | Zlib | PNG, MOV compressed headers, freetype |
| libvpx | 1.17.0 | BSD-3-Clause (+ patent grant) | VP8 and VP9 encode; VP8/VP9 alpha decode |
| Opus | 1.6.1 | BSD-3-Clause (+ patent grants) | Opus encode for WebM proxies |
| dav1d | 1.5.4 | BSD-2-Clause | AV1 decode |
| SVT-AV1 | 4.2.0 | BSD-3-Clause-Clear (+ AOM patent license) | AV1 encode (fixtures, after-cutover export) |
| FreeType | 2.14.3 | FTL (BSD-style) | `drawtext` |
| HarfBuzz | 14.6.0 | MIT | `drawtext` (FFmpeg requires it alongside FreeType) |
| zimg | 3.0.6 | WTFPL | `zscale`, the SDR color reference (plan sections 2 and 25) |
| meson | 1.12.1 | Apache-2.0 | build tool only, never shipped |

Dependencies are static libraries linked into the FFmpeg dylibs (zlib included: the
build fails verification if anything links the system `libz`). The output links only
`@rpath` FFmpeg libraries and these macOS system libraries and frameworks:
`libSystem`, `libc++` (HarfBuzz and zimg are C++), `libiconv` and `libbz2` (subtitle
charset conversion and bzip2-compressed Matroska and FLV streams, used from the OS
rather than built), AudioToolbox, CoreAudio, CoreFoundation, CoreMedia, CoreServices,
CoreVideo, and VideoToolbox.

FFmpeg configuration highlights (the full line is in `build.sh` and in
`out/manifest.json`):

- no `--enable-gpl`, no `--enable-nonfree`, no `--enable-version3`: the build stays
  LGPL 2.1 or later, which `ffmpeg -L` reports;
- `--disable-autodetect`, then each external library enabled explicitly, so nothing
  found on the build machine changes the result;
- `--enable-videotoolbox --enable-audiotoolbox`: VideoToolbox decode (hwaccel) and the
  `h264_videotoolbox`, `hevc_videotoolbox`, and `prores_videotoolbox` encoders;
  `aac_at`;
- `--disable-encoder=prores,prores_aw,prores_ks`: ProRes is encoded only through
  VideoToolbox (decision 12); `--disable-decoder=prores_raw`: camera RAW waits for
  counsel (section 24 item 4).
- no x264, x265, LAME, fdk-aac, libaom, or any GPL filter (`frei0r`, `vidstab`, ...).

### ProRes decode

FFmpeg's native `prores` decoder stays in the build: it is the decoder FFmpeg attaches
the VideoToolbox ProRes hwaccel to (`-hwaccel videotoolbox`), and today's import
proxies, agent frame tools, and transcription read ProRes sources through it. Section 13
moves the engine's ProRes decode to VideoToolbox directly. Whether the `ffmpeg`
executable should also refuse software ProRes decode (which would make it unable to
read ProRes on machines without a ProRes engine, such as base M1) is a product and
licensing question for Phase 2; removing it is one line in `build.sh`
(`--disable-decoder=prores`) and the matching row in `required-components.tsv`.

## Packaging

```sh
engine/third_party/ffmpeg/package.sh [build-out-dir] [stage-dir]   # bun run stage:ffmpeg in apps/desktop
```

`package.sh` runs `verify.sh` on a build (default `$PM_FFMPEG_DIR`, then the shared
`~/Library/Caches/powermove-engine/ffmpeg/out`), stages only what ships into
`apps/desktop/vendor/ffmpeg` (gitignored): `bin/ffmpeg`, `bin/ffprobe`, each dylib once
under its install name as a regular file, `share/licenses/`, `manifest.json`, and
`NOTICE.txt` (the LGPL notice with the exact source URL and SHA-256). It then checks the
stage (checksums against the manifest, every `@rpath` dependency inside `lib/`, the LGPL
from both executables, the VideoToolbox encoders present, `libx264` and `prores_ks`
absent) and replaces the stage atomically. electron-builder copies the stage to
`Contents/Resources/ffmpeg`; `scripts/after-pack.mjs` refuses a bundle without it, and
`scripts/release-mac.mjs` checks that every binary there carries the app's Team ID and the
hardened runtime.

At run time the packaged app uses only `Contents/Resources/ffmpeg`. Development and tests
use `$PM_FFMPEG_DIR`, then `apps/desktop/vendor/ffmpeg`, then the shared build. The CLI
host (`packages/cli`, which no longer depends on `ffmpeg-static`) uses
`$POWERMOVE_FFMPEG`, then `$PM_FFMPEG_DIR` or the shared build, then `ffmpeg` from PATH.

## Output

```
out/
  bin/ffmpeg, bin/ffprobe        rpath @executable_path/../lib
  lib/libav*.dylib, libsw*.dylib install names @rpath/<lib>.<major>.dylib; rpath @loader_path
  lib/pkgconfig/*.pc             relocatable (prefix=${pcfiledir}/../..), for bindgen
  include/                       headers
  share/licenses/<component>/    license and patent texts of every shipped component
  manifest.json                  versions, checksums, configure line, toolchain, outputs
```

All binaries are arm64, minimum macOS 13.0 (Electron 44's minimum), ad-hoc signed by
the linker. Release packaging re-signs them with the Team ID and hardened runtime.

`manifest.json` records each source's version, URL, SHA-256, and license; the configure
flags and `-buildconf`; the toolchain (clang, SDK, cmake, ninja, meson, python,
pkg-config, autotools); and for every shipped binary its SHA-256, size, install name,
rpaths, minimum macOS, and linked libraries. It contains no timestamps or local paths.

Install prefix `/powermove/ffmpeg` is a placeholder compiled into the binaries so no
local path reaches them; nothing is read from it at run time. `-ffile-prefix-map`
keeps the work directory out of dependency `__FILE__` strings, and FFmpeg itself is
built in-tree with relative source paths.

## Verification

`verify.sh [out-dir]` runs at the end of every build and checks:

- `ffmpeg -L` and `ffprobe -L` state the LGPL 2.1 or later; `-buildconf` contains no
  `gpl`, `nonfree`, or `version3`;
- every binary is arm64, minimum macOS 13.0, validly signed, links only `@rpath` and
  system libraries, and contains no local path (`/Users/`, `/opt/homebrew`,
  `/usr/local`, the work directory);
- every dylib's install name is `@rpath/` plus a name in `lib/` that resolves to it;
- `ffmpeg` and `ffprobe` start with no `DYLD_*` help;
- every component in `required-components.tsv` is present, or absent when it must be;
- `manifest.json` checksums match the files.

`required-components.tsv` lists every protocol, device, demuxer, muxer, decoder,
encoder, filter, and hwaccel Powermove uses today, with its call sites, collected from
`render-encoder.ts`, `media-proxy.ts`, `agent-tools/media-ffmpeg.ts` and
`media-tools.ts`, `transcription/audio.ts`, `shared/media-formats.ts`,
`apps/desktop/scripts/test-mcp.mjs`, the e2e specs, the unit tests that spawn the bundled binary,
and `e2e/fixtures/gen.sh`, plus the components plan section 24 item 1 names.

`smoke.ts` imports today's argument builders from the desktop app (not copies) and runs
them against the build: agent frames, contact sheets, triage scans, loudness and
silence analysis, waveforms, transcription decode, all four import-proxy converters,
the export audio mux, the `test-mcp.mjs` and e2e verification shapes, VideoToolbox
hwaccel decode, HEVC, AV1, VP8, `zscale`, scene detection, `bwdif`, and `aac_at`. It
checks decoded pixels and parsed results, not only exit codes. It also confirms that
`render-encoder.ts` fails as expected (below) and that `h264_videotoolbox` and
`prores_videotoolbox` produce correct output from the same arguments with the encoder
swapped.

## What broke with this build, and how it was switched

These call sites used encoders the build excludes. All of them now run this build:
`render-encoder.ts` encodes with `h264_videotoolbox` (High profile, `-g 250`) and
`prores_videotoolbox -profile:v 4444` (`ayuv64le` with alpha, `p416le` opaque), both
with `-allow_sw 1` and frames tagged 1-13-1 with `setparams`; the test fixtures below
encode through VideoToolbox. `export-video-frames.spec.ts` uses an 8 Mbps
`h264_videotoolbox` source of flat frames in place of lossless x264. The table records
the original analysis.

Every real MP4 or ProRes export runs through `encoderArgs` in `render-encoder.ts`, so
each test that completes one fails with this build until that function moves to
VideoToolbox. Tests that replace the `render:*` IPC handlers with mocks never spawn
ffmpeg and are not affected: `native-export-performance.spec.ts:6` (pixel parity, both
cases) and `:111` (GPU loss recovery, both cases), `render-queue.test.ts`, and
`export-destination.spec.ts`, which cancels at the save dialog before the encoder starts.
WebM, still, PNG-sequence, code and project exports do not use the encoder.

| Call site | Uses | Phase 2 replacement |
|---|---|---|
| `apps/desktop/src/main/render-encoder.ts:15` (`encoderArgs`, MP4 export, agent `export_video` mp4) | `libx264` | `h264_videotoolbox`; frames need `setparams` color tags, because VideoToolbox takes primaries and transfer from frame attachments (shown in `smoke.ts`). Section 17: the engine encodes through VideoToolbox |
| `apps/desktop/src/main/render-encoder.ts:15` (ProRes export, agent `export_video` prores) | `prores_ks` with `-alpha_bits 16` | `prores_videotoolbox -profile:v 4444 -pix_fmt ayuv64le` after today's scale filter, frames tagged with `setparams` (alpha and exact color kept, shown in `smoke.ts`). Do not hand it BGRA: VideoToolbox then converts with the Rec.709 matrix but the stream is tagged smpte170m regardless of `-colorspace`, so 255,0,0 decodes as 234,0,3 |
| `apps/desktop/src/main/render-encoder.test.ts:15` and `:41` | ProRes 4444 alpha, and MP4 with real frames, through `RenderEncoder` | follows `render-encoder.ts` |
| `apps/desktop/e2e/export-mp4.spec.ts:7` | MP4 export with AAC audio through `render-encoder.ts`, asserts `avc1` | follows `render-encoder.ts` |
| `apps/desktop/e2e/export-mp4.spec.ts:61` | cancels an MP4 export mid-run; ffmpeg exits with "Unknown encoder 'libx264'" about 25 ms after start, so the run ends in an error instead of `{ cancelled: true }` | follows `render-encoder.ts` |
| `apps/desktop/e2e/export-video-frames.spec.ts:11` (all 5 `mp4` export cases: WebM source at 10 and 30 fps, after import and after restore, plus the MP4-source case) | MP4 export through `render-encoder.ts`, then a per-frame level check within 5 code values | follows `render-encoder.ts`; the per-frame tolerance needs rechecking with VideoToolbox rate control |
| `apps/desktop/e2e/native-export-performance.spec.ts:55` (export at `:90`) | HD and 4K MP4 export profile through `render-encoder.ts` | follows `render-encoder.ts` |
| `apps/desktop/e2e/media-intelligence.spec.ts:464` (export at `:540`) | MP4 export with an SRT caption sidecar through `render-encoder.ts` | follows `render-encoder.ts` |
| `apps/desktop/e2e/ae-audit-implementation.spec.ts:95` (export at `:98`) | queued ProRes 4444 alpha delivery through `render-encoder.ts`, asserts `ap4h` | follows `render-encoder.ts`; `prores_videotoolbox -profile:v 4444` also writes `ap4h` (checked in `smoke.ts`) |
| `apps/desktop/scripts/test-mcp.mjs:77` (inspected at `:79`-`:83`) | agent `export_video` MP4 with audio through `render-encoder.ts`, then decode, audio and pixel checks | follows `render-encoder.ts` |
| `apps/desktop/src/main/agent-tools/media-tools.test.ts:36` | `libx264` fixture | `h264_videotoolbox` |
| `apps/desktop/e2e/agent-media-tools.spec.ts:30` | `libx264` fixture | `h264_videotoolbox` |
| `apps/desktop/e2e/media-intelligence.spec.ts:59` | `libx264` fixture | `h264_videotoolbox` |
| `apps/desktop/e2e/video-scrubbing.spec.ts:31` | `libx264` fixture, fixed 120-frame GOP | `h264_videotoolbox` with `-g 120` (closed GOP spacing should be checked) |
| `apps/desktop/e2e/export-video-frames.spec.ts:19` | `libx264 -crf 0` lossless H.264 source | VideoToolbox has no lossless H.264; a lossless source needs another codec (FFV1, lossless VP9 as the WebM branch already does, or high-bitrate `h264_videotoolbox` with a tolerance) |
| `apps/desktop/e2e/import-prores-alpha.spec.ts:22` | `prores_ks` 4444 alpha fixture | `prores_videotoolbox -profile:v 4444` |
| `apps/desktop/e2e/fixtures/gen.sh` (h264-aac.mp4, h264-aac.mov, converted.mkv; prores422.mov, prores4444-alpha.mov and the hevc-alpha.mov made from it; tone.mp3) | `libx264`, `prores_ks`, `libmp3lame` | skipped gracefully by `gen.sh`'s encoder checks; committed fixtures stay valid. HEVC already prefers `hevc_videotoolbox`, AV1 falls back to `libsvtav1` |

Everything else used today, including every decoder and demuxer behind the import
formats in `shared/media-formats.ts`, is present (`verify.sh` checks each one).

MP3 encode (`libmp3lame`) is LGPL and could be added; nothing outside `gen.sh` uses it.

Compared with the bundled `ffmpeg-static` 5.3.0 (FFmpeg 6.0, configured with
`--enable-gpl --enable-nonfree --enable-version3`; its own `ffmpeg -L` says "This
version of ffmpeg has nonfree parts compiled in. Therefore it is not legally
redistributable."), this build drops: the encoders `libx264`, `libx264rgb`, `libx265`,
`libkvazaar`, `prores`, `prores_aw`, `prores_ks`, `libmp3lame`, `libaom-av1`,
`libvorbis`, `libtheora`, `libwebp`, `libwebp_anim`, `libopenjpeg`, `hap`; the decoders
`libaom-av1`, `libopenjpeg`, `libvorbis` (native equivalents remain); and the filters
that are GPL or need components not enabled here (`eq`, `hqdn3d`, `boxblur`, `cropdetect`,
`blackframe`, `mpdecimate`, `pullup`, `subtitles`/`ass`, `libvmaf`, `vidstab*`,
`coreimage`, and others). No call site uses any of them. `libvmaf` (BSD-2-Clause-Patent)
is LGPL-compatible and could be added if section 17's VMAF comparison runs through
FFmpeg.

## Known limits

- macOS arm64 only. The plan's `packages/cli` darwin-x64 prebuilds and the later Linux
  and Windows targets need their own builds.
- `libbz2` and `libiconv` link from the OS (`/usr/lib`), not from pinned sources. They
  are system libraries, so the output stays self-contained.
- Reproducibility is shown on one machine and toolchain. zimg has no release tarball
  with a generated `configure`, so `autogen.sh` runs the host's autoconf, automake and
  libtool; those, cmake, ninja and pkg-config come from the host and are recorded in
  `manifest.json`, not pinned. Meson is pinned.
- `--extra-libs="-lc++ -liconv"` applies to every library, so all seven dylibs link
  `libc++` and `libiconv`, though only `libavfilter` needs `libc++` (HarfBuzz and zimg)
  and only `libavcodec` needs `libiconv`.
  Both are system libraries; this costs nothing at run time beyond the load command.
- The software `prores` decoder stays (see "What is built"); whether to drop it under
  decision 12 is an open decision.
- Binaries are ad-hoc signed by the linker. Team ID signing and the hardened runtime
  belong to release packaging.

## Licensing obligations when shipping

LGPL 2.1 compliance for the dylibs and executables: ship `share/licenses/` with the
app, state that the app uses FFmpeg under the LGPL with a link to the exact source
(`ffmpeg-9.0.2.tar.xz`, SHA-256 in `sources.lock`) and this build script, and keep the
libraries as replaceable dylibs (they are). BSD-licensed dependencies require their
notices (`share/licenses/`). libvpx, Opus, and SVT-AV1 carry patent grants
(`PATENTS`, `PATENTS.md`), also copied.

## Pins and how they were checked

`sources.lock` pins every source by version, URL, and SHA-256; `build.sh` refuses any
archive whose hash differs. When the pins were set (2026-10-11), each was the current
stable release, and each hash was cross-checked against upstream:

| Source | Cross-check |
|---|---|
| FFmpeg 9.0.2 | GPG signature `ffmpeg-9.0.2.tar.xz.asc`, good, FFmpeg release signing key FCF9 86EA 15E6 E293 A564 4F10 B432 2F04 D676 58D8 |
| zlib 1.3.2 | GPG signature `.asc`, good, Mark Adler 5ED4 6A67 21D3 6558 7791 E2AA 783F CD8E 58BC AFBA |
| FreeType 2.14.3 | GPG signature `.sig`, good, Werner Lemberg E306 7470 7856 409F F194 8010 BE6C 3AAC 63AD 8E3F |
| dav1d 1.5.4 | VideoLAN's `dav1d-1.5.4.tar.xz.sha256` matches |
| Opus 1.6.1 | Xiph's `SHA256SUMS.txt` matches |
| HarfBuzz 14.6.0, meson 1.12.1 | GitHub release asset digests match |
| libvpx 1.17.0, SVT-AV1 4.2.0, zimg 3.0.6 | tag archives (no upstream checksum): extracted trees are identical to shallow clones of the tags `v1.17.0` (chromium.googlesource.com/webm/libvpx, 6df3ec34), `v4.2.0` (gitlab.com/AOMediaCodec/SVT-AV1, 9292ec8e), and `release-3.0.6` (github.com/sekrit-twc/zimg, f819b14e) |

To update a pin: change its line in `sources.lock`, cross-check the new hash the same
way, record it here, rebuild, and run `verify.sh` and `smoke.ts`.
