/* Powermove — After Effects-style tool toolbar + Preview panel. */
(() => {
const PM = window.PM, h = PM.h;

/* ── interactive viewer tool (selection / hand / zoom) ── */
PM.tool = PM.tool || 'select';
PM.setTool = (t) => { PM.tool = t; PM.bus.emit('tool'); };

/* ── top tool bar ──────────────────────────────────────── */
PM.registerPanel('toolbar', {
  title: 'Tools', headless: true, flush: true, size: 40, noscroll: true,
  build(body) {
    body.id = 'toolbar';
    const tool = (id, icon, title, run) => {
      const b = h('button.iconbtn.tl', { title, onclick: () => { if (run) run(); else PM.setTool(id); syncTools(); } }, PM.icon(icon));
      b.dataset.tool = id;
      return b;
    };
    const sep = () => h('span.tl-sep');

    /* interactive tools (left, AE-style) */
    const selB   = tool('select', 'cursor', 'Selection (V)');
    const handB  = tool('hand',   'hand',   'Hand — pan view (H)');
    const zoomB  = tool('zoom',   'zoom',   'Zoom (Z)');
    /* creation tools */
    const textB  = tool('text',  'type',  'New text layer (⌘T)',   () => PM.cmd('newText'));
    const shapeB = tool('shape', 'shape', 'New shape layer (⌘⇧Y)', () => PM.cmd('newShape'));
    const solidB = tool('solid', 'solid', 'New solid (⌘Y)',        () => PM.cmd('newSolid'));
    const shdrB  = tool('shader','wand',  'New shader layer (⌘⇧G)',() => PM.cmd('newShader'));
    const nullB  = tool('null',  'frame', 'New null object',       () => PM.cmd('newNull'));
    const camB   = tool('camera','cam',   'Import media (⌘I)',     () => PM.cmd('import'));

    /* right-aligned workspace toggles */
    const snapB = h('button.iconbtn.tl.tg', { title: 'Snapping (S)', onclick: () => { PM.snap = !PM.snap; syncTg(); PM.invalidate(); } }, PM.icon('magnet'));
    const guidB = h('button.iconbtn.tl.tg', { title: 'Guides & safe areas', onclick: () => { PM.guides = !PM.guides; syncTg(); PM.invalidate(); } }, PM.icon('grid'));
    const mbluB = h('button.iconbtn.tl.tg', { title: 'Motion blur preview', onclick: () => { PM.mblurOn = !PM.mblurOn; syncTg(); PM.invalidate(); } }, PM.icon('clock'));

    body.append(
      selB, handB, zoomB, sep(),
      textB, shapeB, solidB, shdrB, nullB, sep(), camB,
      h('span', { style: { flex: 1 } }),
      snapB, guidB, mbluB,
    );

    function syncTools() {
      [selB, handB, zoomB].forEach(b => b.classList.toggle('on', b.dataset.tool === PM.tool));
    }
    function syncTg() {
      snapB.classList.toggle('on', !!PM.snap);
      guidB.classList.toggle('on', !!PM.guides);
      mbluB.classList.toggle('on', !!PM.mblurOn);
    }
    PM.bus.on('tool', syncTools);
    syncTools(); syncTg();
  },
});

})();
