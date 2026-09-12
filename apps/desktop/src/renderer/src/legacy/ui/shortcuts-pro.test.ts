import { afterEach, describe, expect, it, vi } from 'vitest';

import { makePM } from '../__tests__/make-pm';
import { LEGACY_OWNER, orderLayers, reloadShortcuts, setViewerZoom } from './shortcuts';

const previousWindow = (globalThis as any).window;

afterEach(() => {
  (globalThis as any).window = previousWindow;
  vi.unstubAllGlobals();
});

function runtime() {
  (globalThis as any).window = { addEventListener() {}, removeEventListener() {} };
  const PM: any = makePM(
    'core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history',
    'core/editing', 'core/capabilities', 'ui/shortcuts',
  );
  PM.proj = PM.mkProject({ w: 1920, h: 1080, fps: 30, dur: 10 });
  return PM;
}

function add(PM: any, id: string, lock = false) {
  const layer = PM.mkLayer('solid', { name: id });
  layer.id = id;
  layer.lock = lock;
  PM.proj.layers.push(layer);
  return layer;
}

describe('pro shortcut helper contracts', () => {
  it('rolls back non-structural changes attempted through the structural transaction API', () => {
    const PM = runtime();
    const originalName = PM.proj.name;

    const result = PM.Edit.mutate('Invalid structural edit', () => {
      PM.proj.name = 'Changed outside the layer tree';
    }, { origin: 'command' });

    expect(result).toMatchObject({ ok: false });
    expect(PM.proj.name).toBe(originalName);
    expect(PM.proj.revision).toBe(0);
    expect(PM.hist.list()).toEqual([]);
  });

  it('routes Edit-menu clipboard commands to layers outside text fields', () => {
    const PM = runtime();
    const a = add(PM, 'a');
    add(PM, 'b');
    PM.selectLayers([a.id]);

    PM.cmd('contextCopy');
    PM.cmd('contextCut');
    expect(PM.proj.layers.map((layer: any) => layer.id)).toEqual(['b']);

    PM.cmd('contextPaste');
    expect(PM.proj.layers).toHaveLength(2);
    expect(PM.firstSel()?.name).toBe('a 1');
    expect(PM.hist.list()).toEqual(['Selection', 'Cut layers', 'Paste layers']);
    PM.cmd('contextUndo');
    expect(PM.proj.layers).toHaveLength(1);
    PM.cmd('contextRedo');
    expect(PM.proj.layers).toHaveLength(2);
  });

  it('routes Edit-menu commands to focused inputs, textareas, and editable surfaces', () => {
    const PM = runtime();
    const execCommand = vi.fn((_action: string) => true);
    const documentStub = { activeElement: null as any, execCommand };
    vi.stubGlobal('document', documentStub);

    for (const activeElement of [
      { tagName: 'INPUT' },
      { tagName: 'TEXTAREA' },
      { tagName: 'DIV', isContentEditable: true },
    ]) {
      documentStub.activeElement = activeElement;
      expect(PM.cmd('contextUndo')).toBe(true);
      expect(PM.cmd('contextRedo')).toBe(true);
      expect(PM.cmd('contextCut')).toBe(true);
      expect(PM.cmd('contextCopy')).toBe(true);
      expect(PM.cmd('contextPaste')).toBe(true);
      expect(PM.cmd('contextSelectAll')).toBe(true);
    }
    expect(execCommand.mock.calls.map(([action]) => action)).toEqual(Array(3).fill([
      'undo', 'redo', 'cut', 'copy', 'paste', 'selectAll',
    ]).flat());
    expect(PM.hist.list()).toEqual([]);
  });

  it('routes Edit-menu Copy to a selected document transcript', () => {
    const PM = runtime();
    const execCommand = vi.fn((_action: string) => true);
    (globalThis as any).window = {
      getSelection: () => ({ rangeCount: 1, isCollapsed: false, toString: () => 'Agent reply' }),
    };
    vi.stubGlobal('document', {
      activeElement: { tagName: 'BODY' },
      execCommand,
    });

    expect(PM.cmd('contextCopy')).toBe(true);
    expect(execCommand).toHaveBeenCalledExactlyOnceWith('copy');
  });

  it('replaces command registrations in place during hot reload', () => {
    const PM = runtime();
    const before = PM.Kernel.commands.ownerEntries(LEGACY_OWNER);
    expect(before.length).toBeGreaterThan(20);
    const extensionCommand = {
      id: 'split', label: 'Extension split', run: vi.fn(),
    };
    const extension = PM.Kernel.commands.register('test-extension', extensionCommand);
    const panel = { id: 'legacy-panel', title: 'Legacy panel', build: vi.fn() };
    PM.Kernel.panels.register(LEGACY_OWNER, panel);

    reloadShortcuts(PM);

    const after = PM.Kernel.commands.ownerEntries(LEGACY_OWNER);
    expect(after).toHaveLength(before.length);
    expect(PM.Kernel.panels.get('legacy-panel')).toBe(panel);
    expect(PM.Kernel.commands.get('split')).toBe(extensionCommand);
    extension.dispose();
    expect(PM.commands.split.kb).toBe('⌘⇧D');
    const warning = vi.spyOn(console, 'warn').mockImplementation(() => {});
    expect(PM.cmd('missing-after-reload')).toBe(false);
    warning.mockRestore();
  });

  it('keeps front/back ordering atomic and names the history operation', () => {
    const PM = runtime();
    add(PM, 'a');
    const b = add(PM, 'b');
    add(PM, 'locked', true);
    const c = add(PM, 'c');
    PM.selectLayers([b.id, c.id]);

    orderLayers(PM, 'front');
    expect(PM.proj.layers.map((layer: any) => layer.id)).toEqual(['b', 'a', 'locked', 'c']);
    expect(PM.hist.list()).toEqual(['Selection', 'Bring to front']);
    expect(PM.proj.revision).toBe(1);
    expect(PM.proj.edits.at(-1)).toMatchObject({
      origin: 'command', label: 'Bring to front', operations: [],
      structural: {
        format: 'powermove-layer-tree-v1',
        layers: PM.proj.layers,
        comps: PM.proj.comps,
      },
    });
    expect(PM.hist.undo()).toBe(true);
    expect(PM.proj.layers.map((layer: any) => layer.id)).toEqual(['a', 'b', 'locked', 'c']);
  });

  it('clamps viewer zoom without framing the timeline or accepting invalid input', () => {
    const PM = runtime();
    const layout = vi.fn();
    const viewer = { fit: true, zoom: 1, shown: 1, pan: [12, 8], layout };
    const timeline = { frameView: vi.fn() };
    PM.Kernel.services.register('viewer', viewer);
    PM.Kernel.services.register('timeline', timeline);

    expect(setViewerZoom(PM, 100)).toBe(8);
    expect(viewer).toMatchObject({ fit: false, zoom: 8, pan: [0, 0] });
    expect(setViewerZoom(PM, -100)).toBe(.05);
    expect(setViewerZoom(PM, Number.NaN)).toBe(false);
    expect(layout).toHaveBeenCalledTimes(2);
    expect(timeline.frameView).not.toHaveBeenCalled();
  });
});
