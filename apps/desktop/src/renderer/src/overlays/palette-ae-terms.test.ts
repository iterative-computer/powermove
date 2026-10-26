import { describe, expect, it, vi } from 'vitest';
import { paletteEntries } from './palette-model';

describe('command palette After Effects names', () => {
  it('finds a command by its After Effects name and says so', () => {
    const PM: any = {
      commands: { toggleContinuousRaster: { id: 'toggleContinuousRaster', label: 'Continuously Rasterize', cat: 'Layer' } },
      cmd: vi.fn(), proj: { layers: [] }, WS: { list: () => [] },
    };
    const entries = paletteEntries(PM, 'collapse transformations');
    expect(entries.map((entry) => entry.label)).toEqual(['Continuously Rasterize — Collapse Transformations in After Effects']);
    entries[0]!.run();
    expect(PM.cmd).toHaveBeenCalledWith('toggleContinuousRaster');
    // Its own name still matches plainly.
    expect(paletteEntries(PM, 'rasterize').map((entry) => entry.label)).toEqual(['Continuously Rasterize']);
  });
});
