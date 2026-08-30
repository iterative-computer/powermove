/* Ported from js/ui/shortcuts.js — behavior-preserving.
 *
 * The command table lives in the kernel now:
 *   - `def()` registers a `CommandDefinition` under owner `legacy`;
 *     `PM.commands` is a live view over `kernel.commands` in the old
 *     `{ id, label, kb, run, cat }` shape (see kernel-view.ts).
 *   - keybindings are contributed by the built-in `keymap-default` extension
 *     and dispatched by the one key listener the kernel installs.
 */
import type { CommandDefinition } from '../../kernel/api';
import { ensureKernel, registryView } from '../kernel-view';
import type { PMRegistry } from '../registry';

export const LEGACY_OWNER = 'legacy';

export function install(PM: PMRegistry): void {
const h: any = PM.h;
const kernel = ensureKernel(PM);

/* One live registration per command id: re-registering (`def` on reinstall,
   or `PM.commands.palette.run = …` from overlays/install.ts) replaces rather
   than stacking, so the legacy layer never buries its own entries. */
const owned = new Map<string, { dispose(): void }>();
const put = (definition: CommandDefinition): void => {
  owned.get(definition.id)?.dispose();
  owned.set(definition.id, kernel.commands.register(LEGACY_OWNER, definition));
};

const def: any = (id?: any, label?: any, kb?: any, run?: any, cat: any = 'General', opts: any = {}) => {
  put({
    id, label, kb, category: cat,
    /* The kernel key listener treats a `false` return as "not handled" and
       skips preventDefault. Legacy always prevented the default, so a command
       that happens to return false must not change that. */
    run: (...a: any[]) => { const result = run(...a); return result === false ? undefined : result; },
    ...(opts.when ? { when: opts.when } : {})
  });
};
const hidden = { when: () => false };

const activeTextField = (): boolean => {
  if (typeof document === 'undefined') return false;
  const active: any = document.activeElement;
  const tag = typeof active?.tagName === 'string' ? active.tagName.toUpperCase() : '';
  return tag === 'INPUT' || tag === 'TEXTAREA' || active?.isContentEditable === true;
};

const nativeEdit = (action: string): unknown => {
  const bridge: any = typeof window === 'undefined' ? null : (window as any).powermove;
  if (typeof bridge?.nativeEdit === 'function') {
    bridge.nativeEdit(action);
    return true;
  }
  if (typeof document === 'undefined' || typeof document.execCommand !== 'function') return false;
  return document.execCommand(action);
};

PM.commands = registryView<CommandDefinition, any>(kernel.commands, {
  read: (_item, id) => commandView(kernel, id, put),
  write: (id, value: any) => {
    if (!value || typeof value !== 'object') return false;
    put({ ...value, id, category: value.cat ?? value.category ?? 'General' });
    return true;
  },
  remove: (id) => {
    owned.get(id)?.dispose();
    owned.delete(id);
    return true;
  }
});
PM.cmd = (id?: any, ...a: any[]) => {
  const c: any = kernel.commands.get(id);
  if (c) return c.run(...a);
  /* A missing command is deliberately an unhandled dispatch. Extensions can
     replace/remove commands while a stale keybinding is still in flight; the
     kernel uses `false` to let lower-priority bindings continue. */
  console.warn('no cmd', id);
  return false;
};

const center: any = () => ({ 'position.x': PM.proj.w / 2, 'position.y': PM.proj.h / 2 });
function addLayer(type?: any, opts: any = {}) {
  const from: any = opts.from == null ? PM.snapF(PM.time, PM.proj.fps) : opts.from;
  const result: any = PM.Edit.apply({
    type: 'add_layer', layerType: type, name: opts.name,
    from, duration: opts.dur == null ? Math.max(1, PM.proj.dur - from) : opts.dur,
    content: opts.d || {}, properties: opts.p || {}, color: opts.color, select: true,
  }, { label: 'New ' + type, origin: 'command' });
  const id: any = result.ok && result.data.results[0].data.id;
  return id ? PM.L(id) : null;
}
PM.addLayerCmd = addLayer;

/* ── layer creation ────────────────────────────────────── */
def('newText', 'New text layer', '⌘T', () => addLayer('text', { name: 'Headline', p: center() }), 'Create');
def('newSolid', 'New solid', '⌘Y', () => addLayer('solid', { name: 'Solid' }), 'Create');
def('newShape', 'New shape', '⌘⇧Y', () => addLayer('shape', { name: 'Shape', p: center() }), 'Create');
def('newShader', 'New shader layer', '⌘⇧G', () => { const L: any = addLayer('shader', { name: 'Shader' }); PM.syncShaderUniforms?.(L); PM.openShaderEditor(L); return L; }, 'Create');
def('newNull', 'New null object', '⌘⌥⇧Y', () => addLayer('null', { name: 'Null', p: center() }), 'Create');
def('import', 'Import media…', '⌘I', () => PM.pickFiles(), 'Create');
def('toolSelect', 'Selection tool', 'V', () => PM.setTool('select'), 'Tool');
def('toolHand', 'Hand tool', 'H', () => PM.setTool('hand'), 'Tool');
def('toolZoom', 'Zoom tool', 'Z', () => PM.setTool('zoom'), 'Tool');
PM.commandForAsset = (id?: any, at: any = PM.time) => {
  const a: any = PM.proj.assets[id]; if (!a) return;
  const type: any = a.kind === 'audio' ? 'audio' : a.kind === 'video' ? 'video' : 'image';
  const content: any = type === 'audio'
    ? { asset: id, trim: 0, gain: 1, fadeIn: 0, fadeOut: 0 }
    : { asset: id, w: a.w || PM.proj.w, h: a.h || PM.proj.h };
  return {
    type: 'add_layer', layerType: type, name: a.name,
    from: PM.snapF(at, PM.proj.fps),
    /* Keep the whole source even when it extends beyond the current comp. The
       composition still clips playback/export, but extending it later reveals
       the rest instead of permanently discarding the imported media. */
    duration: a.dur || Math.max(1 / PM.proj.fps, PM.proj.dur - at),
    content,
    select: true,
  };
};
def('addFromAsset', 'Add layer from asset', null, (id?: any) => {
  const command: any = PM.commandForAsset(id);
  if (!command) return;
  const result: any = PM.Edit.apply(command, { label: 'New ' + command.layerType, origin: 'command' });
  const layerId: any = result.ok && result.data.results[0].data.id;
  return layerId ? PM.L(layerId) : null;
}, 'Create');

/* ── editing ───────────────────────────────────────────── */
def('duplicate', 'Duplicate layers', '⌘D', () => PM.hist.do('Duplicate', () => {
  const sels: any = PM.selLayers(); if (!sels.length) return;
  const ids: any = [];
  sels.forEach((L: any) => { const c: any = PM.cloneLayer(L); PM.proj.layers.splice(PM.proj.layers.indexOf(L), 0, c); ids.push(c.id); });
  PM.bus.emit('layers'); PM.selectLayers(ids);
}), 'Edit');
def('delete', 'Delete selection', '⌫', () => deleteSelection(PM), 'Edit');
def('split', 'Split at playhead', '⌘B', () => splitLayers(PM), 'Edit');
def('selectAll', 'Select all layers', '⌘A', () => PM.selectLayers(PM.proj.layers.map((l: any) => l.id)), 'Edit');
def('deselect', 'Deselect', '⎋', () => { PM.selectLayers([]); PM.sel.keys = []; }, 'Edit');
def('precompose', 'Precompose selected layers…', '⌘⇧C', () => {
  const sels: any = PM.selLayers(); if (!sels.length) return PM.toast('Select layers to precompose');
  const name: any = h('input', { value: 'Precomp' });
  PM.modal({ title: 'Precompose ' + sels.length + (sels.length === 1 ? ' layer' : ' layers'), body: h('div.field', name), width: 400, actions: [
    { label: 'Cancel' },
    { label: 'Create', pri: true, run: () => PM.hist.do('Precompose', () => PM.precompose(sels.map((l: any) => l.id), name.value.trim() || undefined)) },
  ] });
  window.setTimeout(() => { name.focus(); name.select(); }, 30);
}, 'Edit');

/* ── layer clipboard ───────────────────────────────────── */
let layerClip: any = null;
def('copyLayers', 'Copy layers', '⌘C', () => {
  const sels: any = selectedStackLayers(PM); if (!sels.length) return;
  layerClip = sels.map((L: any) => JSON.parse(JSON.stringify(L)));
  PM.toast(`Copied ${layerClip.length} ${layerClip.length === 1 ? 'layer' : 'layers'}`);
}, 'Edit');
def('cutLayers', 'Cut layers', '⌘X', () => cutLayers(PM, (value: any[]) => { layerClip = value; }), 'Edit');
def('pasteLayers', 'Paste layers', '⌘V', () => pasteLayers(PM, () => layerClip), 'Edit');
def('contextUndo', 'Undo', null, () => activeTextField() ? nativeEdit('undo') : PM.cmd('undo'), 'Edit', hidden);
def('contextRedo', 'Redo', null, () => activeTextField() ? nativeEdit('redo') : PM.cmd('redo'), 'Edit', hidden);
def('contextCut', 'Cut', null, () => activeTextField() ? nativeEdit('cut') : PM.cmd('cutLayers'), 'Edit', hidden);
def('contextCopy', 'Copy', null, () => activeTextField() ? nativeEdit('copy') : PM.cmd('copyLayers'), 'Edit', hidden);
def('contextPaste', 'Paste', null, () => activeTextField() ? nativeEdit('paste') : PM.cmd('pasteLayers'), 'Edit', hidden);
def('contextSelectAll', 'Select all', null, () => activeTextField() ? nativeEdit('selectAll') : PM.cmd('selectAll'), 'Edit', hidden);
def('toggleVisibility', 'Hide/show selected layers', '⌘⇧H', () => toggleVisibility(PM), 'Edit');
def('bringForward', 'Bring forward', '⌘]', () => orderLayers(PM, 'forward'), 'Edit');
def('sendBackward', 'Send backward', '⌘[', () => orderLayers(PM, 'backward'), 'Edit');
def('bringToFront', 'Bring to front', '⌘⇧]', () => orderLayers(PM, 'front'), 'Edit');
def('sendToBack', 'Send to back', '⌘⇧[', () => orderLayers(PM, 'back'), 'Edit');
def('nudgeSelection', 'Nudge selection', null, (dx?: any, dy?: any) => nudgeSelection(PM, dx, dy), 'Edit');
def('undo', 'Undo', '⌘Z', () => PM.hist.undo(), 'Edit');
def('redo', 'Redo', '⌘⇧Z', () => PM.hist.redo(), 'Edit');

/* ── transport ─────────────────────────────────────────── */
def('play', 'Play / pause', '␣', () => PM.toggle(), 'Transport');
def('gotoStart', 'Go to start', '⇱', () => PM.setTime(0), 'Transport');
def('gotoEnd', 'Go to end', '⇲', () => PM.setTime(PM.proj.dur), 'Transport');
def('nextFrame', 'Next frame', '→', () => PM.step(1), 'Transport');
def('prevFrame', 'Previous frame', '←', () => PM.step(-1), 'Transport');
def('nextEdge', 'Next edge', '⇧→', () => { const edge = PM.TL?.nextEdge?.(); if (Number.isFinite(edge)) PM.setTime(edge); }, 'Transport');
def('prevEdge', 'Previous edge', '⇧←', () => { const edge = PM.TL?.prevEdge?.(); if (Number.isFinite(edge)) PM.setTime(edge); }, 'Transport');
def('workIn', 'Work area in', 'B', () => PM.Edit.apply({ type: 'set_composition', patch: { workArea: [Math.min(PM.time, PM.proj.work[1] - 1 / PM.proj.fps), PM.proj.work[1]] } }, { label: 'Work area', origin: 'command' }), 'Transport');
def('workOut', 'Work area out', 'N', () => PM.Edit.apply({ type: 'set_composition', patch: { workArea: [PM.proj.work[0], Math.max(PM.time, PM.proj.work[0] + 1 / PM.proj.fps)] } }, { label: 'Work area', origin: 'command' }), 'Transport');

/* ── reveal properties (AE muscle memory) ──────────────── */
const reveal: any = (keys?: any) => () => {
  const sels: any = PM.selLayers(); if (!sels.length) return;
  sels.forEach((L: any) => { L.collapsed = false; L._reveal = keys; });
  PM.sel.chan = keys[0];
  PM.invalidate('timeline');
};
def('revealPos', 'Reveal position', 'P', reveal(['position.x', 'position.y']), 'Reveal');
def('revealScale', 'Reveal scale', 'S', reveal(['scale.x', 'scale.y']), 'Reveal');
def('revealRot', 'Reveal rotation', 'R', reveal(['rotation']), 'Reveal');
def('revealOpacity', 'Reveal opacity', 'T', reveal(['opacity']), 'Reveal');
def('revealAnchor', 'Reveal anchor point', 'A', reveal(['anchor.x', 'anchor.y']), 'Reveal');
def('revealKeys', 'Reveal animated properties', 'U', () => {
  PM.selLayers().forEach((L: any) => { L.collapsed = false; L._reveal = null; });
  PM.invalidate('timeline');
}, 'Reveal');
def('graph', 'Toggle graph editor', 'G', () => { if (!PM.TL) return; PM.TL.graph = !PM.TL.graph; PM.invalidate('timeline'); }, 'Reveal');

/* ── keyframes ─────────────────────────────────────────── */
/* sel.keys holds keyframe ids (Phase 3a); easing needs the live objects */
const easeTargets: any = () => PM.sel.keys.length ? PM.resolveSelectedKeys() : allSelKeys();
def('easeOut', 'Easy ease keys', 'F9', () => PM.hist.do('Easy ease', () => {
  PM.applyEaseTo(easeTargets(), 'easeInOut'); PM.invalidate();
}), 'Keyframes');
def('easePower', 'Powermove curve', '⇧F9', () => PM.hist.do('Power ease', () => {
  PM.applyEaseTo(easeTargets(), 'power'); PM.invalidate();
}), 'Keyframes');
def('easeLinear', 'Linear keys', '⌘⇧F9', () => PM.hist.do('Linear', () => {
  PM.applyEaseTo(easeTargets(), 'linear'); PM.invalidate();
}), 'Keyframes');
function allSelKeys() {
  const out: any = [];
  PM.selLayers().forEach((L: any) => PM.allProps(L).forEach((p: any) => out.push(...p.prop.kf)));
  return out;
}

/* ── view / files ──────────────────────────────────────── */
def('fitView', 'Fit composition and timeline', '⇧F', () => {
  if (PM.Viewer) {
    PM.Viewer.fit = true;
    if (Array.isArray(PM.Viewer.pan)) PM.Viewer.pan = [0, 0];
    PM.Viewer.layout?.();
  }
  PM.TL?.frameView?.();
}, 'View');
def('fitComposition', 'Fit composition', '⌘0', () => {
  if (!PM.Viewer) return false;
  PM.Viewer.fit = true;
  if (Array.isArray(PM.Viewer.pan)) PM.Viewer.pan = [0, 0];
  PM.Viewer.layout?.();
}, 'View');
def('zoomIn', 'Zoom in', '⌘+', () => zoomViewer(PM, 1.25), 'View');
def('zoomOut', 'Zoom out', '⌘-', () => zoomViewer(PM, .8), 'View');
def('actualSize', 'Actual size', '⌘1', () => setViewerZoom(PM, 1), 'View');
def('palette', 'Command palette', '⌘K', () => PM.palette(), 'View');
def('agent', 'Ask Powermove agent', '⌘⇧K', () => PM.SpatialAssistant?.open?.(), 'View');
def('save', 'Save project', '⌘S', () => PM.saveProject(), 'File');
def('saveAs', 'Save project as…', '⌘⇧S', () => PM.saveProject({ saveAs: true }), 'File');
def('open', 'Open project…', '⌘O', () => PM.openProject(), 'File');
def('export', 'Export…', '⌘E', () => PM.Export.dialog(), 'File');
def('projects', 'Projects screen', '⌘P', () => PM.ProjectsScreen && PM.ProjectsScreen.toggle(), 'File');
def('newProject', 'New project', '⌘N', () => PM.newProject(), 'File');
def('takeSave', 'Save take', '', () => { PM.takes.save(); PM.toast('Take saved'); }, 'File');

/* ── JKL transport + trim handles ──────────────────────── */
/* These were inline in the old keydown handler with no command behind them.
   They are commands now (that is how the kernel dispatches keys) but stay out
   of the palette and the agent's command list, exactly as before. */
def('transportPause', 'Pause', 'K', () => PM.pause(), 'Transport', hidden);
def('transportPlay', 'Play', 'L', () => PM.play(), 'Transport', hidden);
def('trimIn', 'Trim in to playhead', 'I', () => PM.hist.do('Trim in', () => PM.selLayers().forEach((L: any) => {
  if (PM.time <= L.from || PM.time >= L.from + L.dur) return;
  const d: any = PM.time - L.from;
  if (PM.MediaTiming.isTimed(L)) L.d.trim = PM.MediaTiming.trimAtStart(L, PM.time);
  L.dur -= d; L.from = PM.time;
})), 'Edit', hidden);
def('trimOut', 'Trim out to playhead', 'O', () => PM.hist.do('Trim out', () => PM.selLayers().forEach((L: any) => {
  L.dur = Math.max(1 / PM.proj.fps, PM.time - L.from);
})), 'Edit', hidden);

/* ── keymap ────────────────────────────────────────────── */
/* The keymap-default extension binds Escape in fields to this hidden command.
   Returning false leaves the browser default alone after blurring the field. */
put({
  id: 'blurField',
  label: 'Blur focused field',
  category: 'Edit',
  when: () => false,
  run: () => {
    const active: any = typeof document === 'undefined' ? null : document.activeElement;
    const tag = typeof active?.tagName === 'string' ? active.tagName.toUpperCase() : '';
    if (tag === 'INPUT' || tag === 'TEXTAREA' || active?.isContentEditable) active.blur?.();
    return false;
  }
});
}

