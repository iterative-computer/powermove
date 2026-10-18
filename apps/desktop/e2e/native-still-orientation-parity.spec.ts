/* Oriented and container-transformed stills (EXIF orientation in PNG, JPEG,
   WebP and TIFF; HEIF irot/imir/clap; AVIF irot/imir/clap, alpha and range)
   imported the way users import them, drawn by the legacy viewer and by the
   native engine over an opaque background. Every case the native engine
   draws must match the legacy export and keep the box today's import
   measured; the cases it still routes must name their reason. Set
   PM_MEDIA_EVIDENCE to keep fixtures, exports and metrics. */
import { execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { crc32 } from 'node:zlib';
import { test, expect, launchApp, type LaunchedApp } from './helpers/app';
import { fixturePath } from './helpers/media';

const W = 128, H = 128;

type Case = { file: string; drawn: boolean };
type Capture = { pixels: number[]; route: string | null; reason: string | null; w: number; h: number };

function exif(orientation: number): Buffer {
  // Big-endian TIFF header, one IFD entry: Orientation (274) SHORT 1.
  const b = Buffer.alloc(26);
  b.write('MM\0*', 0, 'latin1'); b.writeUInt32BE(8, 4); b.writeUInt16BE(1, 8);
  b.writeUInt16BE(274, 10); b.writeUInt16BE(3, 12); b.writeUInt32BE(1, 14); b.writeUInt16BE(orientation, 18);
  b.writeUInt32BE(0, 22);
  return b;
}

function pngChunk(type: string, data: Buffer): Buffer {
  const head = Buffer.alloc(8); head.writeUInt32BE(data.length, 0); head.write(type, 4, 'latin1');
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), data])) >>> 0, 0);
  return Buffer.concat([head, data, crc]);
}

/** `png` with an eXIf chunk right after IHDR (`late`: right before IEND). */
function pngWithExif(png: Buffer, orientation: number, late = false): Buffer {
  const chunk = pngChunk('eXIf', exif(orientation));
  const at = late ? png.length - 12 : 8 + 8 + png.readUInt32BE(8) + 4;
  return Buffer.concat([png.subarray(0, at), chunk, png.subarray(at)]);
}

/** `webp` without its ICCP chunk (Chromium's encoder embeds sRGB). */
function webpWithoutIcc(webp: Buffer): Buffer {
  if (webp.toString('latin1', 12, 16) !== 'VP8X') return webp;
  const chunks: Buffer[] = [];
  for (let at = 12; at + 8 <= webp.length;) {
    const size = webp.readUInt32LE(at + 4), end = at + 8 + size + (size & 1);
    if (webp.toString('latin1', at, at + 4) !== 'ICCP') chunks.push(webp.subarray(at, end));
    at = end;
  }
  const body = Buffer.concat([Buffer.from('WEBP', 'latin1'), ...chunks]);
  body[12]! &= ~0x20;
  const riff = Buffer.alloc(8); riff.write('RIFF', 0, 'latin1'); riff.writeUInt32LE(body.length, 4);
  return Buffer.concat([riff, body]);
}

/** A lossy WebP (simple `VP8 ` or extended `VP8X`) with an EXIF chunk. */
function webpWithExif(webp: Buffer, orientation: number): Buffer {
  const e = exif(orientation);
  const exifChunk = Buffer.concat([Buffer.from('EXIF', 'latin1'), Buffer.alloc(4), e]);
  exifChunk.writeUInt32LE(e.length, 4);
  let body: Buffer;
  if (webp.toString('latin1', 12, 16) === 'VP8X') {
    body = Buffer.from(webp.subarray(8));
    body[12]! |= 0x08;
  } else {
    if (webp.toString('latin1', 12, 16) !== 'VP8 ') throw new Error('expected a lossy WebP');
    const w = (webp.readUInt16LE(26) & 0x3fff) - 1, h = (webp.readUInt16LE(28) & 0x3fff) - 1;
    const vp8x = Buffer.alloc(18); vp8x.write('VP8X', 0, 'latin1'); vp8x.writeUInt32LE(10, 4); vp8x[8] = 0x08;
    vp8x.writeUIntLE(w, 12, 3); vp8x.writeUIntLE(h, 15, 3);
    body = Buffer.concat([Buffer.from('WEBP', 'latin1'), vp8x, webp.subarray(12)]);
  }
  body = Buffer.concat([body, exifChunk, Buffer.alloc(e.length & 1)]);
  const riff = Buffer.alloc(8); riff.write('RIFF', 0, 'latin1'); riff.writeUInt32LE(body.length, 4);
  return Buffer.concat([riff, body]);
}

