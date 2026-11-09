import type { PowermoveAPI } from 'powermove';

/* The Composition panel's chrome, as in After Effects: a header with the
   path to the open composition (Untitled › Pre-comp 1) and the 3D transform
   tools, and a footer with the view controls (zoom, preview quality and the
   3D view). Open compositions are also tabs in the Timeline. */
type Comps = {
  get(id: string): { id: string; name: string } | null;
  open(id: string): boolean;
  active?(): string | null;
  parents?(id: string): string[];
  children?(id: string): string[];
  onChange?(listener: () => void): () => void;
};

/** Where the open composition sits: the compositions from a top-level one
 * down to it, plus any you came up from (After Effects keeps those to the
 * right of the current one so you can step back down). */
export type CompTrail = { ids: string[]; index: number };

/**
 * Work out the breadcrumb for `active` from the previous one.
 * - Moving to a composition already on the trail keeps the trail and moves
 *   the current position (up, or back down).
 * - Opening a composition nested in the current one appends it.
 * - Anything else rebuilds the path upward through the compositions that
 *   use it, preferring ones that were on the previous trail.
 * A trail whose links no longer hold (a nested layer was deleted) is
 * cut at the first broken link.
 */
export function compTrail(previous: CompTrail | null, active: string | null | undefined,
  parents: (id: string) => string[], exists: (id: string) => boolean = () => true): CompTrail {
  if (!active) return { ids: [], index: -1 };
  const linked = (ids: string[]): string[] => {
    const kept: string[] = [];
    for (const id of ids) {
      if (!exists(id)) break;
      if (kept.length && !parents(id).includes(kept[kept.length - 1]!)) break;
      kept.push(id);
    }
    return kept;
  };
  const before = previous ? linked(previous.ids) : [];
  const at = before.indexOf(active);
  if (at >= 0) return { ids: before, index: at };
  const current = previous && previous.index >= 0 && previous.index < before.length ? before[previous.index] : undefined;
  if (current && parents(active).includes(current)) {
    const ids = [...before.slice(0, before.indexOf(current) + 1), active];
    return { ids, index: ids.length - 1 };
  }
  const ids = [active];
  const seen = new Set(ids);
  for (let guard = 0; guard < 32; guard++) {
    const up = parents(ids[0]!).filter((id) => !seen.has(id) && exists(id));
    if (!up.length) break;
    const next = up.find((id) => before.includes(id)) ?? up[0]!;
    ids.unshift(next);
    seen.add(next);
  }
  return { ids, index: ids.length - 1 };
}

export const VIEWER_CHROME_STYLES = `
  #panel-viewer .body{padding:0}
  #panel-viewer #stage{margin:0}
  /* The header and footer frame the canvas; neither sits over it. */
  #viewer-head{position:relative;flex:none;height:var(--hdr-h);display:flex;align-items:center;gap:8px;min-width:0;padding:0 6px 0 10px;border-bottom:1px solid var(--line)}
  /* The move handle shows only under the pointer, in the header's left inset
     like every panel's grip, so nothing in the header moves when it appears. */
  #viewer-head > .panel-move-handle{position:absolute;left:0;top:0;bottom:0;width:12px;height:auto;margin:0;z-index:1;opacity:0}
  #viewer-head:hover > .panel-move-handle,#viewer-head > .panel-move-handle:focus-visible{opacity:.58}
  #viewer-path{flex:1 1 auto;min-width:0;display:flex;align-items:center;gap:0;overflow:hidden}
  .viewer-crumb{flex:0 1 auto;min-width:24px;max-width:220px;height:24px;padding:0 6px;border:0;border-radius:var(--r-sm);background:none;color:var(--tx-3);font:13px/24px var(--f-ui);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;cursor:default}
  button.viewer-crumb:hover{color:var(--tx);background:var(--bg-hover)}
  button.viewer-crumb:focus-visible{outline:2px solid var(--accent);outline-offset:-2px}
  .viewer-crumb[aria-current="page"]{flex-shrink:0;max-width:260px;color:var(--tx);font-weight:var(--fw-semibold)}
  .viewer-crumb.ahead{color:var(--tx-4)}
  .viewer-crumb-sep{flex:none;width:12px;height:12px;display:grid;place-items:center;color:var(--tx-4)}
  .viewer-crumb-sep svg{width:9px;height:9px}
  #viewer-head > #composition-3d-controls{position:static!important;flex:none;margin-left:auto}
  /* The strip's controls sit 6px in, the panel's corner radius minus theirs,
     so their corners run parallel to the panel's. */
  #viewer-foot{container:viewer-foot/inline-size;flex:none;height:var(--hdr-h);display:flex;align-items:center;gap:6px;padding:0 6px;border-top:1px solid var(--line)}
  #viewer-foot > :not(select){position:relative!important;inset:auto!important}
  #viewer-foot > .pm-select{width:auto!important;height:24px!important;gap:6px;padding:0 8px!important;border-radius:calc(var(--r-lg) - 6px)!important;background:var(--bg-field)!important;color:var(--tx-value)!important;font:11px/14px var(--f-ui)!important}
  #viewer-foot > #composition-zoom-trigger{font-family:var(--f-mono)!important}
  #viewer-foot > #composition-3d-controls{margin-left:auto}
  #viewer-foot > .pm-select .pm-select-chevron{position:static;width:8px;height:8px;margin:0;color:var(--tx-3)}
  #viewer-foot > #composition-3d-view{margin-left:auto}
  /* Segmented controls and menus in the strips match the zoom menu: 24px tall,
     corners parallel to the panel's. */
  #viewer-head .segmented,#viewer-foot .segmented{padding:2px;gap:1px;border-radius:calc(var(--r-lg) - 6px)}
  #viewer-head .segmented button,#viewer-foot .segmented button{height:20px;padding:0 8px;border-radius:calc(var(--r-lg) - 8px);font-size:11px}
  #composition-3d-view .pm-select{width:100%!important;height:24px!important;gap:6px;padding:0 8px!important;border-radius:calc(var(--r-lg) - 6px)!important;background:var(--bg-field)!important;color:var(--tx-value)!important;font:11px/14px var(--f-ui)!important}
  #composition-3d-view .pm-select .pm-select-chevron{width:8px;height:8px;color:var(--tx-3)}
  #viewer-flowchart{position:absolute;z-index:30;min-width:200px;max-width:320px;padding:8px;border-radius:var(--r-lg);background:var(--bg-float);box-shadow:var(--shadow-lift);font:var(--fs-sm) var(--f-ui);color:var(--tx-2)}
  #viewer-flowchart h4{margin:2px 4px 6px;color:var(--tx-4);font-size:var(--fs-xs);font-weight:var(--fw-medium);text-transform:uppercase;letter-spacing:.06em}
  #viewer-flowchart button{display:flex;align-items:center;gap:8px;width:100%;height:26px;padding:0 8px;border:0;border-radius:calc(var(--r-lg) - 8px);background:none;color:inherit;font:inherit;text-align:left;cursor:pointer}
  #viewer-flowchart button svg{width:14px;height:14px;flex:none}
  #viewer-flowchart button:hover,#viewer-flowchart button:focus-visible{outline:0;background:var(--bg-hover);color:var(--tx)}
  #viewer-flowchart .current{color:var(--tx);font-weight:var(--fw-medium);background:var(--accent-dim)}
  #viewer-flowchart .none{padding:4px 8px;color:var(--tx-4)}
  #viewer-flowchart hr{border:0;border-top:1px solid var(--line-2);margin:6px 2px}
`;