type LayerOrder = 'forward' | 'backward' | 'front' | 'back';

function currentLayers(PM: PMRegistry): any[] {
  const comp = typeof PM.curComp === 'function' ? PM.curComp() : PM.proj;
  return Array.isArray(comp?.layers) ? comp.layers : [];
}

function selectedStackLayers(PM: PMRegistry): any[] {
  const layers = currentLayers(PM);
  const ids = new Set((PM.sel?.layers || []).filter(Boolean));
  if (ids.size) return layers.filter((layer: any) => ids.has(layer.id));
  return typeof PM.selLayers === 'function' ? PM.selLayers().filter(Boolean) : [];
}

function selectLayerIds(PM: PMRegistry, ids: any[]): void {
  if (typeof PM.selectLayers === 'function') {
    PM.selectLayers(ids);
    return;
  }
  PM.sel = PM.sel || { layers: [], keys: [] };
  PM.sel.layers = [...ids];
  PM.bus?.emit?.('sel');
  PM.invalidate?.();
}

function finishLayerMutation(PM: PMRegistry): void {
  if (PM.Edit?.mutate) return;
  PM.bus?.emit?.('layers');
  /* PM.selectLayers already invalidates when a command changes selection. */
  if (typeof PM.selectLayers !== 'function') PM.invalidate?.();
}

