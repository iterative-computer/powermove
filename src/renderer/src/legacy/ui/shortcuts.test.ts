import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './shortcuts';
import { makePM } from '../__tests__/make-pm';

const previousWindow = (globalThis as any).window;

afterEach(() => {
  (globalThis as any).window = previousWindow;
});

function shortcutsRegistry(): PMRegistry {
  (globalThis as any).window = { addEventListener() {} };
  const PM: PMRegistry = {
    h() {},
    proj: {
      w: 1920, h: 1080, fps: 30, dur: 8,
      assets: {
        'asset-1': { id: 'asset-1', name: 'Diamonds.mp3', kind: 'audio', dur: 29.58, w: 0, h: 0 },
      },
      layers: [],
    },
    time: 0.9,
    snapF(value: number) { return value; },
  };
  install(PM);
  return PM;
}

function editorRuntime() {
  (globalThis as any).window = { addEventListener() {}, removeEventListener() {} };
  const PM: any = makePM(
    'core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history',
    'core/editing', 'core/capabilities', 'ui/shortcuts',
  );
  PM.proj = PM.mkProject({ w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.time = 5;
  return PM;
}

function layer(PM: any, id: string, opts: any = {}) {
  const value = PM.mkLayer(opts.type || 'solid', {
    name: opts.name || id, from: opts.from ?? 0, dur: opts.dur ?? 10,
    d: opts.d || {},
  });
  value.id = id;
  value.from = opts.from ?? 0;
  value.dur = opts.dur ?? 10;
  if (opts.lock != null) value.lock = opts.lock;
  if (opts.on != null) value.on = opts.on;
  if (opts.parent !== undefined) value.parent = opts.parent;
  PM.proj.layers.push(value);
  return value;
}

function ids(PM: any) {
  return PM.proj.layers.map((value: any) => value.id);
}

function sourceWithoutAudit(project: any) {
  const source = JSON.parse(JSON.stringify(project));
  delete source.revision;
  delete source.edits;
  return source;
}

function expectStructuralReplay(before: any, PM: any, label: string) {
  const edit = PM.proj.edits.at(-1);
  expect(edit).toMatchObject({
    origin: 'command', label, operations: [],
    structural: { format: 'powermove-layer-tree-v1' },
  });
  const replay = JSON.parse(JSON.stringify(before));
  replay.layers = JSON.parse(JSON.stringify(edit.structural.layers));
  replay.comps = JSON.parse(JSON.stringify(edit.structural.comps));
  expect(sourceWithoutAudit(replay)).toEqual(sourceWithoutAudit(PM.proj));
}

describe('legacy shortcut install', () => {
  it('deletes only selected keys and never falls through to layers on repeated Delete', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'ui/shortcuts');
    PM.proj = PM.mkProject();
    const layer = PM.mkLayer('solid'); PM.proj.layers = [layer];
    PM.TL = { keySelectionActive: false };
    PM.setKey(layer, 'scale.x', 0, 100);
    PM.setKey(layer, 'scale.y', 0, 50);
    PM.sel.layers = [layer.id];
    PM.sel.keys = [layer.p['scale.x'].kf[0].i, layer.p['scale.y'].kf[0].i];
    PM.cmd('delete');
    expect(PM.proj.layers).toHaveLength(1);
    expect(layer.p['scale.x'].kf).toHaveLength(0);
    expect(layer.p['scale.y'].kf).toHaveLength(0);
    PM.cmd('delete'); PM.cmd('delete');
    expect(PM.proj.layers).toHaveLength(1);
    PM.hist.undo();
    expect(PM.L(layer.id).p['scale.x'].kf).toHaveLength(1);
    expect(PM.L(layer.id).p['scale.y'].kf).toHaveLength(1);
    PM.selectLayers(layer.id);
    PM.cmd('delete');
    expect(PM.proj.layers).toHaveLength(0);
  });
  it('keeps the canonical command registrations', () => {
    const PM = shortcutsRegistry();

    expect(PM.commands.undo.label).toBe('Undo');
    expect(PM.commands.redo.kb).toBe('⌘⇧Z');
    expect(PM.commands.export.run).toEqual(expect.any(Function));
  });

  it('keeps complete audio source duration and editable content', () => {
    const PM = shortcutsRegistry();
    const command = PM.commandForAsset('asset-1');

    expect(command.layerType).toBe('audio');
    expect(command.duration).toBe(29.58);
    expect(command.from).toBe(0.9);
    expect(command.content).toEqual({ asset: 'asset-1', trim: 0, gain: 1, fadeIn: 0, fadeOut: 0 });
  });

  it('lets the active effect clipboard handle global paste before layers', () => {
    const PM = shortcutsRegistry();
    const pasteCopiedEffects = vi.fn(() => true);
    PM.Inspector = { pasteCopiedEffects };

    PM.cmd('pasteLayers');

    expect(pasteCopiedEffects).toHaveBeenCalledOnce();
  });

  it('lets selected effects handle global copy before layers', () => {
    const PM = shortcutsRegistry();
    const copySelectedEffects = vi.fn(() => true);
    PM.Inspector = { copySelectedEffects };

    PM.cmd('copyLayers');

    expect(copySelectedEffects).toHaveBeenCalledOnce();
  });
});

