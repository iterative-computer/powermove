#!/bin/sh
# Five 2 s, 160x90, 30 fps H.264 (High, 4:2:0) clips of one flat colour each, so every take
# of the interview fixture can be told apart on screen. FFmpeg colour sources
# only (synthetic, no outside media). generate.py packs the committed outputs;
# rerun this only to change them, then regenerate and bump the suite version.
set -eu
cd "$(dirname "$0")"
make() {
  ffmpeg -nostdin -loglevel error -y -f lavfi -i "color=c=$2:s=160x90:r=30:d=2" \
    -c:v libx264 -preset veryslow -crf 10 -profile:v high -pix_fmt yuv420p -threads 1 \
    -bitexact -fflags +bitexact -map_metadata -1 -movflags +faststart "$1.mp4"
}
make take-1 0xC83C3C
make take-2 0x3CA03C
make take-3 0x3C5AC8
make take-4 0x963CAA
make broll 0xD2B428
