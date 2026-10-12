/* Shared by the capture pages: the variable-font `FontFace` path from
   typography/font-renderer.ts and a verbatim copy of raster.ts's private
   `fontStr` (it lives inside `install()` and cannot be imported). */

import { createVariableFontRenderer } from '../../../../apps/desktop/src/renderer/src/typography/font-renderer';

// Node integration is on in the capture window; keep these out of the bundle.
export const nodeRequire = (window as any).require as (id: string) => any;
const fs = nodeRequire('fs');

export const VARIABLE_SOURCES: Record<string, string> = {
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
