import type { PowermoveAPI } from 'powermove';

import { createViewerRuntime, viewerPanelOptions } from './viewer';

const VIEWER_STYLES = `
  #panel-viewer{background:var(--bg-panel-2)}
  #panel-viewer .body{display:flex;flex-direction:column;padding:14px;overflow:hidden}
  #stage{flex:1;min-height:0;display:grid;place-items:center;position:relative}
  #stage-inner{position:relative;border-radius:0;corner-shape:round;overflow:hidden;box-shadow:none;outline:0}
  [data-preview-corners="rounded"] #stage-inner{border-radius:var(--r-md);corner-shape:var(--ui-corner-smoothing)}
  #gl{display:block;width:100%;height:100%;background:#000}
  #overlay{position:absolute;inset:0;pointer-events:none}
`;

export default function activate(api: PowermoveAPI): void {
  const PM = api.host.pm as Record<string, any>;

  // Install at activation time so legacy consumers retain the PM.Viewer and
  // PM.setOrKey contracts even before the panel has mounted.
  createViewerRuntime(PM);

  api.panels.register({
    id: 'viewer',
    ...viewerPanelOptions,
    build(body) {
      const stage = document.createElement('div');
      stage.id = 'stage';

      const inner = document.createElement('div');
      inner.id = 'stage-inner';

      const gl = document.createElement('canvas');
      gl.id = 'gl';
      const overlay = document.createElement('canvas');
      overlay.id = 'overlay';
      inner.append(gl, overlay);

      const styles = document.createElement('style');
      styles.textContent = VIEWER_STYLES;
      stage.append(inner, styles);
      body.replaceChildren(stage);

      createViewerRuntime(PM).attach(stage);
    }
  });
}
