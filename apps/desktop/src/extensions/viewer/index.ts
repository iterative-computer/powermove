import type { PowermoveAPI } from 'powermove';

import { createViewerRuntime, viewerPanelOptions } from './viewer';
import { installSourcePreview } from './source-preview';

const VIEWER_STYLES = `
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
  #composition-recovery{position:absolute;z-index:5;left:50%;top:50%;transform:translate(-50%,-50%);display:flex;align-items:center;gap:10px;padding:7px 8px 7px 11px;border:1px solid color-mix(in srgb,var(--line) 72%,white 10%);border-radius:var(--r-md);background:color-mix(in srgb,var(--bg-panel) 92%,transparent);box-shadow:0 10px 32px rgb(0 0 0/.28);backdrop-filter:blur(14px);color:var(--tx-3);font-size:var(--fs-xs);white-space:nowrap}
  #composition-recovery[hidden]{display:none}
  #composition-recovery b{padding:4px 7px;border-radius:var(--r-sm);background:var(--accent);color:var(--on-accent);font-weight:var(--fw-medium)}
  #composition-recovery:hover{color:var(--tx);border-color:color-mix(in srgb,var(--accent) 38%,var(--line))}
  #composition-recovery:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  #stage.fx-drop-over > #stage-inner{outline:2px solid var(--accent);outline-offset:-2px;box-shadow:0 0 0 4px color-mix(in oklab, var(--accent) 28%, transparent);transition:box-shadow 120ms ease}
`;

export default function activate(api: PowermoveAPI): void {
  const PM = api.host.pm as Record<string, any>;

  // Install at activation time so legacy consumers retain the PM.Viewer and
  // PM.setOrKey contracts even before the panel has mounted.
  const runtime = createViewerRuntime(PM, api.space3d);
  const disposeRuntime = runtime.dispose as () => void;
  api.onDispose(() => disposeRuntime());

  api.panels.register({
    id: 'viewer',
    icon: 'frame',
    ...viewerPanelOptions,
    build(body) {
      // Reuse the live WebGL surface during extension/HMR updates. A new canvas
      // cannot inherit the old GL context, textures, or viewer event bindings.
      if (PM.Viewer?.stage) {
        const stage = PM.Viewer.stage as HTMLElement;
        const styles = stage.querySelector('style');
        if (styles) styles.textContent = VIEWER_STYLES;
        const overlay = stage.querySelector('#overlay');
        if (overlay && overlay.parentElement !== stage) stage.appendChild(overlay);
        body.replaceChildren(stage);
        PM.Viewer.layout();
        installSourcePreview(PM, stage);
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

      createViewerRuntime(PM, api.space3d).attach(stage);
      installSourcePreview(PM, stage);
    }
  });
}
