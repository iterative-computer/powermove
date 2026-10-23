// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { addTimelineMarker, drawTimelineMarkers, markerAtTime, removeTimelineMarker, timelineMarkerHit } from './markers';
import { fakePowermoveAPI } from './fake-api.test-helper';
import { makePM } from '../../renderer/src/legacy/__tests__/make-pm';
import { packProjectFile, unpackProjectFile } from '../../renderer/src/legacy/core/project-file';
import { sanitizeProject } from '../../renderer/src/core/validate/project';

describe('native timeline markers', () => {
  it('adds frame-aligned markers without duplicate pins and clamps the composition bounds', () => {
    const { api, state } = fakePowermoveAPI(vi);
    addTimelineMarker(api, 1.017);
    expect(api.edit.apply).toHaveBeenLastCalledWith({ type: 'add_marker', time: 31 / 30 }, { label: 'Add marker', origin: 'timeline' });
    state.project.markers = [{ id: 'one', t: 31 / 30, name: 'Beat' }];
    vi.mocked(api.edit.apply).mockClear();
    addTimelineMarker(api, 1.017);
    addTimelineMarker(api, NaN);
    expect(api.edit.apply).not.toHaveBeenCalled();
    expect(markerAtTime(api, 31 / 30)).toBe(state.project.markers[0]);
    addTimelineMarker(api, -2);
    expect(api.edit.apply).toHaveBeenLastCalledWith({ type: 'add_marker', time: 0 }, expect.anything());
    addTimelineMarker(api, 20);
    expect(api.edit.apply).toHaveBeenLastCalledWith({ type: 'add_marker', time: 10 }, expect.anything());
  });

  it('resolves a marker when its menu runs, including legacy markers without IDs', () => {
    const { api, state } = fakePowermoveAPI(vi);
    const marker = { t: 2, name: 'Legacy' };
    state.project.markers = [{ id: 'new', t: 1, name: 'New' }, marker];
    removeTimelineMarker(api, marker);
    expect(api.edit.apply).toHaveBeenLastCalledWith({ type: 'remove_marker', index: 1 }, { label: 'Remove marker', origin: 'timeline' });
    state.project.markers.shift();
    removeTimelineMarker(api, marker);
    expect(api.edit.apply).toHaveBeenLastCalledWith({ type: 'remove_marker', index: 0 }, expect.anything());
    state.project.markers = [];
    vi.mocked(api.edit.apply).mockClear();
    removeTimelineMarker(api, marker);
    expect(api.edit.apply).not.toHaveBeenCalled();
  });

  it('hits only visible ruler pins, leaving work-area handles and track interactions alone', () => {
    const markers = [{ t: -1, name: 'Offscreen' }, { t: 2, name: 'Beat' }, { t: 9, name: 'Later' }];
    const hit = (x: number, y: number) => timelineMarkerHit(markers, x, y, 200, 600, 28, t => 200 + t * 50);
    expect(hit(300, 22)).toBe(markers[1]);
    expect(hit(306, 22)).toBe(markers[1]);
    expect(hit(310, 22)).toBeUndefined();
    expect(hit(300, 5)).toBeUndefined();
    expect(hit(300, 45)).toBeUndefined();
    expect(hit(150, 22)).toBeUndefined();
    expect(hit(650, 22)).toBeUndefined();
  });

  it('draws only visible pins and neutral guides, restoring the canvas state', () => {
    const c = { save: vi.fn(), restore: vi.fn(), beginPath: vi.fn(), rect: vi.fn(), clip: vi.fn(),
      moveTo: vi.fn(), lineTo: vi.fn(), stroke: vi.fn(), closePath: vi.fn(), fill: vi.fn() };
    drawTimelineMarkers(c as any, [{ t: -1, name: '' }, { t: 2, name: '' }, { t: 9, name: '' }], 200, 600, 400, 28, t => 200 + t * 50, 'gray');
    expect(c.fill).toHaveBeenCalledTimes(1);
    expect(c.moveTo).toHaveBeenCalledWith(300.5, 28);
    expect(c.lineTo).toHaveBeenCalledWith(300.5, 400);
    expect(c.restore).toHaveBeenCalledTimes(1);
  });

  it('adds and removes through the real edit boundary, persists in .pmv, and restores Undo/Redo', async () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ dur: 10 });
    PM.proj.markers = [{ t: 1, name: 'Legacy' }];
    expect(PM.Edit.apply({ type: 'add_marker', id: 'beat', time: 2.5, name: 'Beat' }).ok).toBe(true);
    expect(PM.Edit.apply({ type: 'remove_marker', index: 0 }).ok).toBe(true);
    expect(PM.proj.markers).toEqual([{ id: 'beat', t: 2.5, name: 'Beat' }]);
    const store = { get: async () => null, put: async () => true };
    const saved = unpackProjectFile(await packProjectFile({ proj: PM.proj, history: PM.hist.export() }, store));
    PM.proj = sanitizeProject(saved.proj);
    PM.hist.clear();
    PM.hist.import(saved.history);
    expect(PM.proj.markers).toEqual([{ id: 'beat', t: 2.5, name: 'Beat' }]);
    PM.hist.undo();
    expect(PM.proj.markers).toHaveLength(2);
    PM.hist.undo();
    expect(PM.proj.markers).toEqual([{ t: 1, name: 'Legacy' }]);
    PM.hist.redo(); PM.hist.redo();
    expect(PM.proj.markers).toEqual([{ id: 'beat', t: 2.5, name: 'Beat' }]);
    const before = JSON.stringify(PM.proj);
    expect(PM.Edit.apply({ type: 'remove_marker', index: 3 }).ok).toBe(false);
    expect(JSON.stringify(PM.proj)).toBe(before);
  });
});
