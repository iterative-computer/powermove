// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import ProjectGhost from './ProjectGhost.svelte';
import { ghostRows, holdBody, measureGhostTiles, readProjectLoading } from './project-ghost';

function fakeBus() {
  const listeners = new Map<string, Set<() => void>>();
  return {
    on(event: string, listener: () => void) {
      let set = listeners.get(event);
      if (!set) listeners.set(event, (set = new Set()));
      set.add(listener);
      return () => set!.delete(listener);
    },
    emit(event: string) {
      for (const listener of [...(listeners.get(event) ?? [])]) listener();
    }
  };
}

function place(element: Element, left: number, top: number, width: number, height: number): void {
  vi.spyOn(element, 'getBoundingClientRect').mockReturnValue(new DOMRect(left, top, width, height));
}

describe('project loading ghost', () => {
  let instance: ReturnType<typeof mount> | null;

  beforeEach(() => {
    instance = null;
    document.body.innerHTML = '<div id="app"><div id="titlebar"></div><div id="body"></div><div id="status"></div></div><div id="host"></div>';
  });

  afterEach(async () => {
    if (instance) await unmount(instance);
    vi.restoreAllMocks();
    document.body.replaceChildren();
  });

  function start(PM: Record<string, any>) {
    instance = mount(ProjectGhost, { target: document.getElementById('host')!, props: { PM } });
    flushSync();
  }

  it('reads only a well-formed loading state', () => {
    expect(readProjectLoading({})).toBeNull();
    expect(readProjectLoading({ projectLoading: { id: 'p1', name: 'Film', generation: 3 } })).toEqual({ id: 'p1', name: 'Film', generation: 3 });
    expect(readProjectLoading({ projectLoading: { id: 7, generation: 'x' } })).toEqual({ id: null, name: '', generation: 0 });
  });

  it('holds the body busy and inert and restores exactly what was there', () => {
    const body = document.getElementById('body')!;
    body.setAttribute('aria-busy', 'false');
    const release = holdBody(body);
    expect(body.getAttribute('aria-busy')).toBe('true');
    expect(body.inert).toBe(true);
    release();
    expect(body.getAttribute('aria-busy')).toBe('false');
    expect(body.inert).toBe(false);
  });

  it('measures visible panels relative to the body, clipped to their dock', () => {
    const body = document.getElementById('body')!;
    body.innerHTML = `
      <div class="dock" id="dock-left"><div data-panel-slot="project" style="display:contents"><section class="panel" id="a"><div class="panel" id="nested"></div></section></div>
        <div data-panel-slot="layers" style="display:contents"><section class="panel" id="b"></section></div></div>
      <div class="dock" id="dock-center"><div data-panel-slot="viewer" style="display:contents"><section class="panel" id="c"></section></div></div>
      <div class="dock" id="dock-bottom"><div data-panel-slot="timeline" style="display:contents"><section class="panel" id="d"></section></div></div>`;
    place(body, 0, 44, 1200, 700);
    place(body.querySelector('#dock-left')!, 12, 44, 250, 400);
    place(body.querySelector('#a')!, 12, 44, 250, 300);
    place(body.querySelector('#nested')!, 12, 44, 250, 300);
    // Scrolled below the dock's viewport: only 46px of it shows.
    place(body.querySelector('#b')!, 12, 398, 250, 300);
    place(body.querySelector('#dock-center')!, 272, 44, 600, 380);
    place(body.querySelector('#c')!, 272, 44, 600, 380);
    place(body.querySelector('#dock-bottom')!, 12, 434, 860, 306);
    place(body.querySelector('#d')!, 12, 434, 860, 306);

    expect(measureGhostTiles(body)).toEqual([
      { x: 12, y: 0, width: 250, height: 300, kind: 'list', rows: ghostRows('list', 300) },
      { x: 12, y: 354, width: 250, height: 46, kind: 'list', rows: 0 },
      { x: 272, y: 0, width: 600, height: 380, kind: 'viewer', rows: 0 },
      { x: 12, y: 390, width: 860, height: 306, kind: 'timeline', rows: ghostRows('timeline', 306) }
    ]);
  });

  it('covers the body while a project opens and clears with the load', () => {
    const bus = fakeBus();
    const PM: Record<string, any> = { bus, projectLoading: null };
    start(PM);
    const body = document.getElementById('body')!;
    const status = document.querySelector('.project-loading-status')!;
    expect(status.getAttribute('role')).toBe('status');
    expect(status.textContent).toBe('');
    expect(document.querySelector('[data-project-ghost]')).toBeNull();

    flushSync(() => {
      PM.projectLoading = { id: 'p2', name: 'Long Film', generation: 1 };
      bus.emit('project:loading');
    });
    const ghost = document.querySelector<HTMLElement>('[data-project-ghost]')!;
    expect(ghost.getAttribute('aria-hidden')).toBe('true');
    // Nothing laid out to measure: the default dock layout stands in.
    expect(ghost.classList.contains('generic')).toBe(true);
    expect([...ghost.querySelectorAll<HTMLElement>('.pg-tile')].map((tile) => tile.style.gridArea)).toEqual(['left', 'center', 'bottom', 'right']);
    expect(body.getAttribute('aria-busy')).toBe('true');
    expect(body.inert).toBe(true);
    expect(status.textContent).toBe('Opening Long Film');
    expect(status.closest('[aria-hidden="true"]')).toBeNull();

    flushSync(() => {
      PM.projectLoading = null;
      bus.emit('project:loading');
    });
    expect(document.querySelector('[data-project-ghost]')).toBeNull();
    expect(body.hasAttribute('aria-busy')).toBe(false);
    expect(body.inert).toBe(false);
    expect(status.textContent).toBe('');
  });

  it('hands startup visuals to the measured loading overlay when the UI mounts', () => {
    document.body.insertAdjacentHTML('beforeend', '<div id="boot-ghost" aria-hidden="true"></div>');
    const bus = fakeBus();
    const PM: Record<string, any> = { bus, projectLoading: { id: null, name: '', generation: 1 } };
    start(PM);

    expect(document.getElementById('boot-ghost')).toBeNull();
    expect(document.querySelector('[data-project-ghost]')).not.toBeNull();
    expect(document.getElementById('body')!.inert).toBe(true);
    expect(document.querySelector('.project-loading-status')!.textContent).toBe('Opening Untitled');
  });

  it('removes the static startup placeholder once the interactive UI has mounted', () => {
    document.body.insertAdjacentHTML('beforeend', '<div id="boot-ghost" aria-hidden="true"></div>');
    start({ bus: fakeBus(), projectLoading: null });
    expect(document.getElementById('boot-ghost')).toBeNull();
    expect(document.querySelector('[data-project-ghost]')).toBeNull();
    expect(document.getElementById('body')!.inert).toBe(false);
  });

  it('releases the body when unmounted mid-load', async () => {
    const bus = fakeBus();
    start({ bus, projectLoading: { id: 'p1', name: 'Film', generation: 1 } });
    const body = document.getElementById('body')!;
    expect(body.inert).toBe(true);
    await unmount(instance!);
    instance = null;
    expect(body.inert).toBe(false);
    expect(body.hasAttribute('aria-busy')).toBe(false);
  });
});
