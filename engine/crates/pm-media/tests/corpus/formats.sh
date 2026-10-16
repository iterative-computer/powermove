#!/usr/bin/env bash
# Generates the format matrix of plan section 13 ("Formats at cutover") with
# our LGPL FFmpeg build (engine/third_party/ffmpeg, PM_FFMPEG_DIR): one file
# per format and variant, for tests/formats.rs and docs/engine/formats.md.
#
#   engine/crates/pm-media/tests/corpus/formats.sh [out-dir]
#
# Output (synthetic media only, outside the repository) defaults to
# $PM_MEDIA_CORPUS/formats, else ~/Library/Caches/powermove-engine/media-corpus/formats.
#
# Where our build has no encoder, a named tool writes the file and the
# reason is given next to it:
#   - H.264 4:2:2 10-bit, open-GOP and B-pyramid H.264, HEVC from x265: only
#     the GPL libx264/libx265 write them; tests/corpus/gen.sh (Homebrew
#     FFmpeg) already makes those seek files and tests/formats.rs reads them
#     from the parent corpus.
#   - MP3: libmp3lame (Homebrew FFmpeg) — FFmpeg has no native MP3 encoder.
#   - WebP and animated WebP: cwebp and img2webp (libwebp, Homebrew).
#   - HEIF/HEIC: sips (ImageIO).
#   - 12-bit and lossless JPEG: cjpeg (libjpeg-turbo, Homebrew) — FFmpeg's
#     MJPEG encoder writes 8-bit baseline only.
#   - Float TIFF, SVG, and MOV boxes FFmpeg does not write (mdcv, clli, dvcC,
#     clap): formats_tools.py.
#
# Video frames come from testsrc2 (every frame unique, so a seek that lands
# one frame off never matches). Ramps and bars use geq with exact codes so
# tests can check bit depth and color values.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BASE="${PM_MEDIA_CORPUS:-$HOME/Library/Caches/powermove-engine/media-corpus}"
OUT="${1:-$BASE/formats}"
FFDIR="${PM_FFMPEG_DIR:-$HOME/Library/Caches/powermove-engine/ffmpeg/out}"
FFMPEG="$FFDIR/bin/ffmpeg"
BREW_FFMPEG="${BREW_FFMPEG:-/opt/homebrew/bin/ffmpeg}"
TOOLS=(python3 -I "$HERE/formats_tools.py")
[[ -x "$FFMPEG" ]] || { echo "error: our FFmpeg build is missing at $FFMPEG (engine/third_party/ffmpeg/build.sh)" >&2; exit 1; }

rm -rf "$OUT.partial"
mkdir -p "$OUT.partial"/{video,image,sequence,animated,audio,hdr}
cd "$OUT.partial"

ff() { SVT_LOG=1 nice -n 15 "$FFMPEG" -hide_banner -loglevel error -y -threads 4 "$@"; }
brew_ff() { nice -n 15 "$BREW_FFMPEG" -hide_banner -loglevel error -y -threads 4 "$@"; }
src() { # rate size [pixfmt]
  printf 'testsrc2=size=%s:rate=%s,format=%s' "$2" "$1" "${3:-yuv420p}"
}
# Moving alpha: a diagonal wipe so every frame's alpha differs.
asrc() { # rate size pixfmt
  printf 'testsrc2=size=%s:rate=%s,format=%s,geq=lum=lum(X\\,Y):cb=cb(X\\,Y):cr=cr(X\\,Y):a=mod(X+Y+N*7\\,256)' "$2" "$1" "$3"
}
NTSC24=24000/1001
NTSC30=30000/1001
S=320x180

# ── Intermediates ─────────────────────────────────────────────────────────────
cd video
# ProRes through VideoToolbox (our build has no software ProRes encoder,
# amendment 13): every profile; 4444 and XQ with alpha.
for p in proxy lt standard hq; do
  ff -f lavfi -i "$(src 25 $S p210le)" -frames:v 30 -c:v prores_videotoolbox -profile:v $p "prores-$p.mov"
