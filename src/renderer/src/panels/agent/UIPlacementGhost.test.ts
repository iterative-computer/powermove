// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import UIPlacementGhost from './UIPlacementGhost.svelte';
import { agentState, resetAgentState, setAgentSnapshot } from './agent-state.svelte';
import type { UIPlacement } from './ui-placement';

let component: ReturnType<typeof mount> | undefined;
const disconnect = vi.fn();
const panel: UIPlacement = { kind: 'panel', id: 'timeline', label: 'Timeline controls' };

function motionPreference(reduced: boolean) {
  return {
    matches: reduced,
    media: '(prefers-reduced-motion: reduce)',
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn()
  } as unknown as MediaQueryList;
}

/** happy-dom's navigator uses private fields, so the flag is defined in place. */
function stubWebGPU() {
  Object.defineProperty(navigator, 'gpu', { value: {}, configurable: true });
}

function snapshot(placement: UIPlacement | null, phase = 'working', token = 1) {
  flushSync(() => setAgentSnapshot({ ...agentState, legacyPhase: phase, requestToken: token, uiPlacement: placement }));
}

beforeEach(() => {
  resetAgentState();
  disconnect.mockClear();
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect = disconnect; });
  document.body.innerHTML = '<main id="body"><div class="dock" id="dock-center"><div class="panel" id="panel-timeline"><button>Play</button></div></div></main>';
  for (const id of ['dock-center', 'panel-timeline']) {
    vi.spyOn(document.getElementById(id)!, 'getBoundingClientRect').mockReturnValue(
      { left: 100, top: 150, width: 600, height: 240, right: 700, bottom: 390, x: 100, y: 150, toJSON() {} }
    );
  }
  component = mount(UIPlacementGhost, { target: document.getElementById('body')! });
  flushSync();
});

afterEach(async () => {
  if (component) await unmount(component);
  resetAgentState();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  Reflect.deleteProperty(navigator, 'gpu');
  document.body.replaceChildren();
});

