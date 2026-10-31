// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { makePM } from '../__tests__/make-pm';
import { ModalController } from '../../overlays/modal';
import { confirm as nativeConfirm } from '../../overlays/confirm';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import type { ModalOptions } from '../../overlays/types';
import { compositionSettingsDialog, deleteCompositionPrompt, precomposeDialog } from './compositions';

const controllers: ModalController[] = [];
afterEach(() => {
  controllers.splice(0).forEach(controller => controller.closeAll());
  flushSync(); document.body.replaceChildren(); resetBridgeForTests(); vi.restoreAllMocks();
});

function editor() {
  const PM = makePM('core/util', 'core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'ui/controls');
  PM.proj = PM.mkProject({ name: 'Original project', w: 640, h: 360, dur: 3 });
  const target = PM.Comps.create({ name: 'Original nested composition' }, { open: false });
  const use = PM.Comps.addToTimeline(target); PM.hist.clear();
  const scrim = document.createElement('div'); scrim.id = 'scrim'; document.body.append(scrim);
  const modal = new ModalController(PM); controllers.push(modal);
  PM.modal = (options: ModalOptions) => modal.open(options);
  PM.confirm = (options: any) => nativeConfirm(PM, options);
  const toast = vi.spyOn(PM, 'toast');
  const state = () => JSON.stringify({ project: PM.proj, selection: PM.sel, history: PM.hist.list(), loading: PM.projectLoading ?? null });
  const confirm = () => {
    [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find(item => ['Apply', 'Create', 'Pre-compose'].includes(item.textContent ?? ''))!.click();
    flushSync();
  };
  const renameForm = () => { document.querySelector<HTMLInputElement>('[aria-label="Composition Name"]')!.value = 'Edited composition'; };
  const switchProject = (reuseTarget = false) => {
    const replacement = PM.mkProject({ name: 'Another project', w: 320, h: 180, dur: 3 });
    if (reuseTarget) {
      replacement.comps[target] = JSON.parse(JSON.stringify(PM.Comps.get(target)));
      replacement.layers = [PM.mkLayer('precomp', { d: { comp: target, w: 640, h: 360 } }, replacement)];
    }
    PM.replaceProject(replacement); PM.hist.clear();
  };
  return { PM, target, use, state, toast, confirm, renameForm, switchProject };
}

function deferredConfirmation() {
  let resolve!: (value: boolean) => void;
  const confirm = vi.fn(() => new Promise<boolean>(done => { resolve = done; }));
  installBridgeForTests({ confirm } as any);
  return { confirm, async finish(value: boolean) { resolve(value); await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); flushSync(); } };
}

describe('composition dialogs retain their project owner', () => {
  it.each([false, true])('rejects settings after a project switch (target id reused: %s)', reuseTarget => {
    const app = editor(); compositionSettingsDialog(app.PM, app.target); flushSync(); app.renameForm();
    app.switchProject(reuseTarget); const before = app.state(); app.confirm();
    expect(app.state()).toBe(before);
    expect(document.querySelector('.modal-action-error')?.textContent?.toLowerCase() ?? '').toContain('project');
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  });

  it('keeps an error visible when the settings target was removed while its dialog was open', () => {
    const app = editor(); compositionSettingsDialog(app.PM, app.target); flushSync(); app.renameForm();
    app.PM.Comps.remove(app.target); const before = app.state(); app.confirm();
    expect(app.state()).toBe(before); expect(document.querySelector('.modal-action-error')).not.toBeNull();
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  });

  it('still edits an inactive composition after switching tabs in the same project, with Undo', () => {
    const app = editor(), other = app.PM.Comps.create({ name: 'Another tab' }, { open: false });
    app.PM.hist.clear(); compositionSettingsDialog(app.PM, app.target); flushSync(); app.renameForm();
    app.PM.Comps.open(other); app.confirm();
    expect(app.PM.Comps.active()).toBe(other);
    expect(app.PM.Comps.get(app.target).name).toBe('Edited composition');
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(app.PM.hist.list()).toEqual(['Composition Settings']);
    expect(app.PM.hist.undo()).toBe(true); expect(app.PM.Comps.get(app.target).name).toBe('Original nested composition');
  });

  it('rejects New Composition after the project has changed', () => {
    const app = editor(); compositionSettingsDialog(app.PM); flushSync(); app.renameForm();
    app.switchProject(); const before = app.state(); app.confirm();
    expect(app.state()).toBe(before);
    expect(document.querySelector('.modal-action-error')?.textContent?.toLowerCase() ?? '').toContain('project');
  });

  it('keeps New Composition available after switching tabs in the same project', () => {
    const app = editor(); compositionSettingsDialog(app.PM); flushSync(); app.renameForm();
    app.PM.Comps.open(app.target); app.confirm();
    expect(app.PM.Comps.get(app.PM.Comps.active()).name).toBe('Edited composition');
    expect(app.PM.proj.w).toBe(640); expect(app.PM.hist.list()).toEqual(['New Composition']);
    expect(app.PM.hist.undo()).toBe(true);
  });

  it('does not delete a matching composition from a new project after native confirmation', async () => {
    const app = editor(), native = deferredConfirmation();
    deleteCompositionPrompt(app.PM, app.target); expect(native.confirm).toHaveBeenCalledOnce();
    app.switchProject(true); const before = app.state(); await native.finish(true);
    expect(app.state()).toBe(before); expect(app.toast).toHaveBeenCalled();
    expect(String(app.toast.mock.calls.at(-1)?.[0]).toLowerCase()).toContain('project');
  });

  it('reports when another actor already removed a confirmed deletion target', async () => {
    const app = editor(), native = deferredConfirmation(); deleteCompositionPrompt(app.PM, app.target);
    app.PM.Comps.remove(app.target); const before = app.state(); await native.finish(true);
    expect(app.state()).toBe(before); expect(app.toast).toHaveBeenCalled();
  });

  it('still deletes the requested composition after a same-project tab switch, with Undo', async () => {
    const app = editor(), native = deferredConfirmation(); deleteCompositionPrompt(app.PM, app.target);
    app.PM.Comps.open(app.target); await native.finish(true);
    expect(app.PM.Comps.exists(app.target)).toBe(false);
    expect(app.PM.hist.list()).toEqual(['Delete Composition']);
    expect(app.PM.hist.undo()).toBe(true); expect(app.PM.Comps.exists(app.target)).toBe(true);
  });

  it('reports when a confirmed target became the last remaining composition', async () => {
    const app = editor(), native = deferredConfirmation(), original = app.PM.Comps.active();
    deleteCompositionPrompt(app.PM, app.target); app.PM.Comps.remove(original);
    expect(app.PM.Comps.list()).toHaveLength(1);
    const before = app.state(); await native.finish(true);
    expect(app.state()).toBe(before); expect(app.toast).toHaveBeenCalled();
    expect(String(app.toast.mock.calls.at(-1)?.[0]).toLowerCase()).toContain('at least one composition');
  });

  it.each(['settings', 'new composition', 'precompose'])('rejects the stale %s dialog while the incoming project is still opening', kind => {
    const app = editor();
    if (kind === 'settings') { compositionSettingsDialog(app.PM, app.target); flushSync(); app.renameForm(); }
    else if (kind === 'new composition') { compositionSettingsDialog(app.PM); flushSync(); app.renameForm(); }
    else { precomposeDialog(app.PM, [app.use.id]); flushSync(); }
    // beginProjectLoading leaves the outgoing model visible until media/storage
    // preparation finishes. The existing modal is outside the inert #body.
    app.PM.projectLoading = { id: 'incoming-project', name: 'Incoming', generation: 1 };
    const before = app.state(); app.confirm();
    expect(app.state()).toBe(before);
    expect(document.querySelector('.modal-action-error')?.textContent?.toLowerCase() ?? '').toContain('project');
  });

  it('rejects a confirmed native deletion while the incoming project is still opening', async () => {
    const app = editor(), native = deferredConfirmation(); deleteCompositionPrompt(app.PM, app.target);
    app.PM.projectLoading = { id: 'incoming-project', name: 'Incoming', generation: 1 };
    const before = app.state(); await native.finish(true);
    expect(app.state()).toBe(before); expect(app.toast).toHaveBeenCalled();
    expect(String(app.toast.mock.calls.at(-1)?.[0]).toLowerCase()).toContain('project');
  });

  it('allows cancelling a native deletion after a project switch without warning', async () => {
    const app = editor(), native = deferredConfirmation(); deleteCompositionPrompt(app.PM, app.target);
    app.switchProject(true); const before = app.state(); await native.finish(false);
    expect(app.state()).toBe(before); expect(app.toast).not.toHaveBeenCalled();
  });
});