done
ff -f lavfi -i "$(asrc 25 $S yuva444p)" -frames:v 30 -c:v prores_videotoolbox -profile:v 4444 -pix_fmt ayuv64le prores-4444-alpha.mov
ff -f lavfi -i "$(asrc $NTSC24 $S yuva444p)" -frames:v 30 -c:v prores_videotoolbox -profile:v xq -pix_fmt ayuv64le prores-xq-alpha.mov
ff -f lavfi -i "$(src 25 $S p410le)" -frames:v 30 -c:v prores_videotoolbox -profile:v 4444 prores-4444.mov
# DNxHD (fixed sizes and rates) and every DNxHR profile.
ff -f lavfi -i "$(src 25 1920x1080 yuv422p)" -frames:v 10 -c:v dnxhd -b:v 120M dnxhd-120-1080p25.mov
ff -f lavfi -i "$(src 25 1920x1080 yuv422p10le)" -frames:v 10 -c:v dnxhd -b:v 185M -pix_fmt yuv422p10le dnxhd-185x-1080p25.mxf
for p in lb sq hq; do
  ff -f lavfi -i "$(src 25 640x360 yuv422p)" -frames:v 25 -c:v dnxhd -profile:v dnxhr_$p "dnxhr-$p.mov"
done
ff -f lavfi -i "$(src 25 640x360 yuv422p10le)" -frames:v 25 -c:v dnxhd -profile:v dnxhr_hqx -pix_fmt yuv422p10le dnxhr-hqx.mov
ff -f lavfi -i "$(src 25 640x360 yuv444p10le)" -frames:v 25 -c:v dnxhd -profile:v dnxhr_444 -pix_fmt yuv444p10le dnxhr-444.mxf
# CineForm: 4:2:2 10-bit, RGB 12-bit, RGBA 12-bit.
ff -f lavfi -i "$(src 25 $S yuv422p10le)" -frames:v 25 -c:v cfhd -pix_fmt yuv422p10le cineform-422.mov
ff -f lavfi -i "$(src 25 $S gbrp12le)" -frames:v 25 -c:v cfhd -pix_fmt gbrp12le cineform-rgb.mov
ff -f lavfi -i "$(asrc 25 $S gbrap12le)" -frames:v 25 -c:v cfhd -pix_fmt gbrap12le cineform-rgba.avi

# ── Camera and delivery ───────────────────────────────────────────────────────
ff -f lavfi -i "$(src $NTSC30 $S)" -frames:v 90 -c:v h264_videotoolbox -profile:v high -g 30 -b:v 2M h264-vt.mp4
ff -f lavfi -i "$(src $NTSC30 $S)" -frames:v 60 -c:v h264_videotoolbox -profile:v main -g 15 -b:v 2M h264-vt.mov
ff -f lavfi -i "$(src 25 $S nv12)" -frames:v 60 -c:v hevc_videotoolbox -profile:v main -g 25 -b:v 2M -tag:v hvc1 hevc-8.mov
ff -f lavfi -i "$(src 25 $S p010le)" -frames:v 60 -c:v hevc_videotoolbox -profile:v main10 -g 25 -b:v 2M -tag:v hvc1 hevc-10.mp4
ff -f lavfi -i "$(src 25 $S p210le)" -frames:v 50 -c:v hevc_videotoolbox -profile:v main42210 -g 25 -b:v 4M -tag:v hvc1 hevc-422-10.mov
ff -f lavfi -i "$(asrc 30 $S bgra)" -frames:v 60 -c:v hevc_videotoolbox -alpha_quality 0.9 -b:v 4M -tag:v hvc1 hevc-alpha.mov
ff -f lavfi -i "$(src 30 $S)" -frames:v 60 -c:v libsvtav1 -g 30 -preset 8 av1-8.mp4
ff -f lavfi -i "$(src 25 $S yuv420p10le)" -frames:v 50 -c:v libsvtav1 -g 25 -preset 8 -pix_fmt yuv420p10le av1-10.mkv
ff -f lavfi -i "$(src 30 $S yuvj422p)" -frames:v 30 -c:v mjpeg -q:v 2 mjpeg-422.avi
ff -f lavfi -i "$(src 25 $S yuvj420p)" -frames:v 30 -c:v mjpeg -q:v 2 mjpeg-420.mov
ff -f lavfi -i "$(src 25 $S yuvj444p)" -frames:v 30 -c:v mjpeg -q:v 2 mjpeg-444.mkv
# DV: NTSC 4:1:1 (bottom field first, 4:3), PAL 4:2:0, DVCPRO25 PAL 4:1:1,
# DVCPRO50, DVCPRO HD 1080i50.
ff -f lavfi -i "$(src $NTSC30 720x480 yuv411p),setfield=bff" -frames:v 30 -c:v dvvideo -aspect 4:3 dv-ntsc.dv
ff -f lavfi -i "$(src 25 720x576 yuv420p),setfield=bff" -frames:v 25 -c:v dvvideo -aspect 16:9 dv-pal.dv
ff -f lavfi -i "$(src 25 720x576 yuv411p),setfield=bff" -frames:v 25 -c:v dvvideo -aspect 4:3 dvcpro25-pal.mov
ff -f lavfi -i "$(src 25 720x576 yuv422p),setfield=bff" -frames:v 25 -c:v dvvideo -b:v 50M dvcpro50.mov
ff -f lavfi -i "$(src 25 1440x1080 yuv422p),setfield=tff" -frames:v 10 -c:v dvvideo dvcprohd-1080i50.mov

