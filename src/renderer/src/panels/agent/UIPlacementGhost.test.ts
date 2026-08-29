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

function webGLContext() {
  const shader = {} as WebGLShader;
  const program = {} as WebGLProgram;
  const buffer = {} as WebGLBuffer;
  const uniform = {} as WebGLUniformLocation;
  return {
    VERTEX_SHADER: 0x8b31,
    FRAGMENT_SHADER: 0x8b30,
    COMPILE_STATUS: 0x8b81,
    LINK_STATUS: 0x8b82,
    ARRAY_BUFFER: 0x8892,
    STATIC_DRAW: 0x88e4,
    FLOAT: 0x1406,
    COLOR_BUFFER_BIT: 0x4000,
    TRIANGLES: 0x0004,
    createShader: vi.fn(() => shader),
    shaderSource: vi.fn(),
    compileShader: vi.fn(),
    getShaderParameter: vi.fn(() => true),
    getShaderInfoLog: vi.fn(() => ''),
    deleteShader: vi.fn(),
    createProgram: vi.fn(() => program),
    attachShader: vi.fn(),
    linkProgram: vi.fn(),
    getProgramParameter: vi.fn(() => true),
    getProgramInfoLog: vi.fn(() => ''),
    deleteProgram: vi.fn(),
    createBuffer: vi.fn(() => buffer),
    bindBuffer: vi.fn(),
    bufferData: vi.fn(),
    getAttribLocation: vi.fn(() => 0),
    getUniformLocation: vi.fn(() => uniform),
    enableVertexAttribArray: vi.fn(),
    vertexAttribPointer: vi.fn(),
    clearColor: vi.fn(),
    viewport: vi.fn(),
    clear: vi.fn(),
    useProgram: vi.fn(),
    uniform2f: vi.fn(),
    uniform1f: vi.fn(),
    drawArrays: vi.fn(),
    deleteBuffer: vi.fn()
  } as unknown as WebGL2RenderingContext;
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
  document.body.replaceChildren();
});

describe('UI placement ghost', () => {
  it('appears only after a location arrives, keeping the original panel mounted', () => {
    const original = document.getElementById('panel-timeline');
    expect(document.querySelector('[role="status"]')).toBeNull();
    snapshot(panel);
    const ghost = document.querySelector<HTMLElement>('[data-ui-placement-ghost="timeline"]')!;
    expect(ghost.textContent).toContain('Updating Timeline controls');
    expect(ghost.style.left).toBe('105px');
    expect(ghost.style.height).toBe('230px');
    expect(document.getElementById('panel-timeline')).toBe(original);
    expect(original?.querySelector('button')?.textContent).toBe('Play');
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

  it('uses a temporary dock placeholder without inserting a real panel', () => {
    snapshot({ kind: 'dock', id: 'center', label: 'New controls', beforePanelId: null });
    expect(document.querySelector('[role="status"]')?.textContent).toContain('Building New controls');
    expect(document.querySelectorAll('.panel')).toHaveLength(1);
    expect(document.querySelector<HTMLElement>('[role="status"]')?.style.height).toBe('148px');
  });

  it('keeps a transparent canvas mounted and falls back to the static CSS edge when WebGL is unavailable', () => {
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    snapshot(panel);
    const canvas = document.querySelector<HTMLCanvasElement>('.ghost-edge-field')!;
    expect(canvas.getAttribute('aria-hidden')).toBe('true');
    expect(canvas.classList.contains('ghost-edge-field')).toBe(true);
    expect(canvas.dataset.ghostRenderer).toBe('static');
    expect(canvas.dataset.ghostFallback).toBe('webgl-unavailable');
    expect(getContext).toHaveBeenCalledWith('webgl2', expect.objectContaining({ alpha: true, powerPreference: 'low-power' }));

    snapshot(panel, 'result');
    expect(document.querySelector('.ghost-edge-field')).toBeNull();
  });

  it('uses the static edge without requesting a GPU context when reduced motion is preferred', () => {
    vi.stubGlobal('matchMedia', vi.fn(() => motionPreference(true)));
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext');
    snapshot(panel);
    const canvas = document.querySelector<HTMLCanvasElement>('.ghost-edge-field')!;
    expect(canvas.dataset.ghostRenderer).toBe('static');
    expect(canvas.dataset.ghostFallback).toBe('reduced-motion');
    expect(getContext).not.toHaveBeenCalled();
  });

  it('caps GPU resolution, stops frames on context loss, restores, and cleans up with the ghost', () => {
    const gl = webGLContext();
    vi.stubGlobal('matchMedia', vi.fn(() => motionPreference(false)));
    vi.spyOn(window, 'devicePixelRatio', 'get').mockReturnValue(3);
    vi.spyOn(HTMLCanvasElement.prototype, 'getBoundingClientRect').mockReturnValue(
      { left: 0, top: 0, width: 200, height: 100, right: 200, bottom: 100, x: 0, y: 0, toJSON() {} }
    );
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(gl as any);
    const requestFrame = vi.spyOn(globalThis, 'requestAnimationFrame').mockReturnValue(41);
    const cancelFrame = vi.spyOn(globalThis, 'cancelAnimationFrame');

    snapshot(panel);
    const canvas = document.querySelector<HTMLCanvasElement>('.ghost-edge-field')!;
    expect(canvas.dataset.ghostRenderer).toBe('webgl');
    expect(canvas.width).toBe(300);
    expect(canvas.height).toBe(150);
    expect(requestFrame).toHaveBeenCalled();

    canvas.dispatchEvent(new Event('webglcontextlost', { cancelable: true }));
    expect(cancelFrame).toHaveBeenCalledWith(41);
    expect(canvas.dataset.ghostRenderer).toBe('static');
    expect(canvas.dataset.ghostFallback).toBe('context-lost');

    canvas.dispatchEvent(new Event('webglcontextrestored'));
    expect(canvas.dataset.ghostRenderer).toBe('webgl');
    expect(canvas.dataset.ghostFallback).toBeUndefined();

    snapshot(panel, 'result');
    expect(gl.deleteBuffer).toHaveBeenCalled();
    expect(gl.deleteProgram).toHaveBeenCalled();
  });
});
