import { Vector3 } from 'three';
import { describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../legacy/registry';
import { applyViewportPose, setViewportMode } from '../core/scene3d/viewport';
import { compositionHas3D, engineCanvasViewParams, watchEngineCanvasView } from './canvas-view';

vi.mock('../core/scene3d/service', async () => {
  const { PerspectiveCamera } = await import('three');
  return { compositionRuntime: () => ({ camera: new PerspectiveCamera(50, 16 / 9, 0.1, 100) }) };
});

function project() {
  const listeners = new Map<string, Set<() => void>>();
  const PM = {
    proj: { id: 'P', layers: [{ id: 'cube', d: { definition: 'powermove.3d.object' } }] },
    time: 0,
    active: () => false,
    invalidate: vi.fn(),
    bus: {
      on(event: string, fn: () => void) {
        const set = listeners.get(event) ?? new Set<() => void>();
        set.add(fn);
        listeners.set(event, set);
        return () => set.delete(fn);
      }
    }
  } as unknown as PMRegistry;
  return { PM, emit: (event: string) => { for (const fn of listeners.get(event) ?? []) fn(); } };
}

describe('engine canvas view', () => {
  it('carries the editor camera only in the Editor view, as the built-in viewer does', () => {
    const { PM } = project();
    expect(compositionHas3D(PM)).toBe(true);
    expect(engineCanvasViewParams(PM)).toEqual({});
    setViewportMode(PM, 'editor');
    expect(engineCanvasViewParams(PM).camera?.projection.kind).toBe('perspective');
    setViewportMode(PM, 'camera');
    expect(engineCanvasViewParams(PM)).toEqual({});
  });

  it('coalesces view changes into one refresh and stops after disposal', async () => {
    const { PM, emit } = project();
    const refresh = vi.fn();
    const off = watchEngineCanvasView(PM, refresh);
    setViewportMode(PM, 'editor');
    applyViewportPose(PM, { position: new Vector3(0, 0, 5), target: new Vector3(), zoom: 1 });
    emit('composition');
    await Promise.resolve();
    expect(refresh).toHaveBeenCalledTimes(1);
    off();
    emit('composition');
    await Promise.resolve();
    expect(refresh).toHaveBeenCalledTimes(1);
  });
});
