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
PM.cmd = (id?: any, ...a: any[]) => { const c: any = kernel.commands.get(id); if (c) return c.run(...a); console.warn('no cmd', id); };

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
def('newShader', 'New shader layer', '⌘⇧G', () => { const L: any = addLayer('shader', { name: 'Shader' }); PM.syncShaderUniforms(L); PM.openShaderEditor(L); return L; }, 'Create');
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
def('delete', 'Delete layers', '⌫', () => {
  if (!PM.sel.keys.length) return PM.Edit.apply({ type: 'delete_layers', targets: PM.sel.layers }, { label: 'Delete', origin: 'command' });
  return PM.hist.do('Delete', () => {
  PM.selLayers().forEach((L: any) => PM.allProps(L).forEach((p: any) => { p.prop.kf = p.prop.kf.filter((k: any) => !PM.sel.keys.includes(k.i)); }));
  PM.sel.keys = []; PM.touch();
  });
}, 'Edit');
def('split', 'Split at playhead', '⌘⇧D', () => PM.hist.do('Split', () => {
  PM.selLayers().forEach((L: any) => {
    if (PM.time <= L.from || PM.time >= L.from + L.dur) return;
    const c: any = PM.cloneLayer(L);
    c.from = PM.time; c.dur = L.from + L.dur - PM.time;
    if (PM.MediaTiming.isTimed(L)) c.d.trim = PM.MediaTiming.trimAtStart(L, PM.time);
    L.dur = PM.time - L.from;
    PM.proj.layers.splice(PM.proj.layers.indexOf(L), 0, c);
  });
  PM.bus.emit('layers');
}), 'Edit');
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
  const sels: any = PM.selLayers(); if (!sels.length) return;
  layerClip = sels.map((L: any) => JSON.parse(JSON.stringify(L)));
  PM.toast(`Copied ${layerClip.length} ${layerClip.length === 1 ? 'layer' : 'layers'}`);
}, 'Edit');
def('pasteLayers', 'Paste layers', '⌘V', () => {
  if (!layerClip || !layerClip.length) return;
  PM.hist.do('Paste layers', () => {
    /* clones keep their relative stack order; parenting inside the set survives */
    const pairs: any = layerClip.map((src: any) => [src, PM.cloneLayer(JSON.parse(JSON.stringify(src)))]);
    const map: any = new Map(pairs.map(([src, c]: any) => [src.id, c]));
    pairs.forEach(([src, c]: any) => { c.parent = src.parent && map.has(src.parent) ? map.get(src.parent).id : null; });
    const pasted: any = pairs.map(([, c]: any) => c);
    for (let i: any = pasted.length - 1; i >= 0; i--) PM.addLayer(pasted[i], 0);
    PM.selectLayers(pasted.map((c: any) => c.id));
  });
  PM.toast(`Pasted ${layerClip.length} ${layerClip.length === 1 ? 'layer' : 'layers'}`);
}, 'Edit');
def('undo', 'Undo', '⌘Z', () => PM.hist.undo(), 'Edit');
def('redo', 'Redo', '⌘⇧Z', () => PM.hist.redo(), 'Edit');

/* ── transport ─────────────────────────────────────────── */
def('play', 'Play / pause', '␣', () => PM.toggle(), 'Transport');
def('gotoStart', 'Go to start', '⇱', () => PM.setTime(0), 'Transport');
def('gotoEnd', 'Go to end', '⇲', () => PM.setTime(PM.proj.dur), 'Transport');
def('nextFrame', 'Next frame', '→', () => PM.step(1), 'Transport');
def('prevFrame', 'Previous frame', '←', () => PM.step(-1), 'Transport');
def('nextEdge', 'Next edge', '⇧→', () => PM.setTime(PM.TL.nextEdge()), 'Transport');
def('prevEdge', 'Previous edge', '⇧←', () => PM.setTime(PM.TL.prevEdge()), 'Transport');
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
def('graph', 'Toggle graph editor', 'G', () => { PM.TL.graph = !PM.TL.graph; PM.invalidate('timeline'); }, 'Reveal');

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
def('fitView', 'Fit composition in view', '⇧F', () => { PM.Viewer.fit = true; PM.Viewer.layout(); PM.TL.frameView(); }, 'View');
def('palette', 'Command palette', '⌘K', () => PM.palette(), 'View');
def('agent', 'Ask Powermove agent', '⌘⇧K', () => PM.SpatialAssistant?.open?.(), 'View');
def('save', 'Save project', '⌘S', () => PM.saveProject(), 'File');
def('open', 'Open project…', '⌘O', () => PM.openProject(), 'File');
def('export', 'Export…', '⌘E', () => PM.Export.dialog(), 'File');
def('projects', 'Projects screen', '⌘P', () => PM.ProjectsScreen && PM.ProjectsScreen.toggle(), 'File');
def('newProject', 'New project', '⌘N', () => PM.newProject(), 'File');
def('takeSave', 'Save take', '⌘⇧S', () => { PM.takes.save(); PM.toast('Take saved'); }, 'File');

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
