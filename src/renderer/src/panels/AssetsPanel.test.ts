// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';

import { doc } from '../state/document.svelte';
import AssetsPanel from './AssetsPanel.svelte';

interface AssetFixture {
  id: string;
  name: string;
  kind: string;
  w?: number;
  h?: number;
  dur?: number;
  size?: number;
}

const IMAGE: AssetFixture = {
  id: 'image-1',
  name: 'Backdrop.png',
  kind: 'image',
  w: 1920,
  h: 1080,
  dur: 65.4,
  size: 1.5 * 1024 * 1024
};
const AUDIO: AssetFixture = {
  id: 'audio-1',
  name: 'Theme.wav',
  kind: 'audio',
  dur: 30,
  size: 900
};

let target: HTMLDivElement;
let instance: Record<string, any> | undefined;

function domHelper(tag: string, attrs: Record<string, any> | string | null, ...children: unknown[]): HTMLElement {
  const element = document.createElement(tag.split(/[.#]/)[0] || 'div');
  if (attrs && typeof attrs === 'object') {
    for (const [key, value] of Object.entries(attrs)) {
      if (key === 'style') Object.assign(element.style, value);
      else element.setAttribute(key, String(value));
    }
  } else if (typeof attrs === 'string') {
    children.unshift(attrs);
  }
  for (const child of children) element.append(child instanceof Node ? child : document.createTextNode(String(child)));
  return element;
}

function setup(
  fixtureAssets: AssetFixture[] = [IMAGE, AUDIO],
  options: { references?: Record<string, number>; removals?: Record<string, { removedLayers: number; removedLayerIds: string[] }> } = {}
) {
  const project = {
    id: 'project-1',
    assets: Object.fromEntries(fixtureAssets.map((asset) => [asset.id, { ...asset }])),
    layers: []
  };
  const events: string[] = [];
  const removeAsset = vi.fn((currentProject: typeof project, assetId: string) => {
    delete currentProject.assets[assetId as keyof typeof currentProject.assets];
    return options.removals?.[assetId] ?? { removedLayers: 0, removedLayerIds: [] };
  });
  const PM: Record<string, any> = {
    proj: project,
    ICONS: {
      plus: '<path data-test-icon="plus"></path>',
      trash: '<path data-test-icon="trash"></path>',
      frame: '<path data-test-icon="frame"></path>',
      cam: '<path data-test-icon="cam"></path>',
      clock: '<path data-test-icon="clock"></path>',
      project: '<path data-test-icon="project"></path>',
      missing: '<path data-test-icon="missing"></path>'
    },
    assets: new Map([['image-1', { url: 'blob:backdrop' }]]),
    pickFiles: vi.fn(),
    cmd: vi.fn(),
    hist: { do: vi.fn((_label: string, operation: () => unknown) => operation()) },
    MediaImport: {
      removeAsset,
      referenceCount: vi.fn((_project: unknown, assetId: string) => options.references?.[assetId] ?? 0)
    },
    sel: { layers: ['image-layer', 'keep-layer'] },
    bus: {
      emit: vi.fn((event: string) => {
        events.push(event);
        if (event === 'assets') doc.bump('assets');
      })
    },
    toast: vi.fn(),
    modal: vi.fn(),
    h: domHelper
  };

  (window as any).PM = PM;
  doc.replace(project as any);
  instance = mount(AssetsPanel, { target, props: { panelId: 'assets', spec: {} } });
  flushSync();
  return { PM, project, events, removeAsset };
}

function rows(): HTMLElement[] {
  return [...target.querySelectorAll<HTMLElement>('[role="option"]')];
}

beforeEach(() => {
  target = document.createElement('div');
  document.body.append(target);
});

afterEach(async () => {
  if (instance) await unmount(instance);
  instance = undefined;
  target.remove();
  vi.restoreAllMocks();
});

describe('AssetsPanel', () => {
  it('renders media previews and details with named controls and the legacy host class', () => {
    const { PM } = setup();

    expect(target.classList.contains('assets-panel-body')).toBe(true);
    expect(target.querySelector('[data-svelte-panel="assets"]')).not.toBeNull();
    expect(target.querySelector('[role="listbox"]')?.getAttribute('aria-label')).toBe('Project media');
    expect(rows()).toHaveLength(2);
    expect(rows()[0]!.textContent).toContain('Backdrop.png');
    expect(rows()[0]!.textContent).toContain('1920×1080 · 1:05 · 1.5 MB');
    expect(rows()[1]!.textContent).toContain('Theme.wav');
    expect(rows()[1]!.textContent).toContain('0:30 · 1 KB');
    expect(target.querySelector<HTMLImageElement>('.asset-preview.image img')?.src).toBe('blob:backdrop');
    const icon = target.querySelector<SVGElement>('svg.pm-icon')!;
    expect(icon.style.fill).toBe('currentColor');
    expect(icon.style.stroke).toBe('none');
    expect(icon.dataset.icon).toBe('plus');

    const importButton = [...target.querySelectorAll('button')].find((button) => button.textContent?.trim() === 'Import');
    expect(importButton?.title).toBe('Import media (⌘I)');
    expect(target.querySelector('button[aria-label="Add Backdrop.png to timeline"]')).not.toBeNull();
    expect(target.querySelector('button[aria-label="Delete Backdrop.png"]')).not.toBeNull();

    importButton?.click();
    expect(PM.pickFiles).toHaveBeenCalledOnce();
  });

  it('adds media from its button and double-click', () => {
    const { PM } = setup();

    target.querySelector<HTMLButtonElement>('button[aria-label="Add Backdrop.png to timeline"]')?.click();
    rows()[1]!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true, detail: 2 }));

    expect(PM.cmd).toHaveBeenNthCalledWith(1, 'addFromAsset', 'image-1');
    expect(PM.cmd).toHaveBeenNthCalledWith(2, 'addFromAsset', 'audio-1');
  });

  it('uses roving tabindex and supports arrows, Home, End, Space, and Enter', () => {
    const { PM } = setup();
    let options = rows();

    expect(options.map((row) => row.tabIndex)).toEqual([0, -1]);
    expect(options.map((row) => row.getAttribute('aria-selected'))).toEqual(['false', 'false']);
    options[0]!.focus();
    options[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    flushSync();
    options = rows();
    expect(options.map((row) => row.tabIndex)).toEqual([-1, 0]);
    expect(options[1]!.getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(options[1]);

    options[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
    flushSync();
    options = rows();
    expect(document.activeElement).toBe(options[0]);
    expect(options[0]!.getAttribute('aria-selected')).toBe('true');

    options[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }));
    flushSync();
    options = rows();
    expect(document.activeElement).toBe(options[1]);

    options[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }));
    options[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(PM.cmd).toHaveBeenCalledWith('addFromAsset', 'audio-1');

    options[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true }));
    flushSync();
    expect(document.activeElement).toBe(rows()[0]);
  });

  it('deletes unreferenced media through history and preserves the legacy cleanup events', () => {
    const { PM, events, removeAsset } = setup();

    target.querySelector<HTMLButtonElement>('button[aria-label="Delete Backdrop.png"]')?.click();
    flushSync();

    expect(PM.MediaImport.referenceCount).toHaveBeenCalledWith(PM.proj, 'image-1');
    expect(PM.hist.do).toHaveBeenCalledWith('Delete media', expect.any(Function));
    expect(removeAsset).toHaveBeenCalledWith(PM.proj, 'image-1');
    expect(PM.sel.layers).toEqual(['image-layer', 'keep-layer']);
    expect(events).toEqual(['assets', 'layers', 'sel', 'project']);
    expect(PM.toast).toHaveBeenCalledWith('Deleted Backdrop.png');
    expect(target.querySelector('[data-asset-id="image-1"]')).toBeNull();
  });

  it('confirms referenced deletion and removes affected layer selections after approval', () => {
    const { PM, events } = setup([IMAGE], {
      references: { 'image-1': 1 },
      removals: { 'image-1': { removedLayers: 1, removedLayerIds: ['image-layer'] } }
    });

    rows()[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true }));
    expect(PM.hist.do).not.toHaveBeenCalled();
    expect(PM.modal).toHaveBeenCalledOnce();
    const modal = PM.modal.mock.calls[0][0];
    expect(modal).toMatchObject({ title: 'Delete “Backdrop.png”?', width: 420 });
    expect(modal.body.textContent).toContain('This also removes 1 layer that uses this media. You can undo this.');

    modal.actions[1].run();
    flushSync();
    expect(PM.sel.layers).toEqual(['keep-layer']);
    expect(events).toEqual(['assets', 'layers', 'sel', 'project']);
    expect(PM.toast).toHaveBeenCalledWith('Deleted Backdrop.png and 1 layer');
  });

  it('refreshes only after the assets tick changes', () => {
    const { project } = setup([IMAGE]);
    const next = { ...AUDIO };

    project.assets[next.id as keyof typeof project.assets] = next as any;
    flushSync();
    expect(rows()).toHaveLength(1);

    doc.bump('assets');
    flushSync();
    expect(rows()).toHaveLength(2);
    expect(target.textContent).toContain('Theme.wav');

    delete project.assets[IMAGE.id as keyof typeof project.assets];
    doc.bump('assets');
    flushSync();
    expect(rows()).toHaveLength(1);
    expect(rows()[0]!.tabIndex).toBe(0);
  });
});
