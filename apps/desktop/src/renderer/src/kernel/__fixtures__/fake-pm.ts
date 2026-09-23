import { vi } from 'vitest';

export type LegacyPM = Record<string, any>;

/* The legacy PM registry, as much of it as installKernel reads. */
export function fakePM(): LegacyPM {
  const listeners = new Map<string, Set<(...args: any[]) => void>>();
  const store: Record<string, unknown> = {};
  const workspace = { id: 'design', panels: ['viewer'] };
  const PM: LegacyPM = {
    proj: { id: 'p', revision: 7, assets: {} },
    sel: { layers: ['L1'], keys: ['k1', 4], chan: 'position.x' },
    time: 12,
    playing: false,
    PANELS: { viewer: {} },
    bus: {
      on: (event: string, fn: (...args: any[]) => void) => {
        const set = listeners.get(event) ?? new Set();
        set.add(fn);
        listeners.set(event, set);
        return () => void set.delete(fn);
      },
      emit: (event: string, ...args: any[]) => listeners.get(event)?.forEach((fn) => fn(...args))
    },
    Edit: { apply: vi.fn(() => ({ ok: true, message: 'ok', data: {} })) },
    hist: { undo: vi.fn(), redo: vi.fn() },
    Export: { snapshot: vi.fn(async () => 'data:image/jpeg;base64,x') },
    assets: {
      add: vi.fn(async (file: File, options: any) => {
        const asset = { id: 'asset-1', name: file.name, kind: 'model', sourceText: await file.text() };
        PM.proj.assets[asset.id] = { id: asset.id, name: asset.name, kind: asset.kind, layerDefinition: options?.layerDefinition };
        return asset;
      }),
      get: vi.fn(() => null)
    },
    selectLayers: vi.fn(),
    setTime: vi.fn(),
    play: vi.fn(),
    pause: vi.fn(),
    toast: vi.fn(),
    menu: vi.fn(),
    icon: (name: string) => `<svg data-icon="${name}"></svg>`,
    modal: vi.fn((opts: any) => ({ close: vi.fn(), body: document.createElement('div'), opts })),
    palette: vi.fn(),
    cmd: vi.fn(),
    store: {
      get: (key: string, fallback: unknown) => store[key] ?? fallback,
      set: (key: string, value: unknown) => void (store[key] = value)
    },
    WS: { current: workspace, mutate: vi.fn((fn: (w: unknown) => void) => fn(workspace)) },
    Layout: { addPanel: vi.fn(), removePanel: vi.fn(), hasPanel: vi.fn(() => false), refresh: vi.fn() },
    listeners,
    store_: store
  };
  return PM;
}
