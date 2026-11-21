#!/bin/sh
# Checks every render (codec, profile, size, duration, decoded frame count) and writes one contact sheet
# per output (10 frames, one per second) beside it as <render>.sheet.png. Prints "VERIFY {...}" lines.
set -eu
for f in "$1"/*.mp4 "$1"/*.mov; do
  [ -f "$f" ] || continue
  info=$(ffprobe -v error -count_frames -select_streams v:0 -show_entries stream=codec_name,profile,width,height,nb_read_frames,pix_fmt:format=duration -of json "$f")
  # CSV can emit a blank line for a MOV stream group before the audio stream.
  audio_info=$(ffprobe -v error -select_streams a -show_entries stream=codec_name -of json "$f")
  audio=$(printf '%s\n' "$audio_info" | python3 -c 'import json, sys; print(next((s["codec_name"] for s in json.load(sys.stdin)["streams"] if "codec_name" in s), ""))')
  ffmpeg -v error -y -i "$f" -vf "fps=1,scale=384:-1,tile=5x2" -frames:v 1 "$f.sheet.png" || true
  printf 'VERIFY {"file":"%s","audio":"%s","probe":%s}\n' "$(basename "$f")" "$audio" "$(echo "$info" | tr -d '\n ')"
done
