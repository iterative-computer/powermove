// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import AgentAtmosphere from './AgentAtmosphere.svelte';

const { mountField, destroyField } = vi.hoisted(() => ({ mountField: vi.fn(), destroyField: vi.fn() }));
vi.mock('./gpu-field', () => ({ mountGpuField: mountField }));

let target: HTMLDivElement;
let instance: ReturnType<typeof mount> | undefined;
let intersection: IntersectionObserverCallback;
let hidden = false;
const disconnect = vi.fn();

function onScreen(value: boolean) {
  intersection([{ isIntersecting: value } as IntersectionObserverEntry], {} as IntersectionObserver);
  flushSync();
}

beforeEach(() => {
  vi.clearAllMocks();
  mountField.mockReturnValue(destroyField);
  hidden = false;
  vi.spyOn(document, 'hidden', 'get').mockImplementation(() => hidden);
  vi.stubGlobal('IntersectionObserver', class {
    constructor(callback: IntersectionObserverCallback) { intersection = callback; }
    observe() {}
    disconnect = disconnect;
  });
  target = document.createElement('div');
  document.body.appendChild(target);
});

afterEach(async () => {
  if (instance) await unmount(instance);
  instance = undefined;
  target.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function render(active = false, compact = false) {
  instance = mount(AgentAtmosphere, { target, props: { active, compact } });
  flushSync();
}

describe('agent atmosphere resource ownership', () => {
  it('keeps idle content static and outside the accessibility tree', () => {
    render();
    onScreen(true);
    expect(mountField).not.toHaveBeenCalled();
    expect(target.querySelector('[data-agent-atmosphere]')?.getAttribute('aria-hidden')).toBe('true');
    expect(target.querySelector('button, input, [tabindex]')).toBeNull();
  });

  it('allocates only when visible and releases the field when scrolled away', () => {
    render(true);
    expect(mountField).not.toHaveBeenCalled();
    onScreen(true);
    expect(mountField).toHaveBeenCalledTimes(1);
    expect(mountField.mock.calls[0]?.[2].maxDevicePixelRatio).toBe(1);
    onScreen(false);
    expect(destroyField).toHaveBeenCalledTimes(1);
    expect(target.querySelector('.atmosphere-field')).toBeNull();
    onScreen(true);
    expect(mountField).toHaveBeenCalledTimes(2);
  });

  it('releases GPU work while the document is hidden', () => {
    render(true, true);
    onScreen(true);
    hidden = true;
    document.dispatchEvent(new Event('visibilitychange'));
    flushSync();
    expect(destroyField).toHaveBeenCalledTimes(1);
    hidden = false;
    document.dispatchEvent(new Event('visibilitychange'));
    flushSync();
    expect(mountField).toHaveBeenCalledTimes(2);
    expect(target.querySelector('.compact')).not.toBeNull();
  });

  it('cleans up the GPU field and observer when its owner unmounts', async () => {
    render(true);
    onScreen(true);
    await unmount(instance!);
    instance = undefined;
    expect(destroyField).toHaveBeenCalledTimes(1);
    expect(disconnect).toHaveBeenCalledTimes(1);
    document.dispatchEvent(new Event('visibilitychange'));
    flushSync();
    expect(mountField).toHaveBeenCalledTimes(1);
  });
});
