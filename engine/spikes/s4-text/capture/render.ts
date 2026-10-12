/* Runs inside the hidden Electron window: draws the S4 render scenes
   (out/scenes.json, written by `s4-render scenes`) with Canvas2D exactly as
   today's rasterizers do and writes each canvas's pixels.

   Text items follow legacy/gl/raster.ts rasterText at density 1: lines from
   textSourceLines, `fontStr(d)`, `letterSpacing` from tracking, textAlign
   from d.align, and one fillText per line at baseline y + lh * i + size * 0.82.
   A caption stroke follows captions/render.ts: strokeText with lineWidth
   2 × stroke, round joins, miter limit 2, before the fill. Path items are
   Path2D fills or strokes (raster.ts shapes use round joins).

   Output: <out>/<id>.rgba (getImageData bytes, unpremultiplied RGBA8),
   <out>/manifest.json; with stress scenes, per-frame timings. */

import { textSourceLines } from '../../../../apps/desktop/src/renderer/src/legacy/core/text-layout';
import { fontStr, nodeRequire, variationFontFamily } from './fonts';

const fs = nodeRequire('fs');
const path = nodeRequire('path');
const { ipcRenderer } = nodeRequire('electron');

type Color = [number, number, number, number];
const css = (c: Color) => `rgba(${Math.round(c[0] * 255)}, ${Math.round(c[1] * 255)}, ${Math.round(c[2] * 255)}, ${c[3]})`;

function drawScene(c: CanvasRenderingContext2D, scene: any, dx = 0) {
  const cx = c as any;
  for (const item of scene.items) {
    if (item.kind === 'text') {
      const d = item.d;
      const size = Math.max(1, Number(d.size) || 16);
      const lh = size * (d.leading || 1.15);
      c.font = fontStr(d);
      if ('letterSpacing' in cx) cx.letterSpacing = (d.tracking || 0) + 'px';
      c.textBaseline = 'alphabetic';
      c.textAlign = d.align === 'center' ? 'center' : d.align === 'right' ? 'right' : 'left';
      const lines = textSourceLines(d, c, lh);
      lines.forEach((line: any, i: number) => {
        const x = item.x + dx, y = item.y + lh * i + size * 0.82;
        if (item.stroke > 0) {
          c.lineJoin = 'round';
          c.miterLimit = 2;
          c.strokeStyle = css(item.strokeColor);
          c.lineWidth = item.stroke * 2;
          c.strokeText(line.text, x, y);
        }
        c.fillStyle = css(item.color);
        c.fillText(line.text, x, y);
      });
    } else if (item.kind === 'path') {
      const p = new Path2D(item.d);
      c.save();
      c.translate(item.x + dx, item.y);
      if (item.stroke > 0) {
        c.lineJoin = item.join === 'miter' ? 'miter' : 'round';
        c.lineCap = 'butt';
        c.lineWidth = item.stroke;
        c.strokeStyle = css(item.color);
        c.stroke(p);
      } else {
        c.fillStyle = css(item.color);
        c.fill(p, item.rule === 'evenodd' ? 'evenodd' : 'nonzero');
      }
      c.restore();
    }
  }
}

function canvasFor(scene: any) {
  const canvas = document.createElement('canvas');
  canvas.width = scene.width;
  canvas.height = scene.height;
  return canvas.getContext('2d')!;
}

async function main() {
  const { scenesPath, outPath } = await ipcRenderer.invoke('config');
  const file = JSON.parse(fs.readFileSync(scenesPath, 'utf8'));
  fs.mkdirSync(outPath, { recursive: true });
  await document.fonts.ready;
  for (const scene of [...file.scenes, ...file.stress]) for (const item of scene.items) if (item.kind === 'text') variationFontFamily(item.d);
  const manifest: any = { userAgent: navigator.userAgent, scenes: [], stress: [] };
  for (const scene of file.scenes) {
    const c = canvasFor(scene);
    drawScene(c, scene);
    const data = c.getImageData(0, 0, scene.width, scene.height).data;
    fs.writeFileSync(path.join(outPath, `${scene.id}.rgba`), Buffer.from(data.buffer, data.byteOffset, data.byteLength));
    manifest.scenes.push({ id: scene.id, width: scene.width, height: scene.height });
  }
  // Stress: redraw every frame with a subpixel shift (results cannot be
  // reused; glyph caches can), synchronized by a one-pixel readback.
  const frames = file.stressFrames ?? 30;
  for (const scene of file.stress) {
    const c = canvasFor(scene);
    const times: number[] = [];
    for (let f = 0; f <= frames; f++) {
      const started = performance.now();
      c.clearRect(0, 0, scene.width, scene.height);
      drawScene(c, scene, (f * 0.137) % 1);
      c.getImageData(0, 0, 1, 1);
      times.push(performance.now() - started);
    }
    const data = c.getImageData(0, 0, scene.width, scene.height).data;
    fs.writeFileSync(path.join(outPath, `${scene.id}.rgba`), Buffer.from(data.buffer, data.byteOffset, data.byteLength));
    manifest.stress.push({ id: scene.id, width: scene.width, height: scene.height, firstMs: times[0], frameMs: times.slice(1) });
  }
  fs.writeFileSync(path.join(outPath, 'manifest.json'), JSON.stringify(manifest, null, 2));
  await ipcRenderer.invoke('rendered', manifest.scenes.length + manifest.stress.length);
}

main().catch(error => ipcRenderer.invoke('failed', String(error?.stack || error)));
