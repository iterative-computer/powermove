import type { PowermoveAPI } from 'powermove';
import { addTimelineMarker, markerAtTime, removeTimelineMarker } from './markers';

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
#tl-head{position:absolute;z-index:3;left:0;top:0;width:var(--tl-gutter,494px);height:var(--tl-ruler,32px);display:flex;align-items:center;justify-content:flex-start;gap:8px;padding:0 10px;background:var(--bg-panel);border-right:1px solid var(--line);overflow:hidden}
/* The move handle shows only under the pointer; at rest the row is play, time and the graph switch. */
#tl-head>.tl-gutter-grip{position:absolute;top:0;right:0;bottom:0;width:5px;cursor:col-resize}
#tl-head>.tl-gutter-grip[hidden]{display:none}
#tl-head>.panel-move-handle{opacity:0;margin-right:-4px}
#tl-head:hover>.panel-move-handle,#tl-head>.panel-move-handle:focus-visible{opacity:.58}
#tl-head .iconbtn{width:24px;height:24px;flex:none;background:none;border:0;border-radius:var(--r-sm);box-shadow:none;color:var(--tx-2)}
#tl-head .iconbtn:hover{background:var(--bg-hover);color:var(--tx)}
#tl-head .iconbtn svg{width:14px;height:14px}
#tl-head .tl-transport .iconbtn{color:var(--tx-label)}
#tl-head .tl-transport .iconbtn svg{width:12px;height:12px}
#tl-head .tl-group{min-width:0;display:flex;align-items:center;gap:8px}
#tl-head .tl-graph-slot{margin-left:auto;gap:2px}
#tl-head .tl-graph-slot .iconbtn.on{color:var(--accent-tx);background:var(--accent-dim)}
#tl-time{font:13px/16px var(--f-mono);letter-spacing:0;color:var(--accent-tx);font-variant-numeric:tabular-nums;white-space:nowrap;cursor:ew-resize}
#tl-time.edit{color:var(--accent)}
#tl-canvas-wrap{flex:1;position:relative;min-height:0;overflow:hidden}
.tl-empty{position:absolute;left:0;right:0;bottom:0;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:12px;pointer-events:none}
.tl-empty[hidden]{display:none}
.tl-empty-title{color:var(--tx-2);font-size:var(--fs-md);font-weight:var(--fw-medium)}
.tl-empty-actions{display:flex;flex-wrap:wrap;justify-content:center;gap:6px;pointer-events:auto}
.tl-empty-button{display:flex;align-items:center;gap:7px;height:30px;padding:0 10px;border:0;border-radius:var(--r-md);background:var(--ink-1);color:var(--tx-2);font:var(--fs-sm) var(--f-ui);cursor:pointer}
.tl-empty-button:hover{background:var(--ink-2);color:var(--tx)}
.tl-empty-button svg{width:14px;height:14px}
.tl-empty-button kbd{color:var(--tx-4);font:var(--fs-xs) var(--f-mono)}
.tl-empty-hint{color:var(--tx-4);font-size:var(--fs-xs)}
#tl-canvas{position:absolute;inset:0;width:100%;height:100%;display:block}
#tl-canvas-wrap .tl-caption-editor{position:absolute;z-index:9;margin:0;padding:3px 7px;border:0;border-radius:var(--r-sm);background:var(--bg-float);color:var(--tx);font-family:inherit;font-size:11.5px;line-height:16px;resize:none;overflow:hidden;outline:0;box-shadow:var(--shadow-float,0 4px 14px rgb(0 0 0 / .18)),inset 0 0 0 1px var(--line);cursor:text}
${COMP_TABS_STYLES}
#tl-comp-tabs{padding-right:168px}
/* Centred in the tab row; inset 4px with its radius 4px smaller than the panel's, so the curves run parallel. */
#tl-mode{position:absolute;z-index:4;top:4px;right:4px;height:28px;display:flex;align-items:stretch;padding:2px;gap:2px;border-radius:var(--r-md);background:rgb(0 0 0 / .19)}
#tl-mode .tl-mode-button{display:flex;align-items:center;padding:0 10px;border:0;border-radius:calc(var(--r-md) - 2px);background:none;color:var(--tx-2);font:12px/16px var(--f-ui);cursor:default}
#tl-mode .tl-mode-button svg{display:none}
#tl-mode .tl-mode-button:hover{color:var(--tx)}
#tl-mode .tl-mode-button.on{color:var(--tx);font-weight:var(--fw-medium);background:var(--bg-float)}`;

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
  api.commands.register({ id: 'timeline.addMarker', label: 'Add marker', category: 'Timeline', run: () => addTimelineMarker(api) });
  api.commands.register({
    id: 'timeline.removeMarker', label: 'Remove marker at playhead', category: 'Timeline',
    when: () => !!markerAtTime(api, api.transport.time()),
    run: () => { const marker = markerAtTime(api, api.transport.time()); if (marker) removeTimelineMarker(api, marker); },
  });
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
