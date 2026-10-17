import type { PowermoveAPI } from 'powermove';

/* After Effects keeps one Timeline tab per open composition. The model
   (open/close/list) lives in the legacy composition registry. */
type Comps = {
  openTabs(): string[];
  get(id: string): { id: string; name: string } | null;
  isOpen(id: string): boolean;
  open(id: string): boolean;
  close(id: string): boolean;
  onChange(listener: () => void): () => void;
};

export const COMP_TABS_STYLES = `
#tl-comp-tabs{flex:none;height:28px;display:flex;align-items:stretch;gap:1px;padding:0 6px;overflow-x:auto;overflow-y:hidden;scrollbar-width:none;background:var(--bg-panel);border-bottom:1px solid var(--line)}
#tl-comp-tabs::-webkit-scrollbar{display:none}
#panel-timeline>.body>#tl-head{top:28px}
.tl-comp-tab{position:relative;flex:none;max-width:200px;display:flex;align-items:center;gap:2px;padding:0 4px 0 10px;border:0;background:none;color:var(--tx-3);font-size:var(--fs-xs);cursor:default}
.tl-comp-tab:hover{color:var(--tx-2)}
.tl-comp-tab[aria-selected="true"]{color:var(--tx)}
.tl-comp-tab[aria-selected="true"]::after{content:"";position:absolute;left:8px;right:8px;bottom:0;height:2px;border-radius:1px;background:var(--accent)}
.tl-comp-tab>span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.tl-comp-tab>button{width:16px;height:16px;flex:none;display:grid;place-items:center;border:0;border-radius:var(--r-xs);background:none;color:inherit;opacity:0;font-size:12px;line-height:1}
.tl-comp-tab:hover>button,.tl-comp-tab[aria-selected="true"]>button,.tl-comp-tab>button:focus-visible{opacity:.7}
.tl-comp-tab>button:hover{opacity:1;background:var(--bg-hover)}
`;

export function mountCompTabs(api: PowermoveAPI, host: HTMLElement): () => void {
  const comps = (): Comps | null => api.services.get<Comps>('compositions');
  host.id = 'tl-comp-tabs';
  host.setAttribute('role', 'tablist');
  host.setAttribute('aria-label', 'Open compositions');

  /* What the strip shows, to skip rebuilding an identical one: the strip is
     asked to render on every project change (three times per edit), and
     each rebuild ended in `scrollIntoView`, a forced layout of the editor
     (docs/engine/perf.md). */
  let shown: { model: Comps; key: string } | null = null;
  const render = () => {
    const model = comps();
    if (!model) { shown = null; host.replaceChildren(); return; }
    const tabs = model.openTabs().map(id => model.get(id)).filter(Boolean) as Array<{ id: string; name: string }>;
    const closable = tabs.length > 1;
    const key = JSON.stringify(tabs.map(comp => [comp.id, comp.name, model.isOpen(comp.id)]));
    if (shown && shown.model === model && shown.key === key && host.childElementCount === tabs.length) return;
    shown = { model, key };
    host.replaceChildren(...tabs.map(comp => {
      const tab = document.createElement('div');
      tab.className = 'tl-comp-tab';
      tab.setAttribute('role', 'tab');
      tab.tabIndex = model.isOpen(comp.id) ? 0 : -1;
      tab.setAttribute('aria-selected', String(model.isOpen(comp.id)));
      tab.dataset.compId = comp.id;
      tab.title = comp.name;
      const label = document.createElement('span');
      label.textContent = comp.name;
      tab.append(label);
      if (closable) {
        const close = document.createElement('button');
        close.type = 'button';
        close.textContent = '×';
        close.title = `Close ${comp.name}`;
        close.setAttribute('aria-label', `Close ${comp.name}`);
        close.addEventListener('click', event => { event.stopPropagation(); model.close(comp.id); render(); });
        tab.append(close);
      }
      tab.addEventListener('pointerdown', event => {
        if (event.button !== 0 || (event.target as Element).closest('button')) return;
        if (!model.isOpen(comp.id)) model.open(comp.id);
      });
      tab.addEventListener('keydown', event => {
        if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); model.open(comp.id); }
      });
      tab.addEventListener('contextmenu', event => {
        event.preventDefault();
        api.ui.menu(tab, [
          { header: comp.name },
          { label: 'New Composition…', kb: '⌘N', run: () => { api.commands.run('newComposition'); } },
          { label: 'Composition Settings…', run: () => { api.commands.run('compositionSettings', comp.id); } },
          ...(closable ? ['-' as const, { label: 'Close', run: () => { model.close(comp.id); render(); } }] : []),
        ]);
      });
      return tab;
    }));
    host.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  };

  const offComps = comps()?.onChange(render);
  const subscription = api.events.on('project:changed', ({ kind }) => { if (kind !== 'values') render(); });
  render();
  return () => {
    offComps?.();
    subscription.dispose();
  };
}