function runAtomic(PM: PMRegistry, label: string, action: () => unknown): unknown {
  if (PM.Edit?.mutate) {
    return PM.Edit.mutate(label, action, { origin: 'command' });
  }
  if (PM.hist && typeof PM.hist.do === 'function') return PM.hist.do(label, action);
  return action();
}

function cloneForCommand(PM: PMRegistry, layer: any): any {
  const clone = typeof PM.cloneLayer === 'function'
    ? PM.cloneLayer(layer)
    : JSON.parse(JSON.stringify(layer));
  return clone;
}

function ensureLayerId(PM: PMRegistry, layer: any, used: Set<any>, source?: any): void {
  if (layer.id && layer.id !== source?.id && !used.has(layer.id)) {
    used.add(layer.id);
    return;
  }
  const next = typeof PM.uid === 'function' ? PM.uid('L') : `${source?.id || 'layer'}-copy`;
  layer.id = next;
  used.add(next);
}

function rebaseTailAnimation(PM: PMRegistry, layer: any, offset: number): void {
  if (!Number.isFinite(offset) || offset === 0 || typeof PM.allProps !== 'function') return;
  for (const { prop } of PM.allProps(layer)) {
    if (!Array.isArray(prop?.kf)) continue;
    for (const keyframe of prop.kf) {
      if (Number.isFinite(Number(keyframe.t))) keyframe.t = Number(keyframe.t) - offset;
    }
  }
  /* Expressions deliberately remain layer-local: `t` restarts on the new
     clip, while authors who need composition continuity can use global `T`.
     Keyframes, by contrast, are rebased so their value is continuous. */
}

