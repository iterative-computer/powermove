/** Small original panoramas with values above 1, no external assets. */
export function hdrAreaLightPanorama(point=false):Uint8Array<ArrayBuffer>{
  const width=128,height=64,bytes=hdrPanorama(width,height),offset=bytes.length-width*height*4;
  for(let y=0;y<height;y++)for(let x=0;x<width;x++)
    bytes.set(point?(x===80&&y===44?[128,128,128,140]:[0,0,0,0]):x>=78&&x<90&&y>=42&&y<50?[128,128,128,134]:[1,1,1,129],offset+(y*width+x)*4);
  return bytes;
}
export function hdrPanorama(width = 128, height = 64): Uint8Array<ArrayBuffer> {
  const header = new TextEncoder().encode(`#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y ${height} +X ${width}\n`);
  const out = new Uint8Array(header.length + width * height * 4); out.set(header);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) out.set(x < width / 2 ? [128, 16, 8, 131] : [8, 32, 128, 129], header.length + (y * width + x) * 4);
  return out;
}
export function exrPanorama(): Uint8Array<ArrayBuffer> {
  const bytes: number[] = [];
  const u32 = (v: number) => { for (let i = 0; i < 4; i++) bytes.push((v >>> (8 * i)) & 255); };
  const str = (s: string) => { bytes.push(...new TextEncoder().encode(s), 0); };
  const attr = (name: string, type: string, data: number[]) => { str(name); str(type); u32(data.length); bytes.push(...data); };
  const ints = (...values: number[]) => values.flatMap(v => [v & 255, (v >>> 8) & 255, (v >>> 16) & 255, v >>> 24]);
  u32(20000630); u32(2);
  const channels: number[] = [];
  for (const name of ['B', 'G', 'R']) channels.push(name.charCodeAt(0), 0, ...ints(1), 0, 0, 0, 0, ...ints(1, 1));
  channels.push(0);
  attr('channels', 'chlist', channels); attr('compression', 'compression', [0]);
  attr('dataWindow', 'box2i', ints(0, 0, 127, 63)); attr('displayWindow', 'box2i', ints(0, 0, 127, 63));
  attr('lineOrder', 'lineOrder', [0]); attr('pixelAspectRatio', 'float', ints(0x3f800000));
  attr('screenWindowCenter', 'v2f', ints(0, 0)); attr('screenWindowWidth', 'float', ints(0x3f800000)); bytes.push(0);
  const scanlineSize = 8 + 128 * 3 * 2, start = bytes.length + 8 * 64;
  for (let y = 0; y < 64; y++) { u32(start + y * scanlineSize); u32(0); }
  for (let y = 0; y < 64; y++) {
    u32(y); u32(128 * 3 * 2);
    for (const channel of [2, 1, 0]) for (let x = 0; x < 128; x++) {
      const v = (x < 64 ? [0x4400, 0x3800, 0x3400] : [0x3000, 0x3400, 0x3c00])[channel]!;
      bytes.push(v & 255, v >>> 8);
    }
  }
  return new Uint8Array(bytes);
}
