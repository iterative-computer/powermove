/* Runs inside the hidden Electron window: draws every rendering case with
   today's Canvas2D code and writes the pixels the compositor would sample.

   - Text: `textGeometry`, `textRasterGeometry`, and `rasterText` are copied
     from legacy/gl/raster.ts (they are private to its `install`); the
     wrapping is legacy/core/text-layout.ts itself and `fontStr` the
     verbatim copy in fonts.ts.
   - Captions: captions/render.ts `createCaptionRasterizer` itself, with
     raster.ts's dependencies (`getCanvas`, `fontStr`, `document.fonts`).

   Each canvas is uploaded to a WebGL2 texture exactly as the compositor's
   `texFor` uploads it (UNPACK_PREMULTIPLY_ALPHA_WEBGL) and read back with
   readPixels: the premultiplied RGBA8 texture the compositor samples,
   top-down rows. Output: <out>/<id>.rgba and <out>/manifest.json. */

import { textSourceLines } from '../../../../apps/desktop/src/renderer/src/legacy/core/text-layout';
import { createCaptionRasterizer } from '../../../../apps/desktop/src/renderer/src/captions/render';
import { fontStr, nodeRequire, variableSources, variationFontFamily } from './fonts';

const fs = nodeRequire('fs');
const path = nodeRequire('path');
const { ipcRenderer } = nodeRequire('electron');

function getCanvas(w: any, h: any) {
  const cv = window.document.createElement('canvas');
  cv.width = Math.max(1, Math.ceil(w)); cv.height = Math.max(1, Math.ceil(h));
  return cv;
}
const clamp = (v: number, a: number, b: number) => Math.max(a, Math.min(b, v));

function textLines(d: any, context: any, lineHeight: number): string[] {
  return textSourceLines(d, context, lineHeight).map(line => line.text);
}

// Verbatim from legacy/gl/raster.ts (cache removed).
function textGeometry(d: any) {
  const size = Math.max(1, Number(d.size) || 16);
  const pad = Math.ceil(size * .6) + 24;
  const meas = getCanvas(8, 8).getContext('2d') as any;
  const align = d.align === 'center' ? 'center' : d.align === 'right' ? 'right' : 'left';
  meas.font = fontStr(d);
  meas.textAlign = align;
  meas.textBaseline = 'alphabetic';
  if ('letterSpacing' in meas) meas.letterSpacing = (d.tracking || 0) + 'px';
  const lh = size * (d.leading || 1.15);
  const lines = textLines(d, meas, lh);
  let wMax = 1;
  for (const line of lines) wMax = Math.max(wMax, meas.measureText(line).width);
  const paragraphWidth = d.paragraph && Number.isFinite(Number(d.boxWidth)) ? Math.max(1, Number(d.boxWidth)) : 0;
  const paragraphHeight = d.paragraph && Number.isFinite(Number(d.boxHeight)) ? Math.max(1, Number(d.boxHeight)) : 0;
  const w = Math.ceil(Math.max(wMax, paragraphWidth)) + pad * 2;
  const hh = Math.ceil(Math.max(lh * lines.length, paragraphHeight)) + pad * 2;
  const x = d.align === 'center' ? w / 2 : d.align === 'right' ? w - pad : pad;
  const anchorX = d.align === 'center' ? w / 2 : d.align === 'right' ? w - pad : pad;
  const anchorY = pad;
  void clamp;
  return { w, h: hh, anchorX, anchorY, lines, lh, size, pad, x, align };
}

function textRasterGeometry(d: any, scale: number) {
  const geometry = textGeometry(d);
  const density = Math.min(scale, 8192 / geometry.w, 8192 / geometry.h);
  return { ...geometry, density, width: Math.max(1, Math.ceil(geometry.w * density)), height: Math.max(1, Math.ceil(geometry.h * density)) };
}

