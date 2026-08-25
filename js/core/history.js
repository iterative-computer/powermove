/* No longer loaded — superseded by src/renderer/src/legacy/core/history.ts; kept for the legacy test oracle until Phase 6. */
/* Powermove — undo/redo. Snapshot based, cheap because projects are small JSON. */
(() => {
const PM = window.PM;
const MAX = 120;
let stack = [], idx = -1, pending = null, depth = 0;

const snap = () => JSON.stringify(PM.proj);
const restore = (json) => {
  PM.replaceProject(JSON.parse(json));
};

const H = {
  begin(label, group = null) {
    if (depth++ > 0) return;
    pending = { label, before: snap(), group };
  },
  commit(label) {
    if (--depth > 0) return;
    if (!pending) return;
    const after = snap();
    if (after === pending.before) { pending = null; return; }
    stack = stack.slice(0, idx + 1);
    stack.push({ id: PM.uid('history'), label: label || pending.label, before: pending.before, after, group: pending.group });
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
    if (e.undo) e.undo();
    else restore(e.before);
    PM.toast('Undo · ' + e.label);
    PM.bus.emit('history');
    return true;
  },
  redo() {
    if (idx >= stack.length - 1) return false;
    const e = stack[++idx];
    if (e.redo) e.redo();
    else restore(e.after);
    PM.toast('Redo · ' + e.label);
    PM.bus.emit('history');
    return true;
  },
  canUndo: () => idx >= 0,
  canRedo: () => idx < stack.length - 1,
  label: () => (idx >= 0 ? stack[idx].label : null),
  /** Capture the current project-history position before a bounded async run. */
  mark() { return { index: idx, topId: stack[idx]?.id || null, before: snap() }; },
  /** Replace all project entries created after a mark with one atomic step.
      Agent review/repair passes use this so one run is always one Undo. */
  squash(mark, label = 'Agent change', group = null) {
    if (!mark || !Number.isInteger(mark.index) || typeof mark.before !== 'string') return null;
    const baseIndex = mark.topId ? stack.findIndex(entry => entry.id === mark.topId) : -1;
    if (mark.topId && baseIndex < 0) return null;
    if (mark.index < -1 || baseIndex > idx || idx !== stack.length - 1) return null;
    const entries = stack.slice(baseIndex + 1);
    if (!entries.length || entries.some(entry => entry.undo || entry.redo || group && entry.group !== group)) return null;
    const after = snap();
    if (after === mark.before) return null;
    stack = stack.slice(0, baseIndex + 1);
    const entry = { id: PM.uid('history'), label, before: mark.before, after };
    stack.push(entry);
    if (stack.length > MAX) stack.shift();
    idx = stack.length - 1;
    PM.bus.emit('history');
    return entry.id;
  },
  /** Add a non-project change (workspace, panels, or interface chrome) to the
      same Undo/Redo stack without pretending it changed composition source. */
  external(label, undo, redo) {
    if (typeof undo !== 'function' || typeof redo !== 'function') return null;
    stack = stack.slice(0, idx + 1);
    const entry = { id: PM.uid('history'), label: label || 'Interface change', undo, redo };
    stack.push(entry);
    if (stack.length > MAX) stack.shift();
    idx = stack.length - 1;
    PM.bus.emit('history');
    return entry.id;
  },
  undoIfTop(id) {
    if (!id || stack[idx]?.id !== id) return false;
    return H.undo();
  },
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
