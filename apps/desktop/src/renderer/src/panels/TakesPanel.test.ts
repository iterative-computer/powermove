// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';

import { doc } from '../state/document.svelte';
import TakesPanel from './TakesPanel.svelte';

interface Take {
  id: string;
  label: string;
  at: number;
}

function fakePM(initial: Take[]) {
  let takes = initial;
  const listeners = new Map<string, Set<() => void>>();
  const bus = {
    on: vi.fn((event: string, listener: () => void) => {
      let set = listeners.get(event);
      if (!set) listeners.set(event, (set = new Set()));
      set.add(listener);
      return () => set!.delete(listener);
    }),
    emit(event: string) {
      for (const listener of [...(listeners.get(event) ?? [])]) listener();
    }
  };
  const PM: Record<string, any> = {
    ICONS: { x: '<path/>', plus: '<path/>', missing: '<path/>' },
    bus,
    takes: {
      all: vi.fn(() => takes),
      restore: vi.fn(),
      drop: vi.fn(),
      save: vi.fn(() => ({ label: 'Take 3' }))
    }
  };
  return {
    PM,
    bus,
    setTakes(next: Take[]) {
      takes = next;
    }
  };
}

function buttonWithText(target: HTMLElement, text: string): HTMLButtonElement {
  const button = [...target.querySelectorAll<HTMLButtonElement>('button')]
    .find((candidate) => candidate.textContent?.trim() === text);
  if (!button) throw new Error(`Missing button: ${text}`);
  return button;
}

describe('TakesPanel', () => {
  let target: HTMLDivElement;
  let instance: ReturnType<typeof mount> | undefined;

  beforeEach(() => {
    target = document.createElement('div');
    document.body.append(target);
    doc.replace({ id: 'project', layers: [] } as any);
  });

  afterEach(async () => {
    if (instance) await unmount(instance);
    instance = undefined;
    target.remove();
    delete (window as any).PM;
  });

  it('renders stored newest-first data with named native controls', () => {
    const newest = { id: 'take-2', label: 'Newest take', at: Date.now() };
    const oldest = { id: 'take-1', label: 'Older take', at: Date.now() - 60_000 };
    const { PM } = fakePM([newest, oldest]);
    window.PM = PM;

    instance = mount(TakesPanel, { target, props: { panelId: 'takes', spec: {} } });
    flushSync();

    const root = target.querySelector<HTMLElement>('[data-svelte-panel="takes"]');
    expect(root).not.toBeNull();
    expect([...root!.querySelectorAll('.nm')].map((node) => node.textContent)).toEqual(['Newest take', 'Older take']);
    expect(root!.querySelectorAll('button[aria-label]')).toHaveLength(2);
    expect(root!.querySelector('button[aria-label="Delete Newest take"]')).not.toBeNull();
    expect(root!.querySelector('button[aria-label="Delete Older take"]')).not.toBeNull();
    expect(buttonWithText(root!, 'Save take')).toBeTruthy();
    expect(root!.querySelector('[role="status"]')).not.toBeNull();
  });

  it('routes restore, drop, and save through PM.takes', () => {
    const take = { id: 'take-1', label: 'First take', at: Date.now() };
    const { PM } = fakePM([take]);
    window.PM = PM;
    instance = mount(TakesPanel, { target, props: { panelId: 'takes', spec: {} } });
    flushSync();

    flushSync(() => target.querySelector<HTMLButtonElement>('.simple-row-action')!.click());
    expect(PM.takes.restore).toHaveBeenCalledWith('take-1');

    flushSync(() => target.querySelector<HTMLButtonElement>('[aria-label="Delete First take"]')!.click());
    expect(PM.takes.drop).toHaveBeenCalledWith('take-1');

    flushSync(() => buttonWithText(target, 'Save take').click());
    expect(PM.takes.save).toHaveBeenCalledOnce();
    expect(target.querySelector('[role="status"]')?.textContent).toBe('Saved Take 3');
  });

  it('rerenders from PM.takes.all when the takes bus event fires', () => {
    const first = { id: 'take-1', label: 'First take', at: Date.now() };
    const fake = fakePM([first]);
    window.PM = fake.PM;
    instance = mount(TakesPanel, { target, props: { panelId: 'takes', spec: {} } });
    flushSync();
    expect(target.querySelector('.nm')?.textContent).toBe('First take');

    const second = { id: 'take-2', label: 'Second take', at: Date.now() + 1_000 };
    fake.setTakes([second, first]);
    flushSync(() => fake.bus.emit('takes'));

    expect([...target.querySelectorAll('.nm')].map((node) => node.textContent)).toEqual(['Second take', 'First take']);
    expect(fake.PM.takes.all).toHaveBeenCalledTimes(2);
  });

  it('shows the legacy empty state when there are no takes', () => {
    const { PM } = fakePM([]);
    window.PM = PM;
    instance = mount(TakesPanel, { target, props: { panelId: 'takes', spec: {} } });
    flushSync();

    expect(target.querySelector('.empty')?.textContent).toBe('Save a take before exploring a new motion direction.');
    expect(buttonWithText(target, 'Save take')).toBeTruthy();
  });
});