/**
 * Split the selected active layers, or all visible active root layers when
 * there is no selection. The tail is inserted immediately above its source,
 * preserving the source's parent and timed-media continuity.
 */
export function splitLayers(PM: PMRegistry): unknown {
  const layers = currentLayers(PM);
  const selected = selectedStackLayers(PM);
  const selectionRequested = Boolean((PM.sel?.layers || []).length || selected.length);
  const T = Number(PM.time);
  if (!Number.isFinite(T)) return false;

  const targets = (selectionRequested ? selected : layers.filter((layer: any) =>
    !layer.parent && layer.on !== false))
    .filter((layer: any) => !layer.lock && Number.isFinite(Number(layer.from))
      && Number.isFinite(Number(layer.dur))
      && T > Number(layer.from) && T < Number(layer.from) + Number(layer.dur));
  if (!targets.length) return false;

  return runAtomic(PM, 'Split', () => {
    const tails: any[] = [];
    const used = new Set(layers.map((layer: any) => layer.id));
    /* `targets` follows stack order so each source/tail pair stays adjacent
       even when several selected layers are split in one operation. */
    for (const layer of targets) {
      const index = layers.indexOf(layer);
      if (index < 0) continue;
      const sourceEnd = Number(layer.from) + Number(layer.dur);
      const tail = cloneForCommand(PM, layer);
      ensureLayerId(PM, tail, used, layer);
      const sourceOffset = T - Number(layer.from);
      tail.from = T;
      tail.dur = sourceEnd - T;
      rebaseTailAnimation(PM, tail, sourceOffset);
      if (PM.MediaTiming?.isTimed?.(layer)) {
        tail.d = tail.d && typeof tail.d === 'object' ? tail.d : {};
        if (typeof PM.MediaTiming.trimAtStart === 'function') {
          tail.d.trim = PM.MediaTiming.trimAtStart(layer, T);
        }
      }
      /* The cut is an internal boundary, not a new entrance/exit. Keep the
         original entrance on the head and the original exit on the tail. */
      layer.transitionOut = null;
      tail.transitionIn = null;
      if (layer.type === 'audio') {
        if (layer.d && typeof layer.d === 'object') layer.d.fadeOut = 0;
        if (tail.d && typeof tail.d === 'object') tail.d.fadeIn = 0;
      }
      layer.dur = T - Number(layer.from);
      layers.splice(index, 0, tail);
      tails.push(tail);
    }
    if (!tails.length) return false;
    selectLayerIds(PM, tails.map((layer: any) => layer.id));
    finishLayerMutation(PM);
    return tails;
  });
}

