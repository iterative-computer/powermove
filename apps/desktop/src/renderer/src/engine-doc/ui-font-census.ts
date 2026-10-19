/*
 * The UI font census (native engine plan, amendment 19; docs/engine/text.md,
 * "The renderer's font cache").
 *
 * Chromium's font cache is per renderer, and the first description to
 * create an entry (family, effective size, selection and platform options)
 * fixes its optical size for every later canvas with the same key. Which
 * entries this window's renderer has depends on the UI text it has laid
 * out: the density setting (`--fs-md`), the panels and menus shown, page
 * zoom, and the displays the window has been on. This module watches the
 * document for text it lays out and reports each new description (computed
 * family stack, CSS size, weight, slope) with the device pixel ratio it was
 * laid out at. Main resolves the stacks against the families Chromium sees
 * and gives the window's seeds to pm-wasm and the engines
 * (shared/engine-fonts-seeds.ts).
 *
 * Text counts once it is laid out (an element with client rects holding
 * non-blank text, or a form control with a value or placeholder). The
 * census rescans everything when the device pixel ratio changes (another
 * display, page zoom) or the root's attributes change (density, theme), and
 * otherwise only what mutations touched, when the renderer is idle.
 */
import { fontCacheKey, parseFontFamilies, type UiFontCensus, type UiFontUse } from '../../../shared/engine-fonts-seeds';

export interface UiFontCensusHost {
  /** Sends a census message (`engine-fonts:ui-fonts`). */
  report(json: string): Promise<unknown>;
}

export interface FontCensusStats { scans: number; visited: number; reports: number; ms: number; maxMs: number }
const censusStats = new WeakMap<Document, FontCensusStats>();
export const fontCensusStats = (doc: Document): FontCensusStats | null => {
  const stats = censusStats.get(doc);
  return stats ? { ...stats } : null;
};

type Idle = (callback: () => void) => void;

const FORM_CONTROLS = new Set(['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON']);

/** The description an element's text is laid out with. */
export function useOf(style: Pick<CSSStyleDeclaration, 'fontFamily' | 'fontSize' | 'fontWeight' | 'fontStyle' | 'fontStretch'>): UiFontUse | null {
  const size = Number.parseFloat(style.fontSize);
  const weight = style.fontWeight === 'normal' || !style.fontWeight ? 400 : style.fontWeight === 'bold' ? 700 : Number.parseFloat(style.fontWeight);
  if (!(size > 0) || !Number.isFinite(weight)) return null;
  return { families: parseFontFamilies(style.fontFamily), size, weight, italic: !!style.fontStyle && style.fontStyle !== 'normal', cacheKey: fontCacheKey(style) };
}

function hasOwnText(el: Element): boolean {
  if (['SCRIPT', 'STYLE', 'TEMPLATE', 'NOSCRIPT'].includes(el.tagName)) return false;
  if (FORM_CONTROLS.has(el.tagName)) {
    const control = el as HTMLInputElement;
    if (el.tagName === 'INPUT' && ['hidden', 'checkbox', 'radio', 'range', 'color', 'file', 'image'].includes(control.type)) return false;
    return !!(control.value || control.placeholder || el.textContent?.trim());
  }
  for (let n = el.firstChild; n; n = n.nextSibling) {
    if (n.nodeType === 3 && /\S/.test(n.nodeValue ?? '')) return true;
  }
  return false;
}

/**
 * Starts the census on `doc`. Returns the stop. `idle` schedules work when
 * the renderer is idle (requestIdleCallback by default).
 */
