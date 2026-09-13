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
      id: 'shader-1', type: 'shader', d: { code: 'uniform float amount; // @param 0.5', uniforms: {} }
    };
    const setShaderMeta = vi.fn();
    const api = {
      project: { get: () => ({ layers: [shader], comps: {} }) },
      model: { P: vi.fn((value: unknown) => ({ v: value, kf: [] })) },
      uiState: { setShaderMeta },
      services: harness.services,
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) },
      log: vi.fn()
    } as unknown as PowermoveAPI;

    activate(api);

    expect(panel).toMatchObject({ id: 'inspector', title: 'Properties' });
    const inspector = harness.services.get<InspectorService>('inspector');
    expect(inspector).not.toBeNull();
    const shaderHooks = harness.services.get<ShaderHooks>('shaderHooks');
    expect(shaderHooks?.syncShaderUniforms).toBe((inspector as InspectorService & ShaderHooks).syncShaderUniforms);
    expect(shader.d.uniforms.amount).toEqual({ v: 0.5, kf: [] });
    expect(setShaderMeta).toHaveBeenCalledWith(shader, {
      udefs: [expect.objectContaining({ name: 'amount', def: 0.5 })]
    });

    harness.disposeAll();
    expect(harness.services.get('inspector')).toBeNull();
    expect(harness.services.get('shaderHooks')).toBeNull();
  });
});
