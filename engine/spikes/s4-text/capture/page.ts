/* Runs inside a hidden Electron window. Measures every corpus case with the
   legacy engine's own code: `textSourceLines` and `glyphLayout` are imported
   from legacy/core/text-layout.ts and variable fonts go through
   typography/font-renderer.ts, exactly as raster.ts uses them (fonts.ts).
   The caret boundaries follow raster.ts's private textCaretLayout. */

import { glyphLayout, textSourceLines } from '../../../../apps/desktop/src/renderer/src/legacy/core/text-layout';
import { fontStr, nodeRequire, variationFontFamily } from './fonts';

const fs = nodeRequire('fs');
const { ipcRenderer } = nodeRequire('electron');

function graphemesOf(value: string): string[] {
  return [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(value)].map(item => item.segment);
}

function measuringContext(d: any) {
  const canvas = document.createElement('canvas');
  canvas.width = 8; canvas.height = 8;
  const meas = canvas.getContext('2d') as any;
  meas.font = fontStr(d);
  meas.textAlign = 'left';
  meas.textBaseline = 'alphabetic';
  if ('letterSpacing' in meas) meas.letterSpacing = (d.tracking || 0) + 'px';
  return meas;
}

function measureCase(d: any) {
  const size = Math.max(1, Number(d.size) || 16);
  const lh = size * (d.leading || 1.15);
  const meas = measuringContext(d);
  const width = (value: string) => meas.measureText(value).width;
  const lines = textSourceLines(d, meas, lh);
  // Caret boundaries as in raster.ts textCaretLayout (x relative to the line start).
  const caret = lines.map(line => {
    const out: Array<[number, number]> = [[line.start, 0]];
    let prefix = '';
    for (const segment of graphemesOf(line.text)) {
      prefix += segment;
      out.push([line.start + prefix.length, width(prefix)]);
    }
    return out;
  });
  const ink = lines.map(line => {
    const m = meas.measureText(line.text);
    return [m.width, m.actualBoundingBoxLeft, m.actualBoundingBoxRight, m.actualBoundingBoxAscent, m.actualBoundingBoxDescent];
  });
  const layout = glyphLayout(d, measuringContext(d));
  return { font: meas.font, lines, caret, ink, layout };
}

async function main() {
  const { corpusPath, outPath, probeFonts } = await ipcRenderer.invoke('config');
  const cases = JSON.parse(fs.readFileSync(corpusPath, 'utf8'));
  await document.fonts.ready;
  // Warm the variable faces: the first call registers the binary source.
  for (const item of cases) variationFontFamily(item.d);
  const results: any[] = [];
  const started = performance.now();
  for (const item of cases) {
    try { results.push({ id: item.id, ...measureCase(item.d) }); }
    catch (error) { results.push({ id: item.id, error: String(error) }); }
  }
  const elapsed = performance.now() - started;
  if (probeFonts) {
    const host = document.createElement('div');
    host.style.cssText = 'position:absolute;left:0;top:0;white-space:pre;';
    for (const item of cases) {
      if (Object.keys(item.d).some(key => key.startsWith('fontAxis.'))) continue;
      const span = document.createElement('span');
      span.dataset.case = item.id;
      span.style.font = fontStr(item.d);
      span.textContent = item.d.text;
      host.appendChild(span);
      host.appendChild(document.createElement('br'));
    }
    document.body.appendChild(host);
  }
  fs.writeFileSync(outPath, JSON.stringify({
    userAgent: navigator.userAgent,
    devicePixelRatio: window.devicePixelRatio,
    elapsedMs: elapsed,
    results,
  }));
  await ipcRenderer.invoke('measured', results.length);
}

main().catch(error => ipcRenderer.invoke('failed', String(error?.stack || error)));
