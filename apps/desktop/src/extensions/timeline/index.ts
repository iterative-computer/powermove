import type { PowermoveAPI } from 'powermove';

import { createTimelineRuntime, timelinePanelOptions } from './timeline';
import { adjacentKeyframe } from './keyframe-navigation';
import { createPropertyReveal, propertyShortcuts } from './property-reveal';
import { selectedLayers, trimAtStart } from './api-helpers';
import { COMP_TABS_STYLES, mountCompTabs } from './comp-tabs';

let activeApi: PowermoveAPI | null = null;

/* Vite can update a child runtime module without considering this extension
   entry itself changed. Accept that child explicitly and ask the kernel to
   replace only the mounted timeline extension, preserving the window, panel
   placement, current project, and editing session. */
if (import.meta.hot) {
  import.meta.hot.accept('./timeline', async () => {
    try { await activeApi?.extensions.reload('timeline'); }
    catch (error) {
      console.error('[timeline hot update]', error);
      activeApi?.ui.toast('Timeline update failed. Your editing session is still open.');
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
#tl-head .tl-graph-slot{gap:1px}
#tl-head .tl-graph-slot>.iconbtn:not(.tl-view-button){margin-right:5px}
#tl-head .tl-view-button,#tl-head[data-density] .tl-view-button{width:26px;border-radius:0;color:var(--tx-2)}
#tl-head .tl-view-button svg,#tl-head[data-density] .tl-view-button svg{width:15px;height:15px}
#tl-head .tl-view-button.first{border-radius:var(--r-sm) 0 0 var(--r-sm)}
#tl-head .tl-view-button.last{border-radius:0 var(--r-sm) var(--r-sm) 0}
#tl-head .tl-view-button:not(.on):hover{color:var(--tx)}
#tl-head .tl-graph-slot .tl-view-button.on{color:var(--on-accent);background:var(--accent)}
#tl-time{font-family:var(--f-mono);font-size:var(--fs-md);letter-spacing:0;color:var(--tx-2);font-variant-numeric:tabular-nums;margin-left:4px;padding:0 2px;white-space:nowrap;cursor:ew-resize}
#tl-time.edit{color:var(--accent)}
#tl-canvas-wrap{flex:1;position:relative;min-height:0;overflow:hidden}
#tl-canvas{position:absolute;inset:0;width:100%;height:100%;display:block}
#tl-canvas-wrap .tl-caption-editor{position:absolute;z-index:9;margin:0;padding:3px 7px;border:0;border-radius:var(--r-sm);background:var(--bg-float);color:var(--tx);font-family:inherit;font-size:11.5px;line-height:16px;resize:none;overflow:hidden;outline:0;box-shadow:var(--shadow-float,0 4px 14px rgb(0 0 0 / .18)),inset 0 0 0 1px var(--line);cursor:text}
${COMP_TABS_STYLES}
#tl-comp-tabs{padding-right:168px}
#tl-mode{position:absolute;z-index:4;top:4px;right:8px;height:20px;display:flex;align-items:stretch;padding:1px;gap:1px;border-radius:var(--r-sm);background:color-mix(in srgb,var(--tx) 6%,var(--bg-panel))}
#tl-mode .tl-mode-button{display:flex;align-items:center;gap:5px;padding:0 9px;border:0;border-radius:calc(var(--r-sm) - 1px);background:none;color:var(--tx-3);font:inherit;font-size:var(--fs-xs);line-height:1;cursor:default}
#tl-mode .tl-mode-button svg{width:12px;height:12px;flex:none}
#tl-mode .tl-mode-button:hover{color:var(--tx)}
#tl-mode .tl-mode-button.on{color:var(--tx);background:var(--bg-panel);box-shadow:0 0 0 1px var(--line)}`;

export function toggleLayerStrips(api: PowermoveAPI): void {
  const layers = selectedLayers(api);
  if (!layers.length) return;
  /* A mixed selection opens together; only an entirely open selection
     collapses. This avoids leaving selected layers in contradictory states. */
  const collapsed = layers.every((layer) => !api.uiState.getLayerCollapsed(layer));
  for (const layer of layers) {
    layer.collapsed = collapsed;
    api.uiState.setLayerCollapsed(layer, collapsed);
    if (collapsed && layer.type === 'group') api.uiState.setGroupCollapsed(layer, true);
  }
  api.transport.invalidate('timeline');
}

export function splitSelectedLayersAtPlayhead(api: PowermoveAPI): string[] {
  const rightIds: string[] = [];
  api.history.do('Split', () => {
    for (const layer of selectedLayers(api)) {
      if (api.transport.time() <= layer.from || api.transport.time() >= layer.from + layer.dur) continue;
      const right = api.model.cloneLayer(layer);
      // Keyframe times are layer-local. Preserve their composition times
      // when the tail gets a new in point, including keys before the cut.
      const offset = api.transport.time() - layer.from;
      for (const { prop } of api.anim.allProps(right)) {
        for (const key of prop.kf ?? []) key.t -= offset;
      }
      const cut = api.transport.time();
      const tailContent = api.media.timing.isTimed(layer)
        ? { trim: trimAtStart(api, layer, cut) }
        : api.media.timing.startPatch(layer, cut);
      const headContent = api.media.timing.endPatch(layer, cut);
      right.from = cut;
      right.dur = layer.from + layer.dur - cut;
      if (tailContent) Object.assign(right.d as Record<string, unknown>, tailContent);
      if (headContent) Object.assign(layer.d as Record<string, unknown>, headContent);
      layer.dur = cut - layer.from;
      api.project.get().layers.splice(api.project.get().layers.indexOf(layer), 0, right);
      rightIds.push(right.id);
    }
    api.anim.touch();
  });
  // history.do publishes the structural change and retires the project index;
  // resolve the new ids only after that transaction has closed.
  if (rightIds.length) api.selection.select(rightIds);
  return rightIds;
}

export default function activate(api: PowermoveAPI): void {
  activeApi = api;
  const timeline = createTimelineRuntime(api);
  const revealProperty = createPropertyReveal(api);
  for (const [key, , label] of propertyShortcuts) {
    api.commands?.register({
      id: `timeline.revealProperty:${key}`,
      label: `Reveal ${label}`,
      category: 'Timeline',
      run: (shift?: unknown) => revealProperty(key, shift === true)
    });
  }
  api.commands.register({
    id: 'timeline.revealAll',
    label: 'Reveal all properties',
    category: 'Timeline',
    run: () => revealProperty('all')
  });
  for (const [direction, step] of [['prev', -1], ['next', 1]] as const) {
    api.commands?.register({
      id: `timeline.adjacentKeyframe:${direction}`,
      label: `${direction === 'prev' ? 'Previous' : 'Next'} adjacent keyframe`,
      category: 'Timeline',
      run: () => {
        const time = adjacentKeyframe(api, step, undefined, api.space3d);
        if (time != null) api.transport.setTime(time);
      }
    });
  }
  api.commands?.register({
    id: 'toggleLayerStrips',
    label: 'Reveal mask properties',
    category: 'Timeline',
    kb: 'M',
    run: () => revealProperty('m')
  });
  api.keybindings?.bind({ key: 'm', command: 'toggleLayerStrips', priority: 90 });
  api.commands.register({
    id: 'timeline.mode:layers',
    label: 'Use layer timeline (After Effects)',
    category: 'Timeline',
    run: () => timeline.setMode('layers'),
  });
  api.commands.register({
    id: 'timeline.mode:tracks',
    label: 'Use track timeline (Premiere)',
    category: 'Timeline',
    run: () => timeline.setMode('tracks'),
  });
  api.commands.register({
    id: 'timeline.toggleMode',
    label: 'Toggle layer / track timeline',
    category: 'Timeline',
    run: () => timeline.setMode(timeline.mode === 'tracks' ? 'layers' : 'tracks'),
  });
  /* Kernel deactivation runs before replacement activation. Capture this
     module instance's disposer so disabling/reloading the extension cannot
     leave its bus, window, observer, or DOM listeners alive. */
  const disposeRuntime = timeline.disposeRuntime;
  let disposeTabs: (() => void) | null = null;
  api.onDispose(() => {
    if (activeApi === api) activeApi = null;
    disposeTabs?.();
    disposeRuntime();
  });
  api.services.register('timeline', timeline);

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

      const tabs = document.createElement('div');
      const modes = document.createElement('div');
      body.replaceChildren(styles, tabs, modes, head, wrap);
      if (moveHandle) head.prepend(moveHandle);
      disposeTabs?.();
      disposeTabs = mountCompTabs(api, tabs);

      timeline.attachHead(head);
      timeline.attachCanvas(wrap);
      timeline.attachModeSwitch(modes);
    }
  });
}
