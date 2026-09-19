// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';
import type { PMRegistry } from '../renderer/src/legacy/registry';

describe('document engine chain', () => {
  it('boots the editing chain plus the agent tool harness without a browser', async () => {
    const requests: Array<(request: any) => void> = [];
    const responses: any[] = [];
    (window as any).powermove = { agentTools: { onRequest: (cb: any) => { requests.push(cb); return () => {}; }, respond: (r: any) => responses.push(r) } };
    const PM: PMRegistry = ((window as any).PM = (window as any).PM || {});
    const mods = await Promise.all([
      import('../renderer/src/legacy/core/diag'), import('../renderer/src/legacy/core/util'), import('../renderer/src/kernel/install'),
      import('../renderer/src/legacy/core/ui-state'), import('../renderer/src/legacy/core/memory'), import('../renderer/src/legacy/core/fonts'),
      import('../renderer/src/legacy/core/easing'), import('../renderer/src/legacy/core/model'), import('../renderer/src/legacy/core/selection'),
      import('../renderer/src/legacy/core/anim'), import('../renderer/src/legacy/core/history'), import('../renderer/src/legacy/core/library'),
      import('../renderer/src/legacy/core/projects'), import('../renderer/src/legacy/core/editing'), import('../renderer/src/legacy/core/capabilities'),
      import('../renderer/src/legacy/core/workspace'), import('../renderer/src/legacy/assistant/harness')
    ]);
    const names = ['diag','util','kernel','ui-state','memory','fonts','easing','model','selection','anim','history','library','projects','editing','capabilities','workspace','harness'];
    for (let i = 0; i < mods.length; i++) {
      const m: any = mods[i];
      const step = names[i] === 'kernel' ? (PM: any) => void m.installKernel(PM) : m.install;
      try { step(PM); } catch (error) { throw new Error(`${names[i]}: ${(error as Error).stack}`); }
    }
    expect(requests).toHaveLength(1);
    PM.proj = (PM as any).mkProject({ name: 'Engine' });
    PM.proj.id = 'engine-1';
    const ask = async (tool: string, args: any) => {
      requests[0]!({ runId: 'run-1', callId: `c-${responses.length}`, tool, arguments: args, baseRevision: PM.proj.revision || 0 });
      for (let i = 0; i < 200 && responses.length === 0; i++) await new Promise((r) => setTimeout(r, 5));
      return responses.shift();
    };
    const state = await ask('get_project_state', {});
    const applied = await ask('apply_commands', { commands: [{ type: 'add_layer', layerType: 'shape', name: 'From engine' }], label: 'engine add' });
    expect(applied?.ok).toBe(true);
    expect(PM.proj.layers.map((l: any) => l.name)).toEqual(['From engine']);
    const finish = await ask('__finish_run', { commit: true });
    expect(finish?.ok).toBe(true);
    expect(finish?.changed).toBe(true);
    expect(state?.ok).toBe(true);
  });
});