/** Sets the angle of the (single) `irot` property ImageIO writes (0 to 3,
    anticlockwise quarter turns). */
function withIrot(heif: Buffer, angle: number): Buffer {
  const at = heif.indexOf('irot', 0, 'latin1');
  if (at < 0) throw new Error('no irot property');
  const out = Buffer.from(heif); out[at + 4] = angle; return out;
}

/** Replaces that `irot` with an `imir` of the same size (`axis` 0: top and
    bottom exchanged; 1: left and right). */
function withImir(heif: Buffer, axis: number): Buffer {
  const at = heif.indexOf('irot', 0, 'latin1');
  if (at < 0) throw new Error('no irot property');
  const out = Buffer.from(heif); out.write('imir', at, 'latin1'); out[at + 4] = axis; return out;
}

/** Moves the `clap` ImageIO writes one pixel right: a clean aperture whose
    origin is not (0, 0), which Chromium ignores. */
function withShiftedClap(heif: Buffer): Buffer {
  const at = heif.indexOf('clap', 0, 'latin1');
  if (at < 0) throw new Error('no clap property');
  const out = Buffer.from(heif);
  // horizOffN / horizOffD follow the width and height fractions.
  const n = out.readInt32BE(at + 4 + 16), d = out.readUInt32BE(at + 4 + 20);
  out.writeInt32BE(n + d, at + 4 + 16); return out;
}

const WRITER = `import Foundation
import ImageIO
import CoreGraphics
// writer src out uti orientation colorspace(device|srgb|gray|-)
let a = CommandLine.arguments
let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: a[1]) as CFURL, nil)!
var img = CGImageSourceCreateImageAtIndex(src, 0, nil)!
if a[5] == "device" { img = img.copy(colorSpace: CGColorSpaceCreateDeviceRGB())! }
if a[5] == "srgb" { img = img.copy(colorSpace: CGColorSpace(name: CGColorSpace.sRGB)!)! }
if a[5] == "gray" {
  let ctx = CGContext(data: nil, width: img.width, height: img.height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.genericGrayGamma2_2)!, bitmapInfo: CGImageAlphaInfo.none.rawValue)!
  ctx.draw(img, in: CGRect(x: 0, y: 0, width: img.width, height: img.height))
  img = ctx.makeImage()!
}
let props: [CFString: Any] = [kCGImagePropertyOrientation: Int(a[4])!]
let dst = CGImageDestinationCreateWithURL(URL(fileURLWithPath: a[2]) as CFURL, a[3] as CFString, 1, nil)!
CGImageDestinationAddImage(dst, img, props as CFDictionary)
exit(CGImageDestinationFinalize(dst) ? 0 : 1)
`;

async function sources(session: LaunchedApp, dir: string) {
  const make = (w: number, h: number, alpha: boolean, type: string, quality?: number) => session.page.evaluate(({ w, h, alpha, type, quality }) => {
    const c = document.createElement('canvas'); c.width = w; c.height = h; const ctx = c.getContext('2d')!;
    const px = ctx.createImageData(w, h);
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      // A bright top-left marker so every flip and turn is visible.
      const marker = x < w / 4 && y < h / 4;
      px.data.set([marker ? 255 : x * 255 / w, marker ? 40 : y * 255 / h, (x * 13 + y * 29) % 256,
        alpha && !marker ? (x * 11 + y * 5) % 256 : 255], (y * w + x) * 4);
    }
    ctx.putImageData(px, 0, 0); return c.toDataURL(type, quality).split(',')[1]!;
  }, { w, h, alpha, type, quality });
  const write = async (name: string, data: string) => { const p = path.join(dir, name); await writeFile(p, Buffer.from(data, 'base64')); return p; };
  return {
    png: await write('source.png', await make(96, 64, false, 'image/png')),
    odd: await write('source-odd.png', await make(95, 63, false, 'image/png')),
    alpha: await write('source-alpha.png', await make(96, 64, true, 'image/png')),
    webp: await write('source.webp', await make(96, 64, false, 'image/webp', 0.9)),
    webpAlpha: await write('source-alpha.webp', await make(96, 64, true, 'image/webp', 0.9)),
  };
}

