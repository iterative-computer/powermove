import { describe, expect, it } from 'vitest';

import { GeometryAtRevision } from './revision-geometry';

describe('GeometryAtRevision', () => {
  it('draws the geometry of the newest revision the presented frame includes', () => {
    const history = new GeometryAtRevision<string>();
    history.record(10, 'a@0', 'box at 10');
    history.record(12, 'a@0', 'box at 12');
    history.record(15, 'a@0', 'box at 15');
    // The frame on screen is at 13: the drag's later edits are in flight.
    expect(history.at(13, 'a@0', 'box at 15')).toEqual({ value: 'box at 12', revision: 12 });
    expect(history.at(12, 'a@0', 'box at 15')).toEqual({ value: 'box at 12', revision: 12 });
    // Up to date, or no frame: the current geometry.
    expect(history.at(15, 'a@0', 'box at 15').value).toBe('box at 15');
    expect(history.at(null, 'a@0', 'box at 15').value).toBe('box at 15');
    // Older than anything recorded: the current geometry (never invented).
    expect(history.at(9, 'a@0', 'box at 15').value).toBe('box at 15');
  });

  it('never shows another selection or time', () => {
    const history = new GeometryAtRevision<string>();
    history.record(10, 'a@0', 'a');
    history.record(11, 'b@0', 'b');
    expect(history.at(10, 'b@0', 'b now').value).toBe('b now');
    history.record(11, 'b@0', 'b again');
    expect(history.at(11, 'b@0', 'b now').value).toBe('b now');
  });

  it('restarts when revisions go back (a new mirror) and keeps a bounded history', () => {
    const history = new GeometryAtRevision<number>(3);
    for (let r = 1; r <= 5; r++) history.record(r, 'k', r);
    expect(history.at(2, 'k', 99).value).toBe(99);
    expect(history.at(3, 'k', 99).value).toBe(3);
    history.record(1, 'k', 1);
    expect(history.at(0, 'k', 7).value).toBe(7);
    expect(history.at(1, 'k', 7).value).toBe(7);
  });
});
