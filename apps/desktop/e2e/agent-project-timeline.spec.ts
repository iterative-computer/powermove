import { expect, test } from './helpers/app';

test('recovered strip gestures share Undo and library operations retain independent projects', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const a = PM.mkLayer('text', { name: 'Title', from: 2, dur: 3 });
    const b = PM.mkLayer('shape', { name: 'Card', from: 3, dur: 2 });
    PM.proj.layers = [a, b]; PM.ProjectIndex.invalidate(); PM.hist.clear();
    const before = JSON.stringify(PM.proj.layers);
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({ runId: 'recovered-tools', callId: tool, tool, arguments: args });
    const state = JSON.parse((await call('get_timeline')).content[0].text);
    await call('edit_layer_strips', { operation: 'move', layerIds: [a.id, b.id], at: 5 });
    const moved = PM.proj.layers.map((layer: any) => layer.from);
    await call('__finish_run', { commit: true });
    PM.hist.undo();
    const undo = JSON.stringify(PM.proj.layers) === before;
    const originalId = PM.proj.id;
    await call('manage_project', { operation: 'rename', name: 'Original film' });
    const copy = JSON.parse((await call('manage_project', { operation: 'duplicate', name: 'Film copy' })).content[0].text);
    await call('manage_project', { operation: 'create', name: 'New film', w: 640, h: 360, fps: 24, dur: 12 });
    const created = { id: PM.proj.id, name: PM.proj.name, width: PM.proj.w };
    const library = JSON.parse((await call('manage_project', { operation: 'list' })).content[0].text);
    await call('__finish_run', { commit: true });
    return { strips: state.strips.length, moved, undo, originalId, copyId: copy.projectId, created, library };
  });
  expect(result).toMatchObject({ strips: 2, moved: [5, 6], undo: true, created: { name: 'New film', width: 640 } });
  expect(result.copyId).not.toBe(result.originalId);
  expect(result.library.projects).toEqual(expect.arrayContaining([
    expect.objectContaining({ id: result.originalId, name: 'Original film' }),
    expect.objectContaining({ id: result.copyId, name: 'Film copy' }),
    expect.objectContaining({ id: result.created.id, name: 'New film' }),
  ]));
  expect(session.diagnostics.pageErrors).toEqual([]);
});