# ── Web ───────────────────────────────────────────────────────────────────────
ff -f lavfi -i "$(src 30 $S)" -frames:v 60 -c:v libvpx -g 30 -b:v 1M -deadline good -cpu-used 4 vp8.webm
ff -f lavfi -i "$(asrc 30 $S yuva420p)" -frames:v 60 -c:v libvpx -g 30 -b:v 1M -deadline good -cpu-used 4 -auto-alt-ref 0 vp8-alpha.webm
ff -f lavfi -i "$(src 30 $S)" -frames:v 60 -c:v libvpx-vp9 -g 30 -b:v 1M -deadline good -cpu-used 4 vp9.webm
ff -f lavfi -i "$(asrc 30 $S yuva420p)" -frames:v 60 -c:v libvpx-vp9 -g 30 -b:v 1M -deadline good -cpu-used 4 -auto-alt-ref 0 vp9-alpha.webm
ff -f lavfi -i "$(src 30 $S yuv420p10le)" -frames:v 60 -c:v libvpx-vp9 -profile:v 2 -g 30 -b:v 1M -deadline good -cpu-used 4 vp9-10.webm
ff -f lavfi -i "$(src 30 $S yuv444p)" -frames:v 30 -c:v libvpx-vp9 -profile:v 1 -g 30 -b:v 1M -deadline good -cpu-used 4 vp9-444.mkv

# ── Containers ────────────────────────────────────────────────────────────────
W="$(src 25 $S)"
ff -f lavfi -i "$W" -frames:v 50 -c:v h264_videotoolbox -g 25 -b:v 2M -f mov wrap-h264.m4v
ff -f lavfi -i "$W" -frames:v 50 -c:v h264_videotoolbox -g 25 -b:v 2M wrap-h264.mkv
ff -f lavfi -i "$W" -frames:v 50 -c:v h264_videotoolbox -g 25 -b:v 2M wrap-h264.mxf 2>/dev/null \
  || ff -f lavfi -i "$W" -frames:v 50 -c:v mpeg2video -g 12 -bf 2 -b:v 8M wrap-mpeg2.mxf
ff -f lavfi -i "$W" -frames:v 50 -c:v h264_videotoolbox -g 25 -b:v 2M wrap-h264.avi
ff -f lavfi -i "$W" -frames:v 50 -c:v wmv2 -g 25 -b:v 2M wrap-wmv2.wmv
ff -f lavfi -i "$W" -frames:v 50 -c:v wmv1 -g 25 -b:v 2M wrap-wmv1.asf
ff -f lavfi -i "$W" -frames:v 50 -c:v flv -g 25 -b:v 2M wrap-flv1.flv
ff -f lavfi -i "$W" -frames:v 50 -c:v h264_videotoolbox -g 25 -b:v 2M wrap-h264.flv
ff -f lavfi -i "$W" -frames:v 50 -c:v mpeg2video -g 12 -bf 2 -b:v 4M -f mpeg wrap-mpeg2.mpg
ff -f lavfi -i "$W" -frames:v 50 -c:v mpeg1video -g 12 -bf 2 -b:v 4M -f mpeg wrap-mpeg1.mpg
ff -f lavfi -i "$W" -frames:v 50 -c:v mpeg2video -g 12 -bf 2 -b:v 4M -f mpeg2video wrap-mpeg2.m2v
ff -f lavfi -i "$W" -frames:v 50 -c:v mpeg2video -g 12 -bf 2 -b:v 4M -f vob wrap-mpeg2.vob
ff -f lavfi -i "$W" -frames:v 50 -c:v h264_videotoolbox -g 25 -b:v 2M -f mpegts wrap-h264.ts
ff -f lavfi -i "$(src $NTSC30 $S)" -frames:v 60 -c:v h264_videotoolbox -g 30 -b:v 2M -f mpegts -mpegts_m2ts_mode 1 wrap-h264.m2ts
cp wrap-h264.m2ts wrap-h264.mts
ff -f lavfi -i "$(src 25 352x288)" -frames:v 50 -c:v h263 -g 25 -b:v 1M wrap-h263.3gp
ff -f lavfi -i "$W" -frames:v 50 -c:v h264_videotoolbox -g 25 -b:v 2M -f 3g2 wrap-h264.3g2
ff -f lavfi -i "$W" -frames:v 50 -c:v libvpx -g 25 -b:v 1M -deadline good -cpu-used 5 wrap-vp8.ogv
ff -f lavfi -i "$W" -frames:v 50 -c:v mpeg4 -g 25 -bf 2 -b:v 2M -vtag XVID wrap-mpeg4.avi
ff -f lavfi -i "$(src $NTSC30 720x480 yuv411p),setfield=bff" -frames:v 30 -c:v dvvideo -f dv wrap-dv.dv

