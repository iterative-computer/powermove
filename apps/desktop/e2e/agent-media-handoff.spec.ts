import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { collectArtifacts } from '../src/main/codex/artifacts';
import { expect, test } from './helpers/app';

test('generated PNGs reach the timeline and continuation groups and renders the actual layers', async ({ session }) => {
  await session.openEditor();
  const prepared = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const canvas = document.createElement('canvas');
    canvas.width = 64; canvas.height = 64;
    const context = canvas.getContext('2d')!;
    context.fillStyle = '#ff0000'; context.fillRect(0, 0, 64, 64);
    return { projectId: PM.proj.id, png: canvas.toDataURL('image/png').split(',')[1] };
  });
  const runId = 'media-handoff';
  const directory = path.join(session.userData, 'Agent Workspaces', prepared.projectId, 'artifacts', runId);
  await mkdir(directory, { recursive: true });
  for (const name of ['screen.png', 'frame.png']) await writeFile(path.join(directory, name), Buffer.from(prepared.png, 'base64'));
  const artifacts = await collectArtifacts(directory, runId, ['screen.png', 'frame.png'].map(name => ({
    path: `artifacts/${runId}/${name}`, importToTimeline: true,
  })));
  expect(artifacts.every(artifact => artifact.importToTimeline)).toBe(true);
  await session.page.evaluate(({ artifacts }) => {
    const PM = (window as any).PM;
    (window as any).__handoff = { calls: 0, grouped: false, rendered: false };
    PM.CodexBridge.request = async (prompt: string) => {
      const proof = (window as any).__handoff;
      proof.calls++;
      const result = (summary: string, files: any[] = []) => ({ text: JSON.stringify({
        summary, commands: [], artifacts: files, extensions: [], notes: [], externalActions: [],
      }) });
      if (proof.calls === 1) return result('Prepared screen and frame', artifacts);
      if (proof.calls !== 2) throw new Error('Unexpected duplicate continuation');
      proof.prompt = prompt;
      const layers = PM.proj.layers.filter((layer: any) => ['screen.png', 'frame.png'].includes(layer.name));
      if (layers.length !== 2) throw new Error('Expected two real imported image layers');
      proof.layerIds = layers.map((layer: any) => layer.id);
      const request = (tool: string, args: any) => PM.AgentHarness.test.handleLiveAgentTool({
        runId: 'media-verification', callId: tool, tool, arguments: args, baseRevision: PM.proj.revision,
      });
      await request('get_project_state', {});
      await request('apply_commands', { commands: [{ type: 'group_layers', targets: proof.layerIds, name: 'iPad mini' }] });
      const frames = await request('render_frames', { times: [0] });
      proof.rendered = frames.ok && frames.content.some((item: any) => item.type === 'image');
      proof.grouped = PM.proj.layers.some((layer: any) => layer.type === 'group' && layer.name === 'iPad mini');
      const finish = await request('__finish_run', { commit: true });
      return { ...result('Imported, grouped and rendered'), liveEditsApplied: true, liveEditHistoryId: finish.historyId };
    };
    PM.AgentUI.submit('Import screen and frame, group as iPad mini and render the scene');
  }, { artifacts });
  await session.page.waitForFunction(() => (window as any).PM.AgentUI.state.phase === 'result');
  const proof = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    return { ...(window as any).__handoff, artifacts: PM.AgentUI.state.run.artifacts, error: PM.AgentUI.state.run.reviewError };
  });
  expect(proof).toMatchObject({ calls: 2, grouped: true, rendered: true, error: '' });
  expect(proof.artifacts).toHaveLength(2);
  expect(proof.artifacts.every((artifact: any) => artifact.imported && artifact.layerIds.length === 1)).toBe(true);
  for (const id of proof.layerIds) expect(proof.prompt).toContain(id);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
