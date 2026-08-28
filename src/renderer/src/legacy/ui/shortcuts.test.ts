import { afterEach, describe, expect, it } from 'vitest';

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
});
