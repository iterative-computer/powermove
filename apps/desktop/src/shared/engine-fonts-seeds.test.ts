import { describe, expect, it, vi } from 'vitest';
import { appUiFontCacheSeeds, effectiveFontSize, engineSeedsJson, FontSeedTracker, parseFontFamilies, parseUiFontCensus, seedsJson, uiFontSeed, type UiFontUse } from './engine-fonts-seeds';

const UI = ['sf pro text', '-apple-system', 'blinkmacsystemfont', 'helvetica neue', 'sans-serif'];
const use = (size: number, weight = 400, families = UI, italic = false): UiFontUse => ({ families, size, weight, italic });

describe('font cache seeds', () => {
  it('at startup and default density are the UI text entries at each scale factor, as pm-text computes them', () => {
    // pm-text blink::measure seed_tests pins the same values.
    expect(appUiFontCacheSeeds([2]).map(s => [s.size, s.specified])).toEqual([[24, 12], [26, 13]]);
    expect(appUiFontCacheSeeds([2]).every(s => s.family === 'system-ui' && s.weight === 400 && !s.italic)).toBe(true);
    expect(appUiFontCacheSeeds([1]).every(s => s.size === s.specified)).toBe(true);
    const both = appUiFontCacheSeeds([2, Number.NaN, 1, 2]);
    expect(both).toHaveLength(4);
    expect(both[0]!.size).toBe(24);
    expect(appUiFontCacheSeeds([1.1])[0]!.size).toBe(Math.fround(13.2));
    expect(appUiFontCacheSeeds([])).toEqual([]);
    expect(seedsJson(appUiFontCacheSeeds([2]))).toBe('{"seeds":[{"family":"system-ui","weight":400,"italic":false,"size":24,"specified":12},{"family":"system-ui","weight":400,"italic":false,"size":26,"specified":13}]}');
  });

  it('floor the effective size as Blink does, in float', () => {
    // pm-text compat::canvas pins the f32 steps: 1.06 measures at 1.05.
    expect(effectiveFontSize(1.06)).toBe(Math.fround(1.05));
    expect(effectiveFontSize(11.8675)).toBe(Math.fround(11.86));
  });

  it('come from family stacks that reach the system font, as Chromium resolves them', () => {
    expect(parseFontFamilies('"SF Pro Text", -apple-system, BlinkMacSystemFont, "Helvetica Neue", sans-serif')).toEqual(UI);
    expect(parseFontFamilies("'A, B', c")).toEqual(['a, b', 'c']);
    const none = () => false;
    // SF Pro Text is not installed: -apple-system draws the UI.
    expect(uiFontSeed(use(14, 500), 2, none)).toEqual({ family: 'system-ui', weight: 500, italic: false, size: 28, specified: 14 });
    // Installed, it draws the UI and creates no system font entry.
    expect(uiFontSeed(use(14), 2, (f) => f === 'sf pro text')).toBeNull();
    expect(uiFontSeed(use(12, 400, ['menlo', 'system-ui']), 2, (f) => f === 'menlo')).toBeNull();
    expect(uiFontSeed(use(12, 400, ['ui-monospace', 'system-ui']), 2, none)).toBeNull();
    expect(uiFontSeed(use(12, 400, ['missing', 'system-ui']), 1.1, none)!.size).toBe(Math.fround(13.2));
    expect(uiFontSeed(use(12, 400, ['system-ui']), 0, none)).toBeNull();
  });

  it('follow what each window laid out, in order, and announce changes', () => {
    const tracker = new FontSeedTracker();
    const changed = vi.fn();
    tracker.onChange(changed);
    // A comfy window on a 2x display: --fs-md is 14 px.
    tracker.report(1, { dpr: 2, uses: [use(12), use(14), use(15, 600)] });
    tracker.report(1, { dpr: 2, uses: [use(14)] });
    expect(changed).toHaveBeenCalledTimes(1);
    expect(tracker.seedsFor(1).map(s => [s.size, s.weight, s.specified])).toEqual([[24, 400, 12], [28, 400, 14], [30, 600, 15]]);
    // Moving to a 1x display keeps the 2x entries; a key already created
    // keeps its creator's optical size.
    tracker.report(1, { dpr: 1, uses: [use(12), use(24)] });
    expect(tracker.seedsFor(1).map(s => [s.size, s.specified])).toEqual([[24, 12], [28, 14], [30, 15], [12, 12]]);
    tracker.report(2, { dpr: 1, uses: [use(13)] });
    expect(tracker.seedsFor(2).map(s => s.size)).toEqual([13]);
    expect(tracker.allSeeds().map(s => s.size)).toEqual([24, 28, 30, 12, 13]);
    // The engines measure each window with its own seeds.
    const table = tracker.engineSeeds();
    expect(table.owners.map(o => [o.owner, o.seeds.map(s => s.size)])).toEqual([[1, [24, 28, 30, 12]], [2, [13]]]);
    expect(table.seeds).toEqual(tracker.allSeeds());
    expect(JSON.parse(engineSeedsJson(table, [{ session: 5, owner: 2 }])).sessions).toEqual([{ session: 5, owner: 2 }]);
    expect(JSON.parse(engineSeedsJson(table))).not.toHaveProperty('sessions');
    expect(tracker.seedsFor(9)).toEqual([]);
    // SF Pro Text gets installed: the UI no longer creates system entries.
    changed.mockClear();
    tracker.setInstalled((f) => f === 'sf pro text');
    expect(tracker.seedsFor(1)).toEqual([]);
    expect(changed).toHaveBeenCalledTimes(2);
    tracker.setInstalled(() => false);
    tracker.forget(1);
    expect(changed).toHaveBeenLastCalledWith(null);
    expect(tracker.allSeeds().map(s => s.size)).toEqual([13]);
  });

  it('take only well-formed census messages', () => {
    expect(parseUiFontCensus(JSON.stringify({ dpr: 2, uses: [use(12)] }))).toEqual({ dpr: 2, uses: [use(12)] });
    expect(parseUiFontCensus('{')).toBeNull();
    expect(parseUiFontCensus(JSON.stringify({ dpr: -1, uses: [] }))).toBeNull();
    expect(parseUiFontCensus(JSON.stringify({ dpr: 2, uses: [{ families: [1], size: 1, weight: 1, italic: false }] }))).toBeNull();
    expect(parseUiFontCensus(42)).toBeNull();
  });
});
