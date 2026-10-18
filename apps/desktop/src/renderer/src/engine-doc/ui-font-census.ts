/*
 * The UI font census (native engine plan, amendment 19; docs/engine/text.md,
 * "The renderer's font cache").
 *
 * Chromium's font cache is per renderer, and the first description to
 * create an entry (family, effective size, weight, slope) fixes its optical
 * size for every later canvas that measures text at that size. Which
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
import { parseFontFamilies, type UiFontCensus, type UiFontUse } from '../../../shared/engine-fonts-seeds';

export interface UiFontCensusHost {
  /** Sends a census message (`engine-fonts:ui-fonts`). */
  report(json: string): Promise<unknown>;
}

type Idle = (callback: () => void) => void;

const FORM_CONTROLS = new Set(['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON']);

/** The description an element's text is laid out with. */
export function useOf(style: Pick<CSSStyleDeclaration, 'fontFamily' | 'fontSize' | 'fontWeight' | 'fontStyle' | 'fontStretch'>): UiFontUse | null {
  const size = Number.parseFloat(style.fontSize);
  const weight = Number.parseFloat(style.fontWeight);
  if (!(size > 0) || !Number.isFinite(weight)) return null;
  // Only normal-width text reaches the seeds' cache keys.
  if (style.fontStretch && style.fontStretch !== 'normal' && style.fontStretch !== '100%') return null;
  return { families: parseFontFamilies(style.fontFamily), size, weight, italic: style.fontStyle !== 'normal' };
}

function hasOwnText(el: Element): boolean {
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
  const seen = new Set<string>();
  const pending = new Set<Node>();
  let full = true;
  let scheduled = false;
  let stopped = false;

  const visit = (el: Element, dpr: number, out: UiFontUse[]): void => {
    if (!hasOwnText(el) || el.getClientRects().length === 0) return;
    const use = useOf(view.getComputedStyle(el));
    if (!use) return;
    const key = `${dpr}|${use.families.join(',')}|${use.size}|${use.weight}|${use.italic}`;
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
    if (!uses.length) return;
    const census: UiFontCensus = { dpr, uses };
    host.report(JSON.stringify(census)).catch(() => undefined);
  };
  const request = (): void => {
    if (scheduled || stopped) return;
    scheduled = true;
    schedule(flush);
  };

  const observer = new view.MutationObserver((records) => {
    for (const r of records) {
      if (r.type === 'childList') r.addedNodes.forEach((n) => pending.add(n));
      else if (r.type === 'characterData') pending.add(r.target);
      else if (r.target === doc.documentElement || r.target === doc.body) full = true;
      else pending.add(r.target);
    }
    request();
  });
  observer.observe(doc, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ['class', 'style', 'hidden', 'open', 'data-density'] });

  // Another display or page zoom lays every text out again.
  let media: MediaQueryList | null = null;
  const watchRatio = (): void => {
    media?.removeEventListener('change', onRatio);
    media = typeof view.matchMedia === 'function' ? view.matchMedia(`(resolution: ${view.devicePixelRatio || 1}dppx)`) : null;
    media?.addEventListener('change', onRatio);
  };
  function onRatio(): void {
    full = true;
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
