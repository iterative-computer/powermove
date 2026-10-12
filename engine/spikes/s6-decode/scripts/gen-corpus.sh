#!/usr/bin/env bash
# Generates the S6 seek corpus: synthetic files whose every frame is unique, so a
# random-access frame that lands one frame off can never hash equal to the
# sequential one. The e2e fixtures alone cannot show that (two flat colors).
#
# Output goes to engine/spikes/s6-decode/corpus/ (gitignored) or $1.
# Encoders used here (libx264, libx265, libsvtav1, libvpx) only make test media;
# nothing from this script ships.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$HERE/corpus}"
FFMPEG="${FFMPEG:-/opt/homebrew/bin/ffmpeg}"
mkdir -p "$OUT"
cd "$OUT"

ff() { "$FFMPEG" -hide_banner -loglevel error -y "$@"; }

# testsrc2 animates every frame (moving shapes plus a running counter).
src() { # rate size frames [pixfmt]
  printf 'testsrc2=size=%s:rate=%s,format=%s' "$2" "$1" "${4:-yuv420p}"
}
NTSC24=24000/1001
NTSC30=30000/1001
SMALL=320x180

# Long-GOP H.264 with B-pyramid; MP4 writes an edit list for the B-frame delay.
ff -f lavfi -i "$(src $NTSC30 $SMALL 0)" -frames:v 150 -c:v libx264 -profile:v high \
  -x264-params "bframes=3:b-pyramid=normal:keyint=48:min-keyint=48:scenecut=0" \
  -pix_fmt yuv420p h264-bpyramid.mp4

# Open-GOP H.264 (non-IDR I frames with leading B frames).
ff -f lavfi -i "$(src 25 $SMALL 0)" -frames:v 150 -c:v libx264 -profile:v high \
  -x264-params "bframes=3:b-pyramid=normal:keyint=30:min-keyint=30:scenecut=0:open-gop=1" \
  -pix_fmt yuv420p h264-opengop.mov

# Open-GOP HEVC 8-bit (CRA with RASL leading pictures).
ff -f lavfi -i "$(src 24 $SMALL 0)" -frames:v 150 -c:v libx265 -tag:v hvc1 \
  -x265-params "keyint=24:min-keyint=24:scenecut=0:open-gop=1:bframes=4:log-level=error" \
  -pix_fmt yuv420p hevc-opengop.mp4

# HEVC Main10 4:2:0, long GOP, open GOP.
ff -f lavfi -i "$(src $NTSC24 $SMALL 0 yuv420p10le)" -frames:v 120 -c:v libx265 -tag:v hvc1 \
  -x265-params "keyint=24:min-keyint=24:scenecut=0:open-gop=1:bframes=4:log-level=error" \
  -pix_fmt yuv420p10le hevc-main10.mov

# HEVC Main10 from VideoToolbox (Apple encoder output).
ff -f lavfi -i "$(src $NTSC30 $SMALL 0 p010le)" -frames:v 120 -c:v hevc_videotoolbox \
  -profile:v main10 -realtime 0 -b:v 2M -g 30 -tag:v hvc1 hevc-main10-vt.mov

# HEVC 4:2:2 10-bit (exercises x422 from VideoToolbox).
ff -f lavfi -i "$(src 25 $SMALL 0 yuv422p10le)" -frames:v 100 -c:v libx265 -tag:v hvc1 \
  -x265-params "keyint=25:min-keyint=25:scenecut=0:bframes=3:log-level=error" \
  -pix_fmt yuv422p10le hevc-422-10.mov

# H.264 4:2:2 10-bit (the plan routes it to software).
ff -f lavfi -i "$(src 25 $SMALL 0 yuv422p10le)" -frames:v 100 -c:v libx264 -profile:v high422 \
  -x264-params "bframes=2:keyint=25:min-keyint=25:scenecut=0" -pix_fmt yuv422p10le h264-422-10.mov

# ProRes 422 HQ and 4444 XQ with alpha from VideoToolbox (Apple's encoder).
ff -f lavfi -i "$(src $NTSC24 $SMALL 0 p210le)" -frames:v 72 -c:v prores_videotoolbox \
  -profile:v hq prores422hq-vt.mov
ff -f lavfi -i "testsrc2=size=$SMALL:rate=$NTSC24,format=yuva444p,geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='mod(X+N*4,256)'" \
  -frames:v 72 -c:v prores_videotoolbox -profile:v xq -pix_fmt ayuv64le prores4444xq-vt.mov 2>/dev/null \
