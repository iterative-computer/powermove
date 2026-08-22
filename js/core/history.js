/* Powermove — undo/redo. Snapshot based, cheap because projects are small JSON. */
(() => {
const PM = window.PM;
const MAX = 120;
let stack = [], idx = -1, pending = null, depth = 0;

const snap = () => JSON.stringify(PM.proj);
const restore = (json) => {
  PM.proj = JSON.parse(json);
  PM.sel.layers = PM.sel.layers.filter(id => PM.L(id));
  PM.touch();
  PM.bus.emit('layers'); PM.bus.emit('sel'); PM.bus.emit('project');
  PM.invalidate();
};

const H = {
  begin(label) {
    if (depth++ > 0) return;
    pending = { label, before: snap() };
  },
  commit(label) {
    if (--depth > 0) return;
    if (!pending) return;
    const after = snap();
    if (after === pending.before) { pending = null; return; }
    stack = stack.slice(0, idx + 1);
    stack.push({ label: label || pending.label, before: pending.before, after });
    if (stack.length > MAX) stack.shift();
    idx = stack.length - 1;
    pending = null;
    PM.touch();
    PM.bus.emit('history');
    PM.invalidate('status');
    PM.autosave && PM.autosave();
  },
  cancel() { depth = 0; pending = null; },
  /** Wrap a mutation so it becomes one undoable step. */
  do(label, fn) { H.begin(label); try { return fn(); } finally { H.commit(label); } },
  undo() {
    if (idx < 0) return false;
    const e = stack[idx--];
    restore(e.before);
    PM.toast('Undo · ' + e.label);
    PM.bus.emit('history');
    return true;
  },
  redo() {
    if (idx >= stack.length - 1) return false;
    const e = stack[++idx];
    restore(e.after);
    PM.toast('Redo · ' + e.label);
    PM.bus.emit('history');
    return true;
  },
  canUndo: () => idx >= 0,
  canRedo: () => idx < stack.length - 1,
  label: () => (idx >= 0 ? stack[idx].label : null),
  /** Restore a trusted, previously captured project snapshot as one undoable step.
      Agent checkpoints use this instead of assigning PM.proj behind history's back. */
  restoreSnapshot(json, label = 'Restore checkpoint') {
    let parsed;
    try { parsed = JSON.parse(json); } catch { return false; }
    if (!parsed || typeof parsed !== 'object' || !Array.isArray(parsed.layers)) return false;
    H.do(label, () => restore(JSON.stringify(parsed)));
    return true;
  },
  clear() { stack = []; idx = -1; pending = null; depth = 0; PM.bus.emit('history'); },
  list: () => stack.map(s => s.label),
};
PM.hist = H;

/* ── Takes: automatic named versions of the whole project ── */
PM.takes = {
  all: () => PM.store.get('takes', []),
  save(label) {
    const list = PM.takes.all();
    list.unshift({ id: PM.uid('t'), label: label || 'Take ' + (list.length + 1), at: Date.now(), json: snap() });
    PM.store.set('takes', list.slice(0, 24));
    PM.bus.emit('takes');
    return list[0];
  },
  restore(id) {
    const t = PM.takes.all().find(t => t.id === id);
    if (!t) return false;
    PM.hist.do('Restore ' + t.label, () => restore(t.json));
    return true;
  },
  drop(id) { PM.store.set('takes', PM.takes.all().filter(t => t.id !== id)); PM.bus.emit('takes'); },
};
})();
