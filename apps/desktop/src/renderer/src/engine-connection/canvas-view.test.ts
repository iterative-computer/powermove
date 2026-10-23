import { describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../legacy/registry';
import { enterLocalView } from '../core/scene3d/viewport';
import { engineCanvasSceneState, engineCanvasViewParams, watchEngineCanvasView } from './canvas-view';

function pendingProject() {
  const api = vi.fn(() => { throw new Error('camera3d unavailable before document readiness'); });
  const listeners = new Map<string, Set<() => void>>();
  let resolveReady!: (ready: boolean) => void;
  const doc = {
    enabled: true,
    ready: false,
    whenReady: new Promise<boolean>(resolve => { resolveReady = resolve; }),
    onReady: vi.fn(() => () => {})
  };
  const PM = {
    proj: { id: 'P', layers: [{ id: 'cube', d: { definition: 'powermove.3d.object' } }] },
    EngineDoc: doc,
    Kernel: { api },
    bus: {
      on(event: string, fn: () => void) {
        const set = listeners.get(event) ?? new Set<() => void>();
        set.add(fn);
        listeners.set(event, set);
        return () => set.delete(fn);
      }
    }
  } as unknown as PMRegistry;
  const emit = (event: string) => { for (const fn of listeners.get(event) ?? []) fn(); };
  return { PM, doc, api, emit, resolveReady };
}

describe('engine canvas view startup', () => {
  it('waits for the document before reading 3D editor state, then carries Local View', () => {
    const { PM, doc, api } = pendingProject();
    expect(engineCanvasViewParams(PM)).not.toHaveProperty('camera');
    expect(engineCanvasSceneState(PM)).toMatchObject({ hasScene: true, engineCamera: null });
    expect(api).not.toHaveBeenCalled();

    doc.ready = true;
    enterLocalView(PM, ['cube']);
    expect(engineCanvasViewParams(PM)).toMatchObject({
      camera: null, shading: 'solid', xray: false, localView: ['cube']
    });
    expect(engineCanvasSceneState(PM)?.localView).toEqual(['cube']);
    expect(api).not.toHaveBeenCalled();
  });

  it('refreshes on document readiness and editor changes without subscribing to the full viewport snapshot', async () => {
    const { PM, doc, api, emit, resolveReady } = pendingProject();
    const updates: ReturnType<typeof engineCanvasViewParams>[] = [];
    const off = watchEngineCanvasView(PM, () => updates.push(engineCanvasViewParams(PM)));
    emit('selection');
    await Promise.resolve();
    expect(updates).toHaveLength(1);
    expect(updates[0]).not.toHaveProperty('camera');

    doc.ready = true;
    resolveReady(true);
    await Promise.resolve();
    await Promise.resolve();
    expect(updates.at(-1)).toMatchObject({ camera: null, shading: 'solid' });
    expect(api).not.toHaveBeenCalled();

    const count = updates.length;
    off();
    emit('selection');
    await Promise.resolve();
    expect(updates).toHaveLength(count);
  });
});
