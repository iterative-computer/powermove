import type { PowermoveAPI } from 'powermove';

import { createTimelineRuntime, timelinePanelOptions } from './timeline';

const TIMELINE_STYLES = `
#panel-timeline{flex:0 0 340px}
#panel-timeline>.body{display:flex;flex-direction:column;overflow:hidden}
#tl-head{height:var(--hdr-h);flex:none;display:flex;align-items:center;justify-content:space-between;gap:10px;padding:0 10px;border-bottom:1px solid var(--line)}
#tl-head .iconbtn{width:26px;height:26px}
#tl-head .iconbtn svg{width:14px;height:14px}
#tl-head[data-density="compact"]{height:32px;gap:6px;padding-inline:8px}
#tl-head[data-density="compact"] .iconbtn{width:24px;height:24px}
#tl-head[data-density="compact"] .iconbtn svg{width:13px;height:13px}
#tl-head[data-density="compact"] #tl-time{font-size:var(--fs-sm);min-width:0}
#tl-head[data-density="compact"] input[type="range"]{width:64px}
#tl-head .tl-group{min-width:0;display:flex;align-items:center;gap:4px}
#tl-head .tl-view{margin-left:auto;gap:5px}
#tl-time{font-family:var(--f-mono);font-size:var(--fs-md);letter-spacing:0;color:var(--tx);font-variant-numeric:tabular-nums;margin-left:6px;padding:0 2px}
#tl-time.edit{color:var(--accent)}
#tl-head .zoomrow{display:flex;align-items:center;color:var(--tx-3)}
#tl-head input[type="range"]{-webkit-appearance:none;appearance:none;height:3px;border-radius:3px;background:var(--ink-3);width:96px}
#tl-head input[type="range"]::-webkit-slider-thumb{-webkit-appearance:none;width:11px;height:11px;border-radius:50%;background:var(--bg-float);box-shadow:var(--ctl-edge)}
#tl-canvas-wrap{flex:1;position:relative;min-height:0;overflow:hidden}
#tl-canvas{position:absolute;inset:0;width:100%;height:100%;display:block}
`;

export default function activate(api: PowermoveAPI): void {
  const pm = api.host.pm as Record<string, any>;
  const timeline = createTimelineRuntime(pm);

  api.panels.register({
    id: 'timeline',
    icon: 'timeline',
    ...timelinePanelOptions,
    build(body) {
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

      timeline.attachHead(head);
      timeline.attachCanvas(wrap);
    }
  });
}
