import path from 'node:path';
import { expect, launchApp, repoRoot, test as base } from './helpers/app';

const test = base.extend({
  session: async ({}, use) => {
    const session = await launchApp({ env: {
      CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'),
      POWERMOVE_FAKE_CHATGPT_STATUS: 'connected'
    } });
    try { await use(session); } finally { await session.close(); }
  }
});

for (const surface of ['agent', 'floating agent', 'text tool'] as const) {
  test(`${surface} uses native selection menu actions and supplies the text services frame`, async ({ session }) => {
    await session.openEditor();
    const { page, app } = session;
    await app.evaluate(({ Menu, BrowserWindow, shell }) => {
      const state = (globalThis as any).__selectionMenu = { items: [], frameMatches: false, lookup: 0, searches: [] };
      const build = Menu.buildFromTemplate.bind(Menu);
      Menu.buildFromTemplate = items => {
        const menu = build(items);
        menu.popup = options => {
          state.items = items;
          const contents = BrowserWindow.getAllWindows()[0].webContents;
          state.frameMatches = !!options?.frame && options.frame === contents.focusedFrame;
          contents.showDefinitionForSelection = () => { state.lookup++; };
        };
        return menu;
      };
      shell.openExternal = async url => { state.searches.push(url); };
    });
    if (surface === 'floating agent') await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.WindowCapture.request = async () => null;
      PM.SpatialAssistant.activate(400, 300);
    });
    if (surface === 'text tool') {
      await page.evaluate(async () => {
        const PM = (window as any).PM;
        const project = PM.mkProject({ name: 'Native text menus', w: 800, h: 450, dur: 4 });
        const layer = PM.mkLayer('text', {
          d: { text: 'Native controls', font: 'Geist', size: 60, color: '#FFFFFF', align: 'center' },
          p: { 'position.x': 400, 'position.y': 180 }
        }, project);
        project.layers = [layer]; PM.replaceProject(project); PM.selectLayers(layer.id);
        const viewer = PM.Kernel.services.get('viewer');
        PM.Kernel.services.get('tool').setTool('select');
        viewer.fit = true; viewer.layout(); PM.invalidate();
        await document.fonts.ready;
      });
      const point = await page.evaluate(() => {
        const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
        const bounds = viewer.worldBounds(PM.proj.layers[0], PM.time);
        const rect = document.querySelector('#stage-inner')!.getBoundingClientRect();
        return { x: rect.x + bounds.cx * viewer.shown, y: rect.y + bounds.cy * viewer.shown };
      });
      await page.mouse.dblclick(point.x, point.y);
    }
    const editor = surface === 'text tool'
      ? page.getByRole('textbox', { name: 'Edit text on canvas' })
      : surface === 'floating agent' ? page.locator('.spatial-compose textarea')
        : page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
    await editor.fill('Native controls');
    await editor.press('Meta+A');
    const layersBefore = await page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.id));
    const value = () => editor.evaluate((el: any) => el.value ?? el.textContent);
    const invoke = async (label: string) => {
      await expect.poll(() => app.evaluate((_electron, label) => {
        const item = (globalThis as any).__selectionMenu.items.find((i: any) => i.label === label);
        return Boolean(item && item.enabled !== false);
      }, label)).toBe(true);
      await app.evaluate((_electron, label) => {
        const item = (globalThis as any).__selectionMenu.items.find((i: any) => i.label === label);
        if (!item || item.enabled === false) throw new Error(`Missing or disabled menu action: ${label}`);
        item.click();
      }, label);
    };
    await editor.click({ button: 'right' });
    await expect.poll(() => app.evaluate(() => ({
      labels: (globalThis as any).__selectionMenu.items.map((i: any) => i.label),
      frameMatches: (globalThis as any).__selectionMenu.frameMatches
    }))).toMatchObject({
      labels: expect.arrayContaining(['Look Up “Native controls”', 'Search with Google', 'Cut', 'Copy', 'Paste', 'Select All']),
      frameMatches: true
    });
    await invoke('Look Up “Native controls”');
    await invoke('Search with Google');
    expect(await app.evaluate(() => (globalThis as any).__selectionMenu.lookup)).toBe(1);
    expect(await app.evaluate(() => (globalThis as any).__selectionMenu.searches)).toEqual(['https://www.google.com/search?q=Native+controls']);
    await invoke('Copy');
    await expect.poll(() => app.evaluate(({ clipboard }) => clipboard.readText())).toBe('Native controls');
    await invoke('Cut');
    await expect.poll(value).toBe('');
    if (surface === 'text tool') {
      await expect.poll(() => page.evaluate(() => {
        const d = (window as any).PM.proj.layers[0].d;
        return d.text?.v ?? d.text;
      })).toBe('');
    }
    await editor.click({ button: 'right' });
    await invoke('Paste');
    await expect.poll(value).toBe('Native controls');
    await editor.click({ button: 'right' });
    await invoke('Undo');
    await expect.poll(value).toBe('');
    expect(await page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.id))).toEqual(layersBefore);
    if (surface === 'text tool') {
      await expect.poll(() => page.evaluate(() => {
        const d = (window as any).PM.proj.layers[0].d;
        return d.text?.v ?? d.text;
      })).toBe('');
      await editor.press('Escape');
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