|| ff -f lavfi -i "testsrc2=size=$SMALL:rate=$NTSC24,format=yuva444p12le" \
  -frames:v 72 -c:v prores_ks -profile:v 5 -pix_fmt yuva444p12le prores4444xq-vt.mov

# ProRes 4444 with animated alpha from prores_ks (12-bit alpha path).
ff -f lavfi -i "testsrc2=size=$SMALL:rate=25,format=yuva444p,geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='mod(X+N*4,256)',format=yuva444p12le" \
  -frames:v 50 -c:v prores_ks -profile:v 4 -pix_fmt yuva444p12le prores4444-ks.mov

# Edit list that starts mid-GOP (stream copy trim): leading samples decode but
# are not presented.
ff -ss 1.3 -i h264-bpyramid.mp4 -t 3 -c copy -avoid_negative_ts disabled editlist-trim.mov

# VP9 profile 2 (10-bit, LSB-aligned planar in FFmpeg).
ff -f lavfi -i "$(src $NTSC30 $SMALL 0 yuv420p10le)" -frames:v 90 -c:v libvpx-vp9 -profile:v 2 \
  -pix_fmt yuv420p10le -g 30 -b:v 1M -deadline good -cpu-used 4 vp9-10bit.webm

# VP9 with alpha (decodes through libvpx, not FFmpeg's native vp9).
ff -f lavfi -i "testsrc2=size=$SMALL:rate=30,format=yuva420p,geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='mod(X+N*4,256)'" \
  -frames:v 60 -c:v libvpx-vp9 -pix_fmt yuva420p -g 30 -b:v 1M -deadline good -cpu-used 4 \
  -auto-alt-ref 0 vp9-alpha.webm

# VP8 with alpha (libvpx; libvpx has no external frame buffers for VP8).
ff -f lavfi -i "testsrc2=size=$SMALL:rate=30,format=yuva420p,geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='mod(X+N*4,256)'" \
  -frames:v 60 -c:v libvpx -pix_fmt yuva420p -g 30 -b:v 1M -deadline good -cpu-used 4 \
  -auto-alt-ref 0 vp8-alpha.webm

# AV1 10-bit (VideoToolbox AV1 on M3 and later; dav1d otherwise).
ff -f lavfi -i "$(src 30 $SMALL 0 yuv420p10le)" -frames:v 90 -c:v libsvtav1 -g 30 \
  -preset 8 -pix_fmt yuv420p10le av1-10bit.mp4

# Coarse timebases (plan section 2, Seeking bar).
# 1 ms Matroska timebase (WebM default).
for r in "$NTSC24:23976" "$NTSC30:2997"; do
  rate="${r%%:*}"; tag="${r##*:}"
  ff -f lavfi -i "$(src "$rate" $SMALL 0)" -frames:v 150 -c:v libvpx-vp9 -g 24 -b:v 600k \
    -deadline good -cpu-used 5 "webm-1ms-$tag.webm"
  # 90 kHz MPEG-TS with B frames.
  ff -f lavfi -i "$(src "$rate" $SMALL 0)" -frames:v 150 -c:v libx264 -profile:v high \
    -x264-params "bframes=3:b-pyramid=normal:keyint=24:min-keyint=24:scenecut=0" \
    -pix_fmt yuv420p -f mpegts "ts-90k-$tag.ts"
  # 1/600 MOV, B-frame H.264 and intra ProRes.
  ff -f lavfi -i "$(src "$rate" $SMALL 0)" -frames:v 150 -c:v libx264 -profile:v high \
    -x264-params "bframes=3:b-pyramid=normal:keyint=24:min-keyint=24:scenecut=0" \
    -pix_fmt yuv420p -video_track_timescale 600 "mov-600-$tag.mov"
  ff -f lavfi -i "$(src "$rate" $SMALL 0 p210le)" -frames:v 100 -c:v prores_videotoolbox \
    -profile:v standard -video_track_timescale 600 "mov-600-prores-$tag.mov"
done

# 4K timing files (decode cost on realistic sizes).
ff -f lavfi -i "$(src $NTSC24 3840x2160 0 p210le)" -frames:v 48 -c:v prores_videotoolbox \
  -profile:v hq uhd-prores422hq.mov
ff -f lavfi -i "$(src $NTSC24 3840x2160 0 p010le)" -frames:v 48 -c:v hevc_videotoolbox \
  -profile:v main10 -realtime 0 -b:v 40M -g 24 -tag:v hvc1 uhd-hevc-main10.mov

ls -l "$OUT"
