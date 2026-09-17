#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

FFMPEG="/opt/homebrew/bin/ffmpeg"
FFPROBE="/opt/homebrew/bin/ffprobe"
AVCONVERT="/usr/bin/avconvert"
SIPS="/usr/bin/sips"
CWEBP="$(command -v cwebp || true)"
IMG2WEBP="$(command -v img2webp || true)"
MAX_BYTES=$((300 * 1024))

if [[ ! -x "$FFMPEG" || ! -x "$FFPROBE" ]]; then
  echo "ffmpeg and ffprobe are required at /opt/homebrew/bin" >&2
  exit 1
fi

ENCODERS="$($FFMPEG -hide_banner -encoders 2>/dev/null)"

has_encoder() {
  awk -v encoder="$1" '$2 == encoder { found = 1 } END { exit !found }' <<<"$ENCODERS"
}

has_avconvert_preset() {
  [[ -x "$AVCONVERT" ]] && grep -Fq "$1" <<<"$($AVCONVERT --help 2>&1 || true)"
}

file_bytes() {
  if [[ -f "$1" ]]; then
    stat -f '%z' "$1"
  else
    printf 'null'
  fi
}

OUTPUTS=(
  h264-aac.mp4 h264-aac.mov hevc.mov hevc.mp4 hevc-alpha.mov
  prores422.mov prores4444-alpha.mov vp9-opus.webm vp8-opus.webm av1.mp4
  tone.wav tone.m4a tone.mp3 tone.opus tone.flac
  still-red.png still-alpha.png still.webp still.avif still.heic
  animated.gif animated-apng.png animated.webp
  still.tiff converted.mkv
)
rm -f -- "${OUTPUTS[@]}" fixtures.json.tmp

VIDEO_2S="color=c=0xff0000:s=160x90:r=30:d=1[v0];color=c=0x00ff00:s=160x90:r=30:d=1[v1];[v0][v1]concat=n=2:v=1:a=0,format=yuv420p"
VIDEO_12S="color=c=0xff0000:s=160x90:r=30:d=0.6[v0];color=c=0x00ff00:s=160x90:r=30:d=0.6[v1];[v0][v1]concat=n=2:v=1:a=0,format=yuv420p"
ALPHA_2S="color=c=0xff0000:s=160x90:r=30:d=1[v0];color=c=0x00ff00:s=160x90:r=30:d=1[v1];[v0][v1]concat=n=2:v=1:a=0,format=rgb24[base];color=c=white:s=160x90:r=30:d=2,format=gray,geq=lum='if(lt(X,W/2),255,0)'[alpha];[base][alpha]alphamerge,format=yuva444p10le"
ALPHA_12S="color=c=0xff0000:s=160x90:r=30:d=0.6[v0];color=c=0x00ff00:s=160x90:r=30:d=0.6[v1];[v0][v1]concat=n=2:v=1:a=0,format=rgb24[base];color=c=white:s=160x90:r=30:d=1.2,format=gray,geq=lum='if(lt(X,W/2),255,0)'[alpha];[base][alpha]alphamerge,format=yuva444p10le"
AUDIO_2S="sine=frequency=440:sample_rate=48000:duration=2,volume=2,pan=stereo|c0=c0|c1=c0"

COMMON_VIDEO_INPUT=(-f lavfi -i "$VIDEO_2S" -f lavfi -i "$AUDIO_2S" -map 0:v -map 1:a)
COMMON_AUDIO=(-ar 48000 -ac 2)

