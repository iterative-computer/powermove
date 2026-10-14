// @vitest-environment happy-dom
import { existsSync } from 'node:fs';
import path from 'node:path';
import { Storage } from 'happy-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';
import '../renderer/src/kernel/extensions.svelte';
import type { PMRegistry } from '../renderer/src/legacy/registry';
import { canonicalProjectHash } from '../shared/canonical-json';

// happy-dom gives modules http: URLs; tests run from apps/desktop.
const MODULE = path.resolve('src/renderer/public/engine/pm_wasm.wasm');

afterEach(() => { vi.unstubAllGlobals(); vi.unstubAllEnvs(); });

describe.skipIf(!existsSync(MODULE))('the headless document host on pm-wasm', () => {
  it('mirrors the document and answers 2D geometry with no renderer', async () => {
    vi.stubGlobal('localStorage', new Storage());
    vi.stubEnv('POWERMOVE_PM_WASM', MODULE);
    const lines: string[] = [];
    const requests: Array<(request: any) => void> = [];
    const responses: any[] = [];
    (window as any).powermove = { agentTools: { onRequest: (cb: any) => { requests.push(cb); return () => {}; }, respond: (r: any) => responses.push(r) } };
    const PM: PMRegistry = ((window as any).PM = {});
    const { installHeadlessEngineDoc } = await import('./main');
    const mods = await Promise.all([
      import('../renderer/src/legacy/core/diag'), import('../renderer/src/legacy/core/util'), import('../renderer/src/kernel/install'),
      import('../renderer/src/legacy/core/ui-state'), import('../renderer/src/legacy/core/memory'), import('../renderer/src/legacy/core/fonts'),
      import('../renderer/src/legacy/core/easing'), import('../renderer/src/legacy/core/model'), import('../renderer/src/legacy/core/selection'),
      import('../renderer/src/legacy/core/anim'), import('../renderer/src/legacy/core/history'),
    ]);
    const rest = await Promise.all([
      import('../renderer/src/legacy/core/library'), import('../renderer/src/legacy/core/projects'), import('../renderer/src/legacy/core/editing'),
      import('../renderer/src/legacy/core/capabilities'), import('../renderer/src/legacy/core/workspace'), import('../renderer/src/legacy/assistant/harness'),
    ]);
    for (const m of mods as any[]) (m.installKernel ? (P: any) => void m.installKernel(P) : m.install)(PM);
    const engine = await installHeadlessEngineDoc(PM, line => lines.push(line));
    for (const m of rest as any[]) m.install(PM);
    expect(engine).toEqual({ ready: true, mode: 'on' });
    expect(lines.some(line => line.includes('document mirror: pm-wasm'))).toBe(true);

    PM.proj = (PM as any).mkProject({ name: 'Engine', w: 640, h: 360 });
    PM.proj.id = 'engine-1';
    const ask = async (tool: string, args: any) => {
      requests[0]!({ runId: 'run-1', callId: `c-${responses.length}`, tool, arguments: args, baseRevision: PM.proj.revision || 0 });
      for (let i = 0; i < 200 && responses.length === 0; i++) await new Promise((r) => setTimeout(r, 5));
      return responses.shift();
    };
    const applied = await ask('apply_commands', { commands: [{ type: 'add_layer', layerType: 'solid', name: 'Plate' }], label: 'engine add' });
    expect(applied?.ok).toBe(true);
    const layer = PM.proj.layers[0];
    PM.EngineDoc.barrier.flush();
    expect(PM.EngineDoc.wasm.hash()).toBe(canonicalProjectHash(PM.proj));
    // No compositor here: bounds and picks come from pm-wasm.
    expect(PM.GL.headless).toBe(true);
    expect(PM.GL.bounds(layer, 0)).toMatchObject({ w: 640, h: 360 });
    expect(PM.GL.pick(10, 10, 0)).toBe(layer);
    const finish = await ask('__finish_run', { commit: true });
    expect(finish?.ok).toBe(true);
  });
});
