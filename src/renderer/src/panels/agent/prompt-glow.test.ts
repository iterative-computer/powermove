// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { onDestroy } from 'svelte';
import { mountPromptGlow } from './prompt-glow';

type GlowProps = { dpr: number; onError: (report: { message: string }) => void; onFirstFrame: () => void };

let host: HTMLElement;
let props: GlowProps | null;
let mounted: number;
const listeners = new Set<() => void>();
let reducedMotion = false;

/** Stands in for PromptGlowCanvas so the contract is tested without a GPU. */
function fakeGlow(anchor: Node, incoming: GlowProps) {
  props = incoming;
  mounted += 1;
  const canvas = document.createElement('canvas');
  anchor.parentNode?.insertBefore(canvas, anchor);
  onDestroy(() => canvas.remove());
}

const load = vi.fn(async () => ({ default: fakeGlow }));

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

describe('prompt run glow mount', () => {
  it('reports the live renderer and caps the halo below Retina density', async () => {
    vi.spyOn(window, 'devicePixelRatio', 'get').mockReturnValue(3);
    const destroy = mountPromptGlow(host, load);
    expect(host.dataset.glowRenderer).toBe('motion-gpu-initializing');

    await vi.waitFor(() => expect(mounted).toBe(1));
    expect(props?.dpr).toBe(1.5);

    props?.onFirstFrame();
    expect(host.dataset.glowRenderer).toBe('motion-gpu');
    expect(host.dataset.glowFallback).toBeUndefined();

    // A finished run takes the halo down with it, so it costs no more frames.
    destroy();
    expect(host.querySelector('canvas')).toBeNull();
  });

  it('leaves the static ring in place without a GPU or under reduced motion', () => {
    Reflect.deleteProperty(navigator, 'gpu');
    mountPromptGlow(host, load)();
    expect(host.dataset.glowRenderer).toBe('static');
    expect(host.dataset.glowFallback).toBe('webgpu-unavailable');

    Object.defineProperty(navigator, 'gpu', { value: {}, configurable: true });
    reducedMotion = true;
    mountPromptGlow(host, load)();
    expect(host.dataset.glowFallback).toBe('reduced-motion');
    expect(load).not.toHaveBeenCalled();
  });

  it('falls back to the static ring when the halo reports a GPU failure', async () => {
    const warn = vi.spyOn(window.console, 'warn').mockImplementation(() => {});
    mountPromptGlow(host, load);
    await vi.waitFor(() => expect(mounted).toBe(1));

    props?.onFirstFrame();
    props?.onError({ message: 'device lost' });
    expect(host.dataset.glowRenderer).toBe('static');
    expect(host.dataset.glowFallback).toBe('gpu-error');
    expect(host.querySelector('canvas')).toBeNull();
    expect(warn).toHaveBeenCalled();
  });
});