h264_skip=""
h264_generated=""
if has_encoder libx264 && has_encoder aac; then
  "$FFMPEG" -hide_banner -loglevel error -y "${COMMON_VIDEO_INPUT[@]}" \
    -c:v libx264 -profile:v baseline -level:v 3.0 -preset veryfast -crf 28 \
    -pix_fmt yuv420p -g 30 -keyint_min 30 -sc_threshold 0 -tag:v avc1 \
    -c:a aac -b:a 64k "${COMMON_AUDIO[@]}" -movflags +faststart h264-aac.mp4
  "$FFMPEG" -hide_banner -loglevel error -y "${COMMON_VIDEO_INPUT[@]}" \
    -c:v libx264 -profile:v main -level:v 3.0 -preset veryfast -crf 28 \
    -pix_fmt yuv420p -g 30 -keyint_min 30 -sc_threshold 0 -tag:v avc1 \
    -c:a aac -b:a 64k "${COMMON_AUDIO[@]}" h264-aac.mov
  h264_generated="ffmpeg libx264 + aac"
else
  h264_skip="libx264 and/or aac encoder unavailable"
fi

hevc_encoder=""
if has_encoder hevc_videotoolbox; then
  hevc_encoder="hevc_videotoolbox"
elif has_encoder libx265; then
  hevc_encoder="libx265"
fi

hevc_skip=""
hevc_generated=""
if [[ -n "$hevc_encoder" ]] && has_encoder aac; then
  if [[ "$hevc_encoder" == "hevc_videotoolbox" ]]; then
    HEVC_ARGS=(-c:v hevc_videotoolbox -allow_sw 1 -realtime 0 -b:v 120k)
  else
    HEVC_ARGS=(-c:v libx265 -preset fast -crf 32 -x265-params log-level=error)
  fi
  if ! "$FFMPEG" -hide_banner -loglevel error -y "${COMMON_VIDEO_INPUT[@]}" \
      "${HEVC_ARGS[@]}" -pix_fmt yuv420p -g 30 -tag:v hvc1 \
      -c:a aac -b:a 64k "${COMMON_AUDIO[@]}" hevc.mov; then
    if [[ "$hevc_encoder" == "hevc_videotoolbox" ]] && has_encoder libx265; then
      hevc_encoder="libx265"
      HEVC_ARGS=(-c:v libx265 -preset fast -crf 32 -x265-params log-level=error)
      "$FFMPEG" -hide_banner -loglevel error -y "${COMMON_VIDEO_INPUT[@]}" \
        "${HEVC_ARGS[@]}" -pix_fmt yuv420p -g 30 -tag:v hvc1 \
        -c:a aac -b:a 64k "${COMMON_AUDIO[@]}" hevc.mov
    else
      hevc_skip="$hevc_encoder failed at runtime"
    fi
  fi
  if [[ -z "$hevc_skip" ]]; then
    "$FFMPEG" -hide_banner -loglevel error -y "${COMMON_VIDEO_INPUT[@]}" \
      "${HEVC_ARGS[@]}" -pix_fmt yuv420p -g 30 -tag:v hvc1 \
      -c:a aac -b:a 64k "${COMMON_AUDIO[@]}" -movflags +faststart hevc.mp4
    hevc_generated="ffmpeg $hevc_encoder + aac"
  fi
else
  hevc_skip="HEVC and/or aac encoder unavailable"
fi

prores422_t1="1.5"
prores422_t2="0.5"
prores422_generated=""
prores422_skip=""
if has_encoder prores_ks && has_encoder aac; then
  "$FFMPEG" -hide_banner -loglevel error -y "${COMMON_VIDEO_INPUT[@]}" \
    -c:v prores_ks -profile:v 3 -pix_fmt yuv422p10le \
    -c:a aac -b:a 64k "${COMMON_AUDIO[@]}" prores422.mov
  prores422_generated="ffmpeg prores_ks profile 3 + aac"
  if (( $(file_bytes prores422.mov) > MAX_BYTES )); then
    "$FFMPEG" -hide_banner -loglevel error -y \
      -f lavfi -i "$VIDEO_12S" -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=1.2,volume=2,pan=stereo|c0=c0|c1=c0" \
      -map 0:v -map 1:a -c:v prores_ks -profile:v 3 -pix_fmt yuv422p10le \
      -c:a aac -b:a 64k "${COMMON_AUDIO[@]}" prores422.mov
    prores422_t1="0.9"
    prores422_t2="0.3"
    prores422_generated="ffmpeg prores_ks profile 3 + aac (1.2 s size-capped)"
  fi
