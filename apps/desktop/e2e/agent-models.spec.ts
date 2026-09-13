import { test, expect } from './helpers/app';

test('Astra selection reaches the request and Claude selections preserve capabilities', async ({ session }) => {
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = async (_p: unknown, _s: unknown, _i: unknown, options: any) => {
      (window as any).__selectedModel = { model: options.model, effort: options.reasoningEffort };
      return { text: JSON.stringify({ summary: 'Model routing verified', commands: [], artifacts: [], externalActions: [], notes: [] }) };
    };
  });
  await page.locator('select[aria-label="Model"]').selectOption('gpt-6-astra');
  await page.locator('select[aria-label="Reasoning effort"]').selectOption('max');
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Check routing');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__selectedModel)).toEqual({ model: 'gpt-6-astra', effort: 'max' });
  await expect(page.locator('.agent-msg.assistant')).toContainText('Model routing verified');
  const claude = await page.evaluate(() => {
    const ui = (window as any).PM.AgentUI;
    ui.setProvider('claude');
    ui.setModel('claude-haiku-4-5-20251001', 'high');
    const haiku = { ...ui.state };
    ui.setModel('claude-sonnet-4-6', 'xhigh');
    const sonnet = { ...ui.state };
    ui.setModel('claude-fable-5-1', 'max');
    const fable = { ...ui.state };
    return { haiku: haiku.reasoningEfforts, sonnet: sonnet.reasoningEffort, fable: fable.model, effort: fable.reasoningEffort };
  });
  expect(claude).toEqual({ haiku: [], sonnet: 'high', fable: 'claude-fable-5-1', effort: 'max' });
});
