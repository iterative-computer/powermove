// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const lisse = vi.hoisted(() => ({
  observeResize: vi.fn(), observeAnchor: vi.fn(), acquirePosition: vi.fn(),
  releasePosition: vi.fn(), restoreStyles: vi.fn(), getLayoutSize: vi.fn(),
  createSvgEffects: vi.fn(), generateClipPath: vi.fn(),
}));
vi.mock('@lisse/core', () => ({
  ...lisse,
  extractAndStripEffects(el: HTMLElement) {
    const border = el.style.border;
    el.style.border = 'none';
    return { effects: { innerBorder: { width: 1, color: '#888', opacity: 1 } }, savedStyles: { border } };
  },
  hasEffects: () => true,
}));

let mount: typeof import('./squircle')['mountSquircles'];
let stops: Array<() => void>;
let resizes: Map<HTMLElement, () => void>;
let effects: Array<{ update: ReturnType<typeof vi.fn>; destroy: ReturnType<typeof vi.fn> }>;
let frames: FrameRequestCallback[];
let disposers: Array<() => void>;

beforeEach(async () => {
  vi.resetModules(); vi.clearAllMocks();
  stops = []; resizes = new Map(); effects = []; frames = []; disposers = [];
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  lisse.acquirePosition.mockReturnValue(true);
  lisse.observeResize.mockImplementation((el: HTMLElement, callback: () => void) => {
    resizes.set(el, callback);
    const stop = vi.fn(() => resizes.delete(el)); stops.push(stop); return stop;
  });
  lisse.observeAnchor.mockImplementation(() => { const stop = vi.fn(); stops.push(stop); return stop; });
  lisse.getLayoutSize.mockReturnValue({ width: 120, height: 32 });
  lisse.generateClipPath.mockReturnValue('path("M0 0")');
  lisse.createSvgEffects.mockImplementation(() => {
    const handle = { update: vi.fn(), destroy: vi.fn() }; effects.push(handle); return handle;
  });
  lisse.restoreStyles.mockImplementation((el: HTMLElement, saved: { border: string }) => { el.style.border = saved.border; });
  mount = (await import('./squircle')).mountSquircles;
});
afterEach(() => {
  for (const dispose of disposers) dispose();
  document.body.replaceChildren();
  vi.unstubAllGlobals();
});

function control(): HTMLButtonElement {
  const el = document.createElement('button'); el.className = 'smooth';
  el.style.borderRadius = '8px'; el.style.border = '1px solid #888'; return el;
}
function setup(...nodes: HTMLElement[]) {
  const root = document.createElement('div'); root.append(...nodes); document.body.append(root);
  const dispose = mount(root, '.smooth'); disposers.push(dispose); return { root, dispose };
}
const deliver = () => new Promise(resolve => setTimeout(resolve, 0));

describe('squircle lifecycle', () => {
  it('releases a control removed immediately before teardown, before observer delivery', () => {
    const button = control(), border = button.style.border;
    const { dispose } = setup(button);
    expect(button.hasAttribute('data-squircle')).toBe(true);
    button.remove(); dispose();
    expect(stops).toHaveLength(2);
    for (const stop of stops) expect(stop).toHaveBeenCalledTimes(1);
    expect(effects[0]!.destroy).toHaveBeenCalledTimes(1);
    expect(lisse.releasePosition).toHaveBeenCalledTimes(1);
    expect(button.style.border).toBe(border);
    expect(button.style.clipPath).toBe('');
    expect(button.hasAttribute('data-squircle')).toBe(false);
  });

  it('releases removed nested subtrees and attached controls exactly once', () => {
    const first = control(), second = control(), container = document.createElement('div');
    container.append(first);
    const { dispose } = setup(container, second);
    container.remove(); dispose(); dispose();
    for (const stop of stops) expect(stop).toHaveBeenCalledTimes(1);
    for (const handle of effects) expect(handle.destroy).toHaveBeenCalledTimes(1);
    expect(lisse.releasePosition).toHaveBeenCalledTimes(2);
    expect(resizes.size).toBe(0);
  });

  it('does not repaint leaked detached controls after repeated remove/close cycles', () => {
    for (let i = 0; i < 20; i++) {
      const button = control(), { dispose } = setup(button);
      button.remove(); dispose();
    }
    const keeper = control(); setup(keeper);
    lisse.getLayoutSize.mockClear();
    resizes.get(keeper)!();
    for (const callback of frames.splice(0)) callback(0);
    expect(lisse.getLayoutSize).toHaveBeenCalledExactlyOnceWith(keeper);
    expect(resizes.size).toBe(1);
  });

  it('retains ordinary mutation enhancement and cleanup', async () => {
    const { root } = setup();
    const button = control(); root.append(button); await deliver();
    expect(button.hasAttribute('data-squircle')).toBe(true);
    button.remove(); await deliver();
    expect(button.hasAttribute('data-squircle')).toBe(false);
    expect(resizes.size).toBe(0);
  });

  it('does not disconnect another root or overlapping root observer', async () => {
    const first = control(), { root, dispose } = setup(first);
    const inner = document.createElement('div'); root.append(inner);
    const stopInner = mount(inner, '.smooth'); disposers.push(stopInner);
    const other = setup();
    first.remove(); dispose();
    const nested = control(), sibling = control(); inner.append(nested); other.root.append(sibling);
    await deliver();
    expect(nested.hasAttribute('data-squircle')).toBe(true);
    expect(sibling.hasAttribute('data-squircle')).toBe(true);
    stopInner(); other.dispose();
    expect(resizes.size).toBe(0);
  });
});
