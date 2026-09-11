// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readPanel, interactPanel, openPanel } from './panel-tools';
const PM: any = { PANELS: { footage: { title: 'Footage' }, agent: { title: 'Agent' } } };
beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, 'getClientRects').mockReturnValue([{}] as any);
  document.body.innerHTML = `<div id="panel-footage"><label>Search<input type="search"></label><input type="password" value="secret"><button>Search footage</button><button disabled>Import</button><select aria-label="Media"><option value="video">Videos</option><option value="photo">Photos</option></select><p>Ready</p></div>`;
});
afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ''; });
const ref = (state: any, name: string) => state.controls.find((c: any) => c.label === name).ref;
describe('agent panel controls', () => {
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
