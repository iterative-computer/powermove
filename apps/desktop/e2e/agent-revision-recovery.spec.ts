import { expect, test } from './helpers/app';

test('a stale agent run refreshes, applies a complete batch, and preserves interleaved user Undo', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const baseRevision = PM.proj.revision || 0;
    const call = (tool: string, args = {}) => PM.AgentHarness.test.handleLiveAgentTool({
      runId: 'revision-recovery-e2e', callId: tool, tool, arguments: args, baseRevision,
    });
    PM.Edit.apply({ type: 'add_layer', id: 'user-layer', layerType: 'text', name: 'User layer' });
    const commands = Array.from({ length: 90 }, (_, i) => ({
      type: 'add_layer', id: `agent-layer-${i}`, layerType: 'text', content: { text: `Title ${i}` },
    }));
    let conflict = '';
    try { await call('apply_commands', { commands }); }
    catch (error) { conflict = String(error); }
    await call('get_project_state');
    await call('apply_commands', { commands });
    const count = PM.proj.layers.filter((layer: any) => layer.id.startsWith('agent-layer-')).length;
    PM.Edit.apply({ type: 'set_layer', target: 'user-layer', patch: { name: 'User revision' } });
    await call('get_project_state');
    const keyframes = Array.from({ length: 150 }, (_, i) => ({ time: i / 30, value: i % 100 }));
    await call('apply_commands', { commands: [{ type: 'replace_keyframes', target: 'agent-layer-0', path: 'opacity', keyframes }] });
    const keyframeCount = PM.L('agent-layer-0').p.opacity.kf.length;
    const finish = await call('__finish_run', { commit: true });
    PM.hist.undo();
    return {
      conflict, count, keyframeCount, finished: finish.ok,
      userNameAfterUndo: PM.L('user-layer').name,
      keysAfterUndo: PM.L('agent-layer-0').p.opacity.kf.length,
      countAfterUndo: PM.proj.layers.filter((layer: any) => layer.id.startsWith('agent-layer-')).length,
    };
  });
  expect(result.conflict).toContain('get_project_state');
  expect(result).toMatchObject({
    count: 90, keyframeCount: 150, finished: true,
    userNameAfterUndo: 'User revision', keysAfterUndo: 0, countAfterUndo: 90,
  });
});
