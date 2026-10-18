/* Opt-in private format capture. Run explicitly under pm-electron-lock with
   PM_SVGSTILLS_EVIDENCE outside Git. This records the app's actual import
   outcome and uploaded, premultiplied texture; successful capture is not a
   native parity pass. The Rust comparison consumes manifest.json. */
import { execFileSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { crc32, deflateSync } from 'node:zlib';
import { test, expect, launchApp } from '../helpers/app';

function chunk(type: string, data: Buffer) {
  const h = Buffer.alloc(8), c = Buffer.alloc(4);
  h.writeUInt32BE(data.length); h.write(type, 4, 'latin1');
  c.writeUInt32BE(crc32(Buffer.concat([h.subarray(4), data])) >>> 0);
  return Buffer.concat([h, data, c]);
}
function taggedPng(bytes: Buffer, tag: string, data: Buffer, late = false) {
  const chunks: Buffer[] = [bytes.subarray(0, 8)];
  for (let i = 8; i < bytes.length;) {
    const n = bytes.readUInt32BE(i), type = bytes.toString('latin1', i + 4, i + 8);
    if (late && type === 'IEND') chunks.push(chunk(tag, data));
    if (!['iCCP', 'sRGB', 'gAMA', 'cHRM', 'cICP', 'eXIf'].includes(type)) chunks.push(bytes.subarray(i, i + 12 + n));
    if (!late && type === 'IHDR') chunks.push(chunk(tag, data));
    i += n + 12;
  }
  return Buffer.concat(chunks);
}
function grayPng(bits: number, palette = false) {
  const w = 128, h = 64, row = w * bits / 8, data = Buffer.alloc((row + 1) * h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const value = ((x / 8 | 0) + (y / 8 | 0)) % (1 << bits);
    data[y * (row + 1) + 1 + (x * bits / 8 | 0)]! |= value << (8 - bits - x * bits % 8);
  }
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w); ihdr.writeUInt32BE(h, 4); ihdr[8] = bits; ihdr[9] = palette ? 3 : 0;
  const colors = Buffer.alloc((1 << bits) * 3), alpha = Buffer.alloc(1 << bits);
  for (let i = 0; i < alpha.length; i++) { colors.set([i * 67 % 256, i * 113 % 256, i * 179 % 256], i * 3); alpha[i] = i * 255 / (alpha.length - 1); }
  return Buffer.concat([Buffer.from('89504e470d0a1a0a', 'hex'), chunk('IHDR', ihdr), ...(palette ? [chunk('PLTE', colors), chunk('tRNS', alpha)] : []), chunk('IDAT', deflateSync(data)), chunk('IEND', Buffer.alloc(0))]);
}

