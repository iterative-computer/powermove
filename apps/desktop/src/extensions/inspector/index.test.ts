// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { InspectorService, PanelDefinition, PowermoveAPI, ServicesAPI, ShaderHooks } from 'powermove';

import activate from './index';

function serviceHarness(): { services: ServicesAPI; disposeAll(): void } {
  const implementations = new Map<string, unknown>();
  const disposers: Array<() => void> = [];
  return {
    services: {
      register<T>(name: string, implementation: T) {
        implementations.set(name, implementation);
        const dispose = (): void => {
          if (implementations.get(name) === implementation) implementations.delete(name);
        };
        disposers.push(dispose);
        return { dispose };
      },
      get<T>(name: string): T | null {
        return (implementations.get(name) as T | undefined) ?? null;
      }
    },
    disposeAll(): void {
      for (const dispose of disposers.reverse()) dispose();
    }
  };
}

describe('inspector extension', () => {
  it('registers its live inspector and shader services with automatic disposal', () => {
    let panel: PanelDefinition | undefined;
    const harness = serviceHarness();
    const shader: Record<string, any> = {
      id: 'shader-1', type: 'shader', d: { code: 'uniform float amount;', uniforms: {} }
    };
    const PM: Record<string, any> = {
      proj: { layers: [shader], comps: {} },
      parseUniforms: vi.fn(() => [{ name: 'amount', def: 0.5 }]),
      UIState: { setShaderMeta: vi.fn() },
      P: vi.fn((value: unknown) => ({ v: value, kf: [] }))
    };
    const api = {
      host: { pm: PM },
      services: harness.services,
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) },
      log: vi.fn()
    } as unknown as PowermoveAPI;

    activate(api);

    expect(panel).toMatchObject({ id: 'inspector', title: 'Properties' });
    expect(harness.services.get<InspectorService>('inspector')).toBe(PM.Inspector);
    const shaderHooks = harness.services.get<ShaderHooks>('shaderHooks');
    expect(shaderHooks?.syncShaderUniforms).toBe(PM.syncShaderUniforms);
    expect(shader.d.uniforms.amount).toEqual({ v: 0.5, kf: [] });
    expect(PM.UIState.setShaderMeta).toHaveBeenCalledWith(shader, {
      udefs: [{ name: 'amount', def: 0.5 }]
    });

    harness.disposeAll();
    expect(harness.services.get('inspector')).toBeNull();
    expect(harness.services.get('shaderHooks')).toBeNull();
  });
});
