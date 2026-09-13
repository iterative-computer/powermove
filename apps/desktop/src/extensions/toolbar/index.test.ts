// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { PowermoveAPI, ServicesAPI, ToolService } from 'powermove';

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

describe('toolbar service', () => {
  it('owns tool state through the registered service', () => {
    const harness = serviceHarness();
    activate({
      services: harness.services,
      panels: { register: vi.fn() }
    } as unknown as PowermoveAPI);

    const toolService = harness.services.get<ToolService>('tool');
    expect(toolService?.tool).toBe('select');
    toolService!.tool = 'hand';
    expect(toolService?.tool).toBe('hand');
    toolService!.tool = 'zoom';
    expect(toolService?.tool).toBe('zoom');
    toolService!.toolShape = 'ellipse';
    expect(toolService?.toolShape).toBe('ellipse');
    toolService!.toolShape = 'star';
    expect(toolService?.toolShape).toBe('star');
    toolService?.setTool('shape', 'rect');
    expect(toolService).toMatchObject({ tool: 'shape', toolShape: 'rect' });

    harness.disposeAll();
    expect(harness.services.get('tool')).toBeNull();
  });
});
