#!/usr/bin/env bash
# Generates pm-media's regression corpus: the S6 random-access corpus
# (docs/engine/spikes/S6.md) plus the professional and legacy formats section 13
# lists that S6 did not cover (interlaced MXF, DNxHD and DNxHR, CineForm, DV,
# MJPEG, fragmented MP4, AVI) and audio files for priming.
#
#   engine/crates/pm-media/tests/corpus/gen.sh [out-dir]
#
# Every video frame is unique (testsrc2 animates a counter and moving shapes),
# so a random access that lands one frame off can never hash equal to the
# right frame. Output (synthetic media only, outside the repository) defaults
# to ~/Library/Caches/powermove-engine/media-corpus; tests read it from
# PM_MEDIA_CORPUS or that default.
#
# Encoders: Homebrew FFmpeg (libx264, libx265, libvpx, SVT-AV1, VideoToolbox,
# prores_ks, dnxhd, cfhd, dvvideo, mjpeg, mpeg2video, mpeg4, h263, wmv2, flv).
# They only make test media; nothing here ships. Takes about three minutes at
# -threads 4.
set -euo pipefail

OUT="${1:-${PM_MEDIA_CORPUS:-$HOME/Library/Caches/powermove-engine/media-corpus}}"
FFMPEG="${FFMPEG:-/opt/homebrew/bin/ffmpeg}"
[[ -x "$FFMPEG" ]] || { echo "error: needs Homebrew FFmpeg at $FFMPEG (brew install ffmpeg)" >&2; exit 1; }
mkdir -p "$OUT.partial"
cd "$OUT.partial"

ff() { nice -n 15 "$FFMPEG" -hide_banner -loglevel error -y -threads 4 "$@"; }
src() { # rate size [pixfmt]
  printf 'testsrc2=size=%s:rate=%s,format=%s' "$2" "$1" "${3:-yuv420p}"
}
NTSC24=24000/1001
NTSC30=30000/1001
NTSC60=60000/1001
SMALL=320x180

# ── S6 corpus (gen-corpus.sh in engine/spikes/s6-decode) ─────────────────────

ff -f lavfi -i "$(src $NTSC30 $SMALL)" -frames:v 150 -c:v libx264 -profile:v high \
  -x264-params "bframes=3:b-pyramid=normal:keyint=48:min-keyint=48:scenecut=0" -pix_fmt yuv420p h264-bpyramid.mp4
ff -f lavfi -i "$(src 25 $SMALL)" -frames:v 150 -c:v libx264 -profile:v high \
  -x264-params "bframes=3:b-pyramid=normal:keyint=30:min-keyint=30:scenecut=0:open-gop=1" -pix_fmt yuv420p h264-opengop.mov
ff -f lavfi -i "$(src 24 $SMALL)" -frames:v 150 -c:v libx265 -tag:v hvc1 \
  -x265-params "keyint=24:min-keyint=24:scenecut=0:open-gop=1:bframes=4:log-level=error" -pix_fmt yuv420p hevc-opengop.mp4
ff -f lavfi -i "$(src $NTSC24 $SMALL yuv420p10le)" -frames:v 120 -c:v libx265 -tag:v hvc1 \
  -x265-params "keyint=24:min-keyint=24:scenecut=0:open-gop=1:bframes=4:log-level=error" -pix_fmt yuv420p10le hevc-main10.mov
ff -f lavfi -i "$(src $NTSC30 $SMALL p010le)" -frames:v 120 -c:v hevc_videotoolbox \
  -profile:v main10 -realtime 0 -b:v 2M -g 30 -tag:v hvc1 hevc-main10-vt.mov
ff -f lavfi -i "$(src 25 $SMALL yuv422p10le)" -frames:v 100 -c:v libx265 -tag:v hvc1 \
  -x265-params "keyint=25:min-keyint=25:scenecut=0:bframes=3:log-level=error" -pix_fmt yuv422p10le hevc-422-10.mov
ff -f lavfi -i "$(src 25 $SMALL yuv422p10le)" -frames:v 100 -c:v libx264 -profile:v high422 \
  -x264-params "bframes=2:keyint=25:min-keyint=25:scenecut=0" -pix_fmt yuv422p10le h264-422-10.mov
