// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest';

import { installUiFontCensus, startUiFontCensus, useOf } from './ui-font-census';
import { CANVAS_FONT_CACHE_KEY, type UiFontCensus } from '../../../shared/engine-fonts-seeds';

// happy-dom does no layout: every element with text counts as laid out
// unless the test hides it.
const hidden = new WeakSet<Element>();
const proto = Element.prototype as any;
const original = proto.getClientRects;
proto.getClientRects = function (this: Element) { return hidden.has(this) ? [] : [{ x: 0, y: 0, width: 1, height: 1 }]; };

const tick = () => new Promise<void>((r) => setTimeout(r, 0));

describe('the UI font census', () => {
  afterEach(() => { document.body.innerHTML = ''; document.head.innerHTML = ''; });

  it('reads the description text is laid out with', () => {
    expect(useOf({ fontFamily: '"SF Pro Text", -apple-system, BlinkMacSystemFont, "Helvetica Neue", sans-serif', fontSize: '14px', fontWeight: '500', fontStyle: 'normal', fontStretch: '100%' }))
      .toEqual({ families: ['sf pro text', '-apple-system', 'blinkmacsystemfont', 'helvetica neue', 'sans-serif'], size: 14, weight: 500, italic: false, cacheKey: CANVAS_FONT_CACHE_KEY });
    expect(useOf({ fontFamily: 'system-ui', fontSize: '12px', fontWeight: '400', fontStyle: 'italic', fontStretch: 'normal' })!.italic).toBe(true);
    expect(useOf({ fontFamily: 'system-ui', fontSize: '12px', fontWeight: '400', fontStyle: 'normal', fontStretch: '75%' })!.cacheKey).not.toBe(CANVAS_FONT_CACHE_KEY);
  });

  it('reports each new description once, with the device pixel ratio, and what mutations add later', async () => {
    const reports: UiFontCensus[] = [];
    const style = document.createElement('style');
    style.textContent = `:root{--fs-md:13px} :root[data-density="comfy"]{--fs-md:14px}
      body{font-family:-apple-system;font-size:var(--fs-md);font-weight:400}
      .title{font-size:15px;font-weight:600} .ghost{font-size:11px}`;
    document.head.append(style);
    document.body.innerHTML = '<div>Layers</div><div>Effects</div><span class="title">Inspector</span><div class="ghost">   </div><div class="hidden">Off screen</div>';
    hidden.add(document.querySelector('.hidden')!);
    const queue: Array<() => void> = [];
    const stop = startUiFontCensus(document, { report: async (json) => { reports.push(JSON.parse(json)); } }, (cb) => queue.push(cb));
    queue.shift()!();
    const sizes = (c: UiFontCensus) => c.uses.map((u) => [u.size, u.weight]);
    expect(reports).toHaveLength(1);
    expect(reports[0]!.dpr).toBe(window.devicePixelRatio || 1);
    expect(sizes(reports[0]!)).toEqual([[13, 400], [15, 600]]);
    // A menu opens with 11 px text: only it is scanned and reported.
    const menu = document.createElement('div');
    menu.className = 'ghost';
    menu.textContent = 'Rename';
    document.body.append(menu);
    await tick();
    queue.shift()!();
    expect(sizes(reports[1]!)).toEqual([[11, 400]]);
    // The density changes on the root: everything is laid out again.
    document.documentElement.setAttribute('data-density', 'comfy');
    await tick();
    queue.shift()!();
    expect(sizes(reports[2]!)).toEqual([[14, 400]]);
    // Nothing new, nothing sent.
    document.body.append(document.createTextNode(''));
    await tick();
    queue.splice(0).forEach((cb) => cb());
    expect(reports).toHaveLength(3);
    stop();
  });

  it('does not seed the DPR-scaled strikes of parked offscreen panels', () => {
    const parked = document.createElement('b');
    parked.style.font = '560 12px system-ui'; parked.textContent = 'Parked panel';
    parked.getClientRects = () => [{ x: -9999, y: 0, width: 200, height: 20 }] as any;
    document.body.append(parked);
    const reports: UiFontCensus[] = [];
    const queue: Array<() => void> = [];
    const stop = startUiFontCensus(document, { report: async json => { reports.push(JSON.parse(json)); } }, cb => queue.push(cb));
    queue.splice(0).forEach(cb => cb());
    expect(reports).toEqual([]);
    stop();
  });

  it('does not walk a timeline for playback transform and opacity updates', async () => {
    const timeline = document.createElement('div');
    timeline.style.font = '400 13px system-ui';
    timeline.textContent = 'Timeline';
    document.body.append(timeline);
    const queue: Array<() => void> = [];
    const stop = startUiFontCensus(document, { report: async () => true }, cb => queue.push(cb));
    queue.splice(0).forEach(cb => cb());
    timeline.style.transform = 'translateX(10px)';
    timeline.style.opacity = '0.5';
    await tick();
    expect(queue).toHaveLength(0);
    timeline.style.fontSize = '15px';
    await tick();
    expect(queue).toHaveLength(1);
    stop();
  });

  it('runs only with the new engine on and a channel to report to', () => {
    expect(installUiFontCensus({ nativeEngine: { enabledSync: () => false }, engineFonts: { reportUiFonts: async () => true } })).toBeNull();
    expect(installUiFontCensus({ nativeEngine: { enabledSync: () => true }, engineFonts: {} })).toBeNull();
    const stop = installUiFontCensus({ nativeEngine: { enabledSync: () => true }, engineFonts: { reportUiFonts: async () => true } });
    expect(typeof stop).toBe('function');
    stop!();
  });
});

void original;
