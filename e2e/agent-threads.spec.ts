import { test, expect } from './helpers/app';

test('hidden renderer switches threads, keeps drafts, and restores history after relaunch', async ({ session }) => {
  const page = session.page;
  await page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  const picker = page.getByRole('combobox', { name: 'Switch thread', exact: true });
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await expect(picker).toBeVisible();
  const first = await picker.inputValue();
  await composer.fill('First thread draft');
  await page.getByRole('button', { name: 'New thread', exact: true }).click();
  const second = await picker.inputValue();
  expect(second).not.toBe(first);
  await expect(composer).toHaveValue('');
  await composer.fill('Second thread draft');
  await picker.selectOption(first);
  await expect(composer).toHaveValue('First thread draft');
  await picker.selectOption(second);
  await expect(composer).toHaveValue('Second thread draft');
  await expect.poll(() => page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.store.get(`agentThreads.${PM.proj.id}`)?.activeId;
  })).toBe(second);
  await session.relaunch();
  await session.page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  await expect(session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true })).toHaveValue('Second thread draft');
  await session.page.getByRole('combobox', { name: 'Switch thread', exact: true }).selectOption(first);
  await expect(session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true })).toHaveValue('First thread draft');
});

test('background app is never visible or focused, but can draw and accept input', async ({ session }, testInfo) => {
  const state = () => session.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().map(w => ({
    visible: w.isVisible(), focused: w.isFocused(), devtools: w.webContents.isDevToolsOpened(),
  })));
  expect(await state()).toEqual([{ visible: false, focused: false, devtools: false }]);
  await session.page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  await session.page.getByRole('button', { name: 'New thread', exact: true }).click();
  await session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Background input works');
  await expect(session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true })).toHaveValue('Background input works');
  await testInfo.attach('hidden-agent-threads', { body: await session.page.screenshot(), contentType: 'image/png' });
  expect(await state()).toEqual([{ visible: false, focused: false, devtools: false }]);
});

test('thread transcripts and titles survive a hidden relaunch without leaking into another thread', async ({ session }) => {
  let page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({state:{},times:[],images:[]});
    PM.CodexBridge.request = async () => ({text:JSON.stringify({summary:'A saved test reply',commands:[],artifacts:[],externalActions:[],notes:[]})});
  });
  await page.getByRole('textbox', {name:'Message Powermove agent',exact:true}).fill('First conversation request');
  await page.getByRole('button', {name:'Send message',exact:true}).click();
  await expect(page.getByRole('log')).toContainText('A saved test reply');
  const first = await page.getByRole('combobox', {name:'Switch thread',exact:true}).inputValue();
  await expect(page.getByRole('combobox', {name:'Switch thread',exact:true})).toContainText('First conversation request');
  await page.getByRole('button', {name:'New thread',exact:true}).click();
  await expect(page.getByRole('log')).not.toContainText('First conversation request');
  await page.getByRole('combobox', {name:'Switch thread',exact:true}).selectOption(first);
  await expect(page.getByRole('log')).toContainText('A saved test reply');
  // Deliberately close immediately after typing: the native quit barrier must
  // flush the draft before its debounce timer fires.
  await page.getByRole('textbox', {name:'Message Powermove agent',exact:true}).fill('Last keystroke');
  await session.relaunch(); page = session.page;
  await page.evaluate(() => (window as any).PM.SpatialAssistant.open());
  await expect(page.getByRole('log')).toContainText('First conversation request');
  await expect(page.getByRole('log')).toContainText('A saved test reply');
  await expect(page.getByRole('textbox', {name:'Message Powermove agent',exact:true})).toHaveValue('Last keystroke');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
