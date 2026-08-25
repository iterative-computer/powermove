/* Ported from js/ui/toolbar.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
const h: any = PM.h;

/* ── interactive viewer tool (selection / hand / zoom) ── */
PM.tool = PM.tool || 'select';
PM.setTool = (t?: any) => { PM.tool = t; PM.bus.emit('tool'); };

/* ── top tool bar ──────────────────────────────────────── */
PM.registerPanel('toolbar', {
  title: 'Tools', headless: true, flush: true, size: 40, noscroll: true,
  build(body?: any) {
    body.id = 'toolbar';
    const tool: any = (id?: any, icon?: any, title?: any, run?: any) => {
      const b: any = h('button.iconbtn.tl', { title, onclick: () => { if (run) run(); else PM.setTool(id); syncTools(); } }, PM.icon(icon));
      b.dataset.tool = id;
      return b;
    };
    const sep: any = () => h('span.tl-sep');

    /* interactive tools (left, AE-style) */
    const selB: any   = tool('select', 'cursor', 'Selection (V)');
    const handB: any  = tool('hand',   'hand',   'Hand — pan view (H)');
    const zoomB: any  = tool('zoom',   'zoom',   'Zoom (Z)');
    /* creation tools */
    const textB: any  = tool('text',  'type',  'New text layer (⌘T)',   () => PM.cmd('newText'));
    const shapeB: any = tool('shape', 'shape', 'New shape layer (⌘⇧Y)', () => PM.cmd('newShape'));
    const solidB: any = tool('solid', 'solid', 'New solid (⌘Y)',        () => PM.cmd('newSolid'));
    const shdrB: any  = tool('shader','wand',  'New shader layer (⌘⇧G)',() => PM.cmd('newShader'));
    const nullB: any  = tool('null',  'frame', 'New null object',       () => PM.cmd('newNull'));
    const camB: any   = tool('camera','cam',   'Import media (⌘I)',     () => PM.cmd('import'));

    body.append(
      selB, handB, zoomB, sep(),
      textB, shapeB, solidB, shdrB, nullB, sep(), camB,
    );

    function syncTools() {
      [selB, handB, zoomB].forEach((b: any) => b.classList.toggle('on', b.dataset.tool === PM.tool));
    }
    PM.bus.on('tool', syncTools);
    syncTools();
  },
});

}
