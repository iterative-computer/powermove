import type { PowermoveAPI } from 'powermove';

import { createTimelineRuntime, timelinePanelOptions } from './timeline';

/* Vite can update a child runtime module without considering this extension
   entry itself changed. Accept that child explicitly and ask the kernel to
   replace only the mounted timeline extension, preserving the window, panel
   placement, current project, and editing session. */
if (import.meta.hot) {
  import.meta.hot.accept('./timeline', async () => {
    try { await window.PM?.Kernel?.loader?.reload('timeline'); }
    catch (error) {
      console.error('[timeline hot update]', error);
      window.PM?.toast?.('Timeline update failed. Your editing session is still open.');
    }
  });
}

const TIMELINE_STYLES = `
#panel-timeline{flex:0 0 340px}
#panel-timeline>.body{position:relative;display:flex;flex-direction:column;overflow:hidden}
#tl-head{position:absolute;z-index:3;left:0;top:0;width:var(--tl-gutter,266px);height:var(--tl-ruler,28px);display:flex;align-items:center;justify-content:flex-start;gap:4px;padding:0 7px;background:var(--bg-panel);border-right:1px solid var(--line);border-bottom:1px solid var(--line);overflow:hidden}
#tl-head .iconbtn{width:22px;height:22px;flex:none;background:color-mix(in srgb,var(--tx) 5%,var(--bg-panel));border:0;border-radius:var(--r-sm);box-shadow:none}
#tl-head .iconbtn:hover{background:var(--bg-hover)}
#tl-head .iconbtn svg{width:13px;height:13px}
#tl-head[data-density="compact"]{gap:3px;padding-inline:6px}
#tl-head[data-density="compact"] .iconbtn{width:22px;height:22px}
#tl-head[data-density="compact"] .iconbtn svg{width:13px;height:13px}
#tl-head[data-density="compact"] #tl-time{font-size:var(--fs-sm);min-width:0}
#tl-head .tl-group{min-width:0;display:flex;align-items:center;gap:4px}
#tl-head .tl-graph-slot{margin-left:auto}
#tl-head .tl-graph-slot .iconbtn.on{color:var(--on-accent);background:var(--accent)}
#tl-head .tl-graph-slot .iconbtn.on:hover{background:var(--accent-hover)}
#tl-time{font-family:var(--f-mono);font-size:var(--fs-md);letter-spacing:0;color:var(--tx-2);font-variant-numeric:tabular-nums;margin-left:4px;padding:0 2px;white-space:nowrap;cursor:ew-resize}
#tl-time.edit{color:var(--accent)}
#tl-canvas-wrap{flex:1;position:relative;min-height:0;overflow:hidden}
#tl-canvas{position:absolute;inset:0;width:100%;height:100%;display:block}
`;

export function toggleLayerStrips(pm: Record<string, any>): void {
  const layers = pm.selLayers?.() ?? [];
  if (!layers.length) return;
  /* A mixed selection opens together; only an entirely open selection
     collapses. This avoids leaving selected layers in contradictory states. */
  const collapsed = layers.every((layer: any) => !pm.UIState?.getLayerCollapsed?.(layer));
  for (const layer of layers) {
    layer.collapsed = collapsed;
    pm.UIState?.setLayerCollapsed?.(layer, collapsed);
  }
  pm.invalidate?.('timeline');
}

export function splitSelectedLayersAtPlayhead(pm: Record<string, any>): string[] {
  const rightIds: string[] = [];
  pm.hist.do('Split', () => {
    for (const layer of pm.selLayers?.() ?? []) {
      if (pm.time <= layer.from || pm.time >= layer.from + layer.dur) continue;
      const right = pm.cloneLayer(layer);
      // Keyframe times are layer-local. Preserve their composition times
      // when the tail gets a new in point, including keys before the cut.
      const offset = pm.time - layer.from;
      for (const { prop } of pm.allProps(right)) {
        for (const key of prop.kf ?? []) key.t -= offset;
      }
      right.from = pm.time;
      right.dur = layer.from + layer.dur - pm.time;
      if (pm.MediaTiming?.isTimed?.(layer)) right.d.trim = pm.MediaTiming.trimAtStart(layer, pm.time);
      layer.dur = pm.time - layer.from;
      pm.proj.layers.splice(pm.proj.layers.indexOf(layer), 0, right);
      rightIds.push(right.id);
    }
    pm.bus.emit('layers');
    if (rightIds.length) {
      // The project index intentionally avoids rescanning large layer stacks
      // on every lookup. This command mutates the array directly, so retire
      // the old index before resolving the newly-created selection.
      pm.ProjectIndex?.invalidate?.();
      pm.selectLayers(rightIds);
    }
  });
  return rightIds;
}

export default function activate(api: PowermoveAPI): void {
  const pm = api.host.pm as Record<string, any>;
  const timeline = createTimelineRuntime(pm);
  api.commands?.register({
    id: 'split',
    label: 'Split at playhead',
    category: 'Edit',
    kb: '⌘⇧D',
    run: () => splitSelectedLayersAtPlayhead(pm)
  });
  api.commands?.register({
    id: 'toggleLayerStrips',
    label: 'Open / collapse layer strips',
    category: 'Timeline',
    kb: 'M',
    run: () => pm.cmd('revealMasks')
  });
  api.keybindings?.bind({ key: 'm', command: 'toggleLayerStrips', priority: 90 });
  /* Kernel deactivation runs before replacement activation. Capture this
     module instance's disposer so disabling/reloading the extension cannot
     leave its bus, window, observer, or DOM listeners alive. */
  const disposeRuntime = timeline.disposeRuntime;
  api.onDispose?.(() => disposeRuntime());

  api.panels.register({
    id: 'timeline',
    icon: 'timeline',
    ...timelinePanelOptions,
    library: {
      ...timelinePanelOptions.library,
      render({ clone, width, height }) {
        const canvas = clone.querySelector<HTMLCanvasElement>('[data-library-source-id="tl-canvas"]');
        if (!canvas) return;
        const preview = timeline.renderPreview(canvas, canvas.clientWidth || width, canvas.clientHeight || height);
        const head = clone.querySelector<HTMLElement>('[data-library-source-id="tl-head"]');
        head?.style.setProperty('--tl-gutter', `${Math.round(preview.gutter)}px`);
        head?.style.setProperty('--tl-ruler', `${Math.round(preview.ruler)}px`);
      }
    },
    build(body) {
      /* Kernel extension reloads rebuild the body in place. Keep the live move
         handle and its layout-owned listeners instead of deleting it with the
         previous toolbar node; the gutter also delegates its empty drag area
         to the panel header for sessions whose older handle was already lost. */
      const moveHandle = body.querySelector<HTMLElement>('.panel-move-handle');
      const styles = document.createElement('style');
      styles.textContent = TIMELINE_STYLES;
      const head = document.createElement('div');
      head.id = 'tl-head';

      const wrap = document.createElement('div');
      wrap.id = 'tl-canvas-wrap';
      const canvas = document.createElement('canvas');
      canvas.id = 'tl-canvas';
      wrap.appendChild(canvas);

      body.replaceChildren(styles, head, wrap);
      if (moveHandle) head.prepend(moveHandle);

      timeline.attachHead(head);
      timeline.attachCanvas(wrap);
    }
  });
}
