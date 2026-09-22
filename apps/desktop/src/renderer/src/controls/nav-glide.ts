/* One highlight glides between sidebar rows, the way the select listbox's
   hover layer does, instead of each row painting its own. At rest it sits on
   the selected row; on hover it follows the pointer and settles back when the
   pointer leaves. Rows keep their text colour change; the fill is the glider. */

export type NavGlideOptions = {
  /** Rows the glider can sit on. */
  row: string;
  /** Marks the selected row. */
  selected: string;
};

const EASE = 'cubic-bezier(.23,1,.32,1)';

export function mountNavGlide(nav: HTMLElement, options: NavGlideOptions): () => void {
  const glider = document.createElement('div');
  glider.className = 'nav-glider';
  glider.setAttribute('aria-hidden', 'true');
  nav.prepend(glider);
  let on = false;
  let hovered: HTMLElement | null = null;

  const rows = () => Array.from(nav.querySelectorAll<HTMLElement>(options.row));
  const selectedRow = () => rows().find((row) => row.matches(options.selected)) ?? null;

  function place(row: HTMLElement | null, instant = false): void {
    if (!row) {
      if (on) glider.classList.remove('on');
      on = false;
      return;
    }
    const navBox = nav.getBoundingClientRect();
    const box = row.getBoundingClientRect();
    const first = !on || instant;
    if (first) glider.style.transition = 'none';
    glider.style.transform = `translate(${Math.round(box.left - navBox.left)}px, ${Math.round(box.top - navBox.top + nav.scrollTop)}px)`;
    glider.style.width = `${Math.round(box.width)}px`;
    glider.style.height = `${Math.round(box.height)}px`;
    if (first) { void glider.offsetHeight; glider.style.transition = ''; }
    if (!on) glider.classList.add('on');
    on = true;
  }

  const settle = (instant = false) => place(hovered ?? selectedRow(), instant);

  const over = (event: PointerEvent) => {
    const row = (event.target as HTMLElement).closest<HTMLElement>(options.row);
    if (!row || !nav.contains(row)) return;
    hovered = row;
    place(row);
  };
  const leave = () => { hovered = null; settle(); };

  nav.addEventListener('pointerover', over);
  nav.addEventListener('pointerleave', leave);
  /* The glider's own writes must not re-enter: only the rows' changes count. */
  const observer = new MutationObserver((records) => {
    if (records.every((record) => record.target === glider)) return;
    settle(!on);
  });
  observer.observe(nav, { attributes: true, subtree: true, childList: true, attributeFilter: ['class', 'aria-current', 'hidden'] });
  const resize = new ResizeObserver(() => settle(true));
  resize.observe(nav);
  nav.style.setProperty('--nav-glide-ease', EASE);
  settle(true);

  return () => {
    nav.removeEventListener('pointerover', over);
    nav.removeEventListener('pointerleave', leave);
    observer.disconnect();
    resize.disconnect();
    glider.remove();
  };
}
