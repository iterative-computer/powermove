// A menu opened from a button. Native menus are for right-click; a button's
// menu is ours, drawn as the select listbox is (select/menu.ts): a raised
// sheet, 30px rows on a 6px corner, one hover layer gliding between rows, and
// the same fade, scale and slide from the trigger's edge. It may carry a
// header above its rows, such as who you are signed in as.

export type PopoverMenuItem = '-' | { label: string; run: () => void };

export interface PopoverMenuRequest {
  anchor: HTMLElement;
  items: PopoverMenuItem[];
  /** Accessible name for the menu. */
  label: string;
  /** Shown above the rows; not an item. */
  header?: HTMLElement;
  /** Which edge of the anchor to open from, when there is room. */
  side?: 'top' | 'bottom';
  onClose?: () => void;
}

export interface PopoverMenuHandle {
  close(): void;
  readonly el: HTMLElement;
}

const GAP = 4;
const EDGE = 8;

let current: PopoverMenuHandle | null = null;

export function openPopoverMenu(req: PopoverMenuRequest): PopoverMenuHandle {
  current?.close();

  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const el = document.createElement('div');
  el.className = 'pm-menu pm-popover';
  el.setAttribute('role', 'menu');
  el.setAttribute('aria-label', req.label);
  el.tabIndex = -1;

  const glider = document.createElement('div');
  glider.className = 'pm-menu-glider';
  glider.setAttribute('aria-hidden', 'true');
  el.append(glider);
  let gliderOn = false;
  if (req.header) {
    req.header.classList.add('pm-menu-header');
    el.append(req.header);
  }

  const rows: Array<{ item: HTMLElement; run: () => void }> = [];
  for (const entry of req.items) {
    if (entry === '-') {
      const sep = document.createElement('div');
      sep.className = 'pm-menu-sep';
      sep.setAttribute('role', 'separator');
      el.append(sep);
      continue;
    }
    const index = rows.length;
    const item = document.createElement('div');
    item.className = 'pm-menu-item';
    item.setAttribute('role', 'menuitem');
    item.tabIndex = -1;
    const label = document.createElement('span');
    label.className = 'pm-menu-label';
    label.textContent = entry.label;
    item.append(label);
    item.addEventListener('pointermove', () => setActive(index));
    item.addEventListener('click', (event) => { event.stopPropagation(); pick(index); });
    rows.push({ item, run: entry.run });
    el.append(item);
  }
  let active = -1;

  function glideTo(item: HTMLElement): void {
    if (!gliderOn) glider.style.transition = 'none';
    glider.style.transform = `translateY(${item.offsetTop}px)`;
    glider.style.height = `${item.offsetHeight}px`;
    if (!gliderOn) { void glider.offsetHeight; glider.style.transition = ''; }
    gliderOn = true;
    glider.classList.add('on');
  }

  function setActive(i: number): void {
    const row = rows[i];
    if (!row) return;
    rows[active]?.item.removeAttribute('data-active');
    active = i;
    row.item.setAttribute('data-active', '');
    glideTo(row.item);
  }

  // The highlight leaves with the pointer, as it does in the listbox.
  function rest(): void {
    rows[active]?.item.removeAttribute('data-active');
    active = -1;
    glider.classList.remove('on');
    gliderOn = false;
  }

  let closed = false;
  function pick(i: number): void {
    const row = rows[i];
    if (!row) return;
    close();
    row.run();
  }

  function place(): void {
    const a = req.anchor.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    el.style.minWidth = `${Math.ceil(a.width)}px`;
    const h = el.offsetHeight;
    const fitsTop = a.top - GAP - EDGE >= h;
    const fitsBottom = vh - a.bottom - GAP - EDGE >= h;
    const side = (req.side ?? 'bottom') === 'top' ? (fitsTop || !fitsBottom ? 'top' : 'bottom') : (fitsBottom || !fitsTop ? 'bottom' : 'top');
    el.dataset.side = side;
    const top = side === 'bottom' ? a.bottom + GAP : a.top - GAP - h;
    const left = Math.max(EDGE, Math.min(a.left, vw - el.offsetWidth - EDGE));
    el.style.top = `${Math.round(Math.max(EDGE, top))}px`;
    el.style.left = `${Math.round(left)}px`;
  }

  function onKey(event: KeyboardEvent): void {
    event.stopPropagation();
    const step = (delta: number) => setActive(active < 0 ? (delta > 0 ? 0 : rows.length - 1) : (active + delta + rows.length) % rows.length);
    switch (event.key) {
      case 'ArrowDown': event.preventDefault(); step(1); break;
      case 'ArrowUp': event.preventDefault(); step(-1); break;
      case 'Home': event.preventDefault(); setActive(0); break;
      case 'End': event.preventDefault(); setActive(rows.length - 1); break;
      case 'Enter': case ' ': event.preventDefault(); if (active >= 0) pick(active); break;
      case 'Escape': event.preventDefault(); close(); req.anchor.focus({ preventScroll: true }); break;
      case 'Tab': event.preventDefault(); close(); break;
    }
  }

  // A press on the trigger belongs to the trigger, which closes the menu itself.
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
    el.removeEventListener('keydown', onKey);
    req.onClose?.();
    let finished = false;
    const finish = () => { if (!finished) { finished = true; el.remove(); } };
    if (reduce) { finish(); return; }
    el.dataset.state = 'closed';
    el.addEventListener('animationend', finish, { once: true });
    window.setTimeout(finish, 200);
  }

  const handle: PopoverMenuHandle = { close, el };
  current = handle;

  document.body.append(el);
  place();
  el.dataset.state = 'open';
  el.addEventListener('pointerleave', rest);
  el.focus({ preventScroll: true });
  el.addEventListener('keydown', onKey);
  document.addEventListener('pointerdown', onPointerDown, true);
  window.addEventListener('resize', close);
  window.addEventListener('blur', close);
  return handle;
}