/** Cut only unlocked selected layers and retain the cut payload for paste. */
export function cutLayers(PM: PMRegistry, setClipboard: (value: any[]) => void = () => {}): unknown {
  const layers = currentLayers(PM);
  const selected = selectedStackLayers(PM);
  const editable = selected.filter((layer: any) => !layer.lock);
  if (!editable.length) {
    PM.toast?.('No unlocked layers to cut');
    return false;
  }

  const payload = editable.map((layer: any) => JSON.parse(JSON.stringify(layer)));
  setClipboard(payload);
  const ids = new Set(editable.map((layer: any) => layer.id));
  return runAtomic(PM, 'Cut layers', () => {
    const comp = typeof PM.curComp === 'function' ? PM.curComp() : PM.proj;
    if (comp === PM.proj && typeof PM.removeLayers === 'function') {
      /* The model helper also unparents survivors and garbage-collects unused
         nested compositions. */
      PM.removeLayers([...ids]);
    } else {
      comp.layers = layers.filter((layer: any) => !ids.has(layer.id));
      comp.layers.forEach((layer: any) => {
        if (ids.has(layer.parent)) layer.parent = null;
      });
      finishLayerMutation(PM);
    }
    const remainingSelection = selected.filter((layer: any) => !ids.has(layer.id)).map((layer: any) => layer.id);
    selectLayerIds(PM, remainingSelection);
    PM.toast?.(`Cut ${editable.length} ${editable.length === 1 ? 'layer' : 'layers'}`);
    return editable;
  });
}

