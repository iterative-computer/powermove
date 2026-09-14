/**
 * Corner smoothing for the Settings screen with Lisse (Figma's squircle
 * curve), applied from one place so the pages keep plain markup. The radius
 * is whatever the element's CSS `border-radius` computes to, so it still
 * derives from the `--r-*` scale; smoothing is 0.85 on cards and 0.5 on
 * controls under 40px, where a full squircle would read as mismatched.
 *
 * Recipe: strip the CSS border into SVG effects drawn on an overlay in the
 * parent, clip the element to the squircle, re-measure on resize. Hover and
 * focus repaint the border from tokens because the stripped CSS border can
 * no longer change on its own.
 */
import {
  acquirePosition,
  createSvgEffects,
  extractAndStripEffects,
  generateClipPath,
  getLayoutSize,
  hasEffects,
  observeAnchor,
  observeResize,
  releasePosition,
  restoreStyles,
  type EffectsConfig,
  type SvgEffectsHandle
} from '@lisse/core';

const CARD_SMOOTHING = 0.85;
const CONTROL_SMOOTHING = 0.5;
const CONTROL_HEIGHT = 40;

export const SQUIRCLE_SELECTOR = [
  '.sg-group',
  '.sg-navbtn',
  '.sg-search',
  '.sg-column .pm-select',
  '.sg-column .settings-input',
  '.sg-column .settings-row > .color-field',
  '.sg-column .field'
].join(', ');

const INTERACTIVE = 'button, select, input, textarea, a, label, [role=tab]';

type Entry = {
  el: HTMLElement;
  anchor: HTMLElement;
  didAcquire: boolean;
  effects?: SvgEffectsHandle;
  extracted: ReturnType<typeof extractAndStripEffects> | null;
  radius: number;
  hover: boolean;
  focus: boolean;
  unobserve: () => void;
  cleanupEvents: () => void;
};

const entries = new WeakMap<HTMLElement, Entry>();
const live = new Set<Entry>();
let repaintQueued = false;

function repaintAll(): void {
  if (repaintQueued) return;
  repaintQueued = true;
  requestAnimationFrame(() => {
    repaintQueued = false;
    for (const entry of live) paint(entry);
  });
}

function radiusOf(el: HTMLElement): number {
  const raw = getComputedStyle(el).borderTopLeftRadius;
  const px = parseFloat(raw);
  return Number.isFinite(px) && !raw.endsWith('%') ? px : 0;
}

function tokenColor(el: HTMLElement, name: string): string {
  return getComputedStyle(el).getPropertyValue(name).trim();
}

function paint(entry: Entry): void {
  const { el, radius } = entry;
  const { width, height } = getLayoutSize(el);
  if (width <= 0 || height <= 0) return;
  const smoothing = height <= CONTROL_HEIGHT ? CONTROL_SMOOTHING : CARD_SMOOTHING;
  const options = { radius: Math.min(radius, Math.min(width, height) / 2), smoothing };
  el.style.clipPath = generateClipPath(width, height, options);
  let effects: EffectsConfig | undefined = entry.extracted?.effects;
  if (effects?.innerBorder && (entry.hover || entry.focus)) {
    const color = entry.focus ? tokenColor(el, '--sg-edge-focus') : tokenColor(el, '--sg-edge-hot');
    if (color) effects = { ...effects, innerBorder: { ...effects.innerBorder, color, opacity: 1 } };
  }
  if (effects && hasEffects(effects)) {
    if (!entry.effects) entry.effects = createSvgEffects(entry.anchor, el);
    entry.effects.update(options, effects, width, height, { x: el.offsetLeft, y: el.offsetTop });
  } else if (entry.effects) {
    entry.effects.destroy();
    entry.effects = undefined;
  }
}

function attach(el: HTMLElement): void {
  if (entries.has(el) || el.closest("[data-slot='smooth-corners-effects']")) return;
  const radius = radiusOf(el);
  if (radius <= 0) return;
  const anchor = el.parentElement;
  if (!anchor) return;
  const didAcquire = acquirePosition(anchor);
  const entry: Entry = {
    el, anchor, didAcquire, radius, hover: false, focus: false,
    extracted: extractAndStripEffects(el),
    unobserve: () => {},
    cleanupEvents: () => {}
  };
  el.setAttribute('data-squircle', '');
  /* The overlay is placed by offsetTop/offsetLeft, so a sibling growing above
     moves this element without resizing it: any resize in the tree repaints
     every entry, and the anchor's own resize re-measures this one. */
  const stopSelf = observeResize(el, () => repaintAll());
  const stopAnchor = observeAnchor(anchor, el);
  entry.unobserve = () => { stopSelf(); stopAnchor(); };
  live.add(entry);
  if (el.matches(INTERACTIVE)) {
    const enter = () => { entry.hover = true; paint(entry); };
    const leave = () => { entry.hover = false; paint(entry); };
    const focusin = () => { entry.focus = true; paint(entry); };
    const focusout = () => { entry.focus = false; paint(entry); };
    el.addEventListener('mouseenter', enter);
    el.addEventListener('mouseleave', leave);
    el.addEventListener('focusin', focusin);
    el.addEventListener('focusout', focusout);
    entry.cleanupEvents = () => {
      el.removeEventListener('mouseenter', enter);
      el.removeEventListener('mouseleave', leave);
      el.removeEventListener('focusin', focusin);
      el.removeEventListener('focusout', focusout);
    };
  }
  entries.set(el, entry);
  paint(entry);
}

function detach(el: HTMLElement): void {
  const entry = entries.get(el);
  if (!entry) return;
  entry.unobserve();
  entry.cleanupEvents();
  entry.effects?.destroy();
  if (entry.extracted) restoreStyles(el, entry.extracted.savedStyles);
  if (entry.didAcquire) releasePosition(entry.anchor);
  el.style.clipPath = '';
  el.removeAttribute('data-squircle');
  entries.delete(el);
  live.delete(entry);
}

/** Smooth every matching element under `root`, now and as the DOM changes. */
export function mountSquircles(root: HTMLElement, selector = SQUIRCLE_SELECTOR): () => void {
  const sweep = () => {
    for (const el of root.querySelectorAll<HTMLElement>(selector)) attach(el);
  };
  sweep();
  const observer = new MutationObserver((records) => {
    for (const record of records) {
      for (const node of record.removedNodes) {
        if (!(node instanceof HTMLElement)) continue;
        if (entries.has(node)) detach(node);
        for (const el of node.querySelectorAll<HTMLElement>('[data-squircle]')) detach(el);
      }
    }
    sweep();
  });
  observer.observe(root, { childList: true, subtree: true });
  return () => {
    observer.disconnect();
    for (const el of root.querySelectorAll<HTMLElement>('[data-squircle]')) detach(el);
  };
}
