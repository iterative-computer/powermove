// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Dock from './Dock.svelte';
import type { PMRegistry } from '../legacy/registry';
import { agentState, resetAgentState, setAgentSnapshot } from '../panels/agent/agent-state.svelte';
import type { UIPlacement } from '../panels/agent/ui-placement';

let component: ReturnType<typeof mount> | undefined;
const specs = [{ id: 'inspector' }, { id: 'library' }];
const dock = { id: 'right', panels: specs };
const PM = {
  PANELS: { inspector: {}, library: {} },
  panelInst: {},
  Layout: { ws: null },
  WS: { save() {} },
  bus: { emit() {} },
  clamp: (value: number) => value
} as unknown as PMRegistry;

function place(placement: UIPlacement | null, phase = 'working') {
  flushSync(() => setAgentSnapshot({ ...agentState, legacyPhase: phase, requestToken: 1, uiPlacement: placement }));
}

/** Dock children in order; panel slots are display:contents wrappers. */
function order() {
  const column = document.querySelector('.dock')!;
  return [...column.children].map(child =>
    child.getAttribute('data-panel-slot') ?? (child.hasAttribute('data-ui-placement-ghost') ? 'ghost' : 'splitter')
  );
}

beforeEach(() => {
  resetAgentState();
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  document.body.innerHTML = '<div id="host"></div>';
  component = mount(Dock, {
    target: document.getElementById('host')!,
    props: { PM, dock, specs, tick: 0, oncommit: () => {} }
  });
  flushSync();
});

afterEach(async () => {
  if (component) await unmount(component);
  resetAgentState();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

describe('dock slot for a panel being built', () => {
  it('holds the announced slot without adding a panel to the layout', () => {
    expect(order()).toEqual(['inspector', 'splitter', 'library']);

    place({ kind: 'dock', id: 'right', label: 'Easing controls', beforePanelId: 'library' });
    expect(order()).toEqual(['inspector', 'splitter', 'ghost', 'library']);
    const ghost = document.querySelector('[data-ui-placement-ghost="right"]')!;
    expect(ghost.textContent).toContain('Building Easing controls');
    // The dock still shows exactly the panels the workspace has.
    expect(document.querySelectorAll('[data-panel-slot]')).toHaveLength(2);
  });

  it('appends the slot when the run names no panel to build in front of', () => {
    place({ kind: 'dock', id: 'right', label: 'Easing controls', beforePanelId: null });
    expect(order()).toEqual(['inspector', 'splitter', 'library', 'ghost']);
  });

  it('stays out of other docks, live panels, and finished runs', () => {
    place({ kind: 'dock', id: 'center', label: 'Elsewhere', beforePanelId: null });
    expect(order()).toEqual(['inspector', 'splitter', 'library']);

    place({ kind: 'panel', id: 'inspector', label: 'Inspector' });
    expect(order()).toEqual(['inspector', 'splitter', 'library']);

    place({ kind: 'dock', id: 'right', label: 'Easing controls', beforePanelId: null });
    place({ kind: 'dock', id: 'right', label: 'Easing controls', beforePanelId: null }, 'result');
    expect(order()).toEqual(['inspector', 'splitter', 'library']);
  });
});