/** Paste beside the topmost selected layer, preserving valid parent links. */
export function pasteLayers(PM: PMRegistry, getClipboard: () => any[] | null = () => null): unknown {
  const clipboard = getClipboard();
  if (!Array.isArray(clipboard) || !clipboard.length) return false;
  const layers = currentLayers(PM);
  const selectedIds = new Set((PM.sel?.layers || []).filter(Boolean));
  const selectedIndexes = layers
    .map((layer: any, index: number) => selectedIds.has(layer.id) ? index : -1)
    .filter((index: number) => index >= 0);
  const insertAt = selectedIndexes.length ? Math.min(...selectedIndexes) : 0;
  const currentIds = new Set(layers.map((layer: any) => layer.id));

  return runAtomic(PM, 'Paste layers', () => {
    const pairs = clipboard.map((source: any) => [source, cloneForCommand(PM, JSON.parse(JSON.stringify(source)))]);
    const used = new Set(currentIds);
    const idMap = new Map<any, any>();
    pairs.forEach(([source, clone]: any) => {
      ensureLayerId(PM, clone, used, source);
      idMap.set(source.id, clone.id);
    });
    const pasted = pairs.map(([source, clone]: any) => {
      if (source.parent && idMap.has(source.parent)) clone.parent = idMap.get(source.parent);
      else clone.parent = source.parent && currentIds.has(source.parent) ? source.parent : null;
      return clone;
    });
    layers.splice(insertAt, 0, ...pasted);
    selectLayerIds(PM, pasted.map((layer: any) => layer.id));
    finishLayerMutation(PM);
    PM.toast?.(`Pasted ${pasted.length} ${pasted.length === 1 ? 'layer' : 'layers'}`);
    return pasted;
  });
}

/** Toggle visibility of the editable portion of the current selection. */
export function toggleVisibility(PM: PMRegistry): unknown {
  const editable = selectedStackLayers(PM).filter((layer: any) => !layer.lock);
  if (!editable.length) return false;
  const show = editable.every((layer: any) => layer.on === false);
  const commands = editable.map((layer: any) => ({
    type: 'set_layer', target: layer.id, patch: { visible: show },
  }));
  if (PM.Edit?.apply) {
    return PM.Edit.apply(commands, { label: show ? 'Show layers' : 'Hide layers', origin: 'command' });
  }
  return runAtomic(PM, show ? 'Show layers' : 'Hide layers', () => {
    editable.forEach((layer: any) => { layer.on = show; });
    finishLayerMutation(PM);
    return editable;
  });
}