ff -f lavfi -i "$(src $NTSC24 $SMALL p210le)" -frames:v 72 -c:v prores_videotoolbox -profile:v hq prores422hq-vt.mov
ff -f lavfi -i "testsrc2=size=$SMALL:rate=$NTSC24,format=yuva444p,geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='mod(X+N*4,256)'" \
  -frames:v 72 -c:v prores_videotoolbox -profile:v xq -pix_fmt ayuv64le prores4444xq-vt.mov 2>/dev/null \
  || ff -f lavfi -i "testsrc2=size=$SMALL:rate=$NTSC24,format=yuva444p12le" \
       -frames:v 72 -c:v prores_ks -profile:v 5 -pix_fmt yuva444p12le prores4444xq-vt.mov
ff -f lavfi -i "testsrc2=size=$SMALL:rate=25,format=yuva444p,geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='mod(X+N*4,256)',format=yuva444p12le" \
  -frames:v 50 -c:v prores_ks -profile:v 4 -pix_fmt yuva444p12le prores4444-ks.mov
ff -ss 1.3 -i h264-bpyramid.mp4 -t 3 -c copy -avoid_negative_ts disabled editlist-trim.mov
ff -f lavfi -i "$(src $NTSC30 $SMALL yuv420p10le)" -frames:v 90 -c:v libvpx-vp9 -profile:v 2 \
  -pix_fmt yuv420p10le -g 30 -b:v 1M -deadline good -cpu-used 4 vp9-10bit.webm
ff -f lavfi -i "testsrc2=size=$SMALL:rate=30,format=yuva420p,geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='mod(X+N*4,256)'" \
  -frames:v 60 -c:v libvpx-vp9 -pix_fmt yuva420p -g 30 -b:v 1M -deadline good -cpu-used 4 -auto-alt-ref 0 vp9-alpha.webm
ff -f lavfi -i "testsrc2=size=$SMALL:rate=30,format=yuva420p,geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='mod(X+N*4,256)'" \
  -frames:v 60 -c:v libvpx -pix_fmt yuva420p -g 30 -b:v 1M -deadline good -cpu-used 4 -auto-alt-ref 0 vp8-alpha.webm
ff -f lavfi -i "$(src 30 $SMALL yuv420p10le)" -frames:v 90 -c:v libsvtav1 -g 30 -preset 8 -pix_fmt yuv420p10le av1-10bit.mp4
# ProRes in MXF (RDD 44) and in Matroska (frames without their icpf header).
ff -f lavfi -i "$(src 25 $SMALL yuv422p10le)" -frames:v 25 -c:v prores_ks -profile:v 3 prores-hq.mxf
# MXF at 29.97: timebase 1001/30000 (a numerator other than 1 reaches VideoToolbox).
ff -f lavfi -i "$(src $NTSC30 $SMALL yuv422p10le)" -frames:v 30 -c:v prores_ks -profile:v 3 prores-hq-2997.mxf
ff -f lavfi -i "$(src 25 $SMALL yuv422p10le)" -frames:v 25 -c:v prores_ks -profile:v 2 prores-422.mkv
for r in "$NTSC24:23976" "$NTSC30:2997"; do
  rate="${r%%:*}"; tag="${r##*:}"
  ff -f lavfi -i "$(src "$rate" $SMALL)" -frames:v 150 -c:v libvpx-vp9 -g 24 -b:v 600k \
    -deadline good -cpu-used 5 "webm-1ms-$tag.webm"
  ff -f lavfi -i "$(src "$rate" $SMALL)" -frames:v 150 -c:v libx264 -profile:v high \
    -x264-params "bframes=3:b-pyramid=normal:keyint=24:min-keyint=24:scenecut=0" -pix_fmt yuv420p -f mpegts "ts-90k-$tag.ts"
  ff -f lavfi -i "$(src "$rate" $SMALL)" -frames:v 150 -c:v libx264 -profile:v high \
    -x264-params "bframes=3:b-pyramid=normal:keyint=24:min-keyint=24:scenecut=0" -pix_fmt yuv420p \
    -video_track_timescale 600 "mov-600-$tag.mov"
  ff -f lavfi -i "$(src "$rate" $SMALL p210le)" -frames:v 100 -c:v prores_videotoolbox \
    -profile:v standard -video_track_timescale 600 "mov-600-prores-$tag.mov"
done
ff -f lavfi -i "$(src $NTSC24 3840x2160 p210le)" -frames:v 48 -c:v prores_videotoolbox -profile:v hq uhd-prores422hq.mov
ff -f lavfi -i "$(src $NTSC24 3840x2160 p010le)" -frames:v 48 -c:v hevc_videotoolbox \
  -profile:v main10 -realtime 0 -b:v 40M -g 24 -tag:v hvc1 uhd-hevc-main10.mov

