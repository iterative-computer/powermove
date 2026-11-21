#!/bin/sh
# Generates scene B media (synthetic only): 6 x 4 s 1080p30 H.264 ~20 Mbps clips + 10 s stereo tone bed.
set -eu
OUT="${1:-$HOME/bench-work/media}"; mkdir -p "$OUT"; cd "$OUT"
SRCS="testsrc2 mandelbrot smptehdbars testsrc2 mandelbrot rgbtestsrc"
i=0
for s in $SRCS; do
  i=$((i+1))
  ffmpeg -y -loglevel error -f lavfi -i "$s=size=1920x1080:rate=30" -t 4 \
    -vf "hue=h=$((i*50)),noise=alls=12:allf=t" -c:v libx264 -pix_fmt yuv420p \
    -b:v 20M -minrate 18M -maxrate 22M -bufsize 40M -g 30 -movflags +faststart "clip$i.mp4"
done
ffmpeg -y -loglevel error -f lavfi -i "sine=frequency=440:duration=10:sample_rate=48000" \
  -f lavfi -i "sine=frequency=660:duration=10:sample_rate=48000" \
  -filter_complex "[0][1]amix=inputs=2,volume=0.5" -ac 2 -c:a pcm_s16le music.wav
