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
#tl-comp-tabs{flex:none;height:var(--hdr-h);display:flex;align-items:stretch;gap:2px;padding:0 4px 0 6px;overflow-x:auto;overflow-y:hidden;scrollbar-width:none;background:var(--bg-panel);border-bottom:1px solid var(--line)}
#tl-comp-tabs::-webkit-scrollbar{display:none}
#panel-timeline>.body>#tl-head{top:var(--hdr-h)}
.tl-comp-tab{position:relative;flex:none;max-width:220px;display:flex;align-items:center;gap:8px;padding:0 10px;border:0;background:none;color:var(--tx-3);font:13px/16px var(--f-ui);cursor:default}
.tl-comp-tab:hover{color:var(--tx-2)}
.tl-comp-tab[aria-selected="true"]{color:var(--tx);font-weight:var(--fw-semibold)}
.tl-comp-tab[aria-selected="true"]::after{content:"";position:absolute;left:0;right:0;bottom:0;height:2px;background:var(--accent)}
.tl-comp-tab>span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.tl-comp-tab>button{width:14px;height:14px;margin-right:-2px;flex:none;display:grid;place-items:center;border:0;border-radius:var(--r-xs);background:none;color:var(--tx-4);padding:0}
.tl-comp-tab>button svg{width:10px;height:10px}
.tl-comp-tab[aria-selected="true"]>button{opacity:0}
.tl-comp-tab:hover>button,.tl-comp-tab>button:focus-visible{opacity:1}
.tl-comp-tab>button:hover{color:var(--tx);background:var(--bg-hover)}
`;

export function mountCompTabs(api: PowermoveAPI, host: HTMLElement): () => void {
  const comps = (): Comps | null => api.services.get<Comps>('compositions');
  host.id = 'tl-comp-tabs';
  host.setAttribute('role', 'tablist');
  host.setAttribute('aria-label', 'Open compositions');

  let pendingRender = 0, pendingScroll = 0;
  let previousModel: Comps | null = null;
  let previousTabs = '';
  let previousActive: string | undefined;

  const render = () => {
    const model = comps();
    if (!model) { host.replaceChildren(); previousModel = null; previousTabs = ''; return; }
    const tabs = model.openTabs().map(id => model.get(id)).filter(Boolean) as Array<{ id: string; name: string }>;
    const signature = JSON.stringify(tabs.map(comp => [comp.id, comp.name, model.isOpen(comp.id)]));
    if (model === previousModel && signature === previousTabs) return;
    previousModel = model; previousTabs = signature;
    const active = tabs.find(comp => model.isOpen(comp.id))?.id;
    const reveal = active !== previousActive;
    previousActive = active;
    const closable = tabs.length > 1;
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
        close.innerHTML = api.ui.icon('x');
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
    // A project switch emits several model/history/assets notifications. Do
    // not rebuild and force layout for each one, or scroll offscreen panels.
    // Scrolling this strip alone also preserves the surrounding panel scroll.
    cancelAnimationFrame(pendingScroll);
    if (reveal && tabs.length > 1) pendingScroll = requestAnimationFrame(() => {
      pendingScroll = 0;
      if (!host.isConnected || host.closest('#pm-panel-pool') || host.clientWidth <= 0) return;
      const selected = host.querySelector<HTMLElement>('[aria-selected="true"]');
      if (!selected) return;
      const bounds = selected.getBoundingClientRect();
      const left = bounds.left - host.getBoundingClientRect().left + host.scrollLeft;
      const right = left + bounds.width;
      if (left < host.scrollLeft) host.scrollLeft = left;
      else if (right > host.scrollLeft + host.clientWidth) host.scrollLeft = right - host.clientWidth;
    });
  };

  const schedule = () => {
    if (!pendingRender) pendingRender = requestAnimationFrame(() => { pendingRender = 0; render(); });
  };
  const offComps = comps()?.onChange(schedule);
  const subscription = api.events.on('project:changed', ({ kind }) => { if (kind !== 'values') schedule(); });
  render();
  return () => {
    offComps?.();
    subscription.dispose();
    cancelAnimationFrame(pendingRender);
    cancelAnimationFrame(pendingScroll);
  };
}
