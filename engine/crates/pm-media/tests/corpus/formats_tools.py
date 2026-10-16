#!/usr/bin/env python3
"""Helpers for formats.sh: things our FFmpeg build cannot write.

Run with `python3 -I`. Subcommands:

  mov-boxes FILE BOX...     append boxes (given as NAME=HEX payload) to the
                            first video sample entry of a MOV/MP4 file, fixing
                            every enclosing size and the chunk offsets
  mdcv-hdr10                print the payload of a P3-D65 1000 cd/m² mastering
                            display box (SMPTE ST 2086 in mdcv order)
  clli MAXCLL MAXFALL       print a content light level payload
  dvcc PROFILE LEVEL COMPAT print a Dolby Vision configuration payload
  clap W H HOFF VOFF        print a clean aperture payload (integers)
  tiff-float OUT W H        write an uncompressed RGB float32 TIFF ramp
  svg OUT                   write a small SVG with gradients and text-free shapes
  ramp16-pgm OUT W H        write a 16-bit grey ramp as a binary PGM (P5)

Everything written is synthetic test media.
"""
import struct
import sys

CONTAINERS = {b"moov", b"trak", b"mdia", b"minf", b"stbl", b"edts", b"udta"}
VISUAL = {b"avc1", b"avc3", b"hvc1", b"hev1", b"dvh1", b"dvhe", b"ap4h", b"apcn", b"apch",
          b"apcs", b"apco", b"ap4x", b"av01", b"vp09", b"mp4v", b"jpeg", b"mjpa"}


def boxes(data, start, end):
    """(offset, size, header length, type) of each box in data[start:end]."""
    out = []
    i = start
    while i + 8 <= end:
        size, kind = struct.unpack(">I4s", data[i:i + 8])
        hdr = 8
        if size == 1:
            size = struct.unpack(">Q", data[i + 8:i + 16])[0]
            hdr = 16
        elif size == 0:
            size = end - i
        out.append((i, size, hdr, kind))
        i += size
    return out


def find_entry(data):
    """The path of (offset, size, hdr, type) boxes from the root to the first
    visual sample entry."""
    def walk(start, end, path):
        for off, size, hdr, kind in boxes(data, start, end):
            here = path + [(off, size, hdr, kind)]
            if kind in CONTAINERS:
                r = walk(off + hdr, off + size, here)
                if r:
                    return r
            elif kind == b"stsd":
                # FullBox (4) + entry count (4), then sample entries.
                for e in boxes(data, off + hdr + 8, off + size):
                    if e[3] in VISUAL:
                        return here + [e]
        return None
    return walk(0, len(data), [])


def shift_chunk_offsets(data, moov, delta, threshold):
    """Adds delta to stco/co64 entries at or after threshold."""
    def walk(start, end):
        for off, size, hdr, kind in boxes(data, start, end):
            if kind in CONTAINERS:
                walk(off + hdr, off + size)
            elif kind in (b"stco", b"co64"):
                n = struct.unpack(">I", data[off + hdr + 4:off + hdr + 8])[0]
                p = off + hdr + 8
                w = 4 if kind == b"stco" else 8
                fmt = ">I" if w == 4 else ">Q"
                for k in range(n):
                    v = struct.unpack(fmt, data[p + k * w:p + (k + 1) * w])[0]
                    if v >= threshold:
                        data[p + k * w:p + (k + 1) * w] = struct.pack(fmt, v + delta)
    walk(moov[0] + moov[2], moov[0] + moov[1])


def mov_boxes(path, specs):
    data = bytearray(open(path, "rb").read())
    path_boxes = find_entry(data)
    if not path_boxes:
        sys.exit(f"{path}: no visual sample entry")
    extra = b""
    for spec in specs:
        name, payload = spec.split("=", 1)
        body = bytes.fromhex(payload)
        extra += struct.pack(">I4s", 8 + len(body), name.encode()) + body
    entry = path_boxes[-1]
    insert_at = entry[0] + entry[1]
    moov = path_boxes[0]
    mdat_after = any(k == b"mdat" and o > moov[0] for o, _, _, k in boxes(data, 0, len(data)))
    data[insert_at:insert_at] = extra
    for off, size, hdr, kind in path_boxes:
        if hdr != 8:
            sys.exit("64-bit box sizes are not handled")
        struct.pack_into(">I", data, off, size + len(extra))
    if mdat_after:
        # moov precedes mdat: media moved by len(extra).
        moov = (moov[0], moov[1] + len(extra), moov[2], moov[3])
        shift_chunk_offsets(data, moov, len(extra), insert_at)
    open(path, "wb").write(data)