describe('UI placement ghost', () => {
  it('appears only after a location arrives, keeping the original panel mounted', () => {
    const original = document.getElementById('panel-timeline');
    expect(document.querySelector('[role="status"]')).toBeNull();
    snapshot(panel);
    const ghost = document.querySelector<HTMLElement>('[data-ui-placement-ghost="timeline"]')!;
    expect(ghost.textContent).toContain('Updating Timeline controls');
    expect(ghost.parentElement).toBe(original);
    expect(ghost.classList.contains('pinned')).toBe(true);
    expect(document.getElementById('panel-timeline')).toBe(original);
    expect(original?.querySelector('button')?.textContent).toBe('Play');
  });

  it('covers only a valid scoped target and restores temporary positioning on cleanup', () => {
    const original = document.getElementById('panel-timeline')!;
    original.innerHTML = '<section class="transport"><button>Play</button></section><section class="tracks"></section>';
    const transport = original.querySelector<HTMLElement>('.transport')!;
    vi.spyOn(window, 'getComputedStyle').mockReturnValue({ position: 'static' } as CSSStyleDeclaration);
    snapshot({ ...panel, selector: '.transport' });
    const ghost = document.querySelector<HTMLElement>('[data-ui-placement-ghost="timeline"]')!;
    expect(ghost.parentElement).toBe(transport);
    expect(transport.style.position).toBe('relative');
    expect(ghost.style.pointerEvents).toBe('');

    snapshot(panel, 'result');
    expect(transport.style.position).toBe('');
    expect(original.querySelector('button')?.textContent).toBe('Play');
  });

  it('reserves a local section, follows a replaced target, and falls back for an invalid selector', () => {
    const original = document.getElementById('panel-timeline')!;
    original.innerHTML = '<section class="existing">Existing</section><section class="after">After</section>';
    snapshot({ ...panel, selector: '.existing', insert: 'after', label: 'Easing section' });
    let ghost = document.querySelector<HTMLElement>('[data-ui-placement-ghost="timeline"]')!;
    expect(ghost.textContent).toContain('Building Easing section');
    expect(ghost.previousElementSibling?.className).toBe('existing');
    expect(ghost.nextElementSibling?.className).toBe('after');
    expect(ghost.classList.contains('section')).toBe(true);

    original.querySelector('.existing')!.replaceWith(Object.assign(document.createElement('section'), { className: 'existing', textContent: 'Replacement' }));
    flushSync();
    ghost = document.querySelector<HTMLElement>('[data-ui-placement-ghost="timeline"]')!;
    expect(ghost.previousElementSibling?.textContent).toBe('Replacement');
    expect(document.querySelectorAll('[data-ui-placement-ghost="timeline"]')).toHaveLength(1);

    snapshot({ ...panel, selector: '[', label: 'Fallback' });
    ghost = document.querySelector<HTMLElement>('[data-ui-placement-ghost="timeline"]')!;
    expect(ghost.parentElement).toBe(original);
  });

  it('does not retarget its own broad selector and repairs a moved insertion placeholder', async () => {
    const original = document.getElementById('panel-timeline')!;
    original.innerHTML = '<section class="section">Anchor</section><div class="destination"></div>';
    snapshot({ ...panel, selector: '.section', insert: 'before', label: 'Local section' });
    let ghost = document.querySelector<HTMLElement>('[data-ui-placement-ghost="timeline"]')!;
    const anchor = original.querySelector<HTMLElement>('section.section')!;
    expect(ghost.nextElementSibling).toBe(anchor);

    original.querySelector('.destination')!.append(ghost);
    flushSync();
    await vi.waitFor(() => {
      ghost = document.querySelector<HTMLElement>('[data-ui-placement-ghost="timeline"]')!;
      expect(ghost.nextElementSibling).toBe(anchor);
    });
    expect(ghost.parentElement).toBe(original);
    expect(document.querySelectorAll('[data-ui-placement-ghost="timeline"]')).toHaveLength(1);
  });

  it('does not overwrite a positioning change made while the scoped ghost is active', () => {
    const original = document.getElementById('panel-timeline')!;
    original.innerHTML = '<section class="transport"></section>';
    const transport = original.querySelector<HTMLElement>('.transport')!;
    vi.spyOn(window, 'getComputedStyle').mockReturnValue({ position: 'static' } as CSSStyleDeclaration);
    snapshot({ ...panel, selector: '.transport' });
    transport.style.position = 'sticky';
    snapshot(panel, 'result');
    expect(transport.style.position).toBe('sticky');
  });

  it('settles on the whole-panel fallback when an insertion selector is missing', async () => {
    const original = document.getElementById('panel-timeline')!;
    snapshot({ ...panel, selector: '.not-here', insert: 'after', label: 'Missing anchor' });
    const ghost = document.querySelector<HTMLElement>('[data-ui-placement-ghost="timeline"]')!;
    expect(ghost.parentElement).toBe(original);
    original.append(document.createElement('div'));
    await vi.waitFor(() => {
      expect(document.querySelector('[data-ui-placement-ghost="timeline"]')).toBe(ghost);
      expect(document.querySelectorAll('[data-ui-placement-ghost="timeline"]')).toHaveLength(1);
    });
  });

  it.each(['result', 'conversation', 'idle'])('cleans up on terminal phase %s even with an older placement snapshot', phase => {
    snapshot(panel);
    snapshot(panel, phase);
    expect(document.querySelector('[role="status"]')).toBeNull();
    expect(agentState.uiPlacement).toBeNull();
    expect(disconnect).toHaveBeenCalled();
  });

  it('clears on steering and ignores unavailable targets', () => {
    snapshot(panel);
    snapshot(null, 'working', 2);
    expect(document.querySelector('[role="status"]')).toBeNull();
    snapshot({ ...panel, id: 'not-mounted' });
    expect(document.querySelector('[role="status"]')).toBeNull();
  });

  it('leaves a new panel\'s ghost to the dock that reserves its slot', () => {
    snapshot({ kind: 'dock', id: 'center', label: 'New controls', beforePanelId: null });
    expect(document.querySelector('[role="status"]')).toBeNull();
  });

  it('keeps a transparent field host mounted and falls back to the static CSS edge without WebGPU', () => {
    snapshot(panel);
    const host = document.querySelector<HTMLElement>('.ghost-edge-field')!;
    expect(host.getAttribute('aria-hidden')).toBe('true');
    expect(host.dataset.ghostRenderer).toBe('static');
    expect(host.dataset.ghostFallback).toBe('webgpu-unavailable');
    expect(host.querySelector('canvas')).toBeNull();

    snapshot(panel, 'result');
    expect(document.querySelector('.ghost-edge-field')).toBeNull();
  });

  it('uses the static edge without reaching for the GPU when reduced motion is preferred', () => {
    stubWebGPU();
    vi.stubGlobal('matchMedia', vi.fn(() => motionPreference(true)));
    snapshot(panel);
    const host = document.querySelector<HTMLElement>('.ghost-edge-field')!;
    expect(host.dataset.ghostRenderer).toBe('static');
    expect(host.dataset.ghostFallback).toBe('reduced-motion');
    expect(host.querySelector('canvas')).toBeNull();
  });

  it('loads the Motion GPU field on demand and keeps the static edge when the device fails', async () => {
    stubWebGPU();
    vi.stubGlobal('matchMedia', vi.fn(() => motionPreference(false)));
    const warn = vi.spyOn(window.console, 'warn').mockImplementation(() => {});

    snapshot(panel);
    const host = document.querySelector<HTMLElement>('.ghost-edge-field')!;
    // The WebGPU module is fetched only once a run has claimed a panel.
    expect(host.dataset.ghostRenderer).toBe('motion-gpu-initializing');

    // happy-dom has no WebGPU device behind navigator.gpu, so the runtime
    // reports the failure and the ghost keeps its edge instead of a dead hole.
    // The first fetch of the WebGPU module is transformed on demand here.
    await vi.waitFor(() => expect(host.dataset.ghostRenderer).toBe('static'), { timeout: 5000 });
    expect(host.dataset.ghostFallback).toBe('gpu-error');
    expect(host.querySelector('canvas')).toBeNull();
    expect(warn).toHaveBeenCalled();

    snapshot(panel, 'result');
    expect(document.querySelector('.ghost-edge-field')).toBeNull();
  });
});
