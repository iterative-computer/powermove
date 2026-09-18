// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';

import { doc } from '../state/document.svelte';
import WorkspacesPanel from './WorkspacesPanel.svelte';

interface Workspace {
  id: string;
  name: string;
  builtin?: boolean;
  theme?: { accent?: string };
}

function fakePM(initial: Workspace[]) {
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
    },
    count(event: string) {
      return listeners.get(event)?.size ?? 0;
    }
  };
  const activate = vi.fn((id: string) => {
    PM.WS.current = PM.WS.all.find((workspace: Workspace) => workspace.id === id);
  });
  const PM: Record<string, any> = {
    ICONS: { trash: '<path/>', plus: '<path/>', code: '<path/>', missing: '<path/>' },
    bus,
    modal: vi.fn(),
    confirm: vi.fn(async () => true),
    WS: {
      all: [...initial],
      current: initial[0],
      activate,
      remove: vi.fn(),
      saveAsNew: vi.fn(),
      editJSON: vi.fn()
    }
  };
  return { PM, bus };
}

function buttonWithText(target: HTMLElement, text: string): HTMLButtonElement {
  const button = [...target.querySelectorAll<HTMLButtonElement>('button')]
    .find((candidate) => candidate.textContent?.trim() === text);
  if (!button) throw new Error(`Missing button: ${text}`);
  return button;
}

describe('WorkspacesPanel', () => {
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

  it('renders PM.WS.all, highlights current, and names every action', () => {
    const design = { id: 'design', name: 'Design', builtin: true, theme: { accent: '#ff6600' } };
    const custom = { id: 'custom', name: 'My workspace' };
    const { PM, bus } = fakePM([design, custom]);
    PM.WS.current = custom;
    window.PM = PM;

    instance = mount(WorkspacesPanel, {
      target,
      props: { panelId: 'workspaces', spec: {} }
    });
    flushSync();

    const root = target.querySelector<HTMLElement>('[data-svelte-panel="workspaces"]');
    expect(root).not.toBeNull();
    expect([...root!.querySelectorAll('.nm')].map((node) => node.textContent)).toEqual(['Design', 'My workspace']);
    expect(root!.querySelector('.lyr.sel .nm')?.textContent).toBe('My workspace');
    expect(root!.querySelector('[aria-current="true"]')?.textContent).toContain('My workspace');
    expect(root!.querySelectorAll('button[aria-label="Delete My workspace"]')).toHaveLength(1);
    expect(root!.querySelector('button[aria-label="Delete Design"]')).toBeNull();
    expect(buttonWithText(root!, 'Save current')).toBeTruthy();
    expect(buttonWithText(root!, 'JSON')).toBeTruthy();
    expect(root!.querySelector('[role="status"]')).not.toBeNull();
    expect(bus.on).toHaveBeenCalledTimes(2);
    expect(bus.on).toHaveBeenCalledWith('workspaces', expect.any(Function));
    expect(bus.on).toHaveBeenCalledWith('layout', expect.any(Function));
  });

  it('routes activation, removal, save, and JSON actions through PM.WS', async () => {
    const design = { id: 'design', name: 'Design', builtin: true };
    const custom = { id: 'custom', name: 'My workspace' };
    const { PM } = fakePM([design, custom]);
    window.PM = PM;
    instance = mount(WorkspacesPanel, { target, props: { panelId: 'workspaces', spec: {} } });
    flushSync();

    flushSync(() => buttonWithText(target, 'My workspace').click());
    expect(PM.WS.activate).toHaveBeenCalledWith('custom');

    flushSync(() => target.querySelector<HTMLButtonElement>('[aria-label="Delete My workspace"]')!.click());
    expect(PM.WS.remove).not.toHaveBeenCalled();
    expect(PM.confirm.mock.calls[0][0]).toEqual({
      message: 'Delete “My workspace” workspace?',
      detail: 'This removes the saved workspace layout. Your project and its layers will not be deleted.',
      confirmLabel: 'Delete Workspace'
    });
    await PM.confirm.mock.results[0].value;
    flushSync();
    expect(PM.WS.remove).toHaveBeenCalledWith('custom');

    flushSync(() => buttonWithText(target, 'Save current').click());
    flushSync(() => buttonWithText(target, 'JSON').click());
    expect(PM.WS.saveAsNew).toHaveBeenCalledOnce();
    expect(PM.WS.editJSON).toHaveBeenCalledOnce();
    expect(target.querySelector('[role="status"]')?.textContent).toBe('Workspace JSON editor opened');
  });

  it('rerenders on workspace and layout events and removes both listeners on unmount', async () => {
    const design = { id: 'design', name: 'Design', builtin: true };
    const { PM, bus } = fakePM([design]);
    window.PM = PM;
    instance = mount(WorkspacesPanel, { target, props: { panelId: 'workspaces', spec: {} } });
    flushSync();
    expect(bus.count('workspaces')).toBe(1);

    const review = { id: 'review', name: 'Review', builtin: true };
    PM.WS.all = [design, review];
    PM.WS.current = review;
    flushSync(() => bus.emit('workspaces'));

    expect([...target.querySelectorAll('.nm')].map((node) => node.textContent)).toEqual(['Design', 'Review']);
    expect(target.querySelector('.lyr.sel .nm')?.textContent).toBe('Review');

    PM.WS.all = [review];
    flushSync(() => bus.emit('layout'));
    expect([...target.querySelectorAll('.nm')].map((node) => node.textContent)).toEqual(['Review']);

    await unmount(instance);
    instance = undefined;
    expect(bus.count('workspaces')).toBe(0);
    expect(bus.count('layout')).toBe(0);
  });
});