def mdcv_hdr10():
    # Display P3 primaries, D65, 1000 / 0.0001 cd/m². mdcv order: G, B, R.
    xy = lambda x, y: struct.pack(">HH", round(x / 0.00002), round(y / 0.00002))
    body = xy(0.265, 0.690) + xy(0.150, 0.060) + xy(0.680, 0.320) + xy(0.3127, 0.3290)
    body += struct.pack(">II", 1000 * 10000, 1)
    return body.hex()


def clli(maxcll, maxfall):
    return struct.pack(">HH", int(maxcll), int(maxfall)).hex()


def dvcc(profile, level, compat):
    profile, level, compat = int(profile), int(level), int(compat)
    # version 1.0; profile(7) level(6) rpu(1) el(1) bl(1); compat(4) + 28 reserved bits.
    bits = (profile << 9) | (level << 3) | (1 << 2) | (0 << 1) | 1
    return (struct.pack(">BBH", 1, 0, bits) + struct.pack(">I", compat << 28) + bytes(16)).hex()


def clap(w, h, hoff, voff):
    w, h, hoff, voff = int(w), int(h), int(hoff), int(voff)
    return struct.pack(">IIIIiIiI", w, 1, h, 1, hoff, 1, voff, 1).hex()


def tiff_float(out, w, h):
    w, h = int(w), int(h)
    pixels = bytearray()
    for y in range(h):
        for x in range(w):
            # Red ramps 0..2 across, green 0..1 down, blue constant 0.25:
            # values above 1 must survive.
            pixels += struct.pack("<fff", 2.0 * x / (w - 1), y / (h - 1), 0.25)
    entries = [
        (256, 4, 1, w), (257, 4, 1, h), (258, 3, 3, None), (259, 3, 1, 1), (262, 3, 1, 2),
        (273, 4, 1, None), (277, 3, 1, 3), (278, 4, 1, h), (279, 4, 1, len(pixels)),
        (284, 3, 1, 1), (339, 3, 3, None),
    ]
    ifd_off = 8
    n = len(entries)
    extra_off = ifd_off + 2 + n * 12 + 4
    bps_off, sf_off = extra_off, extra_off + 6
    data_off = sf_off + 6
    ifd = struct.pack("<H", n)
    for tag, typ, count, value in entries:
        if tag == 258:
            ifd += struct.pack("<HHII", tag, typ, count, bps_off)
        elif tag == 339:
            ifd += struct.pack("<HHII", tag, typ, count, sf_off)
        elif tag == 273:
            ifd += struct.pack("<HHII", tag, typ, count, data_off)
        elif typ == 3:
            ifd += struct.pack("<HHIHH", tag, typ, count, value, 0)
        else:
            ifd += struct.pack("<HHII", tag, typ, count, value)
    ifd += struct.pack("<I", 0)
    body = b"II*\x00" + struct.pack("<I", ifd_off) + ifd
    body += struct.pack("<HHH", 32, 32, 32) + struct.pack("<HHH", 3, 3, 3)
    assert len(body) == data_off
    open(out, "wb").write(body + pixels)


def svg(out):
    open(out, "w").write("""<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" viewBox="0 0 320 180">
  <defs>
    <linearGradient id="g" x1="0" x2="1">
      <stop offset="0" stop-color="#ff0000"/>
      <stop offset="1" stop-color="#0000ff"/>
    </linearGradient>
  </defs>
  <rect x="0" y="0" width="320" height="90" fill="url(#g)"/>
  <circle cx="80" cy="135" r="40" fill="#00ff00" fill-opacity="0.5"/>
  <path d="M180 100 L300 100 L240 170 Z" fill="none" stroke="#ffffff" stroke-width="6"/>
</svg>
""")


def ramp16_pgm(out, w, h):
    w, h = int(w), int(h)
    row = b"".join(struct.pack(">H", round(x * 65535 / (w - 1))) for x in range(w))
    open(out, "wb").write(f"P5\n{w} {h}\n65535\n".encode() + row * h)


def main():
    cmd, *args = sys.argv[1:]
    if cmd == "mov-boxes":
        mov_boxes(args[0], args[1:])
    elif cmd == "mdcv-hdr10":
        print(mdcv_hdr10())
    elif cmd == "clli":
        print(clli(*args))
    elif cmd == "dvcc":
        print(dvcc(*args))
    elif cmd == "clap":
        print(clap(*args))
    elif cmd == "tiff-float":
        tiff_float(*args)
    elif cmd == "svg":
        svg(*args)
    elif cmd == "ramp16-pgm":
        ramp16_pgm(*args)
    else:
        sys.exit(f"unknown command {cmd}")


if __name__ == "__main__":
    main()