async function fixtures(session: LaunchedApp, dir: string): Promise<Case[]> {
  const src = await sources(session, dir);
  const script = path.join(dir, 'writer.swift'), writer = path.join(dir, 'writer');
  await writeFile(script, WRITER);
  execFileSync('/usr/bin/swiftc', ['-O', script, '-o', writer], { stdio: 'pipe' });
  const cases: Case[] = [];
  const add = (file: string, drawn: boolean) => cases.push({ file, drawn });
  const out = (name: string) => path.join(dir, name);
  const imageio = (from: string, name: string, uti: string, orientation: number, space = '-') => {
    execFileSync(writer, [from, out(name), uti, String(orientation), space], { stdio: 'pipe' }); return out(name);
  };
  const png = await readFile(src.png);
  add(src.png, true); add(src.alpha, true);
  for (const o of [2, 3, 4, 5, 6, 7, 8]) {
    await writeFile(out(`png-exif${o}.png`), pngWithExif(png, o)); add(out(`png-exif${o}.png`), true);
  }
  // Chromium reads eXIf only before IDAT; the native decoder does too, but
  // this structure stays routed until it is compared.
  await writeFile(out('png-exif6-late.png'), pngWithExif(png, 6, true)); add(out('png-exif6-late.png'), false);
  const alphaPng = await readFile(src.alpha);
  await writeFile(out('png-alpha-exif6.png'), pngWithExif(alphaPng, 6)); add(out('png-alpha-exif6.png'), true);
  for (const o of [2, 3, 4, 5, 6, 7, 8]) add(imageio(src.png, `jpeg-o${o}.jpg`, 'public.jpeg', o, 'srgb'), true);
  add(imageio(src.png, 'jpeg-gray-o6.jpg', 'public.jpeg', 6, 'gray'), true);
  // Chromium ignores WebP EXIF orientation.
  const webp = webpWithoutIcc(await readFile(src.webp)), webpAlpha = webpWithoutIcc(await readFile(src.webpAlpha));
  await writeFile(out('webp.webp'), webp); add(out('webp.webp'), true);
  await writeFile(out('webp-alpha.webp'), webpAlpha); add(out('webp-alpha.webp'), true);
  await writeFile(out('webp-exif6.webp'), webpWithExif(webp, 6)); add(out('webp-exif6.webp'), true);
  await writeFile(out('webp-alpha-exif8.webp'), webpWithExif(webpAlpha, 8)); add(out('webp-alpha-exif8.webp'), true);
  await writeFile(out('still-exif6.webp'), webpWithExif(await readFile(fixturePath('still.webp')), 6)); add(out('still-exif6.webp'), true);
  for (const o of [2, 3, 4, 5, 6, 7, 8]) add(imageio(src.png, `tiff-o${o}.tiff`, 'public.tiff', o, 'device'), true);
  add(imageio(src.alpha, 'tiff-alpha-o6.tiff', 'public.tiff', 6, 'device'), true);
  for (const o of [2, 3, 4, 5, 6, 7, 8]) add(imageio(src.png, `heic-o${o}.heic`, 'public.heic', o), true);
  add(imageio(src.odd, 'heic-clap.heic', 'public.heic', 1), true);
  add(imageio(src.odd, 'heic-clap-o6.heic', 'public.heic', 6), true);
  add(imageio(src.png, 'avif.avif', 'public.avif', 1), true);
  add(imageio(src.alpha, 'avif-alpha.avif', 'public.avif', 1), true);
  add(imageio(src.odd, 'avif-clap.avif', 'public.avif', 1), true);
  await writeFile(out('avif-clap-shifted.avif'), withShiftedClap(await readFile(out('avif-clap.avif'))));
  add(out('avif-clap-shifted.avif'), true);
  const avif = await readFile(out('avif.avif'));
  for (const angle of [1, 2, 3]) {
    await writeFile(out(`avif-irot${angle}.avif`), withIrot(avif, angle)); add(out(`avif-irot${angle}.avif`), true);
  }
  for (const axis of [0, 1]) {
    await writeFile(out(`avif-imir${axis}.avif`), withImir(avif, axis)); add(out(`avif-imir${axis}.avif`), true);
  }
  await writeFile(out('avif-alpha-irot3.avif'), withIrot(await readFile(out('avif-alpha.avif')), 3)); add(out('avif-alpha-irot3.avif'), true);
  // Limited range (FFmpeg/SVT-AV1); ImageIO writes full range.
  add(fixturePath('still-limited-range.avif'), true);
  return cases;
}