else
  prores422_skip="prores_ks and/or aac encoder unavailable"
fi

alpha_t1="1.5"
alpha_t2="0.5"
prores4444_generated=""
prores4444_skip=""
if has_encoder prores_ks; then
  "$FFMPEG" -hide_banner -loglevel error -y -f lavfi -i "$ALPHA_2S" -map 0:v \
    -c:v prores_ks -profile:v 4 -pix_fmt yuva444p10le -vendor apl0 prores4444-alpha.mov
  prores4444_generated="ffmpeg prores_ks profile 4"
  if (( $(file_bytes prores4444-alpha.mov) > MAX_BYTES )); then
    "$FFMPEG" -hide_banner -loglevel error -y -f lavfi -i "$ALPHA_12S" -map 0:v \
      -c:v prores_ks -profile:v 4 -pix_fmt yuva444p10le -vendor apl0 prores4444-alpha.mov
    alpha_t1="0.9"
    alpha_t2="0.3"
    prores4444_generated="ffmpeg prores_ks profile 4 (1.2 s size-capped)"
  fi
else
  prores4444_skip="prores_ks encoder unavailable"
fi

hevc_alpha_skip=""
hevc_alpha_generated=""
if [[ -z "$prores4444_skip" ]] && has_avconvert_preset PresetHEVCHighestQualityWithAlpha; then
  if "$AVCONVERT" --source "$SCRIPT_DIR/prores4444-alpha.mov" \
      --preset PresetHEVCHighestQualityWithAlpha \
      --output "$SCRIPT_DIR/hevc-alpha.mov" --replace >/dev/null 2>&1; then
    hevc_alpha_generated="avconvert PresetHEVCHighestQualityWithAlpha"
    if (( $(file_bytes hevc-alpha.mov) > MAX_BYTES )); then
      hevc_alpha_skip="avconvert output exceeded 300 KiB"
      rm -f -- hevc-alpha.mov
    fi
  else
    hevc_alpha_skip="avconvert PresetHEVCHighestQualityWithAlpha failed"
  fi
else
  hevc_alpha_skip="ProRes 4444 source or avconvert alpha preset unavailable"
fi

vp9_skip=""
vp9_generated=""
if has_encoder libvpx-vp9 && has_encoder libopus; then
  "$FFMPEG" -hide_banner -loglevel error -y "${COMMON_VIDEO_INPUT[@]}" \
    -c:v libvpx-vp9 -crf 36 -b:v 0 -deadline good -cpu-used 4 -g 30 \
    -pix_fmt yuv420p -c:a libopus -b:a 48k "${COMMON_AUDIO[@]}" vp9-opus.webm
  vp9_generated="ffmpeg libvpx-vp9 + libopus"
else
  vp9_skip="libvpx-vp9 and/or libopus encoder unavailable"
fi

vp8_skip=""
vp8_generated=""
if has_encoder libvpx && has_encoder libopus; then
  "$FFMPEG" -hide_banner -loglevel error -y "${COMMON_VIDEO_INPUT[@]}" \
    -c:v libvpx -crf 24 -b:v 90k -deadline good -cpu-used 4 -g 30 \
    -pix_fmt yuv420p -c:a libopus -b:a 48k "${COMMON_AUDIO[@]}" vp8-opus.webm
  vp8_generated="ffmpeg libvpx VP8 + libopus"
else
  vp8_skip="libvpx and/or libopus encoder unavailable"
fi

av1_skip=""
av1_generated=""
av1_encoder=""
if has_encoder libaom-av1; then
  av1_encoder="libaom-av1"
elif has_encoder libsvtav1; then
  av1_encoder="libsvtav1"
