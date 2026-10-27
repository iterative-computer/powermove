import { describe, expect, it } from 'vitest';
import { ExportDestinations } from './export-destinations';

describe('export destinations', () => {
  const chosen = { file: '/x/Film.mp4', format: 'h264', offline: 'without' as const };

  it('gives a chosen destination once, to the window that chose it, for its format', () => {
    const destinations = new ExportDestinations();
    const token = destinations.add(7, chosen);
    expect(destinations.take(8, token, 'h264')).toBeNull();
    expect(destinations.take(7, token, 'h264')).toEqual(chosen);
    expect(destinations.take(7, token, 'h264')).toBeNull();
    const other = destinations.add(7, chosen);
    expect(destinations.take(7, other, 'prores-hq')).toBeNull();
    expect(destinations.take(7, other, 'h264')).toBeNull();
    expect(destinations.take(7, 42, 'h264')).toBeNull();
  });

  it('forgets a window and expires unused destinations', () => {
    let now = 0;
    const destinations = new ExportDestinations(() => now);
    const gone = destinations.add(7, chosen);
    const kept = destinations.add(8, chosen);
    destinations.forget(7);
    expect(destinations.take(7, gone, 'h264')).toBeNull();
    now = 11 * 60_000;
    expect(destinations.take(8, kept, 'h264')).toBeNull();
  });
});