# ── Metadata the probe reports ────────────────────────────────────────────────
# Drop-frame timecode, reel name, content identifier.
ff -f lavfi -i "$(src $NTSC30 $S p210le)" -frames:v 30 -c:v prores_videotoolbox -profile:v lt \
  -timecode "01:00:00;00" -metadata:s:v:0 reel_name=A001 \
  -metadata "com.apple.quicktime.content.identifier=6B2F5C33-ACDE-4E1C-9A6A-6F1AE3D0F0F1" \
  -movflags use_metadata_tags meta-timecode.mov
# Non-square pixels (pasp 10:11 from -aspect on 720×480), and a clean aperture.
ff -f lavfi -i "$(src $NTSC30 720x480 p210le)" -frames:v 30 -c:v prores_videotoolbox -profile:v standard \
  -aspect 4:3 meta-pasp.mov
cp meta-pasp.mov meta-clap.mov
"${TOOLS[@]}" mov-boxes meta-clap.mov "clap=$("${TOOLS[@]}" clap 704 480 0 0)"
# Display rotation (tkhd matrix), 90° counterclockwise.
ff -f lavfi -i "$(src 30 $S)" -frames:v 30 -c:v h264_videotoolbox -b:v 2M rotate-base.mov
ff -display_rotation 90 -i rotate-base.mov -c copy meta-rotate90.mov
rm rotate-base.mov
# Interlaced: top field first ProRes (fiel), bottom field first MPEG-2 in MXF.
ff -f lavfi -i "$(src 25 720x576 p210le),setfield=tff" -frames:v 25 -c:v prores_videotoolbox -profile:v standard \
  -field_order tt -flags +ilme+ildct interlaced-tff.mov
ff -f lavfi -i "$(src 25 720x576 yuv422p),setfield=bff" -frames:v 25 -c:v mpeg2video -pix_fmt yuv422p \
  -flags +ilme+ildct -g 12 -bf 2 -b:v 30M -field_order bb interlaced-bff.mxf
# Variable frame rate (every fifth frame dropped).
ff -f lavfi -i "$(src 30 $S)" -frames:v 90 -vf "select='not(eq(mod(n,5),4))'" -fps_mode vfr \
  -c:v h264_videotoolbox -b:v 2M vfr.mp4
# Tagged color: BT.601 625-line SD, BT.709 full range, Display P3.
ff -f lavfi -i "$(src 25 720x576 p210le)" -frames:v 10 -c:v prores_videotoolbox -profile:v standard \
  -vf setparams=color_primaries=bt470bg:color_trc=bt709:colorspace=bt470bg -color_primaries bt470bg -color_trc bt709 -colorspace bt470bg color-601-625.mov
ff -f lavfi -i "$(src 25 $S)" -frames:v 10 -c:v h264_videotoolbox -b:v 2M \
  -vf setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=pc -color_primaries bt709 -color_trc bt709 -colorspace bt709 -color_range pc color-709-full.mp4
ff -f lavfi -i "$(src 25 $S p210le)" -frames:v 10 -c:v prores_videotoolbox -profile:v hq \
  -vf setparams=color_primaries=smpte432:color_trc=iec61966-2-1:colorspace=bt709 -color_primaries smpte432 -color_trc iec61966-2-1 -colorspace bt709 color-p3.mov
cd ..

# ── HDR ───────────────────────────────────────────────────────────────────────
cd hdr
# HDR10: HEVC Main10 PQ with mastering display and content light boxes.
ff -f lavfi -i "$(src 25 $S p010le)" -frames:v 25 -c:v hevc_videotoolbox -profile:v main10 -b:v 4M -tag:v hvc1 \
  -vf setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc hdr10.mov
