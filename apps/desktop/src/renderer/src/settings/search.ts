const ROW = '.settings-row, .settings-extension-row';
const SECTION = '.sg-section, .settings-section';
const EXCLUDED = 'script, style, svg, input, textarea, select, [hidden], [aria-hidden="true"], [role="listbox"], [data-slot="smooth-corners-effects"]';

/** Search rendered copy without rewriting text owned by Svelte or live controls. */
function textNodes(root: HTMLElement): Text[] {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const nodes: Text[] = [];
  while (walker.nextNode()) {
    const node = walker.currentNode as Text;
    if (node.textContent?.trim() && !node.parentElement?.closest(EXCLUDED) && node.parentElement?.checkVisibility()) nodes.push(node);
  }
  return nodes;
}

export function clearSettingsSearch(root: HTMLElement): void {
  root.querySelectorAll('[data-settings-search-hidden]').forEach(el => el.removeAttribute('data-settings-search-hidden'));
  CSS.highlights.delete('settings-search');
}

export function searchSettings(root: HTMLElement, query: string): { pages: string[]; count: number } {
  clearSettingsSearch(root);
  const pages: string[] = [];
  let count = 0;
  const matches = (el: HTMLElement | null) => !!el && textNodes(el).some(node => node.data.toLowerCase().includes(query));
  const hide = (el: HTMLElement, visible: boolean) => {
    if (!visible) el.setAttribute('data-settings-search-hidden', '');
  };

  for (const page of root.querySelectorAll<HTMLElement>('.sg-page')) {
    if (!query) {
      pages.push(page.dataset.settingsPage!);
      continue;
    }
    const pageMatch = Array.from(page.querySelectorAll<HTMLElement>('.sg-heading h2, .settings-extension-head')).some(matches);
    const sections = Array.from(page.querySelectorAll<HTMLElement>(SECTION)).filter(el => !el.closest('[hidden]'));
    const matchingSections = new Set(sections.filter(section => matches(section.querySelector('.sg-section-title, .settings-section-heading'))));
    const rows = Array.from(page.querySelectorAll<HTMLElement>(ROW)).filter(el => !el.closest('[hidden]'));
    const matchingRows = new Set(rows.filter(row => pageMatch || matchingSections.has(row.closest(SECTION)!) || matches(row)));
    for (const row of rows) hide(row, matchingRows.has(row));
    for (const section of sections) {
      hide(section, pageMatch || matchingSections.has(section) || [...matchingRows].some(row => section.contains(row))
        || (!section.querySelector(ROW) && matches(section)));
    }
    const visible = pageMatch || matchingRows.size > 0 || matchingSections.size > 0 || (rows.length === 0 && matches(page));
    hide(page, visible);
    if (visible) {
      pages.push(page.dataset.settingsPage!);
      count += matchingRows.size || 1;
    }
  }

  if (query) {
    const highlight = new Highlight();
    for (const node of textNodes(root)) {
      if (node.parentElement?.closest('[data-settings-search-hidden]')) continue;
      const value = node.data.toLowerCase();
      for (let start = value.indexOf(query); start !== -1; start = value.indexOf(query, start + query.length)) {
        const range = new Range();
        range.setStart(node, start);
        range.setEnd(node, start + query.length);
        highlight.add(range);
      }
    }
    CSS.highlights.set('settings-search', highlight);
  }
  return { pages, count };
}