export function mountViewerChrome(api: PowermoveAPI, body: HTMLElement, runtime: any): () => void {
  const comps = (): Comps | null => api.services.get<Comps>('compositions');
  const icon = (name: string) => api.ui.icon(name);
  /* A rebuild keeps the layout-owned move handle and its listeners. */
  const previousHead = body.querySelector<HTMLElement>(':scope > #viewer-head');
  const panel = body.closest<HTMLElement>('.panel');
  const moveHandle = previousHead?.querySelector<HTMLElement>(':scope > .panel-move-handle')
    ?? panel?.querySelector<HTMLElement>(':scope > .panel-move-handle') ?? null;
  previousHead?.remove();
  body.querySelector(':scope > #viewer-foot')?.remove();

  const head = document.createElement('div');
  head.id = 'viewer-head';
  const path = document.createElement('nav');
  path.id = 'viewer-path';
  path.setAttribute('aria-label', 'Composition path');
  head.append(path);
  if (moveHandle) { moveHandle.classList.add('inline'); head.prepend(moveHandle); }

  const foot = document.createElement('div');
  foot.id = 'viewer-foot';
  const place = () => {
    // The zoom and quality menus are built by the viewer runtime; move them in.
    const zoom = runtime.zoomControl as HTMLElement | undefined;
    const quality = runtime.qualityControl as HTMLElement | undefined;
    const view3d = runtime.view3dControls as HTMLElement | undefined;
    const tools3d = runtime.transform3dControls as HTMLElement | undefined;
    // A menu already given its styled trigger moves with it, trigger first.
    const withTrigger = (select: HTMLElement) => {
      const trigger = select.id ? document.getElementById(`${select.id}-trigger`) : null;
      return trigger ? [trigger, select] : [select];
    };
    foot.replaceChildren(...[zoom, quality].filter((item): item is HTMLElement => !!item).flatMap(withTrigger));
    if (view3d) foot.append(view3d);
    if (tools3d) head.append(tools3d);
    document.getElementById('preview-controls')?.remove();
  };
  body.prepend(head);
  body.append(foot);
  place();

  /* The path from a top-level composition to the open one. Each step up is a
     button; the open composition is emphasized; compositions you came up
     from stay to its right, dimmed, so you can step back down. */
  let trail: CompTrail | null = null;
  let signature = '';
  const renderPath = () => {
    const model = comps();
    const active = model?.active?.() ?? null;
    if (!model || !active) { path.replaceChildren(); signature = ''; trail = null; return; }
    trail = compTrail(trail, active, (id) => model.parents?.(id) ?? [], (id) => !!model.get(id));
    const names = trail.ids.map((id) => model.get(id)?.name ?? 'Composition');
    const next = JSON.stringify([trail, names]);
    if (next === signature) return;
    signature = next;
    const nodes: Node[] = [];
    trail.ids.forEach((id, index) => {
      if (index) {
        const sep = document.createElement('span');
        sep.className = 'viewer-crumb-sep';
        sep.setAttribute('aria-hidden', 'true');
        sep.innerHTML = icon('chev');
        nodes.push(sep);
      }
      const name = names[index]!;
      if (index === trail!.index) {
        const current = document.createElement('span');
        current.className = 'viewer-crumb';
        current.setAttribute('aria-current', 'page');
        current.dataset.compId = id;
        current.textContent = name;
        current.title = name;
        current.addEventListener('contextmenu', (event) => {
          event.preventDefault();
          api.ui.menu(current, [
            { header: name },
            { label: 'Composition Settings…', run: () => { api.commands.run('compositionSettings', id); } },
            { label: 'Composition Mini-Flowchart', kb: 'Tab', run: () => { toggleChart(current); } },
          ]);
        });
        nodes.push(current);
        return;
      }
      const crumb = document.createElement('button');
      crumb.type = 'button';
      crumb.className = index > trail!.index ? 'viewer-crumb ahead' : 'viewer-crumb';
      crumb.dataset.compId = id;
      crumb.textContent = name;
      crumb.title = index > trail!.index ? `${name} — Open the composition you came up from` : `${name} — Open the composition this one is nested in`;
      crumb.addEventListener('click', () => { model.open(id); });
      nodes.push(crumb);
    });
    path.replaceChildren(...nodes);
  };
  let pendingPath = 0;
  const schedulePath = () => {
    if (!pendingPath) pendingPath = requestAnimationFrame(() => { pendingPath = 0; renderPath(); });
  };
  const offComps = comps()?.onChange?.(schedulePath);
  const projectSubscription = api.events.on('project:changed', ({ kind }) => { if (kind !== 'values') schedulePath(); });
  renderPath();

  /* Tab, as in After Effects: a mini-flowchart of the compositions that use
     this one and the ones inside it. Click one to open it. */
  let chart: HTMLElement | null = null;
  const closeChart = () => { chart?.remove(); chart = null; };
  const toggleChart = (anchor?: HTMLElement): boolean => {
    if (chart) { closeChart(); return true; }
    const model = comps();
    const active = model?.active?.();
    if (!model || !active) return false;
    chart = document.createElement('div');
    chart.id = 'viewer-flowchart';
    chart.setAttribute('role', 'dialog');
    chart.setAttribute('aria-label', 'Composition flowchart');
    const item = (id: string, current = false) => {
      const button = document.createElement('button');
      button.type = 'button';
      if (current) button.className = 'current';
      button.innerHTML = icon('layers');
      button.append(model.get(id)?.name ?? 'Composition');
      button.addEventListener('click', () => { closeChart(); if (!current) model.open(id); });
      return button;
    };
    const group = (title: string, ids: string[], empty: string) => {
      const heading = document.createElement('h4');
      heading.textContent = title;
      const rows = ids.length ? ids.map((id) => item(id)) : [Object.assign(document.createElement('div'), { className: 'none', textContent: empty })];
      return [heading, ...rows];
    };
    chart.append(
      ...group('Used in', model.parents?.(active) ?? [], 'Not nested anywhere'),
      document.createElement('hr'), item(active, true), document.createElement('hr'),
      ...group('Contains', model.children?.(active) ?? [], 'No nested compositions'),
    );
    const host = panel ?? body;
    host.append(chart);
    // Open under the composition's name in the header.
    const origin = (anchor ?? path.querySelector<HTMLElement>('[aria-current="page"]'))?.getBoundingClientRect();
    const frame = host.getBoundingClientRect();
    chart.style.left = `${Math.max(8, origin ? Math.round(origin.left - frame.left) : 12)}px`;
    chart.style.top = `${Math.max(8, Math.round(head.getBoundingClientRect().bottom - frame.top) + 6)}px`;
    chart.querySelector<HTMLButtonElement>('button')?.focus();
    const outside = (event: PointerEvent) => { if (chart && !chart.contains(event.target as Node)) { closeChart(); document.removeEventListener('pointerdown', outside, true); } };
    setTimeout(() => document.addEventListener('pointerdown', outside, true));
    chart.addEventListener('keydown', (event) => { if (event.key === 'Escape' || event.key === 'Tab') { event.preventDefault(); closeChart(); } });
    return true;
  };
  runtime.toggleFlowchart = () => toggleChart();

  return () => {
    closeChart();
    offComps?.();
    projectSubscription.dispose();
    cancelAnimationFrame(pendingPath);
    // Hand the move handle back to the panel before the header goes.
    if (moveHandle && moveHandle.parentElement === head && panel) { moveHandle.classList.remove('inline'); panel.append(moveHandle); }
    head.remove();
    foot.remove();
  };
}