function stepLayerOrder(layers: any[], selected: Set<any>, movable: Set<any>, direction: 'forward' | 'backward'): boolean {
  let changed = false;
  if (direction === 'forward') {
    for (let index = 0; index < layers.length; index++) {
      const layer = layers[index];
      if (!movable.has(layer.id) || index === 0) continue;
      const previous = layers[index - 1];
      if (selected.has(previous.id) || previous.lock) continue;
      layers[index - 1] = layer;
      layers[index] = previous;
      changed = true;
    }
  } else {
    for (let index = layers.length - 1; index >= 0; index--) {
      const layer = layers[index];
      if (!movable.has(layer.id) || index === layers.length - 1) continue;
      const next = layers[index + 1];
      if (selected.has(next.id) || next.lock) continue;
      layers[index + 1] = layer;
      layers[index] = next;
      changed = true;
    }
  }
  return changed;
}

/** Reorder unlocked selected layers without crossing locked layer barriers. */
export function orderLayers(PM: PMRegistry, mode: LayerOrder): unknown {
  if (!['forward', 'backward', 'front', 'back'].includes(mode)) return false;
  const layers = currentLayers(PM);
  const selectedLayers = selectedStackLayers(PM);
  const selected = new Set(selectedLayers.map((layer: any) => layer.id));
  const movable = new Set(selectedLayers.filter((layer: any) => !layer.lock).map((layer: any) => layer.id));
  if (!movable.size) return false;

  const labels: Record<LayerOrder, string> = {
    forward: 'Bring forward', backward: 'Send backward',
    front: 'Bring to front', back: 'Send to back',
  };
  return runAtomic(PM, labels[mode], () => {
    let changed = false;
    if (mode === 'forward' || mode === 'backward') {
      changed = stepLayerOrder(layers, selected, movable, mode);
    } else {
      const direction = mode === 'front' ? 'forward' : 'backward';
      while (stepLayerOrder(layers, selected, movable, direction)) changed = true;
    }
    if (!changed) return false;
    finishLayerMutation(PM);
    return selectedLayers;
  });
}

function worldNudgeToLocal(PM: PMRegistry, layer: any, dx: number, dy: number): [number, number] {
  if (!layer.parent || typeof PM.worldMatrix !== 'function') return [dx, dy];
  const parent = PM.L?.(layer.parent);
  if (!parent) return [dx, dy];
  const matrix = PM.worldMatrix(parent, PM.time);
  if (!Array.isArray(matrix) || matrix.length < 4) return [dx, dy];
  const determinant = matrix[0] * matrix[3] - matrix[1] * matrix[2];
  if (!Number.isFinite(determinant) || Math.abs(determinant) < 1e-9) return [0, 0];
  return [
    (dx * matrix[3] - dy * matrix[2]) / determinant,
    (dy * matrix[0] - dx * matrix[1]) / determinant,
  ];
}

const NUDGE_HISTORY_GROUP = 'keyboard-nudge';
const nudgeBatches = new WeakMap<object, { mark: any; timer: ReturnType<typeof setTimeout> }>();

function beginNudgeHistory(PM: PMRegistry): boolean {
  if (!PM.hist?.mark || !PM.hist?.squash || nudgeBatches.has(PM as object)) return false;
  nudgeBatches.set(PM as object, {
    mark: PM.hist.mark(), timer: 0 as unknown as ReturnType<typeof setTimeout>,
  });
  return true;
}

function coalesceNudgeHistory(PM: PMRegistry): void {
  const batch = nudgeBatches.get(PM as object);
  if (!batch) {
    return;
  }
  clearTimeout(batch.timer);
  batch.timer = setTimeout(() => {
    PM.hist.squash(batch.mark, 'Nudge selection', NUDGE_HISTORY_GROUP);
    nudgeBatches.delete(PM as object);
  }, 180);
}

/** Move selected layer positions through atomic, keyframe-aware source edits. */
export function nudgeSelection(PM: PMRegistry, dx?: any, dy?: any): unknown {
  if (typeof dx !== 'number' || typeof dy !== 'number'
    || !Number.isFinite(dx) || !Number.isFinite(dy) || (dx === 0 && dy === 0)) return false;
  const commands: any[] = [];
  for (const layer of selectedStackLayers(PM).filter((item: any) => !item.lock)) {
    const [localX, localY] = worldNudgeToLocal(PM, layer, dx, dy);
    for (const [path, delta] of [['position.x', localX], ['position.y', localY]] as const) {
      if (!delta || !layer.p?.[path]) continue;
      const current = typeof PM.ev === 'function' ? PM.ev(layer, path, PM.time) : layer.p[path].v;
      if (!Number.isFinite(Number(current))) continue;
      commands.push({
        type: 'set_property', target: layer.id, path, value: Number(current) + delta,
        time: PM.time, mode: 'auto', preserveHandEdits: false, markIntent: 'human',
      });
    }
  }
  if (!commands.length || !PM.Edit?.apply) return false;
  const meta = { label: 'Nudge selection', origin: 'command', historyGroup: NUDGE_HISTORY_GROUP };
  const startedBatch = beginNudgeHistory(PM);
  const result = PM.Edit.apply(commands, meta);
  if (result?.ok) coalesceNudgeHistory(PM);
  else if (startedBatch) nudgeBatches.delete(PM as object);
  return result;
}

