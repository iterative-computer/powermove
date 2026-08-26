import { afterEach, describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './shortcuts';

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
