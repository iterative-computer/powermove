import { expect, test, chooseNativeMenu } from './helpers/app';
import { connectMcpHost } from '../src/main/agent-tools/mcp-client.mjs';

for (const theme of ['light', 'dark']) {
  test(`dense MCP edits ask before simplifying in ${theme} theme, with separate undo`, async ({ session }) => {
    await session.openEditor();
    const id = await session.page.evaluate(theme => {
      const PM = (window as any).PM;
      PM.theme.apply(theme);
      const layer = PM.mkLayer('solid', { name: 'Dense animation' });
      PM.proj.layers = [layer]; PM.ProjectIndex.invalidate(); PM.touch(); PM.hist.clear();
      return layer.id;
    }, theme);
    const client = await connectMcpHost(session.userData);
    const count = () => session.page.evaluate(id => (window as any).PM.L(id).p.opacity.kf.length, id);
    const write = () => client.callTool('apply_commands', { commands: [{ type: 'replace_keyframes', target: id, path: 'opacity',
      keyframes: Array.from({ length: 300 }, (_, i) => ({ time: i / 30, value: 50 })) }] });
    try {
      await client.callTool('start_session', { prompt: 'Verify dense keyframe consent' });
      const result = await write();
      expect(result.isError).toBe(false);
      expect(JSON.stringify(result)).toContain('nothing was simplified');
      expect(await count()).toBe(300);
      const toast = session.page.locator('.toast').filter({ has: session.page.getByRole('button', { name: 'Keep as is', exact: true }) });
      await expect(toast).toBeVisible();
      await expect(toast).toContainText("The animation won't change.");
      const layout = await toast.evaluate(el => ({ overflow: el.scrollWidth - el.clientWidth,
        buttons: [...el.querySelectorAll('button')].map(b => ({ label: b.textContent?.trim(), width: b.getBoundingClientRect().width })) }));
      expect(layout.overflow).toBeLessThanOrEqual(1);
      expect(layout.buttons.filter(b => ['Simplify', 'Keep as is'].includes(b.label || '')).every(b => b.width > 0)).toBe(true);
      await toast.getByRole('button', { name: 'Keep as is', exact: true }).click();
      expect(await count()).toBe(300);
      await write();
      await client.callTool('finish_session', { commit: true });
      await toast.getByRole('button', { name: 'Simplify', exact: true }).click();
      expect(await count()).toBe(2);
      await session.page.evaluate(() => (window as any).PM.hist.undo());
      expect(await count()).toBe(300);
      await session.page.evaluate(() => (window as any).PM.hist.undo());
      expect(await count()).toBe(0);
      await session.page.evaluate(() => { const PM = (window as any).PM; PM.hist.redo(); PM.hist.redo(); });
      expect(await count()).toBe(2);
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally { await client.close(); }
  });
}

test('native layer menu simplifies only selected layers and agents can request a summary', async ({ session }) => {
  await session.openEditor();
  const ids = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const layers = [PM.mkLayer('solid'), PM.mkLayer('solid')];
    PM.proj.layers = layers; PM.ProjectIndex.invalidate();
    for (const layer of layers) PM.animate(layer, 'opacity', Array.from({ length: 300 }, (_, i) => ({ t: i / 30, v: 50 })));
    PM.selectLayers(layers[0].id); PM.hist.clear(); PM.touch();
    return layers.map(layer => layer.id);
  });
  await chooseNativeMenu(session, 'Simplify Keyframes', () => session.page.evaluate(id => {
    const PM = (window as any).PM; PM.showLayerMenu(PM.L(id), { clientX: 250, clientY: 250 });
  }, ids[0]));
  const counts = () => session.page.evaluate(ids => ids.map(id => (window as any).PM.L(id).p.opacity.kf.length), ids);
  await expect.poll(counts).toEqual([2, 300]);
  await session.page.evaluate(() => (window as any).PM.hist.undo());
  expect(await counts()).toEqual([300, 300]);
  const client = await connectMcpHost(session.userData);
  try {
    await client.callTool('start_session', { prompt: 'Simplify selected property' });
    const result = await client.callTool('apply_commands', { commands: [{ type: 'simplify_keyframes', scope: 'properties',
      properties: [{ layerId: ids[1], path: 'opacity' }] }] });
    expect(result.isError).toBe(false);
    expect(JSON.stringify(result)).toContain('keysBefore');
    expect(await counts()).toEqual([300, 2]);
    await client.callTool('finish_session', { commit: false });
    expect(await counts()).toEqual([300, 300]);
  } finally { await client.close(); }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
