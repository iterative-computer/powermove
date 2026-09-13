// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { expect, it, vi } from 'vitest';
import Turn from './Turn.svelte';

it('renders edited panels in the creation card and opens the selected panel', () => {
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
      // Same card as creation, different label — one design for both.
      expect(target.textContent).toContain('2 panels edited');
      expect(target.textContent).not.toContain('Panel created');
      const buttons = [...target.querySelectorAll<HTMLButtonElement>('.mod-panels button')];
      expect(buttons.map((button) => button.textContent)).toEqual(['Open Search', 'Open Saved clips']);
      buttons[1]!.click();
      expect(reveal).toHaveBeenCalledWith('saved');
    } else {
      expect(target.textContent).toContain('Panel created');
      expect(target.textContent).not.toContain('edited');
    }
    unmount(instance);
    flushSync();
  }
  target.remove();
});
