import { expect, test } from './helpers/app';
import { connectMcpHost } from '../src/main/agent-tools/mcp-client.mjs';

test('one MCP keyframe replacement applies the complete easing curve and rolls back cleanly', async ({ session }) => {
  await session.openEditor();
  const id = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const layer = PM.mkLayer('solid', { name: 'Easing regression' });
    PM.proj.layers = [layer];
    PM.ProjectIndex.invalidate();
    PM.touch();
    return layer.id;
  });
  const client = await connectMcpHost(session.userData);
  try {
    await expect(client.callTool('start_session', { prompt: 'Verify keyframe easing' })).resolves.toMatchObject({ isError: false });
    await expect(client.callTool('apply_commands', { commands: [{
      type: 'replace_keyframes', target: id, path: 'rotation', keyframes: [
        { time: 0, value: 10, ease: 'glide' },
        { time: 2, value: 110, ease: 'glide' },
        { time: 3, value: -40, ease: 'glide' },
      ],
    }] })).resolves.toMatchObject({ isError: false });
    const samples = await session.page.evaluate(id => {
      const PM = (window as any).PM, layer = PM.L(id);
      return [[1, 10, 100], [2.5, 110, -150]].map(([time, start, delta]) => ({
        actual: PM.ev(layer, 'rotation', time), expected: start + delta * PM.Ease.fn('glide')(.5),
      }));
    }, id);
    for (const sample of samples) expect(sample.actual).toBeCloseTo(sample.expected, 4);
    await expect(client.callTool('finish_session', { commit: false })).resolves.toMatchObject({ isError: false });
    expect(await session.page.evaluate(id => (window as any).PM.L(id).p.rotation.kf.length, id)).toBe(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await client.close(); }
});