fi
if [[ -n "$av1_encoder" ]] && has_encoder aac; then
  if [[ "$av1_encoder" == "libaom-av1" ]]; then
    AV1_ARGS=(-c:v libaom-av1 -crf 42 -b:v 0 -cpu-used 8)
  else
    AV1_ARGS=(-c:v libsvtav1 -crf 45 -preset 12)
  fi
  "$FFMPEG" -hide_banner -loglevel error -y "${COMMON_VIDEO_INPUT[@]}" \
    "${AV1_ARGS[@]}" -pix_fmt yuv420p -g 30 \
    -c:a aac -b:a 64k "${COMMON_AUDIO[@]}" -movflags +faststart av1.mp4
  av1_generated="ffmpeg $av1_encoder + aac"
else
  av1_skip="libaom-av1/libsvtav1 and/or aac encoder unavailable"
fi

tone_wav_skip=""
tone_wav_generated="ffmpeg pcm_s16le (1.5 s size-capped)"
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=1.5,volume=2,pan=stereo|c0=c0|c1=c0" \
  -c:a pcm_s16le "${COMMON_AUDIO[@]}" tone.wav

tone_m4a_skip=""
tone_m4a_generated=""
if has_encoder aac; then
  "$FFMPEG" -hide_banner -loglevel error -y -f lavfi -i "$AUDIO_2S" \
    -c:a aac -b:a 64k "${COMMON_AUDIO[@]}" tone.m4a
  tone_m4a_generated="ffmpeg aac"
else
  tone_m4a_skip="aac encoder unavailable"
fi

tone_mp3_skip=""
tone_mp3_generated=""
if has_encoder libmp3lame; then
  "$FFMPEG" -hide_banner -loglevel error -y -f lavfi -i "$AUDIO_2S" \
    -c:a libmp3lame -b:a 64k "${COMMON_AUDIO[@]}" tone.mp3
  tone_mp3_generated="ffmpeg libmp3lame"
else
  tone_mp3_skip="libmp3lame encoder unavailable"
fi

tone_opus_skip=""
tone_opus_generated=""
if has_encoder libopus; then
  "$FFMPEG" -hide_banner -loglevel error -y -f lavfi -i "$AUDIO_2S" \
    -c:a libopus -b:a 48k "${COMMON_AUDIO[@]}" -f ogg tone.opus
  tone_opus_generated="ffmpeg libopus in Ogg"
else
  tone_opus_skip="libopus encoder unavailable"
fi

"$FFMPEG" -hide_banner -loglevel error -y -f lavfi -i "$AUDIO_2S" \
  -c:a flac "${COMMON_AUDIO[@]}" tone.flac
tone_flac_skip=""
tone_flac_generated="ffmpeg flac"

"$FFMPEG" -hide_banner -loglevel error -y -f lavfi -i "color=c=0xff0000:s=160x90:d=1" \
  -frames:v 1 -c:v png -pix_fmt rgb24 still-red.png
still_red_skip=""
still_red_generated="ffmpeg png"

"$FFMPEG" -hide_banner -loglevel error -y -f lavfi \
  -i "color=c=0x00ff00:s=160x90:d=1[base];color=c=white:s=160x90:d=1,format=gray,geq=lum='if(lt(X,W/2),255,0)'[alpha];[base][alpha]alphamerge,format=rgba" \
  -map 0:v -frames:v 1 -c:v png -pix_fmt rgba still-alpha.png
still_alpha_skip=""
still_alpha_generated="ffmpeg png rgba"

still_webp_skip=""
still_webp_generated=""
if [[ -n "$CWEBP" && -x "$CWEBP" ]]; then
  "$CWEBP" -quiet -q 80 still-red.png -o still.webp
  still_webp_generated="cwebp -q 80"
else
  still_webp_skip="cwebp unavailable and ffmpeg has no WebP encoder"
fi

