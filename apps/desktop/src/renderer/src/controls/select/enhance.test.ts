// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest';
import { enhanceSelect } from './enhance';

function build(): HTMLSelectElement {
  const select = document.createElement('select');
  select.setAttribute('aria-label', 'Format');
  select.className = 'sel export-select';
  for (const [v, l] of [['mp4', 'MP4'], ['webm', 'WebM'], ['prores', 'ProRes']] as const) {
    const o = document.createElement('option'); o.value = v; o.textContent = l; select.append(o);
  }
  document.body.append(select);
  return select;
}

describe('enhanceSelect', () => {
  afterEach(() => { document.body.innerHTML = ''; });

  it('mirrors the select into a trigger and keeps the select as the source of truth', async () => {
    const select = build();
    enhanceSelect(select);
    const trigger = document.querySelector('button.pm-select') as HTMLButtonElement;
    expect(trigger).toBeTruthy();
    expect(trigger.className).toContain('export-select');
    expect(trigger.getAttribute('aria-label')).toBe('Format');
    expect(trigger.querySelector('.pm-select-label')?.textContent).toBe('MP4');
    select.value = 'webm';
    expect(trigger.querySelector('.pm-select-label')?.textContent).toBe('WebM');
    select.replaceChildren(...['a', 'b'].map((v) => { const o = document.createElement('option'); o.value = v; o.textContent = v.toUpperCase(); return o; }));
    select.value = 'b';
    await Promise.resolve();
    expect(trigger.querySelector('.pm-select-label')?.textContent).toBe('B');
  });

  it('opens a listbox, picks with the keyboard, and fires change on the select', async () => {
    const select = build();
    enhanceSelect(select);
    const trigger = document.querySelector('button.pm-select') as HTMLButtonElement;
    let changed = '';
    select.addEventListener('change', () => { changed = select.value; });
    trigger.click();
    const menu = document.querySelector('.pm-menu') as HTMLElement;
    expect(menu?.getAttribute('role')).toBe('listbox');
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    menu.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    menu.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(changed).toBe('webm');
    expect(trigger.querySelector('.pm-select-label')?.textContent).toBe('WebM');
  });

  it('leaves Space available to a global shortcut when the select opts in', () => {
    const select = build();
    select.dataset.globalSpaceShortcut = '';
    enhanceSelect(select);
    const trigger = document.querySelector('button.pm-select') as HTMLButtonElement;
    let globalKeydown = false;
    const onKeydown = (event: KeyboardEvent) => {
      if (event.key !== ' ') return;
      globalKeydown = true;
      event.preventDefault();
    };
    window.addEventListener('keydown', onKeydown);

    const event = new KeyboardEvent('keydown', { key: ' ', code: 'Space', bubbles: true, cancelable: true });
    trigger.dispatchEvent(event);

    window.removeEventListener('keydown', onKeydown);
    expect(globalKeydown).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    expect(document.querySelector('.pm-menu')).toBeNull();
  });

  it('continues to open ordinary selects with Space', () => {
    const select = build();
    enhanceSelect(select);
    const trigger = document.querySelector('button.pm-select') as HTMLButtonElement;

    trigger.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', code: 'Space', bubbles: true, cancelable: true }));

    expect(document.querySelector('.pm-menu')).toBeTruthy();
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
  });

  it('leaves data-native selects alone', () => {
    const select = build();
    select.dataset.native = '';
    enhanceSelect(select);
    expect(select.previousElementSibling).toBeNull();
  });

  it('preserves inline viewer control placement and sizing on the visible trigger', async () => {
    const select = build();
    select.style.cssText = 'position:absolute;right:8px;top:7px;width:110px;height:24px';
    enhanceSelect(select);
    const trigger = select.previousElementSibling as HTMLButtonElement;
    expect(trigger.style.position).toBe('absolute');
    expect(trigger.style.right).toBe('8px');
    expect(trigger.style.width).toBe('110px');
    expect(trigger.classList.contains('pm-select-native')).toBe(false);
    select.style.width = '120px';
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(trigger.style.width).toBe('120px');
  });
});
