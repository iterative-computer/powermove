// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { expect, it, vi } from 'vitest';
import Turn from './Turn.svelte';

it('renders edited panel rows and opens the selected panel, while preserving creation labels', () => {
  const target = document.createElement('div');
  document.body.appendChild(target);
  const reveal = vi.fn();
  const PM = { Kernel: { panels: { entries: () => [
    { id: 'search', ownerId: 'pexels', item: { title: 'Search' } },
    { id: 'saved', ownerId: 'pexels', item: { title: 'Saved clips' } },
  ] } }, LibraryUI: { reveal } };
  for (const action of ['updated', 'created'] as const) {
    const instance = mount(Turn, { target, props: { PM, message: {
      role: 'assistant', modResult: { id: 'pexels', name: 'Pexels Browser', action, status: 'ready' }
    } } });
    flushSync();
    if (action === 'updated') {
      expect(target.textContent).toContain('Edited 2 panels');
      expect(target.textContent).not.toContain('Panel created');
      expect(target.querySelectorAll('.edited-panel')).toHaveLength(2);
      target.querySelector<HTMLButtonElement>('[aria-label="Open Saved clips"]')!.click();
      expect(reveal).toHaveBeenCalledWith('saved');
    } else {
      expect(target.textContent).toContain('Panel created');
      expect(target.querySelector('.edited-result')).toBeNull();
    }
    unmount(instance);
    flushSync();
  }
  target.remove();
});
