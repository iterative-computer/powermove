// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { onDestroy } from 'svelte';
import { mountGhostEdgeField } from './ui-placement-ghost-field';

type FieldProps = { dpr: number; onError: (report: { message: string }) => void; onFirstFrame: () => void };

let host: HTMLElement;
let props: FieldProps | null;
let mounted: number;
const listeners = new Set<() => void>();
let reducedMotion = false;

/** Stands in for GhostFieldCanvas so the contract is tested without a GPU. */
function fakeField(anchor: Node, incoming: FieldProps) {
  props = incoming;
  mounted += 1;
  const canvas = document.createElement('canvas');
  anchor.parentNode?.insertBefore(canvas, anchor);
  onDestroy(() => canvas.remove());
}

const load = vi.fn(async () => ({ default: fakeField }));

beforeEach(() => {
  props = null;
  mounted = 0;
  listeners.clear();
  reducedMotion = false;
  load.mockClear();
  host = document.createElement('div');
  document.body.append(host);
  Object.defineProperty(navigator, 'gpu', { value: {}, configurable: true });
  vi.stubGlobal('matchMedia', vi.fn(() => ({
    get matches() { return reducedMotion; },
    addEventListener: (_: string, listener: () => void) => listeners.add(listener),
    removeEventListener: (_: string, listener: () => void) => listeners.delete(listener)
  })));
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  Reflect.deleteProperty(navigator, 'gpu');
  document.body.replaceChildren();
});

describe('ghost generation field mount', () => {
  it('reports the live renderer and caps the field at half resolution', async () => {
    vi.spyOn(window, 'devicePixelRatio', 'get').mockReturnValue(3);
    const destroy = mountGhostEdgeField(host, load);
    expect(host.dataset.ghostRenderer).toBe('motion-gpu-initializing');

    await vi.waitFor(() => expect(mounted).toBe(1));
    expect(props?.dpr).toBe(1.5);
    expect(host.querySelector('canvas')).not.toBeNull();
    expect(host.dataset.ghostRenderer).toBe('motion-gpu-initializing');

    props?.onFirstFrame();
    expect(host.dataset.ghostRenderer).toBe('motion-gpu');
    expect(host.dataset.ghostFallback).toBeUndefined();

    destroy();
    expect(host.querySelector('canvas')).toBeNull();
  });

  it('falls back to the static edge when the field reports a GPU failure', async () => {
    const warn = vi.spyOn(window.console, 'warn').mockImplementation(() => {});
    mountGhostEdgeField(host, load);
    await vi.waitFor(() => expect(mounted).toBe(1));

    props?.onFirstFrame();
    props?.onError({ message: 'device lost' });
    expect(host.dataset.ghostRenderer).toBe('static');
    expect(host.dataset.ghostFallback).toBe('gpu-error');
    expect(host.querySelector('canvas')).toBeNull();
    expect(warn).toHaveBeenCalled();
  });

  it('never fetches the WebGPU module without a GPU or under reduced motion', async () => {
    Reflect.deleteProperty(navigator, 'gpu');
    mountGhostEdgeField(host, load)();
    expect(host.dataset.ghostRenderer).toBe('static');
    expect(host.dataset.ghostFallback).toBe('webgpu-unavailable');

    Object.defineProperty(navigator, 'gpu', { value: {}, configurable: true });
    reducedMotion = true;
    mountGhostEdgeField(host, load)();
    expect(host.dataset.ghostFallback).toBe('reduced-motion');
    expect(load).not.toHaveBeenCalled();
  });

  it('takes the field down and brings it back as the motion preference changes', async () => {
    const destroy = mountGhostEdgeField(host, load);
    await vi.waitFor(() => expect(mounted).toBe(1));

    reducedMotion = true;
    for (const listener of listeners) listener();
    expect(host.dataset.ghostRenderer).toBe('static');
    expect(host.dataset.ghostFallback).toBe('reduced-motion');
    expect(host.querySelector('canvas')).toBeNull();

    reducedMotion = false;
    for (const listener of listeners) listener();
    await vi.waitFor(() => expect(mounted).toBe(2));
    expect(host.dataset.ghostRenderer).toBe('motion-gpu-initializing');

    destroy();
    expect(listeners.size).toBe(0);
  });

  it('keeps the static edge when a late module load resolves after teardown', async () => {
    mountGhostEdgeField(host, load)();
    await vi.waitFor(() => expect(load).toHaveBeenCalled());
    expect(mounted).toBe(0);
    expect(host.querySelector('canvas')).toBeNull();
  });
});
