// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import UIPlacementGhost from './UIPlacementGhost.svelte';
import { agentState, resetAgentState, setAgentSnapshot } from './agent-state.svelte';
import type { UIPlacement } from './ui-placement';

// The mount/fallback integration is tested here; WebGPU is unavailable in happy-dom.
vi.mock('./GhostFieldCanvas.svelte', () => ({ default: (_anchor: unknown, props: any) => { queueMicrotask(() => props.onError({ message: 'WebGPU device unavailable' })); } }));

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
  it('never covers an existing panel when the agent announces a broad target', () => {
    const original = document.getElementById('panel-timeline');
    snapshot(panel);
    expect(document.querySelector('[data-ui-placement-ghost]')).toBeNull();
    expect(document.querySelector('[data-agent-editing]')).toBeNull();
    expect(document.getElementById('panel-timeline')).toBe(original);
    expect(original?.querySelector('button')?.textContent).toBe('Play');
  });

  it('highlights only a verified control without replacing, covering, or disabling it', () => {
    const original = document.getElementById('panel-timeline')!;
    original.innerHTML = '<section class="transport"><button>Play</button></section><section class="tracks">Tracks</section>';
    const button = original.querySelector('button')!;
    const click = vi.fn();
    button.addEventListener('click', click);
    snapshot({ ...panel, selector: '.transport button' });
    expect(button.getAttribute('data-agent-editing')).toBe(panel.label);
    expect(document.querySelector('[data-ui-placement-ghost]')).toBeNull();
    expect(original.querySelector('.tracks')?.hasAttribute('data-agent-editing')).toBe(false);
    button.click();
    expect(click).toHaveBeenCalledOnce();
    snapshot(panel, 'result');
    expect(button.hasAttribute('data-agent-editing')).toBe(false);
    expect(button.textContent).toBe('Play');
  });

  it.each(['[', '.missing', 'section'])('does not fall back to a panel for invalid, stale, or ambiguous targets: %s', selector => {
    document.getElementById('panel-timeline')!.innerHTML = '<section>A</section><section>B</section>';
    snapshot({ ...panel, selector, insert: 'after' });
    expect(document.querySelector('[data-ui-placement-ghost]')).toBeNull();
    expect(document.querySelector('[data-agent-editing]')).toBeNull();
  });

  it('reserves only the new section and follows its anchor without hiding existing controls', async () => {
    const original = document.getElementById('panel-timeline')!;
    original.innerHTML = '<section class="existing">Existing</section><section class="after">After</section>';
    snapshot({ ...panel, selector: '.existing', insert: 'after', label: 'Easing section' });
    let ghost = document.querySelector<HTMLElement>('[data-ui-placement-ghost="timeline"]')!;
    expect(ghost.textContent).toContain('Building Easing section');
    expect(ghost.previousElementSibling?.className).toBe('existing');
    expect(ghost.nextElementSibling?.className).toBe('after');
    expect(ghost.classList.contains('pinned')).toBe(false);
    original.querySelector('.existing')!.replaceWith(Object.assign(document.createElement('section'), { className: 'existing', textContent: 'Replacement' }));
    await vi.waitFor(() => {
      ghost = document.querySelector<HTMLElement>('[data-ui-placement-ghost="timeline"]')!;
      expect(ghost.previousElementSibling?.textContent).toBe('Replacement');
      expect(document.querySelectorAll('[data-ui-placement-ghost]')).toHaveLength(1);
    });
  });

  it('removes a stale highlight and follows a replaced control', async () => {
    const original = document.getElementById('panel-timeline')!;
    const old = original.querySelector('button')!;
    snapshot({ ...panel, selector: 'button' });
    const replacement = document.createElement('button');
    replacement.textContent = 'Play again';
    old.replaceWith(replacement);
    await vi.waitFor(() => expect(replacement.getAttribute('data-agent-editing')).toBe(panel.label));
    expect(old.hasAttribute('data-agent-editing')).toBe(false);
    replacement.remove();
    await vi.waitFor(() => expect(replacement.hasAttribute('data-agent-editing')).toBe(false));
    expect(document.querySelector('[data-ui-placement-ghost]')).toBeNull();
  });

  it.each(['result', 'conversation', 'idle'])('cleans up on terminal phase %s', phase => {
    snapshot({ ...panel, selector: 'button' });
    snapshot(panel, phase);
    expect(document.querySelector('[data-agent-editing]')).toBeNull();
    expect(agentState.uiPlacement).toBeNull();
  });

  it('clears on steering and leaves new panel slots to their dock', () => {
    snapshot({ ...panel, selector: 'button' });
    snapshot(null, 'working', 2);
    expect(document.querySelector('[data-agent-editing]')).toBeNull();
    snapshot({ kind: 'dock', id: 'center', label: 'New controls', beforePanelId: null });
    expect(document.querySelector('[data-ui-placement-ghost]')).toBeNull();
  });

  it('keeps a transparent field host mounted and falls back to the static CSS edge without WebGPU', () => {
    snapshot({ ...panel, selector: 'button', insert: 'after' });
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
    snapshot({ ...panel, selector: 'button', insert: 'after' });
    const host = document.querySelector<HTMLElement>('.ghost-edge-field')!;
    expect(host.dataset.ghostRenderer).toBe('static');
    expect(host.dataset.ghostFallback).toBe('reduced-motion');
    expect(host.querySelector('canvas')).toBeNull();
  });

  it('loads the Motion GPU field on demand and keeps the static edge when the device fails', async () => {
    stubWebGPU();
    vi.stubGlobal('matchMedia', vi.fn(() => motionPreference(false)));
    const warn = vi.spyOn(window.console, 'warn').mockImplementation(() => {});

    snapshot({ ...panel, selector: 'button', insert: 'after' });
    const host = document.querySelector<HTMLElement>('.ghost-edge-field')!;
    // The WebGPU module is fetched only once a run has claimed a panel.
    expect(host.dataset.ghostRenderer).toBe('motion-gpu-initializing');

    // happy-dom has no WebGPU device behind navigator.gpu, so the runtime
    // reports the failure and the ghost keeps its edge instead of a dead hole.
    // The test component reports the same device error through its public callback.
    await vi.waitFor(() => expect(host.dataset.ghostRenderer).toBe('static'), { timeout: 5000 });
    expect(host.dataset.ghostFallback).toBe('gpu-error');
    expect(host.querySelector('canvas')).toBeNull();
    expect(warn).toHaveBeenCalled();

    snapshot(panel, 'result');
    expect(document.querySelector('.ghost-edge-field')).toBeNull();
  });
});
