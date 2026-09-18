// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resetExtensionsStore } from './extensions.svelte';
import { bootExtensions, installKernel, type InstalledKernel } from './install';
import { RUNTIME_GLOBAL, runtimeGlobals } from './runtime-globals';
import { doc } from '../state/document.svelte';
import { sel } from '../state/selection.svelte';
import { perf, transport } from '../state/transport.svelte';

type LegacyPM = Record<string, any>;

function fakePM(): LegacyPM {
  const listeners = new Map<string, Set<(...args: any[]) => void>>();
  const store: Record<string, unknown> = {};
  const workspace = { id: 'design', panels: ['viewer'] };
  const PM: LegacyPM = {
    proj: { id: 'p', revision: 7, assets: {} },
    sel: { layers: ['L1'], keys: ['k1', 4], chan: 'position.x' },
    time: 12,
    playing: false,
    PANELS: { viewer: {} },
    bus: {
      on: (event: string, fn: (...args: any[]) => void) => {
        const set = listeners.get(event) ?? new Set();
        set.add(fn);
        listeners.set(event, set);
        return () => void set.delete(fn);
      },
      emit: (event: string, ...args: any[]) => listeners.get(event)?.forEach((fn) => fn(...args))
    },
    Edit: { apply: vi.fn(() => ({ ok: true, message: 'ok', data: {} })) },
    hist: { undo: vi.fn(), redo: vi.fn() },
    Export: { snapshot: vi.fn(async () => 'data:image/jpeg;base64,x') },
    assets: {
      add: vi.fn(async (file: File, options: any) => {
        const asset = { id: 'asset-1', name: file.name, kind: 'model', sourceText: await file.text() };
        PM.proj.assets[asset.id] = { id: asset.id, name: asset.name, kind: asset.kind, layerDefinition: options?.layerDefinition };
        return asset;
      }),
      get: vi.fn(() => null)
    },
    selectLayers: vi.fn(),
    setTime: vi.fn(),
    play: vi.fn(),
    pause: vi.fn(),
    toast: vi.fn(),
    menu: vi.fn(),
    icon: (name: string) => `<svg data-icon="${name}"></svg>`,
    modal: vi.fn((opts: any) => ({ close: vi.fn(), body: document.createElement('div'), opts })),
    palette: vi.fn(),
    cmd: vi.fn(),
    store: {
      get: (key: string, fallback: unknown) => store[key] ?? fallback,
      set: (key: string, value: unknown) => void (store[key] = value)
    },
    WS: { current: workspace, mutate: vi.fn((fn: (w: unknown) => void) => fn(workspace)) },
    Layout: { addPanel: vi.fn(), removePanel: vi.fn(), hasPanel: vi.fn(() => false), refresh: vi.fn() },
    listeners,
    store_: store
  };
  return PM;
}

let installed: InstalledKernel | null = null;

beforeEach(() => resetExtensionsStore());

afterEach(() => {
  installed?.uninstall();
  installed = null;
  delete (globalThis as Record<string, any>)[RUNTIME_GLOBAL];
  delete (globalThis as Record<string, any>).powermove;
});