test('record reachable still decoder classes through the shipped importer', async () => {
  test.setTimeout(600_000);
  const output = process.env.PM_SVGSTILLS_EVIDENCE;
  expect(output, 'set an evidence directory outside Git').toBeTruthy();
  const sources = path.join(output!, 'sources'); await mkdir(sources, { recursive: true });
  const corpus = path.join(process.env.PM_MEDIA_CORPUS || path.join(os.homedir(), 'Library/Caches/powermove-engine/media-corpus'), 'formats/image');
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: process.env.PM_SVGSTILLS_NATIVE === '1' ? '1' : '0' } });
  const cases: Array<{ name: string; file: string; mime: string }> = [];
  const add = async (name: string, bytes: Buffer, mime = '') => {
    const file = path.join(sources, name); await writeFile(file, bytes); cases.push({ name, file, mime }); return file;
  };
  try {
    await session.openEditor();
    const supplied = process.env.PM_SVGSTILLS_CASES;
    if (supplied) cases.push(...JSON.parse(await readFile(supplied, 'utf8')));
    else {
    for (const [name, mime] of [
      ['bmp24.bmp', 'image/bmp'], ['bmp32.bmp', 'image/bmp'], ['icon.ico', 'image/x-icon'], ['gif-still.gif', 'image/gif'],
      ['jpeg-gray.jpg', 'image/jpeg'], ['jpeg-progressive.jpg', 'image/jpeg'], ['jpeg-12bit.jpg', 'image/jpeg'], ['jpeg-lossless16.jpg', 'image/jpeg'], ['cmyk.jpg', 'image/jpeg'],
      ['tiff8-palette.tiff', 'image/tiff'], ['tiff32f-rgb.tiff', 'image/tiff'], ['cmyk.tiff', 'image/tiff'], ['cmyka.tiff', 'image/tiff'],
      ['avif-10bit.avif', 'image/avif'], ['exr-half-alpha.exr', ''], ['exr-float.exr', ''],
      ['dpx8.dpx', ''], ['dpx10.dpx', ''], ['dpx12.dpx', ''], ['dpx16.dpx', ''],
    ]) await add(name!, await readFile(path.join(corpus, name!)), mime);
    const ffmpeg = path.join(process.env.PM_FFMPEG_DIR || path.join(os.homedir(), 'Library/Caches/powermove-engine/ffmpeg/out'), 'bin/ffmpeg');
    const avif10 = path.join(sources, 'avif-10bit-color.avif');
    execFileSync(ffmpeg, ['-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=size=192x128:rate=1', '-frames:v', '1',
      '-vf', 'setparams=color_primaries=1:color_trc=13:colorspace=1:range=pc,format=yuv420p10le', '-c:v', 'libsvtav1', '-preset', '10', '-crf', '25', avif10], { stdio: 'pipe' });
    cases.push({ name: path.basename(avif10), file: avif10, mime: 'image/avif' });
    // Forced image MIME also exercises programmatic import, beyond the picker.
    await add('forced-image.exr', await readFile(path.join(corpus, 'exr-half-alpha.exr')), 'image/x-exr');
    await add('forced-image.dpx', await readFile(path.join(corpus, 'dpx10.dpx')), 'image/x-dpx');
    for (const bits of [1, 2, 4]) {
      await add(`gray-${bits}.png`, grayPng(bits), 'image/png');
      await add(`palette-${bits}.png`, grayPng(bits, true), 'image/png');
    }
    const png = await readFile(path.join(corpus, 'png8-rgb.png'));
    const gamma = Buffer.alloc(4); gamma.writeUInt32BE(70000);
    await add('gamma-only.png', taggedPng(png, 'gAMA', gamma), 'image/png');
    const hdr = await readFile(path.join(corpus, 'png16-rgb.png'));
    for (const [name, transfer] of [['pq', 16], ['hlg', 18]] as const) await add(`hdr-${name}.png`, taggedPng(hdr, 'cICP', Buffer.from([9, transfer, 0, 1])), 'image/png');
    const exif = Buffer.from('4d4d002a00000008000101120003000000010006000000000000', 'hex');
    await add('late-exif.png', taggedPng(png, 'eXIf', exif, true), 'image/png');
    const rgba = await session.page.evaluate(() => {
      const c = new OffscreenCanvas(192, 128), ctx = c.getContext('2d')!, px = ctx.createImageData(192, 128);
      for (let y = 0; y < 128; y++) for (let x = 0; x < 192; x++) px.data.set([x % 64 * 4, y % 64 * 4, (x + y) % 64 * 4, x < 96 ? 255 : Math.round(y * 255 / 127)], (y * 192 + x) * 4);
      ctx.putImageData(px, 0, 0); return c.convertToBlob({ type: 'image/png' }).then(async b => [...new Uint8Array(await b.arrayBuffer())]);
    });
    const alphaSource = path.join(sources, 'alpha-source.png'); await writeFile(alphaSource, Buffer.from(rgba));
    await add('gamma-ramp.png', taggedPng(Buffer.from(rgba), 'gAMA', gamma), 'image/png');
    const writer = path.join(output!, 'writer.swift');
    await writeFile(writer, `import Foundation
import ImageIO
import CoreGraphics
let a = CommandLine.arguments
let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: a[1]) as CFURL, nil)!
var img = CGImageSourceCreateImageAtIndex(src, 0, nil)!
let spaces = ["p3":CGColorSpace(name: CGColorSpace.displayP3)!, "adobe":CGColorSpace(name: CGColorSpace.adobeRGB1998)!, "device":CGColorSpaceCreateDeviceRGB()]
if let cs = spaces[a[4]] { img = img.copy(colorSpace: cs)! }
var props: [CFString:Any] = [:]
if a[4] == "tile" { props[kCGImagePropertyTIFFDictionary] = [kCGImagePropertyTIFFTileWidth:16, kCGImagePropertyTIFFTileLength:16] }
if a[4] == "progressive" { props[kCGImagePropertyJFIFDictionary] = [kCGImagePropertyJFIFIsProgressive:true] }
let dst = CGImageDestinationCreateWithURL(URL(fileURLWithPath:a[2]) as CFURL,a[3] as CFString,1,nil)!
CGImageDestinationAddImage(dst,img,props as CFDictionary)
exit(CGImageDestinationFinalize(dst) ? 0 : 1)
`);
    for (const cs of ['p3', 'adobe']) for (const [ext, uti] of [['png', 'public.png'], ['tiff', 'public.tiff']]) {
      const file = path.join(sources, `${cs}-alpha.${ext}`); execFileSync('/usr/bin/swift', [writer, alphaSource, file, uti!, cs]);
      cases.push({ name: path.basename(file), file, mime: `image/${ext}` });
    }
    for (const [name, src, uti, mode] of [
      ['tiled.tiff', 'png8-rgb.png', 'public.tiff', 'tile'],
      ['progressive-gray.jpg', 'jpeg-gray.jpg', 'public.jpeg', 'progressive'],
    ]) {
      const file = path.join(sources, name!); execFileSync('/usr/bin/swift', [writer, path.join(corpus, src!), file, uti!, mode!]);
      cases.push({ name: name!, file, mime: uti === 'public.tiff' ? 'image/tiff' : 'image/jpeg' });
    }
    }
    const rows: unknown[] = [];
    await session.page.evaluate(() => {
      const input = document.createElement('input'); input.type = 'file'; input.id = 'svgstills-source'; input.hidden = true; document.body.append(input);
    });
    for (const c of cases) {
      await session.page.locator('#svgstills-source').setInputFiles(c.file);
      const capture = await session.page.evaluate(async input => {
        const PM = (window as any).PM;
        PM.replaceProject(PM.mkProject({ w: 256, h: 256, fps: 30, dur: 1, name: input.name }));
        const chosen = (document.querySelector('#svgstills-source') as HTMLInputElement).files![0]!;
        // Preserve Electron's real file path for TIFF/HEIF conversions.
        const file = input.name.startsWith('forced-image') ? new File([chosen], input.name, { type: input.mime }) : chosen;
        try {
          const a = await PM.assets.add(file, { silent: true });
          const w = a.el.width || a.el.naturalWidth, h = a.el.height || a.el.naturalHeight;
          const gl = new OffscreenCanvas(w, h).getContext('webgl2', { antialias: false })!;
          const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
          gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true); gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
          gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, a.el);
          const fb = gl.createFramebuffer(); gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
          gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t, 0);
          const pixels = new Uint8Array(w * h * 4); gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
          const error = gl.getError(); gl.deleteFramebuffer(fb); gl.deleteTexture(t); gl.getExtension('WEBGL_lose_context')?.loseContext();
          return { reachable: true, kind: a.kind, w, h, error, pixels: [...pixels], rendition: a.rendition ?? false };
        } catch (error) { return { reachable: false, error: String(error) }; }
      }, c);
      const { pixels, ...facts } = capture as typeof capture & { pixels?: number[] };
      const raw = `${c.name}.rgba8`;
      if (pixels) await writeFile(path.join(output!, raw), Buffer.from(pixels));
      rows.push({ ...c, ...facts, raw: pixels ? raw : null });
      await writeFile(path.join(output!, 'manifest.json'), JSON.stringify(rows, null, 2));
      // A decoded float ImageBitmap can itself fail today's RGBA8 WebGL
      // upload. Record that outcome; it is not a usable pixel reference.
    }
    expect(rows).toHaveLength(cases.length);
  } finally { await session.close(); }
});
