// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { makePM } from '../__tests__/make-pm';
import { ModalController } from '../../overlays/modal';
import type { ModalOptions } from '../../overlays/types';

const controllers: ModalController[] = [];
afterEach(() => {
  controllers.splice(0).forEach(controller => controller.closeAll());
  flushSync(); document.body.replaceChildren(); vi.restoreAllMocks();
});

function editor(type = 'shape') {
  const PM = makePM('core/util', 'core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'ui/controls', 'ui/shortcuts');
  PM.proj = PM.mkProject({ w: 640, h: 360, dur: 3 });
  const outer = PM.mkLayer('group'), group = PM.mkLayer('group'), first = PM.mkLayer(type), second = PM.mkLayer('shape');
  group.group = outer.id; first.group = group.id;
  PM.proj.layers = [outer, group, first, second]; PM.ProjectIndex.invalidate(); PM.touch();
  PM.selectLayers(first.id); PM.hist.clear();
  const scrim = document.createElement('div'); scrim.id = 'scrim'; document.body.append(scrim);
  const modal = new ModalController(PM); controllers.push(modal);
  PM.modal = (options: ModalOptions) => modal.open(options);
  const toast = vi.spyOn(PM, 'toast');
  const open = () => { PM.cmd('precompose'); flushSync(); };
  const confirm = () => {
    const button = [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find(item => item.textContent === 'Pre-compose');
    expect(button).toBeDefined(); button!.click(); flushSync();
  };
  const state = () => JSON.stringify({ project: PM.proj, selection: PM.sel, history: PM.hist.list() });
  return { PM, group, outer, first, second, open, confirm, state, toast };
}

describe('precompose dialog respects effective layer locks', () => {
  it.each(['direct', 'group', 'nested group', 'mixed selection'])('rejects %s protection at the shortcut entrypoint', kind => {
    const { PM, group, outer, first, second, open, state, toast } = editor();
    if (kind === 'direct') first.lock = true;
    if (kind === 'group') group.lock = true;
    if (kind === 'nested group') outer.lock = true;
    if (kind === 'mixed selection') { first.lock = true; PM.selectLayers([first.id, second.id]); PM.hist.clear(); }
    const before = state(); open();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(state()).toBe(before); expect(toast).toHaveBeenCalled();
    expect(String(toast.mock.calls.at(-1)?.[0]).toLowerCase()).toContain('locked');
  });

  it.each(['direct replacement', 'group', 'nested group'])('rechecks %s protection after the dialog has opened', kind => {
    const { PM, group, outer, first, open, confirm, state } = editor(); open();
    if (kind === 'direct replacement') {
      const index = PM.proj.layers.indexOf(first); PM.proj.layers[index] = { ...first, lock: true };
    }
    if (kind === 'group') group.lock = true;
    if (kind === 'nested group') outer.lock = true;
    PM.ProjectIndex.invalidate(); PM.touch();
    const before = state(); confirm();
    expect(state()).toBe(before);
    expect(document.querySelector('[role="dialog"]')?.textContent?.toLowerCase()).toContain('locked');
  });

  it('rejects a disappeared target instead of precomposing the rest of a stale selection', () => {
    const { PM, first, second, open, confirm, state } = editor();
    PM.selectLayers([first.id, second.id]); PM.hist.clear(); open();
    PM.proj.layers = PM.proj.layers.filter((layer: any) => layer.id !== first.id); PM.ProjectIndex.invalidate(); PM.touch();
    const before = state(); confirm();
    expect(state()).toBe(before); expect(document.querySelector('[role="dialog"]')).not.toBeNull();
    expect(document.querySelector('.modal-action-error')).not.toBeNull();
  });

  it('rejects a project switch even when the newly opened project contains the same layer id', () => {
    const { PM, first, open, confirm, state } = editor(); open();
    const replacement = PM.mkProject({ w: 640, h: 360, dur: 3 });
    replacement.layers = [{ ...first, group: null }]; PM.proj = replacement; PM.ProjectIndex.invalidate(); PM.touch();
    const before = state(); confirm();
    expect(state()).toBe(before); expect(document.querySelector('.modal-action-error')).not.toBeNull();
  });

  it('rejects a composition switch after another actor has moved the original target into a precomp', () => {
    const { PM, first, open, confirm, state } = editor(); open();
    PM.Comps.precompose([first.id], { open: true });
    expect(PM.L(first.id)).toBeTruthy();
    const before = state(); confirm();
    expect(state()).toBe(before); expect(document.querySelector('.modal-action-error')).not.toBeNull();
  });

  it('still precomposes editable layers in one Undo operation', () => {
    const { PM, first, open, confirm } = editor(); open(); confirm();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(PM.hist.list()).toEqual(['Pre-compose']);
    expect(PM.proj.layers.some((layer: any) => layer.type === 'precomp')).toBe(true);
    expect(PM.hist.undo()).toBe(true); expect(PM.L(first.id)).toBeTruthy();
    expect(PM.hist.undo()).toBe(false);
  });

  it('rechecks a stale lock when Leave all attributes is selected', () => {
    const { first, open, confirm, state } = editor('solid'); open();
    const leave = [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] [role="radio"]')].find(item => item.textContent === 'Keep on layer')!;
    expect(leave.disabled).toBe(false); leave.click(); flushSync();
    first.lock = true; const before = state(); confirm();
    expect(state()).toBe(before);
    expect(document.querySelector('.modal-action-error')?.textContent?.toLowerCase()).toContain('locked');
  });

  it('keeps editable Leave all attributes available with one Undo', () => {
    const { PM, first, open, confirm } = editor('solid'); open();
    const leave = [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] [role="radio"]')].find(item => item.textContent === 'Keep on layer')!;
    leave.click(); flushSync(); confirm();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(PM.L(first.id).type).toBe('precomp'); expect(PM.hist.list()).toEqual(['Pre-compose']);
    expect(PM.hist.undo()).toBe(true); expect(PM.L(first.id).type).toBe('solid');
  });

  it('keeps unsupported shared-appearance errors visible without changing selection or history', () => {
    const { PM, group, open, confirm, state } = editor();
    group.p.opacity.v = 50; group.p['position.x'].v = -900; PM.touch();
    open(); const before = state(); confirm();
    expect(state()).toBe(before);
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
    expect(document.querySelector('.modal-action-error')?.textContent).toContain('Select the whole styled group');
  });

});