still_avif_skip=""
still_avif_generated=""
if [[ -n "$av1_encoder" ]]; then
  if [[ "$av1_encoder" == "libaom-av1" ]]; then
    STILL_AV1_ARGS=(-c:v libaom-av1 -crf 30 -b:v 0 -cpu-used 8)
  else
    STILL_AV1_ARGS=(-c:v libsvtav1 -crf 35 -preset 12)
  fi
  if "$FFMPEG" -hide_banner -loglevel error -y -i still-red.png -frames:v 1 \
      "${STILL_AV1_ARGS[@]}" -pix_fmt yuv420p still.avif; then
    still_avif_generated="ffmpeg $av1_encoder AVIF"
  else
    still_avif_skip="$av1_encoder is present but AVIF still encoding failed"
    rm -f -- still.avif
  fi
else
  still_avif_skip="AV1 encoder unavailable"
fi

still_tiff_skip=""
still_tiff_generated=""
if "$FFMPEG" -hide_banner -loglevel error -y -i still-red.png -frames:v 1 -update 1 -c:v tiff still.tiff; then
  still_tiff_generated="ffmpeg tiff"
else
  still_tiff_skip="ffmpeg TIFF encoding failed"
  rm -f -- still.tiff
fi

still_heic_skip=""
still_heic_generated=""
if [[ -x "$SIPS" ]]; then
  if "$SIPS" -s format heic still-red.png --out still.heic >/dev/null 2>&1; then
    still_heic_generated="sips -s format heic"
  else
    still_heic_skip="sips advertised HEIC output but conversion failed"
    rm -f -- still.heic
  fi
else
  still_heic_skip="sips unavailable"
fi

converted_mkv_skip=""
converted_mkv_generated=""
if has_encoder libx264 && has_encoder aac; then
  "$FFMPEG" -hide_banner -loglevel error -y "${COMMON_VIDEO_INPUT[@]}" \
    -c:v libx264 -profile:v main -preset veryfast -crf 28 -pix_fmt yuv420p -g 30 \
    -c:a aac -b:a 64k "${COMMON_AUDIO[@]}" -f matroska converted.mkv
  converted_mkv_generated="ffmpeg libx264 + aac in Matroska"
else
  converted_mkv_skip="libx264 and/or aac encoder unavailable"
fi

# Animated stills: three 100 ms frames, red then green then blue, right half
# transparent. Every one of these decodes through Chromium's ImageDecoder.
ANIMATION_FRAMES="$SCRIPT_DIR/.animation-frames"
rm -rf -- "$ANIMATION_FRAMES"
mkdir -p "$ANIMATION_FRAMES"
animation_index=1
for animation_color in 0xff0000 0x00ff00 0x0000ff; do
  "$FFMPEG" -hide_banner -loglevel error -y -f lavfi \
    -i "color=c=$animation_color:s=160x90:d=1[base];color=c=white:s=160x90:d=1,format=gray,geq=lum='if(lt(X,W/2),255,0)'[alpha];[base][alpha]alphamerge,format=rgba" \
    -map 0:v -frames:v 1 -c:v png -pix_fmt rgba "$ANIMATION_FRAMES/f-$animation_index.png"
  animation_index=$((animation_index + 1))
done

animated_gif_skip=""
animated_gif_generated=""
if "$FFMPEG" -hide_banner -loglevel error -y -framerate 10 -i "$ANIMATION_FRAMES/f-%d.png" \
    -filter_complex "split[a][b];[a]palettegen=reserve_transparent=1[p];[b][p]paletteuse=alpha_threshold=128" \
    -loop 0 animated.gif; then
  animated_gif_generated="ffmpeg gif with reserved transparency"
else
  animated_gif_skip="ffmpeg GIF encoding failed"
  rm -f -- animated.gif
fi

animated_apng_skip=""
animated_apng_generated=""
if "$FFMPEG" -hide_banner -loglevel error -y -framerate 10 -i "$ANIMATION_FRAMES/f-%d.png" \
    -plays 0 -f apng animated-apng.png; then
  animated_apng_generated="ffmpeg apng"
