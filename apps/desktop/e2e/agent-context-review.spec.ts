import { expect, test } from './helpers/app';

test('live discovery reaches late layers and all keyframes in a large composition', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.proj.layers = Array.from({ length: 320 }, (_, i) => PM.mkLayer('shape', { name: i === 319 ? 'Final logo' : `Shape ${i}` }));
    PM.ProjectIndex.invalidate();
    const logo = PM.proj.layers[319];
    PM.Edit.apply({ type: 'replace_keyframes', target: logo.id, path: 'opacity', keyframes: Array.from({ length: 30 }, (_, i) => ({ time: i / 30, value: i })) });
    const call = async (args: any) => {
      const response = await PM.AgentHarness.test.handleLiveAgentTool({ runId: 'large-context', callId: 'read', tool: 'get_project_state', arguments: args, baseRevision: PM.proj.revision });
      return JSON.parse(response.content[0].text);
    };
    const ids = new Set<string>(); let offset: number | null = 0;
    while (offset !== null) {
      const page: any = await call({ indexOnly: true, layerOffset: offset, layerLimit: 100 });
      page.layers.forEach((layer: any) => ids.add(layer.id)); offset = page.pagination.nextLayerOffset;
    }
    const target = (await call({ layerSearch: 'FINAL LOGO' })).layers[0];
    const values = []; let keyOffset: number | null = 0;
    while (keyOffset !== null) {
      const page = await call({ layerId: target.id, keyframeOffset: keyOffset, keyframeLimit: 8 });
      const opacity = page.layers[0].properties.find((prop: any) => prop.path === 'opacity');
      values.push(...opacity.keyframes.map((key: any) => key.value)); keyOffset = opacity.nextKeyframeOffset;
    }
    return { layers: ids.size, name: target.name, values };
  });
  expect(result.layers).toBe(320);
  expect(result.name).toBe('Final logo');
  expect(result.values).toEqual(Array.from({ length: 30 }, (_, i) => i));
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Composition only reviews and repairs with the generation settings after a picker change', async ({ session }) => {
  await session.openEditor(); await session.openAgent();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentUI.setAccess('editor'); PM.AgentUI.setModel('gpt-6.1-sol', 'high');
    (window as any).__editorRuns = [];
    PM.CodexBridge.request = async (_prompt: string, _schema: unknown, images: string[], options: any) => {
      const runs = (window as any).__editorRuns;
      runs.push({ model: options.model, reasoningEffort: options.reasoningEffort, provider: options.provider, threadId: options.threadId, images: images.length });
      return JSON.stringify(runs.length === 1 ? {
        kind: 'scene', operation: 'modify', message: 'Ready to add the title.', steps: ['Add title', 'Review title'],
        sceneEdit: { label: 'Add title', commands: [{ type: 'add_layer', id: 'review-title', layerType: 'text', content: { text: 'Title' } }], reviewTimes: [] },
      } : runs.length === 2 ? { status: 'repair', message: 'Fix the title.', commands: [{ type: 'set_content', target: 'review-title', patch: { text: 'Repaired title' } }], reviewTimes: [] }
        : { status: 'pass', message: 'Reviewed the title.', commands: [], reviewTimes: [] });
    };
    PM.AgentUI.submit('Create a readable title');
  });
  await expect.poll(() => page.evaluate(() => Boolean((window as any).PM.AgentUI.state.plan))).toBe(true);
  await page.evaluate(() => { const PM = (window as any).PM; PM.AgentUI.setModel('gpt-6-astra', 'max'); PM.AgentUI.applyPlan(); });
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.legacyPhase)).toBe('result');
  const result = await page.evaluate(() => ({ runs: (window as any).__editorRuns, text: (window as any).PM.L('review-title').d.text }));
  expect(result.runs).toHaveLength(3);
  for (const run of result.runs) expect(run).toMatchObject({ model: 'gpt-6.1-sol', reasoningEffort: 'high', provider: 'chatgpt', images: expect.any(Number) });
  expect(result.runs.every((run: any) => run.threadId === result.runs[0].threadId && run.images >= 3)).toBe(true);
  expect(result.text).toBe('Repaired title');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a composition edit reviews final frames with the original settings after a picker change', async ({ session }) => {
  await session.openEditor(); await session.openAgent();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause(); PM.setTime(1);
    PM.AgentUI.setAccess('project');
    PM.AgentUI.setModel('gpt-6.1-sol', 'high');
    (window as any).__contextRuns = [];
    PM.CodexBridge.request = async (prompt: string, _schema: unknown, images: string[], options: any) => {
      const runs = (window as any).__contextRuns;
      runs.push({ prompt, images, provider: options.provider, model: options.model, reasoningEffort: options.reasoningEffort, threadId: options.threadId });
      if (runs.length === 1) PM.AgentUI.setModel('gpt-6-astra', 'max');
      return { text: JSON.stringify({ summary: runs.length === 1 ? 'Created the shape.' : 'Reviewed the rendered shape.',
        commands: runs.length === 1 ? [{ type: 'add_layer', id: 'review-shape', layerType: 'shape', name: 'Review shape', content: { shape: 'rect', w: 400, h: 300, color: '#ffcc00' } }] : [],
        artifacts: [], externalActions: [], notes: [] }), extensions: [] };
    };
  });
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Create a yellow rectangle');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.legacyPhase)).toBe('result');
  const result = await page.evaluate(async () => {
    const PM = (window as any).PM, runs = (window as any).__contextRuns;
    const image = new Image(); image.src = runs[1].images[0]; await image.decode();
    const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height;
    const context = canvas.getContext('2d')!; context.drawImage(image, 0, 0);
    const bytes = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let yellowPixels = 0;
    for (let i = 0; i < bytes.length; i += 4) if (bytes[i]! > 200 && bytes[i + 1]! > 140 && bytes[i + 2]! < 70) yellowPixels++;
    return { pickerModel: PM.AgentUI.state.model, calls: runs.length, settings: runs.map(({ provider, model, reasoningEffort, threadId }: any) => ({ provider, model, reasoningEffort, threadId })),
      images: runs.map((run: any) => run.images.length), yellowPixels, changed: runs[0].images[0] !== runs[1].images[0],
      prompt: runs[1].prompt, layers: PM.proj.layers.filter((layer: any) => layer.id === 'review-shape').length, reviewError: PM.AgentUI.state.run.reviewError, time: PM.time };
  });
  expect(result.pickerModel).toBe('gpt-6-astra');
  expect(result.calls).toBe(2);
  expect(result.settings[1]).toEqual(result.settings[0]);
  expect(result.settings[0]).toMatchObject({ provider: 'chatgpt', model: 'gpt-6.1-sol', reasoningEffort: 'high' });
  expect(result.images.every((count: number) => count >= 3)).toBe(true);
  expect(result.yellowPixels).toBeGreaterThan(100);
  expect(result.changed).toBe(true);
  expect(result.prompt).toContain('AUTOMATIC RESULT VERIFICATION');
  expect(result.prompt).not.toContain('EXTENSION LOAD REPORT');
  expect(result.layers).toBe(1);
  expect(result.reviewError).toBe('');
  expect(result.time).toBe(1);
  await page.evaluate(() => (window as any).PM.AgentUI.undoSceneRun());
  expect(await page.evaluate(() => Boolean((window as any).PM.L('review-shape')))).toBe(false);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('capture failure preserves applied edits and reports the verification limit', async ({ session }) => {
  await session.openEditor(); await session.openAgent();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentUI.setAccess('project');
    PM.Export.snapshotAsync = async () => { throw new Error('Preview capture failed'); };
    let calls = 0;
    PM.CodexBridge.request = async () => ({ text: JSON.stringify({ summary: 'Changed the background.',
      commands: calls++ === 0 ? [{ type: 'set_composition', patch: { background: '#224466' } }] : [],
      artifacts: [], externalActions: [], notes: [] }), extensions: [] });
  });
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Change the background to blue');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.legacyPhase)).toBe('result');
  const result = await page.evaluate(() => ({ error: (window as any).PM.AgentUI.state.run.reviewError, background: (window as any).PM.proj.bg }));
  expect(result.error).toContain('Preview capture failed');
  expect(result.background).toBe('#224466');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
