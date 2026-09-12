import { createServer } from 'node:http';
import { expect, test } from './helpers/app';

test('connects a local model, streams chat, recovers after a broken stream and remembers the connection', async ({ session }) => {
  let fail = false;
  const requests: any[] = [];
  const server = createServer(async (req, res) => {
    const chunks: Buffer[] = []; for await (const chunk of req) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks).toString()); requests.push(body);
    if (!body.stream) { res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify({ choices: [{ message: { content: 'OK' } }] })); return; }
    res.setHeader('Content-Type', 'text/event-stream');
    res.write('data: ' + JSON.stringify({ choices: [{ delta: { content: fail ? 'Partial' : 'Hello from your local model.' } }] }) + '\n\n');
    if (!fail) res.write('data: ' + JSON.stringify({ choices: [{ delta: {}, finish_reason: 'stop' }] }) + '\n\n');
    res.end();
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const address = server.address() as { port: number };
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Local chat' }) }));
      PM.ProjectsScreen.hide(); PM.SpatialAssistant.open();
    });
    const page = session.page;
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
    await settings.getByRole('textbox', { name: 'API base URL', exact: true }).fill(`http://127.0.0.1:${address.port}/v1`);
    await settings.getByRole('textbox', { name: 'Model name', exact: true }).fill('local-test');
    await settings.getByRole('button', { name: 'Test and connect', exact: true }).click();
    await expect(settings).toContainText('Connected model: local-test');
    await settings.getByRole('button', { name: 'Done', exact: true }).click();
    await expect(page.locator('.agent-modelbar select[aria-label="Provider"]')).toHaveValue('compatible');
    const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
    await composer.fill('Hello'); await page.getByRole('button', { name: 'Send message', exact: true }).click();
    await expect(page.getByRole('log', { name: 'Agent conversation' })).toContainText('Hello from your local model.');
    await expect(page.getByRole('button', { name: 'Stop current run' })).toHaveCount(0);
    fail = true;
    await composer.fill('Try a broken connection'); await page.getByRole('button', { name: 'Send message', exact: true }).click();
    const conversation = page.getByRole('log', { name: 'Agent conversation' });
    await expect(conversation).toContainText('before the model finished');
    fail = false;
    await conversation.getByRole('button', { name: 'Try again', exact: true }).last().click();
    await expect(conversation.getByText('Hello from your local model.', { exact: true })).toHaveCount(2);
    await session.relaunch();
    await expect(session.page.locator('.agent-modelbar select[aria-label="Provider"]')).toHaveValue('compatible');
    expect(requests.some(body => body.stream && body.tools?.length)).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve())); }
});