async function capture(session: LaunchedApp, cases: Case[], output: string, label: 'legacy' | 'native'): Promise<Capture[]> {
  await session.openEditor();
  const frames: Capture[] = [];
  for (const { file } of cases) {
    await session.page.evaluate(({ W, H }) => {
      const PM = (window as any).PM;
      PM.replaceProject(PM.mkProject({ w: W, h: H, fps: 30, dur: 1, name: 'Orientation parity', bg: '#5a6e82' }));
      PM.ProjectsScreen.hide();
    }, { W, H });
    const chooser = session.page.waitForEvent('filechooser');
    await session.page.evaluate(() => (window as any).PM.pickFiles());
    await (await chooser).setFiles(file);
    await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'image'), null, { timeout: 30_000 });
    let route: string | null = null, reason: string | null = null;
    if (label === 'native') {
      // The route settles once the host has bound the asset and the
      // capability pass has seen its bytes.
      let last = '', stable = 0;
      for (let i = 0; i < 60 && stable < 4; i++) {
        const s = await session.page.evaluate(() => {
          const s = (window as any).PM.EngineConnection.status(); return { route: s.route, reason: s.routeReason };
        });
        const key = JSON.stringify(s);
        stable = key === last && (s.route === 'engine' || s.route === 'legacy') ? stable + 1 : 0; last = key;
        route = s.route; reason = s.reason;
        await session.page.waitForTimeout(250);
      }
    }
    const { w, h } = await session.page.evaluate(() => {
      const PM = (window as any).PM; const layer = PM.proj.layers.find((l: any) => l.type === 'image');
      const asset = PM.proj.assets[layer.d.asset]; return { w: asset.w, h: asset.h };
    });
    const outputFile = path.join(output, `${label}-${path.basename(file)}.png`);
    await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, outputFile);
    expect(await session.page.evaluate(async ({ W, H }) => (window as any).PM.Export.run({ format: 'still', w: W, h: H, alpha: false, mblur: false }), { W, H })).toEqual({ cancelled: false });
    const pixels = await session.page.evaluate(async ({ bytes, W, H }) => {
      const image = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
      const canvas = new OffscreenCanvas(W, H), ctx = canvas.getContext('2d')!; ctx.drawImage(image, 0, 0); image.close();
      return [...ctx.getImageData(0, 0, W, H).data];
    }, { bytes: [...await readFile(outputFile)], W, H });
    frames.push({ pixels, route, reason, w, h });
  }
  return frames;
}

test('Oriented and transformed stills match the legacy viewer or name their route', async () => {
  test.setTimeout(900_000);
  const output = process.env.PM_MEDIA_EVIDENCE ? path.join(process.env.PM_MEDIA_EVIDENCE, 'orientation') : await mkdtemp(path.join(os.tmpdir(), 'pm-orientation-parity-'));
  await mkdir(output, { recursive: true });
  const diagnostic = process.env.PM_MEDIA_DIAGNOSTIC === '1';
  let session: LaunchedApp | null = null;
  try {
    session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
    const cases = await fixtures(session, output);
    const legacy = await capture(session, cases, output, 'legacy'); await session.close(); session = null;
    session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
    const native = await capture(session, cases, output, 'native');
    const metrics = native.map((n, i) => {
      const l = legacy[i]!;
      let max = 0, over1 = 0, alpha = 0;
      for (let p = 0; p < n.pixels.length; p += 4) {
        const a = n.pixels[p + 3]!, b = l.pixels[p + 3]!; alpha = Math.max(alpha, Math.abs(a - b));
        let d = Math.abs(a - b);
        for (let c = 0; c < 3; c++) d = Math.max(d, Math.abs(n.pixels[p + c]! * a / 255 - l.pixels[p + c]! * b / 255));
        max = Math.max(max, d); if (d > 1) over1++;
      }
      return {
        file: path.basename(cases[i]!.file), expectDrawn: cases[i]!.drawn, route: n.route, reason: n.reason,
        legacySize: [l.w, l.h], nativeSize: [n.w, n.h], max, over1, pixels: n.pixels.length / 4, alpha,
      };
    });
    await writeFile(path.join(output, 'metrics.json'), JSON.stringify(metrics, null, 2));
    if (diagnostic) return;
    for (const m of metrics) {
      const at = JSON.stringify(m);
      expect(m.nativeSize, at).toEqual(m.legacySize);
      if (m.expectDrawn) {
        expect({ route: m.route, reason: m.reason }, at).toEqual({ route: 'engine', reason: null });
        expect(m.max, at).toBeLessThanOrEqual(4);
        expect(m.over1 / m.pixels, at).toBeLessThanOrEqual(0.001);
        expect(m.alpha, at).toBeLessThanOrEqual(1);
      } else {
        expect(m.route, at).toBe('legacy');
        expect(m.reason ?? '', at).toMatch(/oriented stills|AVIF|HEIF/);
      }
    }
  } finally {
    await session?.close(); if (!process.env.PM_MEDIA_EVIDENCE) await rm(output, { recursive: true, force: true });
  }
});