describe('pro editor shortcut behavior', () => {
  it('returns false for a missing command so stale bindings can fall through', () => {
    const PM = shortcutsRegistry();
    const warning = vi.spyOn(console, 'warn').mockImplementation(() => {});
    expect(PM.cmd('removed-command')).toBe(false);
    expect(warning).toHaveBeenCalledWith('no cmd', 'removed-command');
    warning.mockRestore();
  });

  it('splits selected unlocked active layers, preserves media trim, and selects tails', () => {
    const PM = editorRuntime();
    const parent = layer(PM, 'parent', { from: 0, dur: 10 });
    const video = layer(PM, 'video', { type: 'video', from: 1, dur: 8, d: { trim: 3 }, parent: parent.id });
    const locked = layer(PM, 'locked', { from: 1, dur: 8, lock: true });
    const outside = layer(PM, 'outside', { from: 0, dur: 4 });
    PM.MediaTiming = {
      isTimed: (value: any) => value.type === 'video',
      trimAtStart: (value: any, time: number) => value.d.trim + (time - value.from),
    };
    PM.setKeyOn(video.p['position.x'], 0, 0, 'linear', PM.proj.fps);
    PM.setKeyOn(video.p['position.x'], 8, 80, 'linear', PM.proj.fps);
    video.transitionIn = { type: 'fade', dur: 1, p: {} };
    video.transitionOut = { type: 'fade', dur: 1, p: {} };
    expect(PM.ev(video, 'position.x', PM.time)).toBe(40);
    PM.selectLayers([video.id, locked.id]);
    const beforeSplit = JSON.parse(JSON.stringify(PM.proj));
    PM.cmd('split');

    expect(ids(PM)).toEqual(['parent', expect.any(String), 'video', 'locked', 'outside']);
    const tail = PM.proj.layers[1];
    expect(tail.from).toBe(5);
    expect(tail.dur).toBe(4);
    expect(tail.parent).toBe(parent.id);
    expect(tail.d.trim).toBe(7);
    expect(PM.ev(tail, 'position.x', 5)).toBe(40);
    expect(PM.ev(tail, 'position.x', 6)).toBe(50);
    expect(tail.transitionIn).toBeNull();
    expect(tail.transitionOut?.type).toBe('fade');
    expect(video.transitionIn?.type).toBe('fade');
    expect(video.transitionOut).toBeNull();
    expect(video.dur).toBe(4);
    expect(locked.dur).toBe(8);
    expect(outside.dur).toBe(4);
    expect(PM.sel.layers).toEqual([tail.id]);
    expect(PM.hist.list()).toEqual(['Split']);
    expect(PM.proj.revision).toBe(1);
    expectStructuralReplay(beforeSplit, PM, 'Split');
    expect(PM.hist.undo()).toBe(true);
    expect(ids(PM)).toEqual(['parent', 'video', 'locked', 'outside']);
  });

  it('keeps split audio source time without replaying internal edge fades', () => {
    const PM = editorRuntime();
    const audio = layer(PM, 'audio', {
      type: 'audio', from: 1, dur: 8,
      d: { asset: 'asset', trim: 2, gain: 1, fadeIn: 1, fadeOut: 2 },
    });
    PM.MediaTiming = {
      isTimed: () => true,
      trimAtStart: (value: any, time: number) => value.d.trim + (time - value.from),
    };
    PM.selectLayers([audio.id]);
    PM.cmd('split');

    const tail = PM.firstSel();
    expect(audio.d).toMatchObject({ trim: 2, fadeIn: 1, fadeOut: 0 });
    expect(tail.d).toMatchObject({ trim: 6, fadeIn: 0, fadeOut: 2 });
  });

  it('splits every visible unlocked root layer with no selection and treats boundaries as no-ops', () => {
    const PM = editorRuntime();
    const visible = layer(PM, 'visible', { from: 0, dur: 10 });
    const hidden = layer(PM, 'hidden', { from: 0, dur: 10, on: false });
    const locked = layer(PM, 'locked', { from: 0, dur: 10, lock: true });
    const parent = layer(PM, 'parent', { from: 0, dur: 10 });
    layer(PM, 'child', { from: 0, dur: 10, parent: parent.id });
    PM.selectLayers([]);
    PM.time = 5;
    PM.cmd('split');
    expect(ids(PM)).toEqual([expect.any(String), 'visible', 'hidden', 'locked', expect.any(String), 'parent', 'child']);

    PM.hist.clear();
    PM.selectLayers([visible.id]);
    PM.time = visible.from;
    PM.cmd('split');
    expect(ids(PM)).toHaveLength(7);
    expect(PM.hist.list()).toEqual([]);
    PM.time = visible.from + visible.dur;
    PM.cmd('split');
    expect(ids(PM)).toHaveLength(7);
    expect(PM.hist.list()).toEqual([]);
  });

  it('cuts only editable selected layers and pastes in stack order beside the topmost selection', () => {
    const PM = editorRuntime();
    const parent = layer(PM, 'parent', { from: 0, dur: 10 });
    const child = layer(PM, 'child', { from: 0, dur: 10, parent: parent.id });
    const locked = layer(PM, 'locked', { lock: true });
    const target = layer(PM, 'target');
    PM.selectLayers([child.id, parent.id, locked.id]);
    PM.cmd('copyLayers');
    PM.selectLayers([target.id]);
    const beforePaste = JSON.parse(JSON.stringify(PM.proj));
    PM.cmd('pasteLayers');

    const pasted = PM.proj.layers.filter((value: any) => ![parent.id, child.id, locked.id, target.id].includes(value.id));
    expect(pasted).toHaveLength(3);
    expect(PM.proj.layers.indexOf(pasted[0])).toBe(PM.proj.layers.indexOf(target) - 3);
    expect(pasted[0].parent).toBeNull();
    expect(pasted[1].parent).toBe(pasted[0].id);
    expect(PM.sel.layers).toEqual(pasted.map((value: any) => value.id));
    expect(PM.hist.list()).toEqual(['Paste layers']);
    expectStructuralReplay(beforePaste, PM, 'Paste layers');

    PM.hist.clear();
    PM.selectLayers([parent.id, child.id, locked.id]);
    const beforeCut = JSON.parse(JSON.stringify(PM.proj));
    PM.cmd('cutLayers');
    expect(ids(PM)).toEqual(['locked', pasted[0].id, pasted[1].id, pasted[2].id, target.id]);
    expect(PM.L(locked.id)).toBeTruthy();
    expect(PM.sel.layers).toEqual([locked.id]);
    expect(PM.hist.list()).toEqual(['Cut layers']);
    expectStructuralReplay(beforeCut, PM, 'Cut layers');
    PM.cmd('pasteLayers');
    const cutPaste = PM.proj.layers.filter((value: any) =>
      !['locked', 'target', pasted[0].id, pasted[1].id, pasted[2].id].includes(value.id));
    expect(cutPaste).toHaveLength(2);
    expect(cutPaste[1].parent).toBe(cutPaste[0].id);
    PM.hist.undo();
    expect(ids(PM)).toEqual(['locked', pasted[0].id, pasted[1].id, pasted[2].id, target.id]);
    PM.hist.undo();
    expect(ids(PM).slice(0, 3)).toEqual(['parent', 'child', 'locked']);
  });

  it('drops an external clipboard parent only when that parent is gone', () => {
    const PM = editorRuntime();
    const parent = layer(PM, 'parent');
    const child = layer(PM, 'child', { parent: parent.id });
    PM.selectLayers([child.id]);
    PM.cmd('copyLayers');
    PM.selectLayers([]);
    PM.cmd('pasteLayers');
    const firstPaste = PM.proj.layers.find((value: any) => value.id !== parent.id && value.id !== child.id);
    expect(firstPaste.parent).toBe(parent.id);

    PM.selectLayers([parent.id]);
    PM.cmd('cutLayers');
    PM.selectLayers([firstPaste.id]);
    PM.cmd('pasteLayers');
    const copies = PM.proj.layers.filter((value: any) => value.id !== child.id && value.id !== firstPaste.id);
    expect(copies.at(-1).parent).toBeNull();
  });

  it('toggles editable visibility as a group and leaves locked layers alone', () => {
    const PM = editorRuntime();
    const shown = layer(PM, 'shown', { on: true });
    const hidden = layer(PM, 'hidden', { on: false });
    const locked = layer(PM, 'locked', { on: true, lock: true });
    PM.selectLayers([shown.id, hidden.id, locked.id]);
    PM.cmd('toggleVisibility');
    expect([PM.L(shown.id).on, PM.L(hidden.id).on, PM.L(locked.id).on]).toEqual([false, false, true]);
    PM.cmd('toggleVisibility');
    expect([PM.L(shown.id).on, PM.L(hidden.id).on, PM.L(locked.id).on]).toEqual([true, true, true]);
    expect(PM.hist.list()).toEqual(['Hide layers', 'Show layers']);
    PM.hist.undo();
    expect([PM.L(shown.id).on, PM.L(hidden.id).on, PM.L(locked.id).on]).toEqual([false, false, true]);
  });

  it('orders selected layers as a stable group without crossing locked layers', () => {
    const PM = editorRuntime();
    const a = layer(PM, 'a');
    const b = layer(PM, 'b');
    const c = layer(PM, 'c');
    const d = layer(PM, 'd');
    PM.selectLayers([b.id, d.id]);
    PM.cmd('bringForward');
    expect(ids(PM)).toEqual(['b', 'a', 'd', 'c']);
    PM.cmd('sendToBack');
    expect(ids(PM)).toEqual(['a', 'c', 'b', 'd']);
    PM.hist.undo();
    expect(ids(PM)).toEqual(['b', 'a', 'd', 'c']);

    const barrier = layer(PM, 'barrier', { lock: true });
    PM.proj.layers = [a, barrier, b, c, d];
    PM.selectLayers([b.id]);
    PM.hist.clear();
    PM.cmd('bringToFront');
    expect(ids(PM)).toEqual(['a', 'barrier', 'b', 'c', 'd']);
    expect(PM.hist.list()).toEqual([]);
  });

  it('nudges through typed property edits, including current keyframes and locks', () => {
    const PM = editorRuntime();
    const movable = layer(PM, 'movable');
    const locked = layer(PM, 'locked', { lock: true });
    PM.setKeyOn(movable.p['position.x'], 5, 100, 'linear', PM.proj.fps);
    PM.selectLayers([movable.id, locked.id]);
    const apply = vi.spyOn(PM.Edit, 'apply');
    const result = PM.cmd('nudgeSelection', 5, -2);
    expect(result.ok).toBe(true);
    expect(apply).toHaveBeenCalledWith(expect.arrayContaining([
      expect.objectContaining({ type: 'set_property', target: movable.id, path: 'position.x', value: 105, preserveHandEdits: false, markIntent: 'human' }),
      expect.objectContaining({ type: 'set_property', target: movable.id, path: 'position.y', value: -2, preserveHandEdits: false, markIntent: 'human' }),
    ]), expect.objectContaining({ origin: 'command', historyGroup: 'keyboard-nudge' }));
    expect(movable.p['position.x'].kf.at(-1).v).toBe(105);
    expect(PM.evP(movable, movable.p['position.y'], PM.time, 'position.y')).toBe(-2);
    expect(locked.p['position.x'].v).toBe(0);
    expect(PM.hist.list()).toEqual(['Nudge selection']);
    expect(PM.cmd('nudgeSelection', 0, 0)).toBeUndefined();
    expect(PM.cmd('nudgeSelection', Infinity, 1)).toBeUndefined();
    apply.mockRestore();
  });

  it('nudges parented layers in composition space', () => {
    const PM = editorRuntime();
    const parent = layer(PM, 'parent');
    const child = layer(PM, 'child', { parent: parent.id });
    parent.p.rotation.v = 90;
    PM.selectLayers([child.id]);
    const before = PM.worldMatrix(child, PM.time);

    PM.cmd('nudgeSelection', 10, 0);

    const after = PM.worldMatrix(child, PM.time);
    expect(after[4] - before[4]).toBeCloseTo(10, 6);
    expect(after[5] - before[5]).toBeCloseTo(0, 6);
    expect(PM.hist.list()).toEqual(['Nudge selection']);
  });

  it('treats nudging as a human edit after canvas intent has been recorded', () => {
    const PM = editorRuntime();
    const movable = layer(PM, 'movable');
    movable.locked_intent['position.x'] = { by: 'human', at: Date.now(), t: 0 };
    movable.locked_intent['position.y'] = { by: 'human', at: Date.now(), t: 0 };
    PM.selectLayers([movable.id]);

    const result = PM.cmd('nudgeSelection', 2, -1);

    expect(result.ok).toBe(true);
    expect(PM.ev(movable, 'position.x', PM.time)).toBe(2);
    expect(PM.ev(movable, 'position.y', PM.time)).toBe(-1);
    expect(movable.locked_intent['position.x'].by).toBe('human');
  });

  it('coalesces a rapid keyboard nudge sequence into one undo step', () => {
    vi.useFakeTimers();
    try {
      const PM = editorRuntime();
      const movable = layer(PM, 'movable');
      PM.selectLayers([movable.id]);

      PM.cmd('nudgeSelection', 1, 0);
      PM.cmd('nudgeSelection', 1, 0);
      PM.cmd('nudgeSelection', 1, 0);
      expect(PM.hist.list()).toEqual(['Nudge selection', 'Nudge selection', 'Nudge selection']);
      vi.advanceTimersByTime(180);

      expect(PM.hist.list()).toEqual(['Nudge selection']);
      expect(PM.ev(movable, 'position.x', PM.time)).toBe(3);
      expect(PM.hist.undo()).toBe(true);
      expect(PM.ev(PM.L(movable.id), 'position.x', PM.time)).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps zoom commands on the viewer, clamps them, and leaves timeline framing alone', () => {
    const PM = editorRuntime();
    const layout = vi.fn();
    const frameView = vi.fn();
    PM.Viewer = { fit: true, zoom: 1, shown: 2, layout };
    PM.TL = { frameView };
    expect(PM.commands.split.kb).toBe('⌘⇧D');
    expect(PM.commands.cutLayers.kb).toBe('⌘X');
    expect(PM.commands.toggleVisibility.kb).toBeNull();
    expect(PM.commands.toggleLayerControls.kb).toBe('⌘⇧H');
    expect(PM.commands.bringForward.kb).toBe('⌘]');
    expect(PM.commands.zoomIn.kb).toBe('.');
    expect(PM.commands.actualSize.kb).toBe('/');
    expect(PM.commands.fitView.kb).toBe('⇧F');
    expect(PM.commands.fitComposition.kb).toBe('⇧/');

    PM.cmd('zoomIn');
    expect(PM.Viewer).toMatchObject({ fit: false, zoom: 2.5 });
    PM.Viewer.shown = 100;
    PM.cmd('zoomIn');
    expect(PM.Viewer.zoom).toBe(8);
    PM.Viewer.shown = .001;
    PM.cmd('zoomOut');
    expect(PM.Viewer.zoom).toBe(.05);
    PM.cmd('actualSize');
    expect(PM.Viewer).toMatchObject({ fit: false, zoom: 1 });
    expect(layout).toHaveBeenCalledTimes(4);
    expect(frameView).not.toHaveBeenCalled();
    PM.Viewer.pan = [4, 8];
    PM.cmd('fitComposition');
    expect(PM.Viewer).toMatchObject({ fit: true, pan: [0, 0] });
    expect(frameView).not.toHaveBeenCalled();
  });
});