"${TOOLS[@]}" mov-boxes hdr10.mov "mdcv=$("${TOOLS[@]}" mdcv-hdr10)" "clli=$("${TOOLS[@]}" clli 1000 400)"
# HLG (iPhone-style tags).
ff -f lavfi -i "$(src 30 $S p010le)" -frames:v 30 -c:v hevc_videotoolbox -profile:v main10 -b:v 4M -tag:v hvc1 \
  -vf setparams=color_primaries=bt2020:color_trc=arib-std-b67:colorspace=bt2020nc -color_primaries bt2020 -color_trc arib-std-b67 -colorspace bt2020nc hlg.mov
# Dolby Vision profile 8.4 (HLG-compatible base layer) signalled by dvcC.
cp hlg.mov dovi-8.4.mov
"${TOOLS[@]}" mov-boxes dovi-8.4.mov "dvcC=$("${TOOLS[@]}" dvcc 8 4 4)"
# PQ AV1 and VP9 profile 2 (Matroska Colour elements).
ff -f lavfi -i "$(src 25 $S yuv420p10le)" -frames:v 25 -c:v libsvtav1 -preset 8 -pix_fmt yuv420p10le \
  -vf setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc pq-av1.mkv
ff -f lavfi -i "$(src 25 $S yuv420p10le)" -frames:v 25 -c:v libvpx-vp9 -profile:v 2 -b:v 1M -deadline good -cpu-used 5 \
  -vf setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc pq-vp9.webm
cd ..

# ── Ramps and bars (bit depth and color) ──────────────────────────────────────
cd video
RAMP10="nullsrc=s=1024x128:r=25,format=yuv422p10le,geq=lum=64+X*876/1023:cb=512:cr=512"
ff -f lavfi -i "$RAMP10" -frames:v 2 -c:v prores_videotoolbox -profile:v hq -pix_fmt p210le ramp10-prores-hq.mov
ff -f lavfi -i "$RAMP10" -frames:v 2 -c:v dnxhd -profile:v dnxhr_hqx -pix_fmt yuv422p10le ramp10-dnxhr-hqx.mov
ff -f lavfi -i "$RAMP10" -frames:v 2 -c:v cfhd -pix_fmt yuv422p10le ramp10-cineform.mov
ff -f lavfi -i "${RAMP10/yuv422p10le/yuv420p10le}" -frames:v 2 -c:v hevc_videotoolbox -profile:v main10 -b:v 20M -tag:v hvc1 -pix_fmt p010le ramp10-hevc.mov
ff -f lavfi -i "${RAMP10/yuv422p10le/yuv420p10le}" -frames:v 2 -c:v libsvtav1 -preset 6 -crf 4 -pix_fmt yuv420p10le ramp10-av1.mkv
ff -f lavfi -i "${RAMP10/yuv422p10le/yuv420p10le}" -frames:v 2 -c:v libvpx-vp9 -profile:v 2 -crf 2 -b:v 0 -pix_fmt yuv420p10le ramp10-vp9.webm
# 12-bit: a 16-bit grey and alpha ramp (geq is exact on rgba64), converted to
# 4:4:4 12-bit; alpha is X·4 at 12 bits.
ff -f lavfi -i "nullsrc=s=1024x128:r=25,format=rgba64le,geq=r=X*64:g=X*64:b=X*64:a=X*64,format=yuva444p12le" \
  -frames:v 2 -c:v prores_videotoolbox -profile:v xq -pix_fmt ayuv64le ramp12-prores-xq.mov
# 75% bars with exact BT.709 limited-range 8-bit codes (white, yellow, cyan,
# green, magenta, red, blue, black), 40 pixels each.
BARS_Y='if(lt(X,40),180,if(lt(X,80),168,if(lt(X,120),145,if(lt(X,160),134,if(lt(X,200),63,if(lt(X,240),51,if(lt(X,280),28,16)))))))'
BARS_U='if(lt(X,40),128,if(lt(X,80),44,if(lt(X,120),147,if(lt(X,160),63,if(lt(X,200),193,if(lt(X,240),109,if(lt(X,280),212,128)))))))'
BARS_V='if(lt(X,40),128,if(lt(X,80),136,if(lt(X,120),44,if(lt(X,160),52,if(lt(X,200),204,if(lt(X,240),212,if(lt(X,280),120,128)))))))'
BARS="nullsrc=s=320x128:r=25,format=yuv444p,geq=lum='$BARS_Y':cb='$BARS_U':cr='$BARS_V'"
ff -f lavfi -i "$BARS" -frames:v 2 -c:v prores_videotoolbox -profile:v 4444 -pix_fmt p410le \
  -vf setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709 -color_primaries bt709 -color_trc bt709 -colorspace bt709 bars709-prores4444.mov
