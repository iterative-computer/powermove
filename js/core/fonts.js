/* No longer loaded — superseded by src/renderer/src/legacy/core/fonts.ts; kept for the legacy test oracle until Phase 6. */
/* Powermove — bundled and native system-font catalog. */
(() => {
const PM = window.PM;

const BUNDLED = ['Geist', 'Geist Mono'];
const WEB_FALLBACKS = [
  'Helvetica Neue', 'Helvetica', 'Arial', 'Arial Black', 'Arial Narrow',
  'Avenir', 'Avenir Next', 'Futura', 'Gill Sans', 'Optima',
  'Georgia', 'Times New Roman', 'Baskerville', 'Didot', 'American Typewriter',
  'Courier New', 'Menlo', 'Monaco', 'Andale Mono', 'Impact',
];

const clean = value => String(value || '').replace(/[\u0000-\u001f\u007f]/g, '').trim();
const unique = values => [...new Set(values.map(clean).filter(Boolean))];

const Fonts = {
  bundled: [...BUNDLED],
  system: [],
  families: unique([...BUNDLED, ...WEB_FALLBACKS]),

  setSystemFamilies(values) {
    const system = unique(Array.isArray(values) ? values : []).sort((a, b) => a.localeCompare(b));
    this.system = system;
    this.families = unique([...BUNDLED, ...system, ...WEB_FALLBACKS]);
    PM.bus.emit('fonts', this.families);
  },

  options(current) {
    const value = clean(current);
    return unique([value, ...this.families]);
  },

  async ensure(family, weight = 400) {
    const name = clean(family);
    if (!name || !document.fonts || !document.fonts.load) return;
    const safeName = name.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
    try {
      await document.fonts.load(`${Number(weight) || 400} 32px "${safeName}"`, 'Powermove');
      await document.fonts.ready;
    } catch (error) {
      console.warn('Font load failed:', name, error);
    }
    /* A canvas raster made before a webfont finished loading contains fallback
       pixels. Clear both CPU and GPU caches so the chosen face appears now. */
    PM.rasterClear && PM.rasterClear();
    PM.invalidate();
  },
};

PM.Fonts = Fonts;
document.fonts && document.fonts.ready && document.fonts.ready.then(() => {
  BUNDLED.forEach(name => Fonts.ensure(name));
});
})();
