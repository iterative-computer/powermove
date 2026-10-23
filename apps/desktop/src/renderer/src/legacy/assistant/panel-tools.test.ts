// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readPanel, interactPanel, openPanel, panelTarget, assertPanelTarget } from './panel-tools';
import { Registry } from '../../kernel/registry';
const PM: any = { PANELS: { footage: { title: 'Footage' }, agent: { title: 'Agent' } } };
beforeEach(() => {
  delete PM.Kernel;
  PM.PANELS.footage = { title: 'Footage' };
  vi.spyOn(HTMLElement.prototype, 'getClientRects').mockReturnValue([{}] as any);
  document.body.innerHTML = `<div id="panel-footage"><label>Search<input type="search"></label><input type="password" value="secret"><button>Search footage</button><button disabled>Import</button><select aria-label="Media"><option value="video">Videos</option><option value="photo">Photos</option></select><p>Ready</p></div>`;
});
afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ''; });
const ref = (state: any, name: string) => state.controls.find((c: any) => c.label === name).ref;
describe('agent panel controls', () => {
  it('uses registration generations when the same definition is re-enabled in the live registry', () => {
    const panels = new Registry<any>(), definition = { id: 'footage', title: 'Footage' };
    PM.Kernel = { panels };
    PM.PANELS.footage = definition;
    const first = panels.register('extension', definition);
    const target = panelTarget(PM, 'footage');
    first.dispose();
    panels.register('extension', definition);
    expect(() => assertPanelTarget(PM, 'footage', target)).toThrow('unavailable');
    expect(panelTarget(PM, 'footage')).not.toBe(target);
  });

  it('invalidates a coordinate target when extension changes move its panel', () => {
    const root = document.getElementById('panel-footage')!;
    const bounds = vi.spyOn(root, 'getBoundingClientRect').mockReturnValue({ left: 0, top: 0, right: 100, bottom: 100 } as DOMRect);
    const target = panelTarget(PM, 'footage');
    bounds.mockReturnValue({ left: 100, top: 0, right: 200, bottom: 100 } as DOMRect);
    expect(() => assertPanelTarget(PM, 'footage', target)).toThrow('unavailable');
  });

  it('rejects old refs and capture targets after disable/re-enable, even with the same DOM root', async () => {
    for (let toggle = 0; toggle < 3; toggle++) {
      const state = readPanel(PM, 'footage'), target = panelTarget(PM, 'footage');
      delete PM.PANELS.footage;
      expect(() => assertPanelTarget(PM, 'footage', target)).toThrow();
      PM.PANELS.footage = { title: 'Footage' };
      expect(() => assertPanelTarget(PM, 'footage', target)).toThrow('unavailable');
      await expect(interactPanel(PM, { panelId: 'footage', ref: ref(state, 'Search'), action: 'fill', value: 'stale' })).rejects.toThrow('stale');
      const current = readPanel(PM, 'footage');
      await interactPanel(PM, { panelId: 'footage', ref: ref(current, 'Search'), action: 'fill', value: `current-${toggle}` });
    }
    expect(document.querySelector('input')!.value).toBe('current-2');
  });

  it('stops an in-flight mount after its extension is disabled instead of retrying the layout', async () => {
    const ws = { layout: { docks: [{ panels: [{ id: 'footage', collapsed: false }] }] } };
    PM.WS = { current: ws, mutate: (fn: any) => fn(ws) };
    PM.Layout = { apply: vi.fn() };
    document.body.innerHTML = '';
    const opening = openPanel(PM, 'footage');
    delete PM.PANELS.footage;
    await expect(opening).rejects.toThrow('unavailable');
    expect(PM.Layout.apply).not.toHaveBeenCalled();
  });

  it('cancels a pending mount without applying a late layout', async () => {
    PM.WS = { current: { layout: { docks: [{ panels: [{ id: 'footage', collapsed: false }] }] } } };
    PM.Layout = { apply: vi.fn() };
    document.body.innerHTML = '';
    const controller = new AbortController();
    const opening = openPanel(PM, 'footage', controller.signal);
    controller.abort();
    await expect(opening).rejects.toThrow();
    expect(PM.Layout.apply).not.toHaveBeenCalled();
  });

  it('reports a completed action when the panel disappears during its response delay', async () => {
    const state = readPanel(PM, 'footage');
    const click = vi.fn(() => { delete PM.PANELS.footage; document.body.innerHTML = ''; });
    document.querySelector('button')!.addEventListener('click', click);
    const result = await interactPanel(PM, { panelId: 'footage', ref: ref(state, 'Search footage'), action: 'click' });
    expect(result).toMatchObject({ actionSent: true, panelChanged: true });
    expect(result.note).toContain('Do not repeat');
    expect(click).toHaveBeenCalledOnce();
  });

  it('does not deliver the rest of a gesture to a replaced panel', async () => {
    const state = readPanel(PM, 'footage'), click = vi.fn();
    const button = document.querySelector('button')!;
    button.addEventListener('pointerdown', () => { PM.PANELS.footage = { title: 'Replacement' }; });
    button.addEventListener('click', click);
    await expect(interactPanel(PM, { panelId: 'footage', ref: ref(state, 'Search footage'), action: 'click' })).rejects.toThrow('do not repeat');
    expect(click).not.toHaveBeenCalled();
  });

  it('reads, fills, selects and clicks the actual panel controls, without exposing passwords', async () => {
    const state = readPanel(PM, 'footage');
    expect(JSON.stringify(state)).not.toContain('secret');
    expect(state.controls).toHaveLength(4);
    const input = document.querySelector('input')!;
    const inputListener = vi.fn(); input.addEventListener('input', inputListener);
    let next = await interactPanel(PM, { panelId: 'footage', ref: ref(state, 'Search'), action: 'fill', value: 'cinema' });
    expect(input.value).toBe('cinema'); expect(inputListener).toHaveBeenCalledOnce();
    next = await interactPanel(PM, { panelId: 'footage', ref: ref(next, 'Media'), action: 'select', value: 'photo' });
    expect(document.querySelector('select')!.value).toBe('photo');
    const events: string[] = [];
    const button = document.querySelector('button')!;
    for (const event of ['pointerdown', 'pointerup', 'click']) button.addEventListener(event, () => events.push(event));
    await interactPanel(PM, { panelId: 'footage', ref: ref(next, 'Search footage'), action: 'click' });
    expect(events).toEqual(['pointerdown', 'pointerup', 'click']);
  });
  it('rejects stale, replaced, hidden and disabled controls without invoking the action hook', async () => {
    let state = readPanel(PM, 'footage'); const old = ref(state, 'Search');
    state = readPanel(PM, 'footage');
    await expect(interactPanel(PM, { panelId: 'footage', ref: old, action: 'fill', value: 'x' })).rejects.toThrow('stale');
    const hook = vi.fn();
    await expect(interactPanel(PM, { panelId: 'footage', ref: ref(state, 'Import'), action: 'click' }, hook)).rejects.toThrow('disabled');
    const buttonRef = ref(state, 'Search footage');
    document.querySelector('button')!.outerHTML = '<button>Search footage</button>';
    await expect(interactPanel(PM, { panelId: 'footage', ref: buttonRef, action: 'click' }, hook)).rejects.toThrow('stale');
    expect(hook).not.toHaveBeenCalled();
    document.getElementById('panel-footage')!.hidden = true;
    expect(readPanel(PM, 'footage').controls).toHaveLength(0);
  });
  it('restores hidden panels in place and excludes agent permission controls', async () => {
    const ws = { layout: { docks: [{ panels: [{ id: 'footage', collapsed: true }] }] } };
    PM.WS = { current: ws, mutate: (fn: any) => fn(ws) };
    PM.Layout = { hasPanel: () => false, restorePanel: vi.fn(() => true), addPanel: vi.fn(), refresh: vi.fn() };
    await openPanel(PM, 'footage');
    expect(PM.Layout.addPanel).not.toHaveBeenCalled();
    expect(ws.layout.docks[0]!.panels[0]!.collapsed).toBe(false);
    await expect(interactPanel(PM, { panelId: 'agent', action: 'click' })).rejects.toThrow('permission');
  });
});