ff -f lavfi -i "$BARS" -frames:v 2 -c:v dnxhd -profile:v dnxhr_444 -pix_fmt yuv444p10le \
  -vf setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709 -color_primaries bt709 -color_trc bt709 -colorspace bt709 bars709-dnxhr444.mov
ff -f lavfi -i "$BARS,format=yuv422p" -frames:v 2 -c:v mjpeg -q:v 1 -pix_fmt yuvj422p bars709-mjpeg.mov 2>/dev/null || true
cd ..

# ── Images ────────────────────────────────────────────────────────────────────
cd image
IMG="testsrc2=size=$S:rate=1"
ff -f lavfi -i "$IMG,format=rgb24" -frames:v 1 png8-rgb.png
ff -f lavfi -i "$(asrc 1 $S rgba)" -frames:v 1 png8-rgba.png
ff -f lavfi -i "$IMG,format=gray" -frames:v 1 png8-gray.png
ff -f lavfi -i "$IMG,format=pal8" -frames:v 1 png8-palette.png
ff -f lavfi -i "nullsrc=s=1024x8,format=rgb48be,geq=r=X*64:g=X*64:b=65535-X*64" -frames:v 1 png16-rgb.png
ff -f lavfi -i "nullsrc=s=1024x8,format=rgba64be,geq=r=X*64:g=1000:b=2000:a=65535-X*64" -frames:v 1 png16-rgba.png
ff -f lavfi -i "$IMG,format=yuvj444p" -frames:v 1 -q:v 2 jpeg-444.jpg
ff -f lavfi -i "$IMG,format=yuvj422p" -frames:v 1 -q:v 2 jpeg-422.jpg
ff -f lavfi -i "$IMG,format=yuvj420p" -frames:v 1 -q:v 2 jpeg-420.jpg
ff -f lavfi -i "$IMG,format=gray" -frames:v 1 -q:v 2 -pix_fmt gray jpeg-gray.jpg 2>/dev/null \
  || sips -s format jpeg png8-gray.png --out jpeg-gray.jpg >/dev/null
ff -f lavfi -i "$IMG,format=rgb24" -frames:v 1 tiff8-rgb.tiff
ff -f lavfi -i "$(asrc 1 $S rgba)" -frames:v 1 -compression_algo lzw tiff8-rgba-lzw.tiff
ff -f lavfi -i "nullsrc=s=1024x8,format=rgb48le,geq=r=X*64:g=X*64:b=X*64" -frames:v 1 -compression_algo deflate tiff16-rgb.tiff
"${TOOLS[@]}" tiff-float tiff32f-rgb.tiff 256 16
ff -f lavfi -i "nullsrc=s=256x16,format=gbrpf32le,geq=r=X/128:g=Y/15:b=0.25" -frames:v 1 -compression none exr-float.exr
ff -f lavfi -i "nullsrc=s=256x16,format=gbrapf32le,geq=r=X/128:g=Y/15:b=0.25:a=X/255" -frames:v 1 -compression zip16 -format half exr-half-alpha.exr
ff -f lavfi -i "nullsrc=s=1024x8,format=gbrp10le,geq=r=X:g=X:b=1023-X" -frames:v 1 dpx10.dpx
ff -f lavfi -i "nullsrc=s=1024x8,format=gbrp12le,geq=r=X*4:g=X*4:b=X*4" -frames:v 1 dpx12.dpx
ff -f lavfi -i "nullsrc=s=1024x8,format=rgb48le,geq=r=X*64:g=X*64:b=X*64" -frames:v 1 dpx16.dpx
ff -f lavfi -i "$IMG,format=rgb24" -frames:v 1 dpx8.dpx
ff -f lavfi -i "$IMG,format=bgr24" -frames:v 1 bmp24.bmp
ff -f lavfi -i "$(asrc 1 $S bgra)" -frames:v 1 bmp32.bmp
ff -f lavfi -i "testsrc2=size=64x64:rate=1,format=rgba" -frames:v 1 -c:v png -f ico icon.ico
ff -f lavfi -i "$IMG,format=rgb8" -frames:v 1 gif-still.gif
ff -f lavfi -i "$IMG,format=yuv420p" -frames:v 1 -c:v libsvtav1 avif-still.avif
ff -f lavfi -i "nullsrc=s=$S,format=yuv420p10le,geq=lum=64+X*2:cb=512:cr=512" -frames:v 1 -c:v libsvtav1 avif-10bit.avif
"${TOOLS[@]}" ramp-ppm ramp12.ppm 256 64 4095
cjpeg -precision 12 -quality 95 -outfile jpeg-12bit.jpg ramp12.ppm
"${TOOLS[@]}" ramp-ppm ramp16.ppm 256 64 65535
cjpeg -precision 16 -lossless 1 -outfile jpeg-lossless16.jpg ramp16.ppm
rm ramp12.ppm ramp16.ppm
cjpeg -progressive -quality 90 -outfile jpeg-progressive.jpg <(ff -f lavfi -i "$IMG,format=rgb24" -frames:v 1 -f image2pipe -c:v ppm -)
cwebp -quiet -q 90 png8-rgb.png -o webp-lossy.webp
cwebp -quiet -lossless png8-rgba.png -o webp-lossless-alpha.webp
sips -s format heic png8-rgb.png --out heic.heic >/dev/null
# ICC-tagged stills converted by ColorSync (sips): Adobe RGB, Display P3,
# ProPhoto (ROMM), and gray gamma 2.2; and the same converted back to sRGB as
# ColorSync's reference.
P=/System/Library/ColorSync/Profiles
sips -m "$P/AdobeRGB1998.icc" png8-rgb.png --out icc-adobergb.png >/dev/null
sips -m "$P/Display P3.icc" png8-rgb.png --out icc-p3.png >/dev/null
sips -m "$P/ROMM RGB.icc" png16-rgb.png --out icc-prophoto16.png >/dev/null
sips -m "$P/Generic Gray Gamma 2.2 Profile.icc" png8-gray.png --out icc-gray22.png >/dev/null
for f in icc-adobergb icc-p3 icc-prophoto16 icc-gray22; do
  sips -m "$P/sRGB Profile.icc" "$f.png" --out "ref-srgb-$f.png" >/dev/null
