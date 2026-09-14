import { createServer } from 'node:http';
import { expect, test } from './helpers/app';

test('connects a local model, streams chat, recovers after a broken stream and remembers the connection', async ({ session }) => {
  let fail = false;
  const requests: any[] = [];
  const server = createServer(async (req, res) => {
    if (req.method === 'GET') { res.writeHead(404).end(); return; }
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

test('provider models, reasoning, and flat error actions remain usable in a narrow panel', async ({ session }, testInfo) => {
  const requests: any[] = [];
  let fail = false;
  const server = createServer(async (req, res) => {
    res.setHeader('Content-Type', 'application/json');
    if (req.method === 'GET') {
      res.end(JSON.stringify({ data: ['gpt-6-astra', 'gpt-5.6-sol', 'gpt-5.6-terra', 'gpt-5.6-luna'].map(id => ({ id })) })); return;
    }
    const chunks: Buffer[] = []; for await (const chunk of req) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks).toString()); requests.push(body);
    if (!body.stream) { res.end(JSON.stringify({ choices: [{ message: { content: 'OK' } }] })); return; }
    if (fail) { res.writeHead(429).end(); return; }
    res.setHeader('Content-Type', 'text/event-stream');
    res.end('data: ' + JSON.stringify({ choices: [{ delta: { content: 'Selection verified.' }, finish_reason: 'stop' }] }) + '\n\n');
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    await session.openEditor();
    const { page } = session;
    const address = server.address() as { port: number };
    await page.evaluate(async port => {
      const config = await (window as any).powermove.compatible.configure({ baseUrl: `http://127.0.0.1:${port}/v1`, model: 'gpt-6-astra', vision: false });
      window.dispatchEvent(new CustomEvent('pm-provider-connected', { detail: config }));
      const PM = (window as any).PM;
      PM.SpatialAssistant.open();
      PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
      PM.theme.apply('dark');
    }, address.port);
    await expect(page.locator('select[aria-label="Model"] option')).toHaveCount(4);
    const panel = page.locator('#panel-agent');
    await panel.evaluate(el => {
      Object.assign((el as HTMLElement).style, { width: '300px', maxWidth: '300px', minWidth: '0' });
      (el.closest('.dock') as HTMLElement).style.flex = '0 0 300px';
    });
    await page.getByRole('combobox', { name: 'Model', exact: true }).click();
    await page.getByRole('option', { name: 'gpt-5.6-sol', exact: true }).click();
    await page.getByRole('combobox', { name: 'Reasoning effort', exact: true }).click();
    await page.getByRole('option', { name: 'Max', exact: true }).click();
    await expect(page.locator('select[aria-label="Reasoning effort"]')).toHaveValue('max');
    const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
    await composer.fill('Check selected settings');
    await page.getByRole('button', { name: 'Send message', exact: true }).click();
    await expect(panel).toContainText('Selection verified.');
    expect(requests.find(body => body.stream)).toMatchObject({ model: 'gpt-5.6-sol', reasoning_effort: 'max' });
    fail = true;
    await composer.fill('Check error actions');
    await page.getByRole('button', { name: 'Send message', exact: true }).click();
    const card = panel.locator('.error-notice').last();
    await expect(card.getByRole('button', { name: 'Try again', exact: true })).toBeVisible();
    expect(await card.evaluate(el => getComputedStyle(el).borderTopWidth)).toBe('0px');
    const button = card.getByRole('button', { name: 'Try again', exact: true });
    expect(await button.evaluate(el => ({ image: getComputedStyle(el).backgroundImage, shadow: getComputedStyle(el).boxShadow }))).toEqual({ image: 'none', shadow: 'none' });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => (window as any).PM.theme.apply(theme), theme);
      await panel.screenshot({ path: testInfo.outputPath(`api-error-${theme}.png`) });
    }
    fail = false;
    await button.click();
    await expect(panel.getByText('Selection verified.', { exact: true })).toHaveCount(2);
    await session.relaunch();
    await expect(session.page.locator('select[aria-label="Model"]')).toHaveValue('gpt-5.6-sol');
    await expect(session.page.locator('select[aria-label="Reasoning effort"]')).toHaveValue('max');
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve())); }
});