else
  animated_apng_skip="ffmpeg APNG encoding failed"
  rm -f -- animated-apng.png
fi

animated_webp_skip=""
animated_webp_generated=""
if [[ -n "$IMG2WEBP" && -x "$IMG2WEBP" ]]; then
  if "$IMG2WEBP" -lossless -d 100 -loop 0 \
      "$ANIMATION_FRAMES/f-1.png" "$ANIMATION_FRAMES/f-2.png" "$ANIMATION_FRAMES/f-3.png" \
      -o animated.webp >/dev/null 2>&1; then
    animated_webp_generated="img2webp -lossless -d 100"
  else
    animated_webp_skip="img2webp is present but animated WebP encoding failed"
    rm -f -- animated.webp
  fi
else
  animated_webp_skip="img2webp unavailable"
fi
rm -rf -- "$ANIMATION_FRAMES"

FIRST_ENTRY=1
exec 3>fixtures.json.tmp
printf '[\n' >&3

begin_entry() {
  if (( FIRST_ENTRY )); then
    FIRST_ENTRY=0
  else
    printf ',\n' >&3
  fi
}

json_string_or_null() {
  if [[ -n "$1" ]]; then
    printf '"%s"' "$1" >&3
  else
    printf 'null' >&3
  fi
}

emit_video() {
  local file="$1" container="$2" video="$3" audio="$4" has_alpha="$5"
  local layout="$6" t1="$7" t2="$8" generated="$9" skipped="${10}"
  begin_entry
  printf '  { "file": "%s", "kind": "video", "container": "%s", "video": "%s", "audio": ' "$file" "$container" "$video" >&3
  json_string_or_null "$audio"
  printf ', "hasAlpha": %s, "alphaLayout": ' "$has_alpha" >&3
  json_string_or_null "$layout"
  printf ', "expectedColorAt": { "t": %s, "rgb": [0,255,0] }, "expectedColorAt2": { "t": %s, "rgb": [255,0,0] }, "bytes": %s, "generatedBy": ' "$t1" "$t2" "$(file_bytes "$file")" >&3
  json_string_or_null "$generated"
  printf ', "skipped": ' >&3
  json_string_or_null "$skipped"
  printf ' }' >&3
}

emit_audio() {
  local file="$1" container="$2" audio="$3" duration="$4" generated="$5" skipped="$6"
  begin_entry
  printf '  { "file": "%s", "kind": "audio", "container": "%s", "audio": "%s", "expectedToneHz": 440, "sampleRate": 48000, "channels": 2, "duration": %s, "bytes": %s, "generatedBy": ' "$file" "$container" "$audio" "$duration" "$(file_bytes "$file")" >&3
  json_string_or_null "$generated"
  printf ', "skipped": ' >&3
  json_string_or_null "$skipped"
  printf ' }' >&3
}

emit_image() {
  local file="$1" container="$2" video="$3" has_alpha="$4" layout="$5" rgb="$6" generated="$7" skipped="$8"
  begin_entry
  printf '  { "file": "%s", "kind": "image", "container": "%s", "video": "%s", "width": 160, "height": 90, "hasAlpha": %s, "alphaLayout": ' "$file" "$container" "$video" "$has_alpha" >&3
  json_string_or_null "$layout"
  printf ', "expectedColor": { "rgb": %s }, "bytes": %s, "generatedBy": ' "$rgb" "$(file_bytes "$file")" >&3
  json_string_or_null "$generated"
  printf ', "skipped": ' >&3
  json_string_or_null "$skipped"
  printf ' }' >&3
}

