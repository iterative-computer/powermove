// Replace a native <select> with a button trigger and our own listbox, while
// the <select> stays in the DOM as the source of truth: its options, value,
// disabled state and change events keep working for the code that owns it.
// The trigger takes over the select's classes so existing layout CSS applies.

import { closeSelectMenu, openSelectMenu, type MenuHandle } from './menu';

const ENHANCED = new WeakMap<HTMLSelectElement, () => void>();

const CHEVRON = '<svg class="pm-select-chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 6.5 8 10l3.5-3.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>';

export function enhanceSelect(select: HTMLSelectElement): () => void {
  const existing = ENHANCED.get(select);
  if (existing) return existing;
  if (select.dataset.native !== undefined || select.multiple) return () => {};

  const trigger = document.createElement('button');
  trigger.type = 'button';
  trigger.setAttribute('role', 'combobox');
  trigger.setAttribute('aria-haspopup', 'listbox');
  trigger.setAttribute('aria-expanded', 'false');
  const label = document.createElement('span');
  label.className = 'pm-select-label';
  trigger.append(label);
  trigger.insertAdjacentHTML('beforeend', CHEVRON);

  let menu: MenuHandle | null = null;

  function sync(): void {
    trigger.className = `pm-select ${select.className}`.trim();
    const title = select.getAttribute('aria-label') ?? select.title;
    if (title) trigger.setAttribute('aria-label', title); else trigger.removeAttribute('aria-label');
    const by = select.getAttribute('aria-labelledby');
    if (by) trigger.setAttribute('aria-labelledby', by); else trigger.removeAttribute('aria-labelledby');
    if (select.id) trigger.id = `${select.id}-trigger`;
    trigger.disabled = select.disabled;
    const opt = select.options[select.selectedIndex] ?? select.selectedOptions[0];
    label.textContent = opt?.textContent ?? '';
    if (!opt || opt.value === '') trigger.setAttribute('data-empty', ''); else trigger.removeAttribute('data-empty');
  }

  function open(): void {
    if (trigger.disabled || menu) return;
    const options = Array.from(select.options)
      .filter((o) => !o.hidden)
      .map((o) => ({ value: o.value, label: o.textContent ?? '', disabled: o.disabled }));
    trigger.setAttribute('aria-expanded', 'true');
    menu = openSelectMenu({
      anchor: trigger,
      options,
      value: select.value,
      onPick: (value) => {
        if (select.value !== value) {
          select.value = value;
          select.dispatchEvent(new Event('input', { bubbles: true }));
          select.dispatchEvent(new Event('change', { bubbles: true }));
        }
        sync();
      },
      onClose: () => {
        menu = null;
        trigger.setAttribute('aria-expanded', 'false');
        if (document.activeElement === document.body || document.activeElement === null) trigger.focus({ preventScroll: true });
      },
    });
  }

  trigger.addEventListener('click', (event) => { event.stopPropagation(); if (menu) menu.close(); else open(); });
  trigger.addEventListener('pointerdown', (event) => event.stopPropagation());
  trigger.addEventListener('keydown', (event) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp' || event.key === 'Enter' || event.key === ' ') {
      event.preventDefault(); event.stopPropagation(); open();
    }
  });

  // Value writes from the owning code don't mutate the DOM, so intercept them.
  const proto = HTMLSelectElement.prototype;
  for (const key of ['value', 'selectedIndex'] as const) {
    const desc = Object.getOwnPropertyDescriptor(proto, key)!;
    Object.defineProperty(select, key, {
      configurable: true,
      get() { return desc.get!.call(select); },
      set(v) { desc.set!.call(select, v); sync(); },
    });
  }

  const observer = new MutationObserver(sync);
  observer.observe(select, { childList: true, subtree: true, characterData: true, attributes: true });
  select.addEventListener('change', sync);

  select.classList.add('pm-select-native');
  select.tabIndex = -1;
  select.setAttribute('aria-hidden', 'true');
  select.before(trigger);
  sync();

  const dispose = () => {
    menu?.close();
    observer.disconnect();
    select.removeEventListener('change', sync);
    for (const key of ['value', 'selectedIndex'] as const) delete (select as any)[key];
    select.classList.remove('pm-select-native');
    select.removeAttribute('aria-hidden');
    select.tabIndex = 0;
    trigger.remove();
    ENHANCED.delete(select);
  };
  ENHANCED.set(select, dispose);
  return dispose;
}

/** Re-read the select after a value change that bypassed the DOM (Svelte sets option.selected). */
export function refreshSelect(select: HTMLSelectElement): void {
  select.dispatchEvent(new Event('change', { bubbles: false }));
}

/** Svelte action: `<select use:fancySelect={value}>`. The parameter only exists so the action re-syncs when the bound value changes. */
export function fancySelect(select: HTMLSelectElement, _value?: unknown) {
  const dispose = enhanceSelect(select);
  return {
    update() { queueMicrotask(() => refreshSelect(select)); },
    destroy() { dispose(); },
  };
}

/** Enhance every select that enters the document, unless it opts out with data-native. */
export function autoEnhanceSelects(root: ParentNode = document): () => void {
  const visit = (node: Node) => {
    if (!(node instanceof Element)) return;
    if (node instanceof HTMLSelectElement) enhanceSelect(node);
    for (const s of node.querySelectorAll('select')) enhanceSelect(s);
  };
  for (const s of root.querySelectorAll('select')) enhanceSelect(s);
  const observer = new MutationObserver((records) => {
    for (const r of records) r.addedNodes.forEach(visit);
  });
  observer.observe(root, { childList: true, subtree: true });
  return () => { observer.disconnect(); closeSelectMenu(); };
}
