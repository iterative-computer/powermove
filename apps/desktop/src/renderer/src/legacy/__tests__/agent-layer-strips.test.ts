import { describe, expect, it } from 'vitest';
import { makePM } from './make-pm';
import { editLayerStrips, timelineState } from '../assistant/layer-strips';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'assistant/harness');
  PM.proj = PM.mkProject({ fps: 30, dur: 30 }); PM.time = 0;
  PM.Kernel.services.register('shaderHooks', { syncShaderUniforms() {} }); PM.mkEffect = () => null;
  const a = PM.mkLayer('text', { name: 'Title', from: 2, dur: 3 });
  const b = PM.mkLayer('audio', { name: 'Sound', from: 3, dur: 2 });
  const c = PM.mkLayer('shape', { name: 'Card', from: 10, dur: 4 });
  PM.proj.layers = [a, b, c]; PM.ProjectIndex.invalidate();
  const run = (args: any) => editLayerStrips(PM, { layerIds: [a.id, b.id], ...args }, { label: 'Arrange strips', origin: 'agent', historyGroup: 'strips' });
  return { PM, a, b, c, run };
}

describe('agent layer strips', () => {
  it('moves multi-selection on frame boundaries without changing sync or local keyframes, with Undo/Redo', () => {
    const { PM, a, b, run } = editor();
    PM.setKeyOn(a.p['position.x'], 1, 100, 'linear', 30);
    const before = JSON.stringify(PM.proj.layers);
    expect(run({ operation: 'move', at: 5.012 }).ok).toBe(true);
    expect(a.from).toBe(5); expect(b.from).toBe(6); expect(a.p['position.x'].kf[0].t).toBe(1);
    PM.hist.undo(); expect(JSON.stringify(PM.proj.layers)).toBe(before);
    PM.hist.redo(); expect(PM.L(a.id).from).toBe(5);
  });

  it('sequences in requested order, supports overlaps, and rolls back a batch that would overflow', () => {
    const { PM, a, b, run } = editor();
    expect(run({ operation: 'sequence', layerIds: [b.id, a.id], at: 0, gap: -.5 }).ok).toBe(true);
    expect(b.from).toBe(0); expect(a.from).toBe(1.5);
    const before = JSON.stringify(PM.proj);
    expect(run({ operation: 'sequence', at: 28 }).ok).toBe(false);
    expect(JSON.stringify(PM.proj)).toBe(before);
    expect(run({ operation: 'align', at: 8, edge: 'end' }).ok).toBe(true);
    expect(PM.L(a.id).from + PM.L(a.id).dur).toBe(8);
    expect(PM.L(b.id).from + PM.L(b.id).dur).toBe(8);
  });

  it('duplicates layers with independent animation and remaps internal parents and mattes', () => {
    const { PM, a, c, run } = editor();
    c.parent = a.id; c.matteSource = a.id;
    PM.setKeyOn(a.p['position.x'], 1, 100, 'linear', 30);
    const result = run({ operation: 'duplicate', layerIds: [a.id, c.id], delta: 4 });
    expect(result.ok).toBe(true);
    const [copyA, copyC] = result.data.result.layerIds.map((id: string) => PM.L(id));
    expect(copyA.from).toBe(6); expect(copyC.from).toBe(14);
    expect(copyC.parent).toBe(copyA.id); expect(copyC.matteSource).toBe(copyA.id);
    expect(copyA.p['position.x'].kf[0].i).not.toBe(a.p['position.x'].kf[0].i);
    copyA.p['position.x'].kf[0].v = 22; expect(a.p['position.x'].kf[0].v).toBe(100);
  });

  it('reorders without retiming and closes gaps only in the explicitly supplied scope', () => {
    const { PM, a, b, c, run } = editor();
    expect(run({ operation: 'reorder', layerIds: [c.id], anchorId: a.id, placement: 'before' }).ok).toBe(true);
    expect(PM.proj.layers.map((l: any) => l.id)).toEqual([c.id, a.id, b.id]); expect(c.from).toBe(10);
    expect(run({ operation: 'close_gap', layerIds: [a.id, c.id], start: 5, end: 10 }).ok).toBe(true);
    expect(c.from).toBe(5); expect(b.from).toBe(3);
    const before = JSON.stringify(PM.proj);
    expect(run({ operation: 'close_gap', start: 2, end: 4 }).ok).toBe(false);
    expect(JSON.stringify(PM.proj)).toBe(before);
  });

  it('manages strip states, rejects locks atomically, and requires explicit unlocking', () => {
    const { PM, a, b, run } = editor();
    expect(run({ operation: 'state', patch: { solo: true, shy: true, color: '#ff0000' } }).ok).toBe(true);
    expect(a.solo).toBe(true); expect(b.shy).toBe(true);
    b.lock = true;
    const before = JSON.stringify(PM.proj);
    expect(() => run({ operation: 'move', delta: 1 })).toThrow('locked');
    expect(JSON.stringify(PM.proj)).toBe(before);
    expect(() => run({ operation: 'state', patch: { locked: false } })).toThrow('locked');
    expect(b.lock).toBe(true);
    expect(() => run({ operation: 'state', patch: { from: 99 } })).toThrow('Unsupported');
  });

  it('expands group blocks and refuses moving locked children', () => {
    const { PM, a, c, run } = editor();
    const group = PM.mkLayer('group', { from: 0, dur: 15 });
    a.group = group.id; c.group = group.id; PM.proj.layers.unshift(group); PM.ProjectIndex.invalidate();
    a.lock = true;
    expect(() => run({ operation: 'move', layerIds: [group.id], delta: 2 })).toThrow('locked');
    a.lock = false;
    const result = run({ operation: 'duplicate', layerIds: [group.id], delta: 1 });
    expect(result.ok).toBe(true);
    const copies = result.data.result.layerIds.map((id: string) => PM.L(id));
    expect(copies).toHaveLength(3);
    expect(copies[1].group).toBe(copies[0].id);
  });

  it('pages compact timing and state without losing stack order', () => {
    const { PM, b } = editor();
    const state = timelineState(PM, { offset: 1, limit: 1 });
    expect(state.total).toBe(3); expect(state.nextOffset).toBe(2);
    expect(state.strips[0]).toMatchObject({ id: b.id, index: 1, startFrame: 90, endFrame: 150 });
  });

  it('routes strip edits through the live revision guard and groups them into one reversible run', async () => {
    const { PM, a, b } = editor();
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({ tool, arguments: args, runId: 'strip-run', callId: tool, baseRevision: 0 });
    await call('get_timeline');
    await call('edit_layer_strips', { operation: 'move', layerIds: [a.id, b.id], delta: 2 });
    await call('edit_layer_strips', { operation: 'state', layerIds: [a.id], patch: { name: 'Renamed' } });
    const result = await call('__finish_run', { commit: true });
    expect(result.changed).toBe(true); expect(PM.hist.list()).toHaveLength(1);
    PM.hist.undo(); expect(PM.L(a.id).from).toBe(2); expect(PM.L(a.id).name).toBe('Title');
  });
});