emit_animation() {
  local file="$1" container="$2" generated="$3" skipped="$4"
  begin_entry
  printf '  { "file": "%s", "kind": "animation", "container": "%s", "width": 160, "height": 90, "frames": 3, "frameDelayMs": 100, "hasAlpha": true, "alphaLayout": "right-half-transparent", "expectedColors": [[255,0,0],[0,255,0],[0,0,255]], "bytes": %s, "generatedBy": ' "$file" "$container" "$(file_bytes "$file")" >&3
  json_string_or_null "$generated"
  printf ', "skipped": ' >&3
  json_string_or_null "$skipped"
  printf ' }' >&3
}

emit_video h264-aac.mp4 mp4 avc1 aac false "" 1.5 0.5 "$h264_generated" "$h264_skip"
emit_video h264-aac.mov mov avc1 aac false "" 1.5 0.5 "$h264_generated" "$h264_skip"
emit_video hevc.mov mov hvc1 aac false "" 1.5 0.5 "$hevc_generated" "$hevc_skip"
emit_video hevc.mp4 mp4 hvc1 aac false "" 1.5 0.5 "$hevc_generated" "$hevc_skip"
emit_video hevc-alpha.mov mov hvc1 "" true right-half-transparent "$alpha_t1" "$alpha_t2" "$hevc_alpha_generated" "$hevc_alpha_skip"
emit_video prores422.mov mov apch aac false "" "$prores422_t1" "$prores422_t2" "$prores422_generated" "$prores422_skip"
emit_video prores4444-alpha.mov mov ap4h "" true right-half-transparent "$alpha_t1" "$alpha_t2" "$prores4444_generated" "$prores4444_skip"
emit_video vp9-opus.webm webm vp09 opus false "" 1.5 0.5 "$vp9_generated" "$vp9_skip"
emit_video vp8-opus.webm webm vp08 opus false "" 1.5 0.5 "$vp8_generated" "$vp8_skip"
emit_video av1.mp4 mp4 av01 aac false "" 1.5 0.5 "$av1_generated" "$av1_skip"
emit_audio tone.wav wav pcm_s16le 1.5 "$tone_wav_generated" "$tone_wav_skip"
emit_audio tone.m4a ipod aac 2.0 "$tone_m4a_generated" "$tone_m4a_skip"
emit_audio tone.mp3 mp3 mp3 2.0 "$tone_mp3_generated" "$tone_mp3_skip"
emit_audio tone.opus ogg opus 2.0 "$tone_opus_generated" "$tone_opus_skip"
emit_audio tone.flac flac flac 2.0 "$tone_flac_generated" "$tone_flac_skip"
emit_image still-red.png png png false "" '[255,0,0]' "$still_red_generated" "$still_red_skip"
emit_image still-alpha.png png png true right-half-transparent '[0,255,0]' "$still_alpha_generated" "$still_alpha_skip"
emit_image still.webp webp webp false "" '[255,0,0]' "$still_webp_generated" "$still_webp_skip"
emit_image still.avif avif av1 false "" '[255,0,0]' "$still_avif_generated" "$still_avif_skip"
emit_image still.heic heic hevc false "" '[255,0,0]' "$still_heic_generated" "$still_heic_skip"
emit_image still.tiff tiff tiff false "" '[255,0,0]' "$still_tiff_generated" "$still_tiff_skip"
emit_video converted.mkv matroska avc1 aac false "" 1.5 0.5 "$converted_mkv_generated" "$converted_mkv_skip"
emit_animation animated.gif gif "$animated_gif_generated" "$animated_gif_skip"
emit_animation animated-apng.png apng "$animated_apng_generated" "$animated_apng_skip"
emit_animation animated.webp webp "$animated_webp_generated" "$animated_webp_skip"

printf '\n]\n' >&3
exec 3>&-
mv fixtures.json.tmp fixtures.json

for output in "${OUTPUTS[@]}"; do
  if [[ -f "$output" ]] && (( $(file_bytes "$output") > MAX_BYTES )); then
    echo "$output exceeds the 300 KiB limit" >&2
    exit 1
  fi
done

echo "Generated fixtures in $SCRIPT_DIR"
