/* The capture page's font setup: the variable-font `FontFace` path from
   typography/font-renderer.ts with the corpus's variable sources, and a
   verbatim copy of raster.ts's private `fontStr` (it lives inside the
   module and cannot be imported). */

import { createVariableFontRenderer } from '../../../../apps/desktop/src/renderer/src/typography/font-renderer';

// Node integration is on in the capture window; keep these out of the bundle.
export const nodeRequire = (window as any).require as (id: string) => any;
const fs = nodeRequire('fs');

/** Filled from the corpus file before the first measurement. */
export const variableSources: Record<string, string> = {};
const sourceCache = new Map<string, ArrayBuffer | undefined>();
function packagedSource(family: string): ArrayBuffer | undefined {
  if (!sourceCache.has(family)) {
    const path = variableSources[family];
    const bytes = path ? fs.readFileSync(path) : undefined;
    sourceCache.set(family, bytes ? bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) : undefined);
  }
  return sourceCache.get(family);
}
export const variationFontFamily = createVariableFontRenderer(() => {}, packagedSource);

const cssString = (value: unknown) => String(value ?? '').replace(/\\/g, '\\\\').replace(/"/g, '\\"');
// Verbatim from legacy/gl/raster.ts.
export function fontStr(d: any) {
  const variableFamily = variationFontFamily(d);
  const families = [variableFamily, d.font, 'SF Pro Display']
    .filter(Boolean)
    .map((family) => `"${cssString(family)}"`)
    .join(', ');
  return `${d.italic ? 'italic ' : ''}${d.weight || 500} ${d.size}px ${families}, -apple-system, sans-serif`;
}