const VIEWER_MIN_ZOOM = .05;
const VIEWER_MAX_ZOOM = 8;

export function setViewerZoom(PM: PMRegistry, zoom: any): unknown {
  const viewer = PM.Viewer;
  if (!viewer || typeof zoom !== 'number' || !Number.isFinite(zoom)) return false;
  const clamp = typeof PM.clamp === 'function'
    ? PM.clamp.bind(PM)
    : (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));
  const wasFit = viewer.fit === true;
  viewer.fit = false;
  if (wasFit && Array.isArray(viewer.pan)) viewer.pan = [0, 0];
  viewer.zoom = clamp(zoom, VIEWER_MIN_ZOOM, VIEWER_MAX_ZOOM);
  viewer.layout?.();
  return viewer.zoom;
}

export function zoomViewer(PM: PMRegistry, factor: any): unknown {
  const viewer = PM.Viewer;
  if (!viewer || typeof factor !== 'number' || !Number.isFinite(factor) || factor <= 0) return false;
  const current = Number.isFinite(Number(viewer.shown)) ? Number(viewer.shown)
    : Number.isFinite(Number(viewer.zoom)) ? Number(viewer.zoom) : 1;
  return setViewerZoom(PM, current * factor);
}

/** Legacy `{ id, label, kb, run, cat }` façade over one kernel command. */
function commandView(kernel: ReturnType<typeof ensureKernel>, id: string, put: (def: CommandDefinition) => void): any {
  const FIELDS = ['id', 'label', 'kb', 'run', 'cat'];
  const read = (): CommandDefinition | undefined => kernel.commands.get(id);
  const field = (definition: any, key: string): unknown => (key === 'cat' ? definition.category ?? 'General' : definition[key]);
  return new Proxy(Object.create(null) as Record<string, unknown>, {
    get(target, key) {
      if (typeof key === 'symbol') return Reflect.get(target, key);
      const definition = read();
      return definition ? field(definition, key) : undefined;
    },
    set(target, key, value) {
      if (typeof key === 'symbol') return Reflect.set(target, key, value);
      const definition = read();
      if (!definition) return false;
      put(key === 'cat' ? { ...definition, category: value as string } : ({ ...definition, [key]: value } as CommandDefinition));
      return true;
    },
    has(target, key) {
      if (typeof key === 'symbol') return Reflect.has(target, key);
      return !!read() && FIELDS.includes(key);
    },
    ownKeys: () => [...FIELDS],
    getOwnPropertyDescriptor(target, key) {
      if (typeof key === 'symbol') return Reflect.getOwnPropertyDescriptor(target, key);
      const definition = read();
      if (!definition || !FIELDS.includes(key)) return undefined;
      return { value: field(definition, key), enumerable: true, configurable: true, writable: true };
    }
  });
}

/** A keyframe deletion never falls through to its owning layer, including a
 * held Delete key after the first keyframe has already been removed. */
export function deleteSelection(PM: PMRegistry): unknown {
  if (PM.sel.keys.length || PM.TL?.keySelectionActive) {
    if (PM.TL) PM.TL.keySelectionActive = true;
    const ids = new Set(PM.sel.keys);
    if (!ids.size) return;
    return PM.hist.do('Delete keyframes', () => {
      PM.proj.layers.forEach((layer: any) => PM.allProps(layer).forEach(({ prop }: any) => {
        prop.kf = prop.kf.filter((key: any) => !ids.has(key.i));
      }));
      PM.sel.keys = [];
      PM.touch(); PM.bus.emit('sel'); PM.invalidate();
    });
  }
  return PM.Edit.apply({ type: 'delete_layers', targets: PM.sel.layers }, { label: 'Delete', origin: 'command' });
}

/** Replace the legacy command set as one in-place registry update during HMR. */
export function reloadShortcuts(PM: PMRegistry): void {
  const kernel = ensureKernel(PM);
  const batch = kernel.commands.batchChanges();
  try {
    kernel.commands.disposeOwner(LEGACY_OWNER);
    install(PM);
    kernel.commands.demoteOwner(LEGACY_OWNER);
  } finally {
    batch.dispose();
  }
}

if (import.meta.hot) {
  import.meta.hot.accept(next => {
    if (next && window.PM) next.reloadShortcuts(window.PM);
  });
}
