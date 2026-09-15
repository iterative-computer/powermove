import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

test('an API connection creates a keyframeable effect and verifies it through live tools', async ({ session }) => {
  const manifest = await readFile(path.resolve('docs/samples/gradient-tint/manifest.json'), 'utf8');
  const source = await readFile(path.resolve('docs/samples/gradient-tint/index.ts'), 'utf8');
  const seen: any[] = [];
  let run = 0;
  const server = createServer(async (req, res) => {
    try {
      if (req.method === 'GET') { res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({ data: [{ id: 'test-author' }] })); return; }
      let input = '';
      for await (const chunk of req) input += chunk.toString();
      const body = JSON.parse(input);
      if (!body.stream) { res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({ choices: [{ message: { content: 'OK' } }] })); return; }
      seen.push(body);
      const previous = body.messages.filter((message: any) => message.role === 'tool').length;
      if (!previous) run++;
      const stage = body.messages[0].content.match(/The extension staging directory is (.+)\. When the user/)?.[1];
      const result = { summary: run === 1 ? 'Created Gradient Tint.' : 'Verified Gradient Tint in Powermove.', commands: [], artifacts: [], extensions: [], notes: [], externalActions: [] };
      const sequence: [string, any][] = run === 1 ? [
        ['read_file', { path: 'powermove-api/samples/gradient-tint/README.md' }],
        ['write_file', { path: `${stage}/gradient-tint/manifest.json`, text: manifest }],
        ['write_file', { path: `${stage}/gradient-tint/index.ts`, text: source }],
        ['compile_extension', { id: 'gradient-tint' }],
        ['complete_task', { ...result, extensions: [{ id: 'gradient-tint', action: 'created', summary: 'Keyframeable tint' }], commands: [
          JSON.stringify({ type: 'add_layer', id: 'api-effect-proof', layerType: 'shape', name: 'API effect proof', duration: 5 }),
          JSON.stringify({ type: 'add_effect', target: 'api-effect-proof', effect: 'gradient-tint' })
        ] }]
      ] : [
        ['get_workspace_state', {}],
        ['capture_panel', { panelId: 'viewer' }],
        ['render_frames', { times: [0, 1, 2], width: 320 }],
        ['complete_task', result]
      ];
      const [name, args] = sequence[previous] || ['complete_task', result];
      res.writeHead(200, { 'Content-Type': 'text/event-stream' });
      res.end(`data: ${JSON.stringify({ choices: [{ delta: { tool_calls: [{ index: 0, id: `call-${run}-${previous}`, function: { name, arguments: JSON.stringify(args) } }] }, finish_reason: 'tool_calls' }] })}\n\n`);
    } catch (error) { res.writeHead(500); res.end(String(error)); }
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const port = (server.address() as { port: number }).port;
    await session.openEditor();
    await session.page.evaluate(async port => {
      const w = window as any;
      await w.powermove.compatible.configure({ baseUrl: `http://127.0.0.1:${port}/v1`, model: 'test-author', vision: true });
      w.PM.AgentUI.setAccess('project');
      w.PM.AgentUI.setProvider('compatible');
      w.PM.SpatialAssistant.open();
      w.PM.AgentUI.submit('Create a reusable Gradient Tint effect with keyframeable controls and apply it to a shape.');
    }, port);
    await expect.poll(() => session.page.evaluate(() => (window as any).PM.AgentUI.state.phase), { timeout: 25000 }).toBe('result');
    const proof = await session.page.evaluate(() => {
      const PM = (window as any).PM;
      const layer = PM.L('api-effect-proof');
      const effect = layer?.fx[0];
      const param = effect && PM.allProps(layer).find((item: any) => item.key === `${effect.id}.amount`);
      return { registered: !!PM.Kernel.effects.get('gradient-tint'), effects: layer?.fx.length, param: !!param,
        access: PM.AgentUI.state.accessMode, conversation: JSON.stringify(PM.AgentUI.state.conversation) };
    });
    expect(proof).toMatchObject({ registered: true, effects: 1, param: true, access: 'project' });
    expect(proof.conversation).toContain('Verified Gradient Tint in Powermove.');
    expect(run).toBe(2);
    const last = seen.at(-1);
    expect(last.messages.some((message: any) => message.role === 'tool' && message.content.includes('registeredEffects') && message.content.includes('gradient-tint'))).toBe(true);
    const capture = last.messages.find((message: any) => message.role === 'tool' && message.content.includes('panel-relative CSS pixels'));
    expect(capture, 'API screenshots must use the main-process capture handler').toBeTruthy();
    expect(last.messages.some((message: any) => message.role === 'user' && Array.isArray(message.content) && message.content.some((item: any) => item.image_url?.url?.startsWith('data:image/')))).toBe(true);
  } finally {
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
  }
});