describe('installKernel', () => {
  it('routes native and API raster calls through disposable extension overrides', () => {
    const PM = fakePM();
    const native = vi.fn(() => ({ cv: document.createElement('canvas'), w: 20, h: 10 }));
    PM.raster = native;
    installed = installKernel(PM);
    const api = installed.api('typing');
    const original = api.services.get<(...args: any[]) => any>('raster');
    expect(original).toBeTypeOf('function');
    const layer = { type: 'text' } as any;
    const override = api.services.register('raster', (...args: any[]) => ({ ...original!(...args), w: 30 }));
    expect(PM.raster(layer).w).toBe(30);
    expect(api.render.raster(layer)?.w).toBe(30);
    expect(native).toHaveBeenCalledTimes(2);
    override.dispose();
    expect(PM.raster(layer).w).toBe(20);
    installed.uninstall();
    installed = null;
    expect(PM.raster).toBe(native);
  });

  it('accepts the rasterizer installed after kernel boot and forwards all arguments', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const native = vi.fn(() => null);
    PM.raster = native;
    const api = installed.api('typing');
    const original = api.services.get<(...args: any[]) => any>('raster')!;
    const args = [{ type: 'shape' }, 2, 1.5, vi.fn(), { x: 1, y: 2, width: 3, height: 4 }] as const;
    api.services.register('raster', (...values: any[]) => original(...values));
    PM.raster(...args);
    expect(native).toHaveBeenCalledExactlyOnceWith(...args);
  });

  it('allows legacy instrumentation to wrap the installed raster without recursion', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const native = vi.fn(() => ({ w: 20 }));
    PM.raster = native;
    const original = PM.raster;
    PM.raster = vi.fn((...args: any[]) => original(...args));
    expect(installed.api('probe').render.raster({ type: 'text' } as any)?.w).toBe(20);
    expect(native).toHaveBeenCalledOnce();
    expect(PM.raster).toHaveBeenCalledOnce();
  });

  it('installs the svelte runtime globals and publishes the kernel on PM', () => {
    const PM = fakePM();
    installed = installKernel(PM);

    expect(PM.Kernel).toBe(installed);
    expect(runtimeGlobals()?.svelte.mount).toBeTypeOf('function');
    expect(runtimeGlobals()?.['svelte/store'].writable).toBeTypeOf('function');
    expect(installed.loader).toBeNull();
  });

  it('forwards model.cloneLayer to the canonical model helper', () => {
    const PM = fakePM();
    const layer = { id: 'source', name: 'Source', p: {}, d: {} } as any;
    const clone = { ...layer, id: 'clone' };
    PM.cloneLayer = vi.fn(() => clone);
    installed = installKernel(PM);

    expect(installed.api('contracts').model.cloneLayer(layer)).toBe(clone);
    expect(PM.cloneLayer).toHaveBeenCalledWith(layer);
  });

  it('forwards model.normalizeFill with its optional fallback', () => {
    const PM = fakePM();
    const fill = { type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: '#123456', position: 0 }] };
    PM.normalizeFill = vi.fn(() => fill);
    installed = installKernel(PM);

    expect(installed.api('contracts').model.normalizeFill({ color: 'bad' }, '#123456')).toBe(fill);
    expect(PM.normalizeFill).toHaveBeenCalledWith({ color: 'bad' }, '#123456');
  });

  it('forwards uiState.getShaderMeta to the UI-state cache', () => {
    const PM = fakePM();
    const layer = { id: 'shader', name: 'Shader', p: {}, d: {} } as any;
    const meta = { shaderKey: 'shader:key', udefs: [] };
    PM.UIState = { getShaderMeta: vi.fn(() => meta) };
    installed = installKernel(PM);

    expect(installed.api('contracts').uiState.getShaderMeta(layer)).toBe(meta);
    expect(PM.UIState.getShaderMeta).toHaveBeenCalledWith(layer);
  });

  it('forwards render.gl.compileError to the compositor diagnostics', () => {
    const PM = fakePM();
    PM.GL = { compileError: vi.fn(() => 'line 1: syntax error') };
    installed = installKernel(PM);

    expect(installed.api('contracts').render.gl.compileError('shader:key')).toBe('line 1: syntax error');
    expect(PM.GL.compileError).toHaveBeenCalledWith('shader:key');
  });

  it('publishes every Phase 1 adapter member', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const api = installed.api('contracts');

    expect(Object.keys(api.anim)).toEqual(expect.arrayContaining([
      'ev', 'evP', 'active', 'findProp', 'allProps', 'hasKeyAt', 'setKey', 'setKeyOn', 'removeKey',
      'applyEaseTo', 'wouldCycle', 'resolveContent', 'expressionErrors', 'version', 'touch', 'worldMatrix',
      'localMatrix', 'transformParentMatrix', 'mul'
    ]));
    expect(Object.keys(api.model)).toEqual(expect.arrayContaining([
      'P', 'CH', 'KF', 'BLENDS', 'TYPE_META', 'MASK_SHAPES', 'mkLayer', 'mkMask', 'mkProject',
      'cloneLayer', 'normalizeFill', 'layerDefinition', 'curComp', 'layer', 'byName'
    ]));
    expect(Object.keys(api.selection)).toEqual(expect.arrayContaining([
      'get', 'layers', 'first', 'keys', 'chan', 'set', 'select', 'resolveSelectedKeys', 'keySelectionActive'
    ]));
    expect(Object.keys(api.groups)).toEqual(expect.arrayContaining(['ancestors', 'transformRoots', 'span', 'expand', 'normalizeStack', 'moveToGroup']));
    expect(Object.keys(api.transport)).toEqual(expect.arrayContaining(['time', 'setTime', 'play', 'pause', 'toggle', 'playing', 'step', 'quality', 'perf', 'invalidate', 'previewResolution']));
    expect(Object.keys(api.history)).toEqual(expect.arrayContaining(['do', 'begin', 'commit', 'cancel', 'undo', 'redo', 'external', 'selection']));
    expect(Object.keys(api.edit)).toEqual(expect.arrayContaining(['apply', 'begin', 'commit', 'cancel', 'dispatch', 'mutate']));
    expect(Object.keys(api.media)).toEqual(expect.arrayContaining(['timing', 'importFiles', 'commandForAsset', 'audio', 'assets', 'fonts']));
    expect(Object.keys(api.render)).toEqual(expect.arrayContaining(['gl', 'raster', 'renderFrameTo', 'snapshot']));
    expect(Object.keys(api.uiState)).toEqual(expect.arrayContaining([
      'getLayerCollapsed', 'setLayerCollapsed', 'getGroupCollapsed', 'setGroupCollapsed',
      'getKeyHandles', 'setKeyHandles', 'getFxOpen', 'setFxOpen', 'getReveal', 'setReveal',
      'getShaderMeta', 'setShaderMeta',
    ]));
    expect(Object.keys(api.ui)).toEqual(expect.arrayContaining(['drag', 'closeMenus', 'showLayerMenu', 'showParentMenu', 'beginParentPick', 'openShaderEditor', 'gesture']));
    expect(Object.keys(api.dnd)).toEqual(expect.arrayContaining([
      'ASSET_MIME', 'FX_MIME', 'startAssetDrag', 'mediaDrag', 'hasAssetDrag', 'hasFileDrag',
      'hasMediaDrag', 'readAssetDrag', 'hasFxDrag', 'readFxDrag', 'applyFxDrop'
    ]));
    expect(Object.keys(api.workspace)).toEqual(expect.arrayContaining(['current', 'mutate', 'hasPanel', 'addPanel', 'movePanel', 'removePanel', 'hidePanel', 'restorePanel', 'refresh']));
    expect(Object.keys(api.util)).toEqual(expect.arrayContaining(['round', 'clamp', 'lerp', 'snapF', 'tc', 'parseTc', 'uid', 'hex2rgb', 'rgb2hex']));
    expect(Object.keys(api.ease)).toEqual(expect.arrayContaining(['nameOf', 'PRESETS']));
    expect(Object.keys(api.space3d)).toEqual(expect.arrayContaining(['CHANNELS_3D', 'local3D', 'parent3D', 'world3D', 'is3DLayer', 'perspectiveAmount', 'planeMatrix', 'projectPoint', 'inversePlane', 'planeContains']));
    expect(Object.keys(api.services)).toEqual(['register', 'get']);
  });

  it('forwards Phase 1 adapter calls and mutable properties to PM', async () => {
    const PM = fakePM();
    const call = (...args: unknown[]) => args;
    for (const name of ['ev', 'evP', 'active', 'findProp', 'allProps', 'hasKeyAt', 'setKey', 'setKeyOn', 'removeKey',
      'applyEaseTo', 'wouldCycle', 'resolveContent', 'animVersion', 'touch', 'worldMatrix', 'localMatrix',
      'transformParentMatrix', 'mul', 'P', 'KF', 'mkLayer', 'mkMask', 'mkProject', 'cloneLayer', 'normalizeFill', 'curComp', 'L', 'byName',
      'firstSel', 'selectLayers', 'resolveSelectedKeys', 'groupAncestors', 'transformRoots', 'groupSpan',
      'expandGroups', 'normalizeStack', 'moveToGroup', 'toggle', 'step', 'invalidate', 'importFiles',
      'commandForAsset', 'assetKind', 'raster', 'renderFrameTo', 'round', 'clamp', 'lerp', 'snapF', 'tc',
      'parseTc', 'uid', 'hex2rgb', 'rgb2hex', 'drag', 'closeMenus', 'showLayerMenu', 'showParentMenu',
      'beginParentPick', 'openShaderEditor'] as const) PM[name] = vi.fn(call);
    PM.animVersion = vi.fn(() => 9);
    PM.active = vi.fn(() => true);
    PM.wouldCycle = vi.fn(() => true);
    PM.round = vi.fn(() => 2);
    PM.clamp = vi.fn(() => 3);
    PM.lerp = vi.fn(() => 4);
    PM.snapF = vi.fn(() => 5);
    PM.tc = vi.fn(() => 'tc');
    PM.parseTc = vi.fn(() => 6);
    PM.uid = vi.fn(() => 'id');
    PM.rgb2hex = vi.fn(() => '#fff');
    PM.CH = { opacity: { label: 'Opacity', group: 'Transform' } };
    PM.BLENDS = ['normal'];
    PM.TYPE_META = { solid: { icon: 'grid', color: '#fff', label: 'Solid' } };
    PM.MASK_SHAPES = ['rect'];
    PM.expressionErrors = new WeakMap();
    PM.quality = 1;
    PM.previewResolution = 'auto';
    PM.perf = { fps: 30, ms: 2, drops: 0, budget: 16, auto: true };
    PM.TL = { keySelectionActive: false };
    PM.MediaTiming = { isTimed: vi.fn(() => true), rate: vi.fn(() => 2), earliestStart: vi.fn(() => 3) };
    PM.Audio = { drawWaveform: vi.fn(() => true) };
    PM.assets = { get: vi.fn(call), add: vi.fn(async (...args: unknown[]) => args) };
    PM.Fonts = { bundled: [], system: [], families: [], setSystemFamilies: vi.fn(), options: vi.fn(() => []), ensure: vi.fn(async () => {}) };
    PM.GL = {
      bounds: vi.fn(call), pick: vi.fn(call), init: vi.fn(() => true), resize: vi.fn(() => true),
      compileError: vi.fn(() => 'compile failed'),
      previewViewport: { x: 1 }, gl: { drawingBufferWidth: 1 }
    };
    PM.Export.snapshot = vi.fn(() => 'snapshot');
    PM.UIState = Object.fromEntries(['getLayerCollapsed', 'setLayerCollapsed', 'getKeyHandles', 'setKeyHandles', 'getFxOpen', 'setFxOpen', 'getReveal', 'setReveal', 'getShaderMeta', 'setShaderMeta'].map((name) => [name, vi.fn(call)]));
    PM.Ease = { nameOf: vi.fn(() => 'linear'), PRESETS: { linear: [0, 0, 1, 1] } };
    PM.Edit = Object.fromEntries(['apply', 'begin', 'commit', 'cancel', 'dispatch', 'mutate'].map((name) => [name, vi.fn(call)]));
    PM.hist = Object.fromEntries(['do', 'begin', 'commit', 'cancel', 'undo', 'redo', 'external', 'selection'].map((name) => [name, vi.fn(call)]));
    PM.WS.current = {
      schemaVersion: 1, id: 'design', name: 'Design', scope: 'global', projectId: null,
      density: 'normal', theme: {}, chrome: {}, features: {}, custom: [], hiddenPanels: [],
      layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] }
    };
    PM.WS.mutate = vi.fn((fn: (workspace: unknown) => void, options?: unknown) => {
      fn(PM.WS.current);
      return options ? PM.WS.current : PM.WS.current;
    });
    PM.Layout.refresh = vi.fn();

    installed = installKernel(PM);
    const api = installed.api('forwarding');
    const layer = { id: 'L1', from: 0, parent: null, threeD: false, p: {}, d: {} } as any;
    const prop = { v: 1, kf: [], expr: null } as any;
    const key = { i: 'k1', t: 0, v: 1 } as any;

    api.anim.ev(layer, 'x', 1); expect(PM.ev).toHaveBeenLastCalledWith(layer, 'x', 1);
    api.anim.evP(layer, prop, 1, 'x'); expect(PM.evP).toHaveBeenLastCalledWith(layer, prop, 1, 'x');
    api.anim.active(layer, 1); api.anim.findProp(layer, 'x'); api.anim.allProps(layer); api.anim.hasKeyAt(layer, prop, 1);
    api.anim.setKey(layer, 'x', 1, 2, 'linear'); api.anim.setKeyOn(prop, 1, 2, 'linear', 30);
    api.anim.removeKey(prop, key); api.anim.applyEaseTo([key], 'linear'); api.anim.wouldCycle(layer, 'L2');
    api.anim.resolveContent(layer, 1); api.anim.version(); api.anim.touch(); api.anim.worldMatrix(layer, 1);
    api.anim.localMatrix(layer, 1); api.anim.transformParentMatrix(layer, 1, null); api.anim.mul([1, 0, 0, 1, 0, 0], [1, 0, 0, 1, 2, 3]);
    expect(PM.setKey).toHaveBeenLastCalledWith(layer, 'x', 1, 2, 'linear');
    expect(api.anim.expressionErrors).toBe(PM.expressionErrors);

    api.model.P(1, { expr: 'x' }); api.model.KF(1, 2, 'linear'); api.model.mkLayer('solid', {}, PM.proj);
    api.model.mkMask('rect', PM.proj); api.model.mkProject({ name: 'P' } as any); api.model.cloneLayer(layer); api.model.normalizeFill({}, '#123456'); api.model.curComp(); api.model.layer('L1'); api.model.byName('Layer');
    expect(PM.P).toHaveBeenLastCalledWith(1, { expr: 'x' });
    expect(api.model.CH).toBe(PM.CH); expect(api.model.BLENDS).toBe(PM.BLENDS); expect(api.model.TYPE_META).toBe(PM.TYPE_META); expect(api.model.MASK_SHAPES).toBe(PM.MASK_SHAPES);

    api.selection.get(); api.selection.layers(); api.selection.first(); api.selection.keys(); api.selection.chan();
    api.selection.select(['L1'], true); api.selection.resolveSelectedKeys(); api.selection.keySelectionActive = true;
    expect(PM.selectLayers).toHaveBeenLastCalledWith(['L1'], true); expect(PM.TL.keySelectionActive).toBe(true);
    const sameSelection = PM.sel; api.selection.set({ chan: 'opacity' }); expect(PM.sel).toBe(sameSelection);
    expect(PM.hist.selection).toHaveBeenCalled(); expect(PM.bus.emit).toBeTypeOf('function'); expect(PM.invalidate).toHaveBeenCalled();

    api.groups.ancestors(layer, [layer]); api.groups.transformRoots(['L1']); api.groups.span(layer);
    api.groups.expand(['L1']); api.groups.normalizeStack(); api.groups.moveToGroup(['L1'], 'G1');
    expect(PM.groupAncestors).toHaveBeenLastCalledWith(layer, [layer]);

    api.transport.setTime(4, { raw: true }); api.transport.play(); api.transport.pause(); api.transport.toggle(); api.transport.step(2); api.transport.invalidate('render');
    api.transport.quality = .5; api.transport.previewResolution = '0.5';
    expect(PM.setTime).toHaveBeenLastCalledWith(4, { raw: true }); expect(PM.quality).toBe(.5); expect(PM.previewResolution).toBe('0.5'); expect(api.transport.perf).toBe(PM.perf);

    const fn = vi.fn();
    api.history.do('Do', fn); api.history.begin('Begin', 'g'); api.history.commit('Commit'); api.history.cancel(); api.history.undo(); api.history.redo(); api.history.external('External', fn, fn, { bytes: 4 }); api.history.selection(PM.sel, PM.sel);
    expect(PM.hist.external).toHaveBeenLastCalledWith('External', fn, fn, { bytes: 4 });
    api.edit.apply({ type: 'delete_layers', targets: [] }); api.edit.begin('Begin', { origin: 'test' }); api.edit.commit('Commit'); api.edit.cancel(); api.edit.dispatch({ type: 'delete_layers', targets: [] }); api.edit.mutate('Mutate', fn, { origin: 'test' });
    expect(PM.Edit.mutate).toHaveBeenLastCalledWith('Mutate', fn, { origin: 'test' });

    await api.media.importFiles([], { placement: null }); api.media.commandForAsset('a', 2); api.media.timing.isTimed(layer); api.media.timing.rate(layer); api.media.timing.earliestStart(layer);
    const canvas = document.createElement('canvas'); api.media.audio.drawWaveform(canvas.getContext('2d')!, layer, { color: 'red' }); api.media.assets.get('a'); await api.media.assets.add(new File(['x'], 'x.png'), { persist: true }); api.media.assets.kind(new File(['x'], 'x.png'));
    expect(PM.importFiles).toHaveBeenLastCalledWith([], { placement: null }); expect(PM.Audio.drawWaveform).toHaveBeenCalled(); expect(api.media.fonts).toBe(PM.Fonts);

    api.render.gl.bounds(layer, 1); api.render.gl.pick(1, 2, 3, { includeLocked: true }); api.render.gl.init(canvas, { alpha: true }); api.render.gl.resize(100, 50, null); api.render.gl.compileError('shader:key');
    api.render.raster(layer, 2, 3, fn, { x: 1 }); api.render.renderFrameTo(1, 100, 50, { alpha: true }); api.render.snapshot(1, 480);
    expect(PM.GL.pick).toHaveBeenLastCalledWith(1, 2, 3, { includeLocked: true }); expect(api.render.gl.previewViewport).toBe(PM.GL.previewViewport); expect(api.render.gl.context).toBe(PM.GL.gl);

    for (const name of Object.keys(PM.UIState)) (api.uiState as any)[name](layer, {});
    api.ui.drag(new PointerEvent('pointerdown'), { move: fn }); api.ui.closeMenus(); api.ui.showLayerMenu(layer, { clientX: 1, clientY: 2 }, 'timeline'); api.ui.showParentMenu(['L1'], { clientX: 1, clientY: 2 }); api.ui.beginParentPick(new PointerEvent('pointerdown'), ['L1']); api.ui.openShaderEditor(layer);
    expect(PM.showLayerMenu).toHaveBeenLastCalledWith(layer, { clientX: 1, clientY: 2 }, 'timeline');
    const gesture = new api.ui.gesture({ mode: 'command', label: 'Gesture', command: { type: 'delete_layers', targets: [] } }); gesture.begin(); gesture.write(1); gesture.commit(); gesture.cancel(); gesture.once(2);
    expect(PM.Edit.begin).toHaveBeenCalledWith('Gesture', { origin: 'interface' });

    api.dnd.mediaDrag = { id: 'a', name: 'A', kind: 'image' }; expect(PM.mediaDrag).toEqual({ id: 'a', name: 'A', kind: 'image' });
    const transfer = new DataTransfer(); api.dnd.startAssetDrag(transfer, PM.mediaDrag); expect(api.dnd.readAssetDrag(transfer)).toEqual(PM.mediaDrag); expect(api.dnd.hasAssetDrag(transfer)).toBe(true); expect(api.dnd.hasMediaDrag(transfer)).toBe(true); api.dnd.hasFileDrag(transfer); api.dnd.hasFxDrag(transfer); api.dnd.readFxDrag(transfer);
    api.dnd.applyFxDrop({ kind: 'effect', id: 'blur', label: 'Blur' }, 'L1', 'in');

    const mutateOptions = { inPlace: true }; api.workspace.mutate(() => {}, mutateOptions); expect(PM.WS.mutate).toHaveBeenLastCalledWith(expect.any(Function), mutateOptions);
    api.panels.open('indexed', { dock: 'right', index: 0 });
    expect(PM.WS.current.layout.docks.find((dock: any) => dock.id === 'right')?.panels[0]?.id).toBe('indexed');
    api.workspace.hasPanel('viewer'); api.workspace.addPanel('notes', 'right', 0); expect(api.workspace.hasPanel('notes')).toBe(true);
    expect(api.workspace.movePanel('notes', 'center', 1)).toBe(true); api.workspace.hidePanel('notes'); api.workspace.restorePanel('notes'); api.workspace.removePanel('notes'); api.workspace.refresh('viewer'); expect(PM.Layout.refresh).toHaveBeenCalledWith('viewer');

    api.util.round(1, 2); api.util.clamp(1, 0, 2); api.util.lerp(0, 2, .5); api.util.snapF(1, 30); api.util.tc(1, 30, true); api.util.parseTc('1', 30); api.util.uid('x'); api.util.hex2rgb('#fff'); api.util.rgb2hex(1, 1, 1);
    expect(PM.lerp).toHaveBeenLastCalledWith(0, 2, .5); api.ease.nameOf([0, 0], [1, 1]); expect(PM.Ease.nameOf).toHaveBeenCalledWith([0, 0], [1, 1]); expect(api.ease.PRESETS).toBe(PM.Ease.PRESETS);

    api.space3d.local3D(layer, 1); api.space3d.parent3D(layer, 1, null); api.space3d.world3D(layer, 1); api.space3d.is3DLayer(layer); api.space3d.perspectiveAmount(layer, 1); api.space3d.planeMatrix(layer, 1); api.space3d.projectPoint([1, 0, 0, 0, 1, 0, 0, 0, 1], { x: 1, y: 2 }); api.space3d.inversePlane([1, 0, 0, 0, 1, 0, 0, 0, 1]); api.space3d.planeContains(layer, 1, 1, 2, { x0: 0, x1: 3, y0: 0, y1: 3 });
    expect(PM.localMatrix).toHaveBeenCalledWith(layer, 1); expect(api.space3d.CHANNELS_3D).toHaveProperty('position.z');

    const service = { value: 1 }; const handle = api.services.register('demo', service); expect(installed.services.get('demo')).toBe(service); handle.dispose(); expect(api.services.get('demo')).toBeNull();
  });

  it('builds a project façade over PM and forces the extension origin', async () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const api = installed.api('demo');

    expect(api.project.get()).toBe(PM.proj);
    expect(api.project.revision()).toBe(7);
    expect(api.project.time()).toBe(12);
    expect(api.project.playing()).toBe(false);
    expect(api.project.selection()).toEqual({ layers: ['L1'], keys: ['k1'], chan: 'position.x' });

    api.project.apply({ type: 'set_layer', target: 'L1', patch: {} } as never, { label: 'Nudge' } as never);
    expect(PM.Edit.apply).toHaveBeenCalledWith({ type: 'set_layer', target: 'L1', patch: {} }, { label: 'Nudge', origin: 'ext:demo' });

    api.project.select(['L2'], true);
    api.project.setTime(3);
    api.project.play();
    api.project.pause();
    api.project.undo();
    api.project.redo();
    expect(PM.selectLayers).toHaveBeenCalledWith(['L2'], true);
    expect(PM.setTime).toHaveBeenCalledWith(3);
    expect(PM.hist.undo).toHaveBeenCalled();
    expect(await api.project.snapshot()).toBe('data:image/jpeg;base64,x');
  });

  it('imports and reads durable project assets through the extension façade', async () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const api = installed.api('models');
    const file = new File(['v 0 0 0'], 'model.obj', { type: 'model/obj' });
    const asset = await api.assets.import(file, { layerDefinition: 'models.obj' });
    expect(asset).toMatchObject({ id: 'asset-1', name: 'model.obj', kind: 'model', layerDefinition: 'models.obj' });
    expect(PM.assets.add).toHaveBeenCalledWith(file, { layerDefinition: 'models.obj' });
  });

  it('namespaces storage under ext.<id> as one JSON object', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const api = installed.api('notes');

    api.storage.set('draft', 'hello');
    api.storage.set('count', 2);
    expect(PM.store.get('ext.notes', {})).toEqual({ draft: 'hello', count: 2 });

    api.storage.delete('draft');
    expect(PM.store.get('ext.notes', {})).toEqual({ count: 2 });
    expect(api.storage.get('count')).toBe(2);
    expect(installed.api('other').storage.get('count')).toBeUndefined();
  });

  it('routes ui helpers to the overlay members on PM', async () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const api = installed.api('demo');

    // Notices carry the extension that raised them, so the overlay can tell an
    // extension's alert from the editor's own error.
    const source = { source: { id: 'demo', name: 'demo' } };
    api.ui.toast('hi');
    expect(PM.toast).toHaveBeenCalledWith('hi', 2200, source);
    api.ui.toast('sticky', { sticky: true });
    expect(PM.toast).toHaveBeenLastCalledWith('sticky', 8000, { sticky: true, ...source });

    api.ui.menu({ x: 10, y: 20 }, [{ label: 'A' }]);
    expect(PM.menu).toHaveBeenCalledWith(document.body, [{ label: 'A' }], { x: 10, y: 20 });

    const confirmed = api.ui.confirm('Delete?', 'Cannot undo');
    const options = PM.modal.mock.calls[0][0];
    options.actions[1].run();
    await expect(confirmed).resolves.toBe(true);
    expect(api.ui.icon('play')).toContain('data-icon="play"');
    expect(Object.keys(api.ui.controls)).toEqual([
      'NumField', 'ColorField', 'FillField', 'FontField', 'SelectField',
      'TextField', 'ToggleField', 'Row', 'Section', 'binding'
    ]);
    expect(api.ui.controls.binding).toEqual(expect.objectContaining({
      channelBinding: expect.any(Function),
      layerFieldBinding: expect.any(Function),
      contentBinding: expect.any(Function),
      compositionBinding: expect.any(Function)
    }));
    expect(api.host.state).toEqual({ doc, sel, transport, perf });
  });

  it('opens and closes panels through Layout + WS', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const api = installed.api('demo');
    api.panels.register({ id: 'notes', title: 'Notes', build: () => {} });

    api.panels.open('notes', 'right');
    expect(PM.WS.mutate).toHaveBeenCalled();
    expect(PM.Layout.addPanel).toHaveBeenCalledWith(PM.WS.current, 'notes', 'right');
    expect(PM.Layout.refresh).toHaveBeenCalledWith('notes');

    api.panels.close('notes');
    expect(PM.Layout.removePanel).toHaveBeenCalledWith(PM.WS.current, 'notes');
    expect(api.panels.list()).toEqual(expect.arrayContaining(['notes', 'viewer']));
  });

  it('bridges the legacy bus onto kernel events and unsubscribes on uninstall', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const seen: string[] = [];
    installed.events.on('project:changed', (payload) => void seen.push(`project:${payload.kind}`));
    installed.events.on('selection', (payload) => void seen.push(`sel:${payload.layers.join(',')}`));
    installed.events.on('time', (t) => void seen.push(`time:${t}`));
    installed.events.on('transport', (payload) => void seen.push(`play:${payload.playing}`));
    installed.events.on('layout', () => void seen.push('layout'));

    PM.bus.emit('layers');
    PM.bus.emit('assets');
    PM.bus.emit('sel');
    PM.bus.emit('time', 5);
    PM.playing = true;
    PM.bus.emit('transport');
    PM.bus.emit('layout');

    expect(seen).toEqual(['project:structure', 'project:assets', 'sel:L1', 'time:5', 'play:true', 'layout']);

    installed.uninstall();
    installed = null;
    PM.bus.emit('layers');
    expect(seen).toHaveLength(6);
  });

  it('bridges the complete font families payload from the legacy bus', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const seen: string[][] = [];
    installed.events.on('fonts', (families) => seen.push(families));

    const families = ['SF Pro Text', 'Avenir Next', 'Helvetica'];
    PM.bus.emit('fonts', families);

    expect(seen).toEqual([families]);
  });

  it('routes key chords through the PM.cmd seam when present', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const run = vi.fn();
    installed.api('demo').commands.register({ id: 'demo.go', label: 'Go', run });
    installed.api('demo').keybindings.bind({ key: 'cmd+shift+g', command: 'demo.go' });

    const event = new KeyboardEvent('keydown', { key: 'g', metaKey: true, shiftKey: true, bubbles: true, cancelable: true });
    window.dispatchEvent(event);

    /* PM.cmd is the app's public command seam (native menu IPC and tests
       interpose on it); the listener must go through it, not around it. */
    expect(PM.cmd).toHaveBeenCalledWith('demo.go');
    expect(event.defaultPrevented).toBe(true);
  });

  it('falls back to the kernel command table when PM.cmd is absent', () => {
    const PM = fakePM();
    delete (PM as Record<string, unknown>).cmd;
    installed = installKernel(PM);
    const run = vi.fn();
    installed.api('demo').commands.register({ id: 'demo.go', label: 'Go', run });
    installed.api('demo').keybindings.bind({ key: 'cmd+shift+g', command: 'demo.go' });

    const event = new KeyboardEvent('keydown', { key: 'g', metaKey: true, shiftKey: true, bubbles: true, cancelable: true });
    window.dispatchEvent(event);

    expect(run).toHaveBeenCalledTimes(1);
    expect(event.defaultPrevented).toBe(true);
  });

  it('opens the palette through whichever PM shape exists', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    installed.api('demo').palette.open('fx');
    expect(PM.palette).toHaveBeenCalledWith('fx');
  });

  it('tolerates an empty PM without throwing', async () => {
    const PM: LegacyPM = {};
    installed = installKernel(PM);
    const api = installed.api('demo');

    expect(() => api.ui.toast('hi')).not.toThrow();
    expect(api.project.selection()).toEqual({ layers: [], keys: [], chan: null });
    expect(api.project.revision()).toBe(0);
    expect(api.project.apply([] as never)).toEqual({ ok: false, message: 'editing engine unavailable' });
    expect(api.storage.get('anything')).toBeUndefined();
    expect(() => api.panels.open('x')).not.toThrow();
    expect(api.ui.icon('play')).toBe('');
    expect(installed.bridge).toBeNull();

    const layer = { id: 'L1', from: 0, parent: null, threeD: false, p: {}, d: {} } as any;
    const prop = { v: 0, kf: [], expr: null } as any;
    const key = { i: 'k1', t: 0, v: 0 } as any;
    expect(() => {
      api.anim.ev(layer, 'x', 0); api.anim.evP(layer, prop, 0, 'x'); api.anim.active(layer, 0);
      api.anim.findProp(layer, 'x'); api.anim.allProps(layer); api.anim.hasKeyAt(layer, prop, 0);
      api.anim.setKey(layer, 'x', 0, 1); api.anim.setKeyOn(prop, 0, 1); api.anim.removeKey(prop, key);
      api.anim.applyEaseTo([key], 'linear'); api.anim.wouldCycle(layer, null); api.anim.resolveContent(layer);
      void api.anim.expressionErrors; api.anim.version(); api.anim.touch(); api.anim.worldMatrix(layer, 0);
      api.anim.localMatrix(layer, 0); api.anim.transformParentMatrix(layer, 0); api.anim.mul([1, 0, 0, 1, 0, 0], [1, 0, 0, 1, 0, 0]);
      api.model.P(0); api.model.KF(0, 0); void api.model.CH; void api.model.BLENDS; void api.model.TYPE_META;
      void api.model.MASK_SHAPES; api.model.mkLayer('solid'); api.model.mkMask(); api.model.mkProject(); api.model.cloneLayer(layer); api.model.normalizeFill(null);
      api.model.layerDefinition('x'); api.model.curComp(); api.model.layer('x'); api.model.byName('x');
      api.selection.get(); api.selection.layers(); api.selection.first(); api.selection.keys(); api.selection.chan();
      api.selection.set({ layers: [] }); api.selection.select([]); api.selection.resolveSelectedKeys();
      void api.selection.keySelectionActive; api.selection.keySelectionActive = false;
      api.groups.ancestors(layer); api.groups.transformRoots([]); api.groups.span(layer); api.groups.expand([]);
      api.groups.normalizeStack(); api.groups.moveToGroup([], null);
      api.transport.time(); api.transport.setTime(0); api.transport.play(); api.transport.pause(); api.transport.toggle();
      api.transport.playing(); api.transport.step(1); void api.transport.quality; api.transport.quality = 1;
      void api.transport.perf; api.transport.invalidate(); void api.transport.previewResolution; api.transport.previewResolution = null;
      api.history.do('x', () => {}); api.history.begin('x'); api.history.commit(); api.history.cancel();
      api.history.undo(); api.history.redo(); api.history.external('x', () => {}, () => {}); api.history.selection({ layers: [], keys: [], chan: null }, { layers: [], keys: [], chan: null });
      api.edit.apply([]); api.edit.begin('x'); api.edit.commit(); api.edit.cancel();
      api.edit.dispatch({ type: 'delete_layers', targets: [] }); api.edit.mutate('x', () => {});
      api.media.timing.isTimed(layer); api.media.timing.rate(layer); api.media.timing.earliestStart(layer);
      api.media.commandForAsset(); api.media.audio.drawWaveform(document.createElement('canvas').getContext('2d')!, layer);
      api.media.assets.get('x'); api.media.assets.kind(new File([], 'x')); void api.media.fonts;
      api.render.gl.bounds(layer, 0); api.render.gl.pick(0, 0, 0); api.render.gl.init(document.createElement('canvas'));
      api.render.gl.resize(1, 1); api.render.gl.compileError('x'); void api.render.gl.previewViewport; void api.render.gl.context;
      api.render.raster(layer); api.render.renderFrameTo(0, 1, 1); api.render.snapshot(0);
      api.uiState.getLayerCollapsed(layer); api.uiState.setLayerCollapsed(layer, false); api.uiState.getKeyHandles(key);
      api.uiState.setKeyHandles(key, {}); api.uiState.getFxOpen({} as any); api.uiState.setFxOpen({} as any, false);
      api.uiState.getReveal(layer); api.uiState.setReveal(layer, []); api.uiState.getShaderMeta(layer); api.uiState.setShaderMeta(layer, {});
      api.ui.drag(new PointerEvent('pointerdown'), { move: () => {} }); api.ui.closeMenus();
      api.ui.showLayerMenu(layer, { clientX: 0, clientY: 0 }); api.ui.showParentMenu([], { clientX: 0, clientY: 0 });
      api.ui.beginParentPick(new PointerEvent('pointerdown'), []); api.ui.openShaderEditor();
      const gesture = new api.ui.gesture({ mode: 'local', label: 'x', set: () => {} }); gesture.begin(); gesture.write(1); gesture.commit(); gesture.cancel(); gesture.once(1);
      void api.dnd.ASSET_MIME; void api.dnd.FX_MIME; api.dnd.startAssetDrag(null, { id: 'x', name: 'x', kind: 'image' });
      void api.dnd.mediaDrag; api.dnd.mediaDrag = null; api.dnd.hasAssetDrag(null); api.dnd.hasFileDrag(null);
      api.dnd.hasMediaDrag(null); api.dnd.readAssetDrag(null); api.dnd.hasFxDrag(null); api.dnd.readFxDrag(null);
      api.dnd.applyFxDrop({ kind: 'effect', id: 'x', label: 'x' }, null);
      api.workspace.current(); api.workspace.mutate(() => {}); api.workspace.hasPanel('x'); api.workspace.addPanel('x');
      api.workspace.movePanel('x', 'center', 0); api.workspace.removePanel('x'); api.workspace.hidePanel('x');
      api.workspace.restorePanel('x'); api.workspace.refresh('x');
      api.util.round(0); api.util.clamp(0, 0, 1); api.util.lerp(0, 1, .5); api.util.snapF(0, 30);
      api.util.tc(0); api.util.parseTc('0'); api.util.uid(); api.util.hex2rgb('#000'); api.util.rgb2hex(0, 0, 0);
      api.ease.nameOf([0, 0], [1, 1]); void api.ease.PRESETS;
      void api.space3d.CHANNELS_3D; api.space3d.local3D(layer, 0); api.space3d.parent3D(layer, 0);
      api.space3d.world3D(layer, 0); api.space3d.is3DLayer(layer); api.space3d.perspectiveAmount(layer, 0);
      api.space3d.planeMatrix(layer, 0); api.space3d.projectPoint([1, 0, 0, 0, 1, 0, 0, 0, 1], { x: 0, y: 0 });
      api.space3d.inversePlane([1, 0, 0, 0, 1, 0, 0, 0, 1]); api.space3d.planeContains(layer, 0, 0, 0, { x0: 0, x1: 1, y0: 0, y1: 1 });
      const service = api.services.register('x', {}); api.services.get('x'); service.dispose();
    }).not.toThrow();
    await expect(api.media.importFiles([])).resolves.toBeUndefined();
    await expect(api.media.assets.add(new File([], 'x'))).resolves.toBeUndefined();
  });

  it('bootExtensions creates the loader and activates the given built-ins', async () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const log: string[] = [];
    const loader = await bootExtensions(installed, {
      'effects-basic': async () => ({
        default: (api) => {
          log.push('effects-basic');
          api.effects.register({ id: 'wave', label: 'Wave', group: 'Distort', params: [], frag: 'o = src(v_st);' });
        }
      })
    });

    expect(log).toEqual(['effects-basic']);
    expect(installed.loader).toBe(loader);
    expect(installed.effects.get('wave')?.label).toBe('Wave');
    expect(loader.activeIds()).toEqual(['effects-basic']);
    expect(installed.deps.extensions.list().map((record) => record.id)).toEqual(['effects-basic']);

    await loader.dispose();
    expect(installed.effects.list()).toEqual([]);
  });

  it('forwards extension forks through the native bridge', async () => {
    const PM = fakePM();
    const fork = vi.fn(async ({ id }: { id: string }) => ({ id: `${id}-fork` }));
    PM.extensionsBridge = {
      list: vi.fn(async () => []),
      setEnabled: vi.fn(async () => []),
      remove: vi.fn(async () => []),
      reload: vi.fn(async () => []),
      create: vi.fn(async () => []),
      fork,
      reveal: vi.fn(async () => undefined),
      readSource: vi.fn(async () => []),
      reportHealth: vi.fn(),
      onChanged: vi.fn(() => () => {})
    };
    installed = installKernel(PM);

    await expect(installed.api('mods').extensions.fork('timeline')).resolves.toEqual({ id: 'timeline-fork' });
    expect(fork).toHaveBeenCalledExactlyOnceWith({ id: 'timeline' });
  });

  it('emits layout after built-ins without waiting for a hanging user extension', async () => {
    vi.useFakeTimers();
    const PM = fakePM();
    const emit = vi.spyOn(PM.bus, 'emit');
    PM.Layout.ws = PM.WS.current;
    PM.Layout.apply = vi.fn(() => PM.bus.emit('layout'));
    const slowModule = encodeURIComponent('export default function(){ return new Promise(() => {}) }');
    PM.extensionsBridge = {
      list: vi.fn(async () => [
        {
          id: 'toolbar',
          scope: 'builtin',
          manifest: { id: 'toolbar', name: 'Toolbar', version: '1.0.0', apiVersion: 1 },
          dir: 'builtin:toolbar',
          enabled: true,
          bundleUrl: null,
          bundleHash: null,
          health: { state: 'ok' },
          updatedAt: 0
        },
        {
          id: 'slow-user',
          scope: 'user',
          manifest: { id: 'slow-user', name: 'Slow user', version: '1.0.0', apiVersion: 1 },
          dir: '/ext/slow-user',
          enabled: true,
          bundleUrl: `data:text/javascript,${slowModule}`,
          bundleHash: 'slow',
          health: { state: 'ok' },
          updatedAt: 0
        }
      ]),
      setEnabled: vi.fn(),
      remove: vi.fn(),
      reload: vi.fn(),
      create: vi.fn(),
      reveal: vi.fn(),
      readSource: vi.fn(),
      reportHealth: vi.fn(),
      onChanged: vi.fn(() => () => {})
    };
    installed = installKernel(PM);

    let settled = false;
    const booting = bootExtensions(installed, {
      toolbar: async () => ({ default: (api) => void api.panels.register({ id: 'toolbar', title: 'Toolbar', build: () => {} }) })
    }).then((loader) => {
      settled = true;
      return loader;
    });
    await vi.advanceTimersByTimeAsync(0);

    expect(emit).toHaveBeenCalledWith('layout');
    expect(PM.Layout.apply).toHaveBeenCalledWith(PM.WS.current);
    expect(settled).toBe(false);

    await vi.advanceTimersByTimeAsync(10_000);
    await booting;
    expect(emit.mock.calls.filter((call) => call[0] === 'layout')).toHaveLength(2);
    expect(PM.Layout.apply).toHaveBeenCalledTimes(2);
  });
});


it('reads durable text assets larger than 64 MB', async () => {
  const PM = fakePM();
  PM.proj.assets.large = { id: 'large', name: 'large.obj' };
  const text = 'x'.repeat(64 * 1024 * 1024) + 'end';
  PM.MediaStore = { get: vi.fn(async () => new Blob([text])) };
  installed = installKernel(PM);
  expect(await installed.api('models').assets.readText('large')).toBe(text);
});