done
"${TOOLS[@]}" svg vector.svg
cd ..

# ── Image sequences (with gaps) ───────────────────────────────────────────────
cd sequence
ff -f lavfi -i "$(src 24 $S rgb24)" -frames:v 12 seq_png_%04d.png
rm seq_png_0005.png seq_png_0006.png seq_png_0010.png
ff -f lavfi -i "nullsrc=s=128x72:r=24,format=gbrpf32le,geq=r=N/10:g=X/127:b=Y/71" -frames:v 6 -compression zip1 seq_exr_%04d.exr
rm seq_exr_0003.exr
ff -f lavfi -i "$(src 24 $S gbrp10le)" -frames:v 6 seq_dpx_%04d.dpx
ff -f lavfi -i "$(src 24 $S rgb48le)" -frames:v 6 seq_tiff_%04d.tiff
rm seq_tiff_0004.tiff
cd ..

# ── Animated images ───────────────────────────────────────────────────────────
cd animated
ff -f lavfi -i "$(src 10 $S rgb8)" -frames:v 10 -loop 0 anim.gif
ff -f lavfi -i "$(asrc 10 $S rgba)" -frames:v 10 -plays 0 -f apng anim.apng
ff -f lavfi -i "$(src 10 $S)" -frames:v 10 -c:v libsvtav1 -preset 8 -f avif anim.avif
ff -f lavfi -i "$(src 10 $S rgb24)" -frames:v 10 anim_frame_%02d.png
img2webp -loop 0 -d 100 -lossy anim_frame_*.png -o anim.webp >/dev/null
rm anim_frame_*.png
cd ..

# ── Audio ─────────────────────────────────────────────────────────────────────
cd audio
TONE="sine=frequency=1000:sample_rate=48000:duration=2"
STEREO="aevalsrc=sin(2*PI*1000*t)|sin(2*PI*500*t):s=48000:d=2"
ff -f lavfi -i "$TONE" -c:a pcm_s16le wav-s16.wav
ff -f lavfi -i "$STEREO" -c:a pcm_s24le wav-s24-stereo.wav
ff -f lavfi -i "$STEREO" -c:a pcm_f32le wav-f32-stereo.wav
ff -f lavfi -i "aevalsrc=sin(2*PI*1000*t)|sin(2*PI*500*t)|0.5*sin(2*PI*250*t)|0.1*sin(2*PI*60*t)|sin(2*PI*2000*t)|sin(2*PI*3000*t):s=48000:d=2:c=5.1" \
  -c:a pcm_s24le wav-5.1.wav
