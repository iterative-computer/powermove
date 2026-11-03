import { test, expect } from './helpers/app';
import { MAX_PAYLOAD, type FigmaDoc, type FigmaNode } from '../src/shared/figma/doc';
const node = (id = '1', color = { r: 1, g: 0.25, b: 0 }): FigmaNode => ({
  id,
  fileKey: 'synthetic-file',
  type: 'RECTANGLE',
  name: 'Figma card',
  absoluteTransform: [1, 0, 0, 1, 0, 0],
  w: 180,
  h: 100,
  fills: [{ type: 'SOLID', color }],
  strokes: [],
  effects: [],
  children: [],
});
const doc = (nodes = [node()]): FigmaDoc => ({
  format: 'powermove-figma',
  version: 1,
  source: 'plugin',
  nodes,
  images: {},
});
async function connect(session: any) {
  await session.app.evaluate(({ Menu }: any) => {
    const item = Menu.getApplicationMenu().getMenuItemById('connectFigma');
    item.click();
  });
  const label = session.page.getByText(/Enter [0-9]{6} in Send to Powermove/);
  await expect(label).toBeVisible();
  const code = (await label.textContent()).match(/[0-9]{6}/)[0];
  const info = { code, port: 47832 };
  await session.page.getByRole('button', { name: 'Done', exact: true }).click();
  const url = `http://127.0.0.1:${info.port}`;
  const send = (path: string, value: any, token?: string) =>
    fetch(url + path, {
      method: 'POST',
      headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) },
      body: JSON.stringify(value),
    });
  return { info, send };
}
test.beforeEach(async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(() => {
    const P = (window as any).PM;
    P.replaceProject(P.mkProject({ name: 'Figma Link', w: 640, h: 360, dur: 8 }));
    P.setTime(0);
    P.hist.clear();
  });
});
test('loopback rejects unpaired, invalid and oversized sends; imports a paired selection once', async ({ session }) => {
  const { send, info } = await connect(session);
  expect((await send('/send', doc())).status).toBe(401);
  expect((await send('/pair', { code: '000000' })).status).toBe(401);
  const paired = await send('/pair', { code: info.code });
  expect(paired.status).toBe(200);
  const { token } = (await paired.json()) as any;
  expect((await send('/send', { ...doc(), extra: true }, token)).status).toBe(400);
  expect((await send('/send', { large: 'x'.repeat(MAX_PAYLOAD) }, token)).status).toBe(413);
  const result = await send('/send', doc(), token);
  expect(result.status).toBe(200);
  expect(await result.json()).toMatchObject({ layers: 1, flattened: 0 });
  await expect
    .poll(() => session.page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.type)))
    .toEqual(['shape']);
  await session.page.evaluate(() => (window as any).PM.hist.undo());
  expect(await session.page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
test('resend changes content while keyframes, effects, names and timing survive; one undo restores content', async ({
  session,
}) => {
  const { send, info } = await connect(session);
  const { token } = (await (await send('/pair', { code: info.code })).json()) as any;
  expect((await send('/send', doc(), token)).status).toBe(200);
  await expect(session.page.getByText('Imported 1 layer', { exact: true })).toBeVisible();
  const before = await session.page.evaluate(() => {
    const P = (window as any).PM,
      l = P.proj.layers[0];
    l.name = 'Animated card';
    l.from = 1;
    l.dur = 5;
    P.setKey(l, 'position.x', 1, 200);
    P.setKey(l, 'position.x', 3, 400);
    l.p['position.y'].expr = '180';
    l.fx.push(P.mkEffect('blur'));
    P.touch();
    return { id: l.id, keys: structuredClone(l.p['position.x'].kf) };
  });
  const changed = node('1', { r: 0, g: 0.3, b: 1 });
  changed.w = 220;
  const sent = send('/send', doc([changed, node('new')]), token);
  await expect(session.page.getByText('Update from Figma', { exact: true })).toBeVisible();
  await expect(session.page.locator('#toasts .toast').filter({ hasText: /Imported|Updated/ })).toHaveCount(0);
  expect(await session.page.evaluate(() => (window as any).PM.proj.layers[0].d.color)).toBe('#FF4000FF');
  await session.page.getByRole('button', { name: 'Update Existing Layers', exact: true }).click();
  expect((await sent).status).toBe(200);
  await expect(session.page.getByText('Updated 2 layers', { exact: true })).toBeVisible();
  const after = await session.page.evaluate((id) => {
    const P = (window as any).PM,
      l = P.proj.layers.find((l: any) => l.id === id);
    return {
      name: l.name,
      from: l.from,
      dur: l.dur,
      color: l.d.color,
      w: l.d.w,
      keys: l.p['position.x'].kf,
      expr: l.p['position.y'].expr,
      effects: l.fx.map((f: any) => f.type),
      length: P.proj.layers.length,
    };
  }, before.id);
  expect(after).toEqual({
    name: 'Animated card',
    from: 1,
    dur: 5,
    color: '#004DFFFF',
    w: 220,
    keys: before.keys,
    expr: '180',
    effects: ['blur'],
    length: 2,
  });
  await session.page.evaluate(() => (window as any).PM.hist.undo());
  expect(await session.page.evaluate(() => (window as any).PM.proj.layers[0].d.color)).toBe('#FF4000FF');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
test('recognizes a plugin clipboard payload without a loopback connection', async ({ session }) => {
  const payload = 'POWERMOVE_FIGMA_V1\n' + JSON.stringify(doc());
  await session.app.evaluate(async ({ clipboard, ClipboardItem }, text) => {
    await clipboard.write([new ClipboardItem({ 'text/plain': text })]);
  }, payload);
  await session.app.evaluate(({ BrowserWindow }) => {
    BrowserWindow.getAllWindows()[0].isFocused = () => true;
  });
  await session.page.evaluate(() => (document.activeElement as HTMLElement)?.blur());
  await session.page.keyboard.press('Meta+V');
  await expect
    .poll(() => session.page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.figmaSource.fileKey)))
    .toEqual(['synthetic-file']);
});