# ── Formats S6 did not cover ─────────────────────────────────────────────────

# Interlaced MXF. XDCAM HD422-style long-GOP MPEG-2 4:2:2, 1080i59.94 top field
# first, in OP1a with index tables (B frames: temporal offsets in the index).
ff -f lavfi -i "$(src $NTSC30 1920x1080 yuv422p)" -frames:v 45 -c:v mpeg2video -pix_fmt yuv422p \
  -flags +ilme+ildct -top 1 -g 15 -bf 2 -b:v 50M -minrate 50M -maxrate 50M -bufsize 17825792 \
  -field_order tt mxf-xdcam422-1080i-tff.mxf
# DNxHD 1080i50, intra, in MXF OP1a (constant edit-unit size). FFmpeg's DNxHD
# encoder writes top field first whatever -top says; DV below covers bottom first.
ff -f lavfi -i "$(src 25 1920x1080 yuv422p)" -frames:v 30 -c:v dnxhd -b:v 120M -pix_fmt yuv422p \
  -flags +ildct mxf-dnxhd-1080i.mxf
# OP-Atom DNxHD: clip-wrapped essence (one KLV for every frame).
ff -f lavfi -i "$(src 25 1920x1080 yuv422p)" -frames:v 12 -c:v dnxhd -b:v 120M -pix_fmt yuv422p \
  -f mxf_opatom mxf-opatom-dnxhd.mxf
# DNxHR HQ (8-bit 4:2:2), HQX (10-bit 4:2:2), and 444 (10-bit 4:4:4).
ff -f lavfi -i "$(src $NTSC24 1280x720 yuv422p)" -frames:v 48 -c:v dnxhd -profile:v dnxhr_hq -pix_fmt yuv422p dnxhr-hq.mov
ff -f lavfi -i "$(src 25 1280x720 yuv422p10le)" -frames:v 50 -c:v dnxhd -profile:v dnxhr_hqx -pix_fmt yuv422p10le dnxhr-hqx.mov
ff -f lavfi -i "$(src 25 1280x720 yuv444p10le)" -frames:v 30 -c:v dnxhd -profile:v dnxhr_444 -pix_fmt yuv444p10le dnxhr-444.mxf
# CineForm (10-bit 4:2:2) in MOV and AVI.
ff -f lavfi -i "$(src $NTSC24 $SMALL yuv422p10le)" -frames:v 48 -c:v cfhd -quality film1 -pix_fmt yuv422p10le cineform.mov
ff -f lavfi -i "$(src 25 $SMALL yuv422p10le)" -frames:v 50 -c:v cfhd -pix_fmt yuv422p10le cineform.avi
# DV: NTSC 4:1:1 raw DV (no tables), PAL 4:2:0 in AVI, DVCPRO50 4:2:2 in MOV.
ff -f lavfi -i "$(src $NTSC30 720x480 yuv411p),setfield=bff" -frames:v 60 -c:v dvvideo -pix_fmt yuv411p -aspect 4:3 dv-ntsc.dv
ff -f lavfi -i "$(src 25 720x576 yuv420p),setfield=bff" -frames:v 50 -c:v dvvideo -pix_fmt yuv420p -aspect 16:9 dv-pal.avi
ff -f lavfi -i "$(src 25 720x576 yuv422p),setfield=bff" -frames:v 50 -c:v dvvideo -pix_fmt yuv422p -b:v 50M dvcpro50.mov
# MJPEG (full-range yuvj422p) in AVI and MOV.
ff -f lavfi -i "$(src 30 $SMALL yuvj422p)" -frames:v 60 -c:v mjpeg -q:v 3 -pix_fmt yuvj422p mjpeg.avi
ff -f lavfi -i "$(src 25 $SMALL yuvj420p)" -frames:v 50 -c:v mjpeg -q:v 3 -pix_fmt yuvj420p mjpeg.mov
# Fragmented MP4 (moof/trun instead of stbl sample tables).
ff -f lavfi -i "$(src 30 $SMALL)" -frames:v 120 -c:v libx264 -x264-params "bframes=2:keyint=30:min-keyint=30:scenecut=0" \
  -pix_fmt yuv420p -movflags +frag_keyframe+empty_moov+default_base_moof h264-fragmented.mp4
