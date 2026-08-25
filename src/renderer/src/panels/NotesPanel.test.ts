// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { doc } from '../state/document.svelte';
import NotesPanel from './NotesPanel.svelte';

afterEach(() => {
  vi.useRealTimers();
  document.body.replaceChildren();
  delete window.PM;
});

describe('NotesPanel', () => {
  it('renders project notes and debounces the required direct-write fallback', async () => {
    vi.useFakeTimers();
    const project = { notes: 'Opening direction', assets: {}, layers: [] } as any;
    doc.replace(project);
    window.PM = { proj: project } as any;
    const target = document.createElement('div');
    document.body.append(target);
    const component = mount(NotesPanel, { target, props: { panelId: 'notes', spec: {} } });

    const root = target.querySelector('[data-svelte-panel="notes"]');
    const textarea = target.querySelector('textarea') as HTMLTextAreaElement;
    expect(root).toBeTruthy();
    expect(textarea.value).toBe('Opening direction');
    expect(textarea.getAttribute('aria-label')).toBe('Project notes');

    textarea.dispatchEvent(new FocusEvent('focus'));
    textarea.value = 'A quieter ending';
    textarea.dispatchEvent(new InputEvent('input', { bubbles: true }));
    await vi.advanceTimersByTimeAsync(249);
    expect(project.notes).toBe('Opening direction');
    await vi.advanceTimersByTimeAsync(1);
    expect(project.notes).toBe('A quieter ending');
    expect(target.querySelector('[role="status"]')?.textContent).toBe('Notes saved');

    await unmount(component);
  });

  it('flushes a pending note edit on blur and stops editor shortcuts', async () => {
    vi.useFakeTimers();
    const project = { notes: '', assets: {}, layers: [] } as any;
    doc.replace(project);
    window.PM = { proj: project } as any;
    const target = document.createElement('div');
    document.body.append(target);
    const component = mount(NotesPanel, { target, props: { panelId: 'notes', spec: {} } });
    const textarea = target.querySelector('textarea') as HTMLTextAreaElement;
    const parentKeydown = vi.fn();
    target.addEventListener('keydown', parentKeydown);

    textarea.dispatchEvent(new FocusEvent('focus'));
    textarea.value = 'Keep this version';
    flushSync(() => textarea.dispatchEvent(new InputEvent('input', { bubbles: true })));
    textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'b', bubbles: true }));
    textarea.dispatchEvent(new FocusEvent('blur'));

    expect(parentKeydown).not.toHaveBeenCalled();
    expect(project.notes).toBe('Keep this version');
    expect(textarea.value).toBe('Keep this version');
    await unmount(component);
  });

  it('loads replacement project notes without leaking a pending write into them', async () => {
    vi.useFakeTimers();
    const project = { notes: 'First project', assets: {}, layers: [] } as any;
    doc.replace(project);
    window.PM = { proj: project } as any;
    const target = document.createElement('div');
    document.body.append(target);
    const component = mount(NotesPanel, { target, props: { panelId: 'notes', spec: {} } });
    const textarea = target.querySelector('textarea') as HTMLTextAreaElement;

    textarea.dispatchEvent(new FocusEvent('focus'));
    textarea.value = 'Pending for first';
    flushSync(() => textarea.dispatchEvent(new InputEvent('input', { bubbles: true })));

    const replacement = { notes: 'Second project', assets: {}, layers: [] } as any;
    window.PM!.proj = replacement;
    doc.replace(replacement);
    flushSync();

    expect(project.notes).toBe('Pending for first');
    expect(replacement.notes).toBe('Second project');
    expect(textarea.value).toBe('Second project');

    textarea.value = 'Pending for cleanup';
    flushSync(() => textarea.dispatchEvent(new InputEvent('input', { bubbles: true })));
    await unmount(component);

    expect(replacement.notes).toBe('Pending for cleanup');
    expect(vi.getTimerCount()).toBe(0);
  });
});
