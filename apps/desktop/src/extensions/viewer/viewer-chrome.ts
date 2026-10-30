import type { PowermoveAPI } from 'powermove';

/* The Composition panel's chrome: the view controls (zoom and preview
   quality) along the bottom. Open compositions are tabs in the Timeline. */
type Comps = {
  get(id: string): { id: string; name: string } | null;
  open(id: string): boolean;
  active?(): string | null;
  parents?(id: string): string[];
  children?(id: string): string[];
};

export const VIEWER_CHROME_STYLES = `
  #panel-viewer .body{padding:0}
  #panel-viewer #stage{margin:0}
  /* Nothing sits above the viewer; its move handle shows only under the pointer. */
  #panel-viewer .panel-move-handle{opacity:0}
  #panel-viewer:hover .panel-move-handle,#panel-viewer .panel-move-handle:focus-visible{opacity:.62}
  /* The strip's controls sit 6px in, the panel's corner radius minus theirs,
     so their corners run parallel to the panel's. */
  #viewer-foot{flex:none;height:var(--hdr-h);display:flex;align-items:center;gap:6px;padding:0 6px;border-top:1px solid var(--line)}
  #viewer-foot > :not(select){position:relative!important;inset:auto!important}
  #viewer-foot > .pm-select{width:auto!important;height:24px!important;gap:6px;padding:0 8px!important;border-radius:calc(var(--r-lg) - 6px)!important;background:var(--bg-field)!important;color:var(--tx-value)!important;font:11px/14px var(--f-ui)!important}
  #viewer-foot > #composition-zoom-trigger{font-family:var(--f-mono)!important}
  #viewer-foot > .pm-select .pm-select-chevron{position:static;width:8px;height:8px;margin:0;color:var(--tx-3)}
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
  body.querySelector(':scope > #viewer-head')?.remove();
  body.querySelector(':scope > #viewer-foot')?.remove();

  const foot = document.createElement('div');
  foot.id = 'viewer-foot';
  const place = () => {
    // The zoom and quality menus are built by the viewer runtime; move them in.
    const zoom = runtime.zoomControl as HTMLElement | undefined;
    const quality = runtime.qualityControl as HTMLElement | undefined;
    // A menu already given its styled trigger moves with it, trigger first.
    const withTrigger = (select: HTMLElement) => {
      const trigger = select.id ? document.getElementById(`${select.id}-trigger`) : null;
      return trigger ? [trigger, select] : [select];
    };
    foot.replaceChildren(...[zoom, quality].filter((item): item is HTMLElement => !!item).flatMap(withTrigger));
    document.getElementById('preview-controls')?.remove();
  };
  body.append(foot);
  place();

  /* Tab, as in After Effects: a mini-flowchart of the compositions that use
     this one and the ones inside it. Click one to open it. */
  let chart: HTMLElement | null = null;
  const closeChart = () => { chart?.remove(); chart = null; };
  const toggleChart = (): boolean => {
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
    const panel = body.closest<HTMLElement>('.panel') ?? body;
    panel.append(chart);
    chart.style.left = '12px';
    chart.style.top = '12px';
    chart.querySelector<HTMLButtonElement>('button')?.focus();
    const outside = (event: PointerEvent) => { if (chart && !chart.contains(event.target as Node)) { closeChart(); document.removeEventListener('pointerdown', outside, true); } };
    setTimeout(() => document.addEventListener('pointerdown', outside, true));
    chart.addEventListener('keydown', (event) => { if (event.key === 'Escape' || event.key === 'Tab') { event.preventDefault(); closeChart(); } });
    return true;
  };
  runtime.toggleFlowchart = toggleChart;

  return () => { closeChart(); foot.remove(); };
}
