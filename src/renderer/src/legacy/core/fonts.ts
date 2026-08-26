/* Ported from js/core/fonts.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
const NATIVE = ['SF Pro Text', 'SF Pro Display', 'SF Mono'];
const WEB_FALLBACKS = [
  'Helvetica Neue', 'Helvetica', 'Arial', 'Arial Black', 'Arial Narrow',
  'Avenir', 'Avenir Next', 'Futura', 'Gill Sans', 'Optima',
  'Georgia', 'Times New Roman', 'Baskerville', 'Didot', 'American Typewriter',
  'Courier New', 'Menlo', 'Monaco', 'Andale Mono', 'Impact',
];

const clean = (value: any) => String(value || '').replace(/[\u0000-\u001f\u007f]/g, '').trim();
const unique = (values: any[]) => [...new Set(values.map(clean).filter(Boolean))];

const Fonts: any = {
  bundled: [],
  system: [],
  families: unique([...NATIVE, ...WEB_FALLBACKS]),

  setSystemFamilies(values: any) {
    const system = unique(Array.isArray(values) ? values : []).sort((a, b) => a.localeCompare(b));
    this.system = system;
    this.families = unique([...NATIVE, ...system, ...WEB_FALLBACKS]);
    PM.bus.emit('fonts', this.families);
  },

  options(current: any) {
    const value = clean(current);
    return unique([value, ...this.families]);
  },

  async ensure(family: any, weight = 400) {
    const name = clean(family);
    if (!name || !window.document.fonts || !window.document.fonts.load) return;
    const safeName = name.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
    try {
      await window.document.fonts.load(`${Number(weight) || 400} 32px "${safeName}"`, 'Powermove');
      await window.document.fonts.ready;
    } catch (error) {
      window.console.warn('Font load failed:', name, error);
    }
    /* A canvas raster made before a webfont finished loading contains fallback
       pixels. Clear both CPU and GPU caches so the chosen face appears now. */
    PM.rasterClear && PM.rasterClear();
    PM.invalidate();
  },
};

PM.Fonts = Fonts;
window.document.fonts && window.document.fonts.ready && window.document.fonts.ready.then(() => {
  NATIVE.forEach(name => Fonts.ensure(name));
});
}
