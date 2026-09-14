import { cp } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

test('the shipped effect sample validates, registers, animates and undoes without creating a panel', async ({ session }) => {
  await cp(path.resolve('docs/samples/gradient-tint'), path.join(session.userData, 'extensions/gradient-tint'), { recursive: true });
  await session.relaunch();
  await session.openEditor();
  await session.page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.effects?.get('gradient-tint')));

  const proof = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({
      runId: 'effect-authoring-proof', callId: tool, tool, arguments: args, baseRevision: PM.proj.revision || 0
    });
    const definition = PM.Kernel.effects.get('gradient-tint');
    const valid = await call('validate_effect', { definition });
    let rejected = '';
    try {
      await call('validate_effect', { definition: { ...definition,
        params: Array.from({ length: 36 }, (_, i) => ({ k: `p${i}`, label: `P${i}`, def: 0, min: 0, max: 1 }))
      } });
    } catch (error) { rejected = String(error); }
    const workspace = JSON.parse((await call('get_workspace_state')).content[0].text);
    const apply = (commands: any) => {
      const result = PM.Edit.apply(commands, { label: 'Effect authoring proof', origin: 'agent' });
      if (!result.ok) throw new Error(result.message);
    };
    apply({ type: 'add_layer', id: 'effect-proof-layer', layerType: 'shape', name: 'Effect proof', duration: 5 });
    apply({ type: 'add_effect', target: 'effect-proof-layer', effect: 'gradient-tint' });
    const layer = PM.L('effect-proof-layer');
    const effect = layer.fx[0];
    const prop = PM.allProps(layer).find((item: any) => item.key === `${effect.id}.amount`);
    if (!prop) throw new Error('The effect has no editable Amount channel');
    apply({ type: 'replace_keyframes', target: layer.id, path: prop.key,
      keyframes: [{ time: 0, value: 0 }, { time: 2, value: 1 }]
    });
    const untinted = PM.Export.snapshot(0, 320);
    const middle = PM.Export.snapshot(1, 320);
    const tinted = PM.Export.snapshot(2, 320);
    const keys = effect.p.amount.kf.length;
    PM.hist.undo();
    const keysAfterUndo = PM.L(layer.id).fx[0].p.amount.kf.length;
    PM.hist.undo();
    return {
      valid: JSON.parse(valid.content[0].text), rejected,
      registered: workspace.registeredEffects.find((item: any) => item.id === 'gradient-tint'),
      panels: PM.Kernel.panels.ownerEntries('gradient-tint').map((entry: any) => entry.id),
      changedPixels: untinted !== middle && middle !== tinted && untinted !== tinted,
      keys, keysAfterUndo, effectsAfterUndo: PM.L(layer.id).fx.length,
      errors: [...PM.GL.errors.values()]
    };
  });
  expect(proof.valid).toMatchObject({ valid: true, id: 'gradient-tint', parameterCount: 4 });
  expect(proof.rejected).toContain('too many params (36 > 32)');
  expect(proof.registered).toMatchObject({ label: 'Gradient Tint', group: 'Stylize', parameters: ['startColor', 'endColor', 'angle', 'amount'] });
  expect(proof.panels).toEqual([]);
  expect(proof.changedPixels).toBe(true);
  expect(proof.keys).toBe(2);
  expect(proof.keysAfterUndo).toBe(0);
  expect(proof.effectsAfterUndo).toBe(0);
  expect(proof.errors).toEqual([]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
