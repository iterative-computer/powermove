import { test, expect } from './helpers/app';

test('native pointer selection in the agent copies into its composer', async ({ session }) => {
  const page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = async () => ({ text: JSON.stringify({ summary: 'Native selection works across words.', commands: [], artifacts: [], externalActions: [], notes: [] }) });
  });
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill('Show text');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  const reply = page.locator('.agent-msg.assistant p').filter({ hasText: 'Native selection works across words.' });
  await expect(reply).toBeVisible();
  // Actual browser drag selection, starting while the composer retains focus.
  const points = await reply.evaluate(el => {
    const first = el.querySelector('span')!;
    const r = first.getBoundingClientRect();
    return { x: r.left + 1, y: r.top + r.height / 2, end: r.right - 1 };
  });
  await page.mouse.move(points.x, points.y); await page.mouse.down();
  await page.mouse.move(points.end, points.y, { steps: 12 }); await page.mouse.up();
  await expect.poll(() => page.evaluate(() => window.getSelection()?.toString())).toBe('Native');
  await session.app.evaluate(({ clipboard }) => clipboard.writeText('sentinel'));
  await page.keyboard.press('Meta+C');
  await expect.poll(() => session.app.evaluate(({ clipboard }) => clipboard.readText())).toBe('Native');
  await composer.click(); await page.keyboard.press('Meta+V');
  await expect(composer).toHaveValue('Native');
  const layersBefore = await page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.id));
  const selectedBefore = await page.evaluate(() => [...(window as any).PM.sel.layers]);
  await reply.click(); await page.keyboard.press('Meta+A');
  await expect.poll(() => page.evaluate(() => window.getSelection()?.toString())).toContain('Native selection works across words.');
  await page.keyboard.press('Meta+X');
  expect(await page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.id))).toEqual(layersBefore);
  expect(await page.evaluate(() => [...(window as any).PM.sel.layers])) .toEqual(selectedBefore);
});

for (const field of ['input', 'textarea', 'contenteditable', 'shadow']) test(`native editing in a panel ${field}`, async ({ session }) => {
  const page = session.page;
  await page.waitForSelector('#panel-agent textarea');
  await page.evaluate((kind) => {
    const body = document.querySelector('#panel-agent .body')!;
    const wrapper = document.createElement('div'); wrapper.id = 'native-text-fixture';
    wrapper.style.cssText = 'position:fixed;left:400px;top:80px;z-index:100000;background:white;color:black;width:250px;padding:10px';
    wrapper.innerHTML = kind === 'shadow' ? '<div></div>' : kind === 'contenteditable' ? '<div contenteditable="true"><span>Original text</span></div>' : `<${kind}>${kind === 'textarea' ? 'Original text' : ''}</${kind}>`;
    body.append(wrapper);
    const control = wrapper.firstElementChild as HTMLInputElement;
    if (kind === 'input') control.value = 'Original text';
    if (kind === 'shadow') { const root = control.attachShadow({ mode: 'open' }); root.innerHTML = '<input value="Original text">'; }
  }, field);
  const control = page.locator('#native-text-fixture').locator(field === 'contenteditable' ? '[contenteditable]' : field === 'shadow' ? 'input' : field);
  await control.click(); await page.keyboard.press('Meta+A');
  await session.app.evaluate(({ clipboard }) => clipboard.writeText('sentinel'));
  await page.keyboard.press('Meta+C');
  await expect.poll(() => session.app.evaluate(({ clipboard }) => clipboard.readText())).toBe('Original text');
  await page.keyboard.press('Meta+X');
  await expect.poll(() => control.evaluate((el: any) => el.value ?? el.textContent)).toBe('');
  await page.keyboard.press('Meta+V');
  await expect.poll(() => control.evaluate((el: any) => el.value ?? el.textContent)).toBe('Original text');
  await page.keyboard.press('Meta+Z');
  await expect.poll(() => control.evaluate((el: any) => el.value ?? el.textContent)).toBe('');
});

test('panel text fields offer a native right-click editing menu', async ({ session }) => {
  await session.app.evaluate(({ Menu }) => {
    (globalThis as any).__textMenus = [];
    const build = Menu.buildFromTemplate.bind(Menu);
    Menu.buildFromTemplate = (items: any) => {
      const menu = build(items);
      menu.popup = () => { (globalThis as any).__textMenus.push(items.map((i: any) => i.label || i.role)); };
      return menu;
    };
  });
  const composer = session.page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill('Text menu');
  await composer.click({ button: 'right' });
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__textMenus)).toContainEqual(expect.arrayContaining(['Copy', 'Paste', 'Select All']));
});