ff -f lavfi -i "$TONE" -c:a pcm_s16be aiff-s16.aiff
ff -f lavfi -i "$STEREO" -c:a pcm_s24be aiff-s24.aif
ff -f lavfi -i "$STEREO" -c:a pcm_s24le caf-pcm.caf
ff -f lavfi -i "$STEREO" -c:a alac caf-alac.caf
ff -f lavfi -i "$STEREO" -c:a aac -b:a 192k aac-ff.m4a
ff -f lavfi -i "$STEREO" -c:a aac_at -b:a 192k aac-at.m4a
ff -f lavfi -i "$STEREO" -c:a aac_at -profile:a 4 -b:a 64k he-aac.m4a
ff -f lavfi -i "$STEREO" -c:a aac -b:a 192k -f adts aac-adts.aac
ff -f lavfi -i "$STEREO" -c:a flac flac.flac
ff -f lavfi -i "$STEREO" -c:a libopus -b:a 128k opus.opus
ff -f lavfi -i "$STEREO" -c:a vorbis -strict -2 -ac 2 vorbis.ogg
ff -f lavfi -i "$STEREO" -c:a libopus -b:a 128k -f webm weba.weba
ff -f lavfi -i "$STEREO" -c:a alac alac.m4a
ff -f lavfi -i "$STEREO" -c:a mp2 -b:a 192k mp2.mp2
ff -f lavfi -i "$STEREO" -c:a ac3 -b:a 192k ac3.ac3
ff -f lavfi -i "$STEREO" -c:a eac3 -b:a 192k eac3.mka
ff -f lavfi -i "$STEREO" -c:a wmav2 -b:a 128k wma.wma
# MP3 needs libmp3lame (no native encoder): Homebrew FFmpeg.
if [[ -x "$BREW_FFMPEG" ]]; then
  brew_ff -f lavfi -i "$STEREO" -c:a libmp3lame -b:a 192k mp3-cbr.mp3
  brew_ff -f lavfi -i "$STEREO" -c:a libmp3lame -q:a 2 mp3-vbr.mp3
fi
# Audio inside video containers.
V="$(src 25 $S)"
ff -f lavfi -i "$V" -f lavfi -i "$STEREO" -t 2 -c:v prores_videotoolbox -profile:v proxy -pix_fmt p210le -c:a pcm_s24le av-prores-pcm.mov
ff -f lavfi -i "$V" -f lavfi -i "$STEREO" -t 2 -c:v h264_videotoolbox -b:v 1M -c:a aac_at -b:a 128k av-h264-aac.mp4
ff -f lavfi -i "$V" -f lavfi -i "$STEREO" -t 2 -c:v libvpx-vp9 -b:v 500k -c:a libopus av-vp9-opus.webm
ff -f lavfi -i "$V" -f lavfi -i "$STEREO" -t 2 -c:v h264_videotoolbox -b:v 1M -c:a flac av-h264-flac.mkv
ff -f lavfi -i "$V" -f lavfi -i "$STEREO" -t 2 -c:v mpeg2video -b:v 8M -c:a pcm_s16le -ar 48000 av-mpeg2-pcm.mxf
ff -f lavfi -i "$V" -f lavfi -i "$STEREO" -t 2 -c:v mpeg4 -b:v 2M -c:a pcm_s16le av-mpeg4-pcm.avi
ff -f lavfi -i "$V" -f lavfi -i "$STEREO" -t 2 -c:v h264_videotoolbox -b:v 1M -c:a aac -b:a 128k -f mpegts av-h264-aac.ts
ff -f lavfi -i "$V" -f lavfi -i "$STEREO" -t 2 -c:v mpeg2video -b:v 4M -c:a mp2 -f vob av-mpeg2-mp2.vob
ff -f lavfi -i "$V" -f lavfi -i "$STEREO" -t 2 -c:v wmv2 -b:v 2M -c:a wmav2 av-wmv2-wma.wmv
ff -f lavfi -i "$V" -f lavfi -i "$STEREO" -t 2 -c:v flv -b:v 2M -c:a aac -b:a 128k av-flv-aac.flv
ff -f lavfi -i "$(src 25 720x576 yuv420p),setfield=bff" -f lavfi -i "$STEREO" -t 2 -c:v dvvideo -c:a pcm_s16le av-dv.dv
ff -f lavfi -i "$V" -f lavfi -i "$STEREO" -t 2 -c:v libvpx -b:v 1M -c:a vorbis -strict -2 -ac 2 av-vp8-vorbis.ogv
ff -f lavfi -i "$(src 25 352x288)" -f lavfi -i "$STEREO" -t 2 -c:v h263 -b:v 1M -c:a aac -b:a 64k -ar 48000 av-h263-aac.3gp
cd ..

cd /
rm -rf "$OUT"
mv "$OUT.partial" "$OUT"
find "$OUT" -type f | wc -l | xargs echo "files:"
