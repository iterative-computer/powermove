// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { makePM } from '../__tests__/make-pm';
import { ModalController } from '../../overlays/modal';
import { selectionBounds } from '../core/selection-bounds';
import type { ModalOptions } from '../../overlays/types';
import { precomposeDialog } from './compositions';

const controllers: ModalController[] = [];
afterEach(() => {
  controllers.splice(0).forEach(controller => controller.closeAll());
  flushSync(); document.body.replaceChildren(); vi.restoreAllMocks();
});

function editor() {
  const PM = makePM('core/util', 'core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'ui/controls', 'gl/compositor');
  PM.proj = PM.mkProject({ w: 640, h: 360, fps: 30, dur: 6 }); PM.time = 2;
  const a = PM.mkLayer('shape', { from: 1, dur: 3, d: { w: 100, h: 40, stroke: 0 }, p: { 'position.x': 150, 'position.y': 120, rotation: 30 } });
  const b = PM.mkLayer('shape', { from: 2, dur: 3, d: { w: 60, h: 60, stroke: 0 }, p: { 'position.x': 300, 'position.y': 200 } });
  const audio = PM.mkLayer('audio');
  PM.proj.layers = [a, b, audio]; PM.ProjectIndex.invalidate(); PM.touch(); PM.hist.clear();
  const scrim = document.createElement('div'); scrim.id = 'scrim'; document.body.append(scrim);
  const modal = new ModalController(PM); controllers.push(modal);
  PM.modal = (options: ModalOptions) => modal.open(options);
  const sizeRow = () => document.querySelector<HTMLElement>('[data-precompose-size]')!;
  const choose = (row: HTMLElement, label: 'On' | 'Off') => {
    [...row.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === label)!.click(); flushSync();
  };
  const durationRow = () => [...document.querySelectorAll<HTMLElement>('.new-project-field')].find(row => row.textContent?.startsWith('Fit duration'))!;
  const summary = () => document.querySelector('.precompose-summary')!.textContent;
  const confirm = () => {
    [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find(item => item.textContent === 'Pre-compose')!.click();
    flushSync();
  };
  return { PM, a, b, audio, sizeRow, durationRow, choose, summary, confirm };
}

it('previews the fitted size and crops the new composition without moving anything', () => {
  const app = editor(), { PM, a, b } = app;
  const bounds = selectionBounds(PM, [a, b], 2);
  PM.beginEval(2);
  const before = [a, b].map(layer => [...PM.worldMatrix(layer, 2)]);
  precomposeDialog(PM, [a.id, b.id]); flushSync();
  expect(app.sizeRow().textContent).toContain('Fit size to layers');
  expect(app.summary()).toBe('New composition: 640 × 360 px, 6 s');
  app.choose(app.sizeRow(), 'On');
  expect(app.summary()).toBe(`New composition: ${bounds.w} × ${bounds.h} px, 6 s`);
  app.choose(app.durationRow(), 'On');
  expect(app.summary()).toBe(`New composition: ${bounds.w} × ${bounds.h} px, 4 s`);
  app.confirm();
  const outer = PM.proj.layers.find((layer: any) => layer.type === 'precomp');
  const comp = PM.proj.comps[outer.d.comp];
  expect(comp).toMatchObject({ w: bounds.w, h: bounds.h, dur: 4 });
  PM.beginEval(2);
  [a, b].forEach((layer, i) => {
    PM.scope.push(comp);
    const inner = [...PM.worldMatrix(layer, 2 - outer.from)];
    PM.scope.pop();
    PM.mul(PM.worldMatrix(outer, 2), inner).forEach((value: number, k: number) => expect(value).toBeCloseTo(before[i]![k]!, 6));
  });
  expect(PM.hist.list()).toHaveLength(1);
});

it('leaves the size alone unless asked, and dims the option when it cannot apply', () => {
  const app = editor(), { PM, a, audio } = app;
  precomposeDialog(PM, [a.id]); flushSync(); app.confirm();
  const outer = PM.proj.layers.find((layer: any) => layer.type === 'precomp');
  expect(PM.proj.comps[outer.d.comp]).toMatchObject({ w: 640, h: 360 });

  precomposeDialog(PM, [audio.id]); flushSync();
  expect(app.sizeRow().classList.contains('is-dim')).toBe(true);
  expect([...app.sizeRow().querySelectorAll('button')].every(button => (button as HTMLButtonElement).disabled)).toBe(true);
  expect(app.sizeRow().title).toContain('visual layer');
});
