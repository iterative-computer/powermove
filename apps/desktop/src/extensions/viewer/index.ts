import type { PowermoveAPI } from 'powermove';

import { createViewerRuntime, viewerPanelOptions } from './viewer';
import { installSourcePreview } from './source-preview';
import { mountViewerChrome, VIEWER_CHROME_STYLES } from './viewer-chrome';

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
${VIEWER_CHROME_STYLES}`;

export default function activate(api: PowermoveAPI): void {
  // Install at activation time so engine and panels can consume the viewer
  // service even before the panel has mounted.
  const runtime = createViewerRuntime(api);
  const disposeRuntime = runtime.dispose as () => void;
  api.onDispose(() => disposeRuntime());
  api.services.register('viewer', runtime);
  let disposeChrome: (() => void) | null = null;
  const chrome = (body: HTMLElement) => { disposeChrome?.(); disposeChrome = mountViewerChrome(api, body, runtime); };
  api.onDispose(() => disposeChrome?.());

  // Tab opens the composition mini-flowchart, as in After Effects.
  api.commands.register({
    id: 'composition.flowchart', label: 'Composition Mini-Flowchart', category: 'Composition',
    run: () => {
      // Tab keeps moving keyboard focus between controls; it opens the
      // flowchart only from the canvas surfaces, as in After Effects.
      const target = document.activeElement;
      const surface = !target || target === document.body || target instanceof HTMLCanvasElement || target.id === 'stage';
      if (!surface && !document.getElementById('viewer-flowchart')) return false;
      return !!runtime.toggleFlowchart?.();
    },
  });
  // inFields: Tab is cancelled only when the flowchart actually handles it.
  api.keybindings.bind({ key: 'tab', command: 'composition.flowchart', inFields: true });

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

  api.panels.register({
    id: 'viewer',
    icon: 'frame',
    ...viewerPanelOptions,
    build(body) {
      // Reuse the live WebGL surface during extension/HMR updates. A new canvas
      // cannot inherit the old GL context, textures, or viewer event bindings.
      if (runtime.stage) {
        const stage = runtime.stage;
        const styles = stage.querySelector('style');
        if (styles) styles.textContent = VIEWER_STYLES;
        const overlay = stage.querySelector('#overlay');
        if (overlay && overlay.parentElement !== stage) stage.appendChild(overlay);
        body.replaceChildren(stage);
        chrome(body);
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
      chrome(body);
      installSourcePreview(api, runtime, stage);
    }
  });
}