export function startUiFontCensus(doc: Document, host: UiFontCensusHost, idle?: Idle): () => void {
  const view = doc.defaultView;
  if (!view) return () => undefined;
  const schedule: Idle = idle ?? ((cb) => {
    if (typeof view.requestIdleCallback === 'function') view.requestIdleCallback(() => cb(), { timeout: 1000 });
    else view.setTimeout(cb, 200);
  });
  const stats: FontCensusStats = { scans: 0, visited: 0, reports: 0, ms: 0, maxMs: 0 };
  censusStats.set(doc, stats);
  const seen = new Set<string>();
  const pending = new Set<Node>();
  let full = true;
  let scheduled = false;
  // Bumped by every mutation that can change computed fonts: a connected
  // canvas's inherited style is read again only after one.
  let styleGeneration = 0;
  let stopped = false;

  const visit = (el: Element, dpr: number, out: UiFontUse[]): void => {
    stats.visited++;
    if (!hasOwnText(el)) return;
    // The panel pool parks live panels at -9999px. They have layout boxes
    // but never paint their DPR-scaled font strike. Counting those boxes
    // poisons keys the first visible canvas actually creates later.
    if (!Array.from(el.getClientRects()).some(r => r.width > 0 && r.height > 0
      && r.x < view.innerWidth && r.y < view.innerHeight && r.x + r.width > 0 && r.y + r.height > 0)) return;
    const style = view.getComputedStyle(el);
    if (style.visibility !== 'visible' && style.visibility !== '') return;
    const use = useOf(style);
    if (!use) return;
    const key = `${dpr}|${use.families.join(',')}|${use.size}|${use.weight}|${use.italic}|${use.cacheKey}`;
    if (seen.has(key)) return;
    seen.add(key);
    out.push(use);
  };
  const scan = (root: Node, dpr: number, out: UiFontUse[]): void => {
    const start = root.nodeType === 1 ? (root as Element) : root.parentElement;
    if (!start || !start.isConnected) return;
    visit(start, dpr, out);
    const walker = doc.createTreeWalker(start, 1 /* NodeFilter.SHOW_ELEMENT */);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) visit(n as Element, dpr, out);
  };
  const flush = (): void => {
    scheduled = false;
    if (stopped) return;
    const started = view.performance.now();
    stats.scans++;
    const dpr = view.devicePixelRatio || 1;
    const uses: UiFontUse[] = [];
    if (full) {
      full = false;
      pending.clear();
      if (doc.documentElement) scan(doc.documentElement, dpr, uses);
    } else {
      const nodes = [...pending];
      pending.clear();
      for (const n of nodes) scan(n, dpr, uses);
    }
    const ms = view.performance.now() - started;
    stats.ms += ms;
    stats.maxMs = Math.max(stats.maxMs, ms);
    if (!uses.length) return;
    stats.reports++;
    const census: UiFontCensus = { dpr, uses };
    host.report(JSON.stringify(census)).catch(() => undefined);
  };
  const request = (): void => {
    if (scheduled || stopped) return;
    scheduled = true;
    schedule(flush);
  };

  // Playback mutates transforms and opacity every frame. Those cannot
  // create a font-cache entry, so do not traverse the timeline for them.
  const previousStyle = doc.createElement('span').style;
  const fontStyleChanged = (record: MutationRecord): boolean => {
    if (record.attributeName !== 'style') return true;
    const before = previousStyle;
    const after = (record.target as HTMLElement).style;
    before.cssText = record.oldValue ?? '';
    const names = new Set([...Array.from(before), ...Array.from(after)]);
    return [...names].some((name) => /^(font|text-rendering|writing-mode|text-orientation|line-height|display|content-visibility|visibility|--)/.test(name)
      && before.getPropertyValue(name) !== after.getPropertyValue(name));
  };
  const takeRecords = (records: MutationRecord[]): void => {
    for (const r of records) {
      if (r.type === 'childList') r.addedNodes.forEach((n) => pending.add(n));
      else if (r.type === 'characterData') pending.add(r.target);
      else if (!fontStyleChanged(r)) continue;
      else if (r.target === doc.documentElement || r.target === doc.body) full = true;
      else pending.add(r.target);
      styleGeneration++;
    }
    if (full || pending.size) request();
  };
  const observer = new view.MutationObserver(takeRecords);
  observer.observe(doc, { subtree: true, childList: true, characterData: true, attributes: true, attributeOldValue: true, attributeFilter: ['class', 'style', 'hidden', 'open', 'data-density'] });

  const restoreCanvas: Array<() => void> = [];
  for (const proto of [(view as any).CanvasRenderingContext2D?.prototype, (view as any).OffscreenCanvasRenderingContext2D?.prototype]) {
    if (!proto) continue;
    const declaration = doc.createElement('span').style;
    // The descriptions each context used, by state. A connected canvas's
    // state includes the style generation (its inherited style can change);
    // a detached or offscreen canvas has no inherited style. Only a new
    // state parses the font and reads the computed style.
    type Description = { key: string; use: UiFontUse } | null;
    const described = new WeakMap<object, Map<string, Description>>();
    const stateOf = (ctx: CanvasRenderingContext2D): string => {
      const connected = ctx.canvas instanceof view.HTMLCanvasElement && ctx.canvas.isConnected;
      return `${ctx.font}|${ctx.textRendering}|${ctx.fontStretch}${connected ? `|${styleGeneration}` : ''}`;
    };
    const describe = (ctx: CanvasRenderingContext2D): Description => {
      const state = stateOf(ctx);
      let states = described.get(ctx);
      if (!states) described.set(ctx, states = new Map());
      const known = states.get(state);
      if (known !== undefined) return known;
      // A context cycling through many fonts or generations stays bounded.
      if (states.size >= 64) states.clear();
      declaration.font = ctx.font;
      const use = useOf(declaration);
      if (!use) { states.set(state, null); return null; }
      const canvas = ctx.canvas;
      const inherited = canvas instanceof view.HTMLCanvasElement && canvas.isConnected ? view.getComputedStyle(canvas) : null;
      use.cacheKey = fontCacheKey({
        textRendering: ctx.textRendering,
        fontOpticalSizing: inherited?.fontOpticalSizing,
        fontSynthesis: inherited?.fontSynthesis,
        fontStyle: declaration.fontStyle,
        fontStretch: ctx.fontStretch === 'normal' ? declaration.fontStretch : ctx.fontStretch,
        fontVariantEmoji: inherited?.getPropertyValue('font-variant-emoji'),
        fontSizeAdjust: inherited?.fontSizeAdjust,
        fontVariationSettings: inherited?.fontVariationSettings,
        fontPalette: inherited?.fontPalette,
        fontVariantAlternates: inherited?.fontVariantAlternates,
      });
      const description = { key: `1|${use.families.join(',')}|${use.size}|${use.weight}|${use.italic}|${use.cacheKey}`, use };
      states.set(state, description);
      return description;
    };
    for (const name of ['measureText', 'fillText', 'strokeText']) {
      const descriptor = Object.getOwnPropertyDescriptor(proto, name);
      if (typeof descriptor?.value !== 'function' || !descriptor.configurable) continue;
      const method = function(this: CanvasRenderingContext2D, ...args: unknown[]) {
        takeRecords(observer.takeRecords());
        // Platform data is lazy: font assignments can be followed by
        // changes to textRendering/stretch before the first measure or
        // draw, so the description is read at the call, after native
        // parsing. Only a description not yet seen creates a font-cache
        // entry: the DOM text laid out before it is recorded first (the
        // first creator of a key fixes it). A seen one leaves pending DOM
        // text to its idle scan.
        const entry = describe(this);
        if (entry && !seen.has(entry.key) && (full || pending.size)) flush();
        const result = descriptor.value.apply(this, args);
        if (entry && !seen.has(entry.key)) {
          seen.add(entry.key);
          stats.reports++;
          host.report(JSON.stringify({ dpr: 1, uses: [{ ...entry.use }] } satisfies UiFontCensus)).catch(() => undefined);
        }
        return result;
      };
      Object.defineProperty(proto, name, { ...descriptor, value: method });
      restoreCanvas.push(() => {
        if (Object.getOwnPropertyDescriptor(proto, name)?.value === method) Object.defineProperty(proto, name, descriptor);
      });
    }
  }

  // Another display or page zoom lays every text out again.
  let media: MediaQueryList | null = null;
  const watchRatio = (): void => {
    media?.removeEventListener('change', onRatio);
    media = typeof view.matchMedia === 'function' ? view.matchMedia(`(resolution: ${view.devicePixelRatio || 1}dppx)`) : null;
    media?.addEventListener('change', onRatio);
  };
  function onRatio(): void {
    full = true;
    styleGeneration++;
    watchRatio();
    request();
  }
  watchRatio();
  // Fonts that finish loading lay their text out again.
  const fontsDone = (): void => { full = true; request(); };
  doc.fonts?.addEventListener?.('loadingdone', fontsDone);
  request();

  return () => {
    stopped = true;
    observer.disconnect();
    for (const restore of restoreCanvas) restore();
    media?.removeEventListener('change', onRatio);
    doc.fonts?.removeEventListener?.('loadingdone', fontsDone);
  };
}

/** Starts the census when "Use New Engine" is on and main takes reports. */
export function installUiFontCensus(bridge: { nativeEngine?: { enabledSync?(): boolean }; engineFonts?: { reportUiFonts?(json: string): Promise<unknown> } } | null | undefined, doc: Document = document): (() => void) | null {
  let enabled = false;
  try {
    enabled = bridge?.nativeEngine?.enabledSync?.() === true;
  } catch {
    enabled = false;
  }
  const report = bridge?.engineFonts?.reportUiFonts;
  if (!enabled || typeof report !== 'function') return null;
  return startUiFontCensus(doc, { report: (json) => report.call(bridge!.engineFonts, json) });
}
