// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import {
  applyResolution, createPreviewButton, kernelMenuItems, loadMediaPrefs, resolutionChoice, saveMediaPrefs, shownLabel, toggledMedia,
  viewerMenuItems, type PopoverItem
} from './media-menu';
import { viewParamsFor } from './engine-present/view-params';

const transport = (auto: boolean, quality: number) => ({ perf: { auto }, quality, previewResolution: null as string | number | null });
const rows = (items: PopoverItem[]) => items.map((item) => item === '-' ? '-' : 'heading' in item ? `# ${item.heading}` : `${item.label}${item.check ? ' [check]' : ''}${item.checked ? ' ✓' : ''}`);

describe('preview resolution', () => {
  it('reads and sets the transport as the select did', () => {
    expect(resolutionChoice(transport(true, 0.5))).toBe('auto');
    expect(resolutionChoice(transport(false, 1))).toBe('1');
    expect(resolutionChoice(transport(false, 0.5))).toBe('0.5');
    expect(resolutionChoice(transport(false, 0.25))).toBe('0.25');
    const t = transport(false, 0.25);
    applyResolution(t, 'auto');
    expect(t).toEqual({ perf: { auto: true }, quality: 1, previewResolution: 'auto' });
    applyResolution(t, '0.5');
    expect(t).toEqual({ perf: { auto: false }, quality: 0.5, previewResolution: '0.5' });
  });
});

describe('the button says what is on screen', () => {
  it('names the frame’s resolved level and its media', () => {
    expect(shownLabel({ resolution: 'half', representation: 'proxy', draft3d: false, cached: false }, 1)).toBe('Half · Proxy');
    expect(shownLabel({ resolution: 'full', representation: 'original', draft3d: false, cached: false }, 0.5)).toBe('Full');
    expect(shownLabel({ resolution: 'quarter', representation: 'reducedResolution', draft3d: false, cached: true }, 1)).toBe('Quarter');
    expect(shownLabel({ resolution: 'automatic', representation: 'optimizedMedia', draft3d: false, cached: false }, 1)).toBe('Auto · Optimized');
  });
  it('falls back to the current engine’s preview quality', () => {
    expect(shownLabel(null, 1)).toBe('Full');
    expect(shownLabel(null, 0.5)).toBe('Half');
    expect(shownLabel(undefined, 0.25)).toBe('Quarter');
  });
  it('draws as the select trigger and keeps an accessible name', () => {
    const { button, setLabel } = createPreviewButton('width:auto');
    setLabel('Half · Proxy');
    expect(button.className).toBe('pm-select');
    expect(button.getAttribute('aria-label')).toBe('Preview quality');
    expect(button.getAttribute('aria-haspopup')).toBe('menu');
    expect(button.textContent).toBe('Half · Proxy');
    expect(button.querySelector('.pm-select-chevron')).not.toBeNull();
  });
});

describe('the viewer menu', () => {
  const act = { resolution: vi.fn(), media: vi.fn(), playEveryFrame: vi.fn() };
  it('has one radio group per section and Play Every Frame as a check item', () => {
    const items = viewerMenuItems({ resolution: '0.5', media: 'proxies', playEveryFrame: true, engine: true }, act);
    expect(rows(items)).toEqual([
      '# Resolution', 'Automatic', 'Full', 'Half ✓', 'Quarter',
      '# Media', 'Automatic', 'Originals', 'Proxies ✓',
      '-', 'Play Every Frame [check] ✓'
    ]);
    const pick = (label: string, from = 0) => {
      const item = items.slice(from).find((entry) => entry !== '-' && !('heading' in entry) && entry.label === label);
      (item as { run(): void }).run();
    };
    pick('Automatic');
    expect(act.resolution).toHaveBeenCalledWith('auto');
    pick('Automatic', 6);
    expect(act.media).toHaveBeenCalledWith('automatic');
    pick('Play Every Frame');
    expect(act.playEveryFrame).toHaveBeenCalledWith(false);
  });
  it('shows only Resolution with the current engine', () => {
    expect(rows(viewerMenuItems({ resolution: 'auto', media: 'proxies', playEveryFrame: true, engine: false }, act)))
      .toEqual(['# Resolution', 'Automatic ✓', 'Full', 'Half', 'Quarter']);
  });
  it('maps to the kernel menu’s headers and check marks', () => {
    const items = viewerMenuItems({ resolution: '1', media: 'automatic', playEveryFrame: false, engine: true }, act);
    const kernel = kernelMenuItems(items);
    expect(kernel[0]).toEqual({ header: 'Resolution' });
    expect(kernel[2]).toMatchObject({ label: 'Full', on: true });
    expect(kernel.at(-1)).toMatchObject({ label: 'Play Every Frame', on: false });
  });
});

describe('media preference', () => {
  it('toggles Originals and Proxies', () => {
    expect(toggledMedia('automatic')).toBe('proxies');
    expect(toggledMedia('proxies')).toBe('originals');
    expect(toggledMedia('originals')).toBe('proxies');
  });
  it('is kept in the viewer’s storage, defaults left out', () => {
    const store = new Map<string, unknown>();
    const storage = { get: <T,>(k: string) => store.get(k) as T | undefined, set: (k: string, v: unknown) => void store.set(k, v), delete: (k: string) => void store.delete(k) };
    expect(loadMediaPrefs(storage)).toEqual({ media: 'automatic', playEveryFrame: false });
    saveMediaPrefs(storage, { media: 'proxies', playEveryFrame: true });
    expect(loadMediaPrefs(storage)).toEqual({ media: 'proxies', playEveryFrame: true });
    saveMediaPrefs(storage, { media: 'automatic', playEveryFrame: false });
    expect(store.size).toBe(0);
    store.set('media', 'bogus');
    expect(loadMediaPrefs(storage).media).toBe('automatic');
  });
  it('goes to the engine in the viewer state', () => {
    const base = { shown: 1, fit: true, pan: [0, 0] as const, previewResolution: 'auto' };
    expect(viewParamsFor(base)).toMatchObject({ media: 'automatic', playEveryFrame: false });
    expect(viewParamsFor({ ...base, media: 'proxies', playEveryFrame: true })).toMatchObject({ media: 'proxies', playEveryFrame: true, resolution: 'automatic' });
  });
});