# Negative composition offsets (ctts version 1) and no edit list.
ff -f lavfi -i "$(src 25 $SMALL)" -frames:v 75 -c:v libx264 \
  -x264-params "bframes=3:b-pyramid=normal:keyint=25:min-keyint=25:scenecut=0" -pix_fmt yuv420p \
  -movflags +negative_cts_offsets -use_editlist 0 h264-negative-cts.mp4
# Variable frame rate: 30 fps with every fifth frame dropped (VFR timestamps).
ff -f lavfi -i "$(src 30 $SMALL)" -frames:v 150 -vf "select='not(eq(mod(n,5),4))'" -fps_mode vfr \
  -c:v libx264 -x264-params "bframes=0:keyint=30" -pix_fmt yuv420p vfr-h264.mp4
# MPEG-2 program stream and AVI with B frames (demux scan path).
ff -f lavfi -i "$(src 25 $SMALL)" -frames:v 75 -c:v mpeg2video -g 12 -bf 2 -b:v 4M -f vob mpeg2.vob

# The rest of section 13's containers (container_breadth): ASF, FLV, M2TS and
# MTS, 3GP, 3G2, Ogg, a raw MPEG-2 elementary stream, M4V, and AVI with B
# frames (no presentation timestamps: order from a decode pass).
W25="$(src 25 $SMALL)"
ff -f lavfi -i "$W25" -frames:v 75 -c:v wmv2 -g 25 -b:v 2M wrap-wmv2.wmv
ff -f lavfi -i "$W25" -frames:v 75 -c:v flv -g 25 -b:v 2M wrap-flv1.flv
ff -f lavfi -i "$W25" -frames:v 75 -c:v libx264 -x264-params "bframes=2:keyint=25:min-keyint=25:scenecut=0" \
  -pix_fmt yuv420p wrap-h264.flv
ff -f lavfi -i "$(src $NTSC30 $SMALL)" -frames:v 90 -c:v libx264 -x264-params "bframes=3:keyint=30:min-keyint=30:scenecut=0" \
  -pix_fmt yuv420p -f mpegts -mpegts_m2ts_mode 1 wrap-h264.m2ts
cp wrap-h264.m2ts wrap-h264.mts
ff -f lavfi -i "$(src 25 352x288)" -frames:v 75 -c:v h263 -g 25 -b:v 1M wrap-h263.3gp
ff -f lavfi -i "$W25" -frames:v 75 -c:v libx264 -x264-params "bframes=2:keyint=25:min-keyint=25:scenecut=0" \
  -pix_fmt yuv420p -f 3g2 wrap-h264.3g2
ff -f lavfi -i "$W25" -frames:v 75 -c:v libvpx -g 25 -b:v 1M -deadline good -cpu-used 5 wrap-vp8.ogv
ff -f lavfi -i "$W25" -frames:v 75 -c:v mpeg2video -g 12 -bf 2 -b:v 4M -f mpeg2video wrap-mpeg2.m2v
ff -f lavfi -i "$W25" -frames:v 75 -c:v mpeg4 -g 25 -bf 2 -b:v 2M wrap-mpeg4.m4v
ff -f lavfi -i "$W25" -frames:v 75 -c:v mpeg4 -g 25 -bf 2 -b:v 2M -vtag XVID wrap-mpeg4.avi
ff -f lavfi -i "$W25" -frames:v 75 -c:v libx264 -x264-params "bframes=2:keyint=25:min-keyint=25:scenecut=0" \
  -pix_fmt yuv420p wrap-h264.avi

# ── Audio (priming) ──────────────────────────────────────────────────────────

ff -f lavfi -i "sine=frequency=1000:sample_rate=48000" -t 2 -c:a libmp3lame -b:a 192k sine-lame.mp3
ff -f lavfi -i "sine=frequency=1000:sample_rate=48000" -t 2 -c:a aac_at -b:a 192k sine-aac-at.m4a
ff -f lavfi -i "sine=frequency=1000:sample_rate=48000" -t 2 -c:a aac -b:a 192k sine-aac-ff.m4a
ff -f lavfi -i "$(src 30 $SMALL)" -f lavfi -i "sine=frequency=440:sample_rate=48000" -t 2 \
  -c:v libx264 -pix_fmt yuv420p -c:a aac_at -b:a 128k h264-aac-at.mov

cd /
rm -rf "$OUT"
mv "$OUT.partial" "$OUT"
ls -l "$OUT"
