/* Runs inside a hidden Electron window. Measures every corpus case with the
   legacy engine's own code: `textSourceLines` and `glyphLayout` are imported
   from legacy/core/text-layout.ts and variable fonts go through
   typography/font-renderer.ts, exactly as raster.ts uses them. `fontStr` and
   the caret boundaries are verbatim copies of the private helpers in
   legacy/gl/raster.ts (they live inside `install()` and cannot be imported). */

import { glyphLayout, textSourceLines } from '../../../../apps/desktop/src/renderer/src/legacy/core/text-layout';
import { createVariableFontRenderer } from '../../../../apps/desktop/src/renderer/src/typography/font-renderer';

// Node integration is on in the capture window; keep these out of the bundle.
const nodeRequire = (window as any).require as (id: string) => any;
const fs = nodeRequire('fs');
const { ipcRenderer } = nodeRequire('electron');

const VARIABLE_SOURCES: Record<string, string> = {
  'Skia': '/System/Library/Fonts/Supplemental/Skia.ttf',
  'STIX Two Text': '/System/Library/Fonts/Supplemental/STIXTwoText.ttf',
  'Kefa III': '/System/Library/Fonts/Supplemental/KefaIII.ttf',
  'Noto Sans Syriac': '/System/Library/Fonts/Supplemental/NotoSansSyriac-Regular.ttf',
  'System Font': '/System/Library/Fonts/SFNS.ttf',
};
const sourceCache = new Map<string, ArrayBuffer | undefined>();
function packagedSource(family: string): ArrayBuffer | undefined {
  if (!sourceCache.has(family)) {
    const path = VARIABLE_SOURCES[family];
    const bytes = path ? fs.readFileSync(path) : undefined;
    sourceCache.set(family, bytes ? bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) : undefined);
  }
  return sourceCache.get(family);
}
const variationFontFamily = createVariableFontRenderer(() => {}, packagedSource);

const cssString = (value: unknown) => String(value ?? '').replace(/\\/g, '\\\\').replace(/"/g, '\\"');
// Verbatim from legacy/gl/raster.ts.
function fontStr(d: any) {
  const variableFamily = variationFontFamily(d);
  const families = [variableFamily, d.font, 'SF Pro Display']
    .filter(Boolean)
    .map((family) => `"${cssString(family)}"`)
    .join(', ');
  return `${d.italic ? 'italic ' : ''}${d.weight || 500} ${d.size}px ${families}, -apple-system, sans-serif`;
}

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
