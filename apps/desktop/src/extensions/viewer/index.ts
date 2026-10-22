import type { PowermoveAPI } from 'powermove';

import { createViewerRuntime, viewerPanelOptions } from './viewer';
import { installSourcePreview } from './source-preview';
import { installViewportKeys } from './viewport3d/keymap';
import { paintViewerCard, viewerCardSize, viewerCardTarget } from './library-card';

const VIEWER_STYLES = `
  .canvas-text-input::selection{background:transparent;color:transparent}
  #panel-viewer{background:var(--bg-panel-2)}
  #panel-viewer > .panel-move-handle{
    top:7px;left:8px;transform:none;width:24px;height:24px;border:0;border-radius:0;
    color:var(--tx-3);background:transparent;box-shadow:none
  }
  #panel-viewer > .panel-move-handle:hover,
  #panel-viewer > .panel-move-handle:focus-visible,
  #panel-viewer > .panel-move-handle:active{
    color:var(--tx);background:transparent;box-shadow:none
  }
  #panel-viewer .body{display:flex;flex-direction:column;padding:14px;overflow:hidden}
  #stage{flex:1;min-height:0;position:relative;overflow:hidden}
  #stage-inner{position:absolute;border-radius:0;corner-shape:round;overflow:hidden;box-shadow:none;outline:0}
  [data-preview-corners="rounded"] #stage-inner{border-radius:var(--r-md);corner-shape:var(--ui-corner-smoothing)}
  #gl{display:block;width:100%;height:100%;background:#000}
  #overlay{position:absolute;inset:0;pointer-events:none}
  #composition-recovery{position:absolute;z-index:5;left:50%;top:50%;transform:translate(-50%,-50%);display:flex;align-items:center;gap:10px;padding:7px 8px 7px 11px;border:0;border-radius:var(--r-md);background:var(--bg-float);box-shadow:none;color:var(--tx-3);font-size:var(--fs-xs);white-space:nowrap}
  #composition-recovery[hidden]{display:none}
  #composition-recovery b{padding:4px 7px;border-radius:var(--r-sm);background:var(--accent);color:var(--on-accent);font-weight:var(--fw-medium)}
  #composition-recovery:hover{color:var(--tx);background:var(--bg-row-hi)}
  #composition-recovery:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  #stage.fx-drop-over > #stage-inner{outline:2px solid var(--accent);outline-offset:-2px;box-shadow:0 0 0 4px color-mix(in oklab, var(--accent) 28%, transparent);transition:box-shadow 120ms ease}
`;

export default function activate(api: PowermoveAPI): void {
  // Install at activation time so engine and panels can consume the viewer
  // service even before the panel has mounted.
  const runtime = createViewerRuntime(api);
  const disposeRuntime = runtime.dispose as () => void;
  api.onDispose(() => disposeRuntime());
  api.services.register('viewer', runtime);
  // Blender's 3D viewport keymap; it only applies while the pointer is over a 3D composition.
  const viewportKeys = api.scene3d?.viewport ? installViewportKeys(api, { pointer: () => (runtime as any).viewport3d?.pointer() ?? null }) : null;
  api.onDispose(() => viewportKeys?.dispose());

  // Enter on a single selected text layer starts editing it (Figma).
  api.commands.register({
    id: 'text.editSelected', label: 'Edit selected text', category: 'Text',
    run: () => {
      const selected = api.selection.layers().map((id) => api.model.layer(id)).filter(Boolean);
      const layer = selected.length === 1 ? selected[0] : null;
      if (!layer || layer.type !== 'text' || layer.lock || runtime.textSession) return false;
      return !!runtime.editText?.(layer, { selectAll: true });
    },
  });
  api.keybindings.bind({ key: 'enter', command: 'text.editSelected' });

  /* With the engine on, the live #gl's pixels belong to the engine and do not
     copy, so the card asks for the full composition at the size the live
     viewer renders it (its zoom, density, preview quality or fixed
     resolution, never above the composition) and the paused live viewer's
     motion blur (12 samples), stretched over the clone's #stage-inner.
     `enginePreview` tells the host this callback fills the card, so the
     host's frozen-fork fill stays out of it. The fill outlives a detached
     clone: the Library caches it off-shelf and shows it again as is. */
  const renderLibraryCard = Object.assign(({
    source, clone,
  }: { source: HTMLElement; clone: HTMLElement; width: number; height: number }) => {
    const project = api.project.get();
    const render = (api.render as typeof api.render & {
      renderFrameToAsync?: (time: number, width: number, height: number, options?: Record<string, unknown>) => Promise<HTMLCanvasElement>;
    }).renderFrameToAsync;
    const engine = api.services.get<{ exclusive?: boolean }>('engine.viewer');
    const target = viewerCardTarget(clone);
    if (!project || !render || !engine?.exclusive || !target) return;
    const size = viewerCardSize(target.inner, project, window.devicePixelRatio || 1,
      api.transport.quality, api.transport.previewResolution);
    if (!size) return;
    const composition = (project as { compId?: unknown }).compId;
    const blocked = () => api.transport.playing() || runtime.isNavigating?.();
    const alive = () => source.isConnected && api.project.get() === project
      && (project as { compId?: unknown }).compId === composition
      && api.services.get('engine.viewer') === engine;
    const pause = () => new Promise((resolve) => window.setTimeout(resolve, 200));
    const present = async () => {
      while (alive()) {
        while (alive() && blocked()) await pause();
        if (!alive()) return;
        const time = api.transport.time();
        const revision = api.project.revision();
        const frame = await render(time, size.width, size.height, { mblur: true, mbSamples: 12, priority: 'idle' });
        while (alive() && blocked()) await pause();
        if (!alive()) return;
        if (api.transport.time() === time && api.project.revision() === revision) {
          paintViewerCard(clone, frame);
          return;
        }
        await pause();
      }
    };
    void present().catch((error) => console.warn('Viewer Library preview failed', error));
  }, { enginePreview: true });

  api.panels.register({
    id: 'viewer',
    icon: 'frame',
    ...viewerPanelOptions,
    library: { ...viewerPanelOptions.library, render: renderLibraryCard },
    build(body) {
      // Reuse the live WebGL surface during extension/HMR updates. A new canvas
      // cannot inherit the old GL context, textures, or viewer event bindings.
      if (runtime.stage) {
        const stage = runtime.stage;
        const styles = stage.querySelector('style');
        if (styles) styles.textContent = VIEWER_STYLES;
        const overlay = stage.querySelector('#overlay');
        if (overlay && overlay.parentElement !== stage) stage.appendChild(overlay);
        const toolbar = stage.parentElement?.querySelector('[data-3d-toolbar]');
        body.replaceChildren(...(toolbar ? [toolbar, stage] : [stage]));
        runtime.layout();
        installSourcePreview(api, runtime, stage);
        return;
      }
      const stage = document.createElement('div');
      stage.id = 'stage';

      const inner = document.createElement('div');
      inner.id = 'stage-inner';

      const gl = document.createElement('canvas');
      gl.id = 'gl';
      const overlay = document.createElement('canvas');
      overlay.id = 'overlay';
      inner.append(gl);

      const styles = document.createElement('style');
      styles.textContent = VIEWER_STYLES;
      stage.append(inner, overlay, styles);
      body.replaceChildren(stage);

      runtime.attach(stage);
      installSourcePreview(api, runtime, stage);
    }
  });
}
