// A floating listbox for select controls. One instance at a time, anchored to
// its trigger, drawn above everything else. Opens with a short fade, scale and
// slide from the trigger's edge; closes the same way in reverse.

export interface MenuOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface MenuRequest {
  anchor: HTMLElement;
  options: MenuOption[];
  value: string;
  onPick: (value: string) => void;
  onClose?: () => void;
}

export interface MenuHandle {
  close(): void;
  readonly el: HTMLElement;
}

const GAP = 4;
const EDGE = 8;
const MAX_HEIGHT = 320;

let current: MenuHandle | null = null;

export function closeSelectMenu(): void {
  current?.close();
}

export function openSelectMenu(req: MenuRequest): MenuHandle {
  current?.close();

  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const el = document.createElement('div');
  el.className = 'pm-menu';
  el.setAttribute('role', 'listbox');
  el.tabIndex = -1;

  // One hover layer glides between rows instead of each row painting its own
  // background (after beautiful-ui's GlideMenu and the command palette).
  const glider = document.createElement('div');
  glider.className = 'pm-menu-glider';
  glider.setAttribute('aria-hidden', 'true');
  el.append(glider);
  let gliderOn = false;

  const items: HTMLElement[] = [];
  let active = Math.max(0, req.options.findIndex((o) => o.value === req.value));
  for (const [i, option] of req.options.entries()) {
    const item = document.createElement('div');
    item.className = 'pm-menu-item';
    item.setAttribute('role', 'option');
    item.setAttribute('aria-selected', String(option.value === req.value));
    if (option.disabled) item.setAttribute('aria-disabled', 'true');
    const label = document.createElement('span');
    label.className = 'pm-menu-label';
    label.textContent = option.label;
    const check = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    check.setAttribute('viewBox', '0 0 16 16');
    check.setAttribute('class', 'pm-menu-check');
    check.innerHTML = '<path d="M3.5 8.5l3 3 6-6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>';
    item.append(label, check);
    item.addEventListener('pointermove', () => setActive(i));
    item.addEventListener('click', (event) => { event.stopPropagation(); if (!option.disabled) pick(i); });
    items.push(item);
    el.append(item);
  }

  function glideTo(item: HTMLElement): void {
    // The first appearance lands in place; every move after that glides.
    if (!gliderOn) glider.style.transition = 'none';
    glider.style.transform = `translateY(${item.offsetTop}px)`;
    glider.style.height = `${item.offsetHeight}px`;
    if (!gliderOn) { void glider.offsetHeight; glider.style.transition = ''; }
    gliderOn = true;
    glider.classList.add('on');
  }

  function rest(): void {
    // The highlight leaves with the pointer; the keyboard index stays put.
    items[active]?.removeAttribute('data-active');
    glider.classList.remove('on');
    gliderOn = false;
  }

  function setActive(i: number): void {
    if (i < 0 || i >= items.length) return;
    items[active]?.removeAttribute('data-active');
    active = i;
    const item = items[active]!;
    item.setAttribute('data-active', '');
    item.scrollIntoView({ block: 'nearest' });
    glideTo(item);
  }

  function step(delta: number): void {
    let i = active;
    for (let n = 0; n < items.length; n++) {
      i = (i + delta + items.length) % items.length;
      const option = req.options[i];
      if (option && !option.disabled) { setActive(i); return; }
    }
  }

  let typed = '';
  let typedAt = 0;
  function typeahead(key: string): void {
    const now = performance.now();
    typed = now - typedAt < 600 ? typed + key : key;
    typedAt = now;
    const start = typed.length === 1 ? active + 1 : active;
    for (let n = 0; n < items.length; n++) {
      const i = ((start + n) % items.length + items.length) % items.length;
      const option = req.options[i];
      if (option && option.label.toLowerCase().startsWith(typed.toLowerCase()) && !option.disabled) { setActive(i); return; }
    }
  }

  let closed = false;
  function pick(i: number): void {
    const option = req.options[i];
    if (!option || option.disabled) return;
    close();
    req.onPick(option.value);
  }

  function place(): void {
    const a = req.anchor.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    el.style.minWidth = `${Math.ceil(a.width)}px`;
    el.style.maxHeight = `${MAX_HEIGHT}px`;
    const h = Math.min(el.offsetHeight, MAX_HEIGHT);
    const below = Math.max(0, vh - a.bottom - EDGE - GAP);
    const above = Math.max(0, a.top - EDGE - GAP);
    const side = below >= h || below >= above ? 'bottom' : 'top';
    el.dataset.side = side;
    el.style.maxHeight = `${Math.min(MAX_HEIGHT, side === 'bottom' ? below : above)}px`;
    const top = side === 'bottom' ? a.bottom + GAP : a.top - GAP - el.offsetHeight;
    const left = Math.max(EDGE, Math.min(a.left, vw - el.offsetWidth - EDGE));
    el.style.top = `${Math.round(top)}px`;
    el.style.left = `${Math.round(left)}px`;
  }

  function onKey(event: KeyboardEvent): void {
    event.stopPropagation();
    switch (event.key) {
      case 'ArrowDown': event.preventDefault(); step(1); break;
      case 'ArrowUp': event.preventDefault(); step(-1); break;
      case 'Home': event.preventDefault(); setActive(0); break;
      case 'End': event.preventDefault(); setActive(items.length - 1); break;
      case 'Enter': case ' ': event.preventDefault(); if (!req.options[active]?.disabled) pick(active); break;
      case 'Escape': case 'Tab': event.preventDefault(); close(); break;
      default: if (event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey) typeahead(event.key);
    }
  }

  function onPointerDown(event: PointerEvent): void {
    const t = event.target as Node;
    if (el.contains(t) || req.anchor.contains(t)) return;
    close();
  }

  function close(): void {
    if (closed) return;
    closed = true;
    if (current === handle) current = null;
    document.removeEventListener('pointerdown', onPointerDown, true);
    window.removeEventListener('resize', close);
    window.removeEventListener('blur', close);
    document.removeEventListener('scroll', onScroll, true);
    el.removeEventListener('keydown', onKey);
    const finish = () => { el.remove(); req.onClose?.(); };
    if (reduce) { finish(); return; }
    el.dataset.state = 'closed';
    el.addEventListener('animationend', finish, { once: true });
    // If the animation never fires (display none, tab hidden), still clean up.
    window.setTimeout(finish, 200);
  }

  function onScroll(event: Event): void {
    if (el.contains(event.target as Node)) return;
    close();
  }

  const handle: MenuHandle = { close, el };
  current = handle;

  document.body.append(el);
  place();
  el.dataset.state = 'open';
  // Nothing is lit until the pointer or the arrow keys pick a row; the index still starts on the current value.
  el.addEventListener('pointerleave', rest);
  el.focus({ preventScroll: true });
  el.addEventListener('keydown', onKey);
  document.addEventListener('pointerdown', onPointerDown, true);
  window.addEventListener('resize', close);
  window.addEventListener('blur', close);
  document.addEventListener('scroll', onScroll, true);
  return handle;
}