function rasterText(d: any, scale: number) {
  const g = textRasterGeometry(d, scale);
  const cv = getCanvas(g.width, g.height);
  const c = cv.getContext('2d') as any;
  c.scale(g.density, g.density);
  c.font = fontStr(d);
  if ('letterSpacing' in c) c.letterSpacing = (d.tracking || 0) + 'px';
  c.textBaseline = 'alphabetic'; c.textAlign = g.align; c.fillStyle = d.color || '#fff';
  g.lines.forEach((line: string, i: number) => c.fillText(line, g.x, g.pad + g.lh * i + g.size * .82));
  return { cv, geometry: { w: g.w, h: g.h, anchorX: g.anchorX, anchorY: g.anchorY, density: g.density, pad: g.pad, x: g.x, width: g.width, height: g.height } };
}

const captions = createCaptionRasterizer({
  canvas: getCanvas,
  font: (style) => fontStr(style),
  fontReady: (font, text) => document.fonts.check(font, text),
});

// One WebGL2 context reads every canvas back the way texFor uploads it.
const glCanvas = getCanvas(4, 4);
const gl = glCanvas.getContext('webgl2', { premultipliedAlpha: true, antialias: false })!;
const tex = gl.createTexture();
const fb = gl.createFramebuffer();
function readTexture(cv: HTMLCanvasElement): Uint8Array {
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, cv);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
  gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
  const out = new Uint8Array(cv.width * cv.height * 4);
  gl.readPixels(0, 0, cv.width, cv.height, gl.RGBA, gl.UNSIGNED_BYTE, out);
  return out;
}

async function main() {
  const { corpusPath, outPath } = await ipcRenderer.invoke('config');
  const corpus = JSON.parse(fs.readFileSync(corpusPath, 'utf8'));
  Object.assign(variableSources, corpus.variableSources);
  for (const font of corpus.projectFonts) {
    const bytes = fs.readFileSync(font.path);
    const data = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    const face = new FontFace(font.family, data, { weight: font.weight, style: font.style });
    await face.load();
    document.fonts.add(face);
  }
  await document.fonts.ready;
  for (const item of corpus.cases) if (item.kind === 'text') variationFontFamily(item.d);
  // Let variable FontFaces registered by the first calls finish loading.
  await new Promise(resolve => setTimeout(resolve, 500));
  await document.fonts.ready;
  fs.mkdirSync(outPath, { recursive: true });
  const manifest: any = { userAgent: navigator.userAgent, devicePixelRatio: window.devicePixelRatio, cases: [] };
  for (const item of corpus.cases) {
    try {
      let cv: HTMLCanvasElement, geometry: any, blank = false;
      if (item.kind === 'text') {
        const r = rasterText(item.d, item.scale);
        cv = r.cv; geometry = r.geometry;
        blank = !document.fonts.check(fontStr(item.d), String(item.d.text));
      } else {
        const frame = captions.frame(item.content, item.local, item.compositionWidth, item.compositionHeight);
        if (!frame) { manifest.cases.push({ id: item.id, empty: true }); continue; }
        const r = captions.draw(frame, item.scale);
        cv = r.cv; blank = r.blank;
        geometry = { w: r.w, h: r.h, anchorX: r.anchorX, anchorY: r.anchorY, width: cv.width, height: cv.height, states: frame.states, size: frame.layout.size };
      }
      const pixels = readTexture(cv);
      fs.writeFileSync(path.join(outPath, `${item.id}.rgba`), Buffer.from(pixels.buffer, pixels.byteOffset, pixels.byteLength));
      manifest.cases.push({ id: item.id, width: cv.width, height: cv.height, geometry, blank });
      cv.width = cv.height = 0;
    } catch (error) {
      manifest.cases.push({ id: item.id, error: String((error as any)?.stack || error) });
    }
  }
  fs.writeFileSync(path.join(outPath, 'manifest.json'), JSON.stringify(manifest, null, 1));
  await ipcRenderer.invoke('measured', manifest.cases.length);
}

main().catch(error => ipcRenderer.invoke('failed', String(error?.stack || error)));
