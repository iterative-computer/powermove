// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import UIPlacementGhost from './UIPlacementGhost.svelte';
import { agentState, resetAgentState, setAgentSnapshot } from './agent-state.svelte';
import type { UIPlacement } from './ui-placement';

let component: ReturnType<typeof mount> | undefined;
const disconnect = vi.fn();
const panel: UIPlacement = { kind: 'panel', id: 'timeline', label: 'Timeline controls' };

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
});
