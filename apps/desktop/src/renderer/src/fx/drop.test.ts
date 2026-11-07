import { describe, expect, it, vi } from 'vitest';

import { applyFxDrop, readFxDrag } from './drop';

const transfer = (value: unknown) => ({ getData: () => JSON.stringify(value) }) as unknown as DataTransfer;

describe('fx drop payloads', () => {
  it('reads My Presets drags alongside effects and transitions', () => {
    expect(readFxDrag(transfer({ kind: 'preset', id: 'p1', label: 'Soft entrance' }))).toEqual({ kind: 'preset', id: 'p1', label: 'Soft entrance' });
    expect(readFxDrag(transfer({ kind: 'macro', id: 'x' }))).toBeNull();
  });

  it('applies a dropped preset to the layer under the pointer at the playhead, even one that takes no effects', () => {
    const layer = { id: 'audio-1', name: 'Music', type: 'audio' };
    const PM = {
      time: 4,
      L: (id: string) => (id === layer.id ? layer : null),
      TYPE_META: { audio: { label: 'Audio', effects: false } },
      Presets: { get: () => ({ id: 'p1', name: 'Fade', effects: [], properties: [] }) },
      Edit: { apply: vi.fn(() => ({ ok: true, data: { results: [{ data: { skipped: [] } }] } })) },
      toast: vi.fn(),
      invalidate: vi.fn(),
    };
    expect(applyFxDrop({ kind: 'preset', id: 'p1', label: 'Fade' }, 'audio-1', undefined, PM)).toBe(true);
    expect(PM.Edit.apply).toHaveBeenCalledWith({ type: 'apply_preset', target: 'audio-1', preset: 'p1', time: 4 }, { label: 'Apply “Fade”', origin: 'fx-browser' });
  });
});