it('keeps duplicated children inside their separate group blocks', () => {
  const { PM, a, c, run } = editor();
  const first = PM.mkLayer('group'), second = PM.mkLayer('group');
  a.group = first.id; c.group = second.id;
  PM.proj.layers = [first, a, second, c]; PM.ProjectIndex.invalidate();
  const result = run({ operation: 'duplicate', layerIds: [a.id, c.id], delta: 0 });
  expect(result.ok).toBe(true);
  const [aCopy, cCopy] = result.data.result.layerIds;
  expect(PM.proj.layers.map((layer: any) => layer.id)).toEqual([first.id, aCopy, a.id, second.id, cCopy, c.id]);
  expect(PM.L(aCopy).group).toBe(first.id); expect(PM.L(cCopy).group).toBe(second.id);
});

it('refreshes a conflicted strip edit only after reading the current timeline', async () => {
  const { PM, a } = editor();
  const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({ tool, arguments: args, runId: 'conflict', callId: tool, baseRevision: 0 });
  await call('get_timeline');
  await call('edit_layer_strips', { operation: 'move', layerIds: [a.id], at: 4 });
  PM.Edit.apply({ type: 'set_layer', target: a.id, patch: { name: 'Human title' } }, { origin: 'interface', label: 'Rename' });
  await expect(call('edit_layer_strips', { operation: 'move', layerIds: [a.id], at: 6 })).rejects.toThrow('project changed');
  await call('get_timeline');
  await call('edit_layer_strips', { operation: 'move', layerIds: [a.id], at: 6 });
  expect(PM.L(a.id).name).toBe('Human title'); expect(PM.L(a.id).from).toBe(6);
  await expect(call('rollback_changes')).rejects.toThrow('Other edits');
});
