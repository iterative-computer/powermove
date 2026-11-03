import { expect, test } from './helpers/app';
import { figmaHtml, figmaNode, guid, pathBlob } from '../src/renderer/src/legacy/core/figma/__fixtures__/clipboard';

test.beforeEach(async ({ session }) => {
  await session.openEditor();
  await session.app.evaluate(({ BrowserWindow }) => {
    BrowserWindow.getAllWindows()[0].isFocused = () => true;
  });
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Figma paste', w: 640, h: 360, dur: 8, bg: '#000000' }));
    PM.setTime(2);
    PM.hist.clear();
    (document.activeElement as HTMLElement)?.blur();
  });
});

test('native copy and paste creates editable grouped layers, preserves rendered placement and undoes once', async ({
  session,
}) => {
  const { app, page } = session;
  const saved = await app.evaluateHandle(async ({ clipboard, ClipboardItem }) =>
    Promise.all(
      (await clipboard.read()).map(
        async (item) =>
          new ClipboardItem(
            Object.fromEntries(await Promise.all(item.types.map(async (type) => [type, await item.getType(type)]))),
          ),
      ),
    ),
  );
  try {
    const html = await figmaHtml(
      [
        figmaNode(1, 'GROUP', {
          name: 'Figma artwork',
          fillPaints: [],
          size: { x: 200, y: 100 },
          transform: { m00: 1, m01: 0, m02: 1000, m10: 0, m11: 1, m12: 500 },
        }),
        figmaNode(2, 'RECTANGLE', {
          name: 'Orange card',
          size: { x: 200, y: 100 },
          parentIndex: { guid: guid(1), position: '!' },
        }),
        figmaNode(3, 'VECTOR', {
          name: 'Blue triangle',
          parentIndex: { guid: guid(1), position: '"' },
          fillGeometry: [{ commandsBlob: 0 }],
          fillPaints: [{ type: 'SOLID', color: { r: 0, g: 0, b: 1, a: 1 } }],
        }),
        figmaNode(4, 'TEXT', {
          name: 'Editable title',
          parentIndex: { guid: guid(1), position: '#' },
          size: { x: 80, y: 20 },
          fontName: { family: 'Arial', style: 'Regular', postscript: 'ArialMT' },
          fontSize: 16,
          textAutoResize: 'WIDTH_AND_HEIGHT',
          textData: { characters: 'Hello Figma' },
          transform: { m00: 1, m01: 0, m02: 110, m10: 0, m11: 1, m12: 10 },
          fillPaints: [{ type: 'SOLID', color: { r: 1, g: 1, b: 1, a: 1 } }],
        }),
      ],
      [pathBlob()],
      'zstd',
    );
    await app.evaluate(async ({ clipboard, ClipboardItem }, html) => {
      await clipboard.write([new ClipboardItem({ 'text/html': html })]);
    }, html);
    await page.keyboard.press('Meta+V');
    await expect
      .poll(() => page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.name)))
      .toEqual(['Figma artwork', 'Editable title', 'Blue triangle', 'Orange card']);
    const proof = await page.evaluate(() => {
      const PM = (window as any).PM;
      const canvas = PM.renderFrameTo(2, 640, 360),
        ctx = canvas.getContext('2d');
      const pixel = (x: number, y: number) => [...ctx.getImageData(x, y, 1, 1).data];
      return {
        pixels: [pixel(225, 135), pixel(400, 210), pixel(210, 125)],
        types: PM.proj.layers.map((l: any) => l.type),
        times: PM.proj.layers.map((l: any) => l.from),
        text: PM.proj.layers[1].d.text,
        vertices: PM.proj.layers[2].d.paths[0].vertices.length,
      };
    });
    expect(proof.types).toEqual(['group', 'text', 'shape', 'shape']);
    expect(proof.times).toEqual([2, 2, 2, 2]);
    expect(proof.text).toBe('Hello Figma');
    expect(proof.vertices).toBe(3);
    expect(proof.pixels[0]).toEqual([0, 0, 255, 255]);
    expect(proof.pixels[1]).toEqual([255, 64, 0, 255]);
    expect(proof.pixels[2]).toEqual([0, 0, 0, 255]);
    await page.keyboard.press('Meta+Z');
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(0);
    await page.keyboard.press('Meta+Shift+Z');
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(4);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await app.evaluate(({ clipboard }, items) => clipboard.write(items), saved);
    await saved.dispose();
  }
});

test('unsupported clipboard layers flatten independently without a confirmation and undo once', async ({ session }) => {
  const { app, page } = session;
  const html = await figmaHtml([
    figmaNode(1, 'RECTANGLE', { name: 'Editable card' }),
    figmaNode(2, 'RECTANGLE', {
      name: 'Inner shadow',
      transform: { m00: 1, m01: 0, m02: 200, m10: 0, m11: 1, m12: 0 },
      effects: [
        { type: 'INNER_SHADOW', visible: true, radius: 6, color: { r: 0, g: 0, b: 0, a: 0.5 }, offset: { x: 0, y: 4 } },
      ],
    }),
  ]);
  const png = await page.evaluate(() => {
    const c = document.createElement('canvas');
    c.width = 600;
    c.height = 120;
    const ctx = c.getContext('2d')!;
    ctx.fillStyle = '#ff4000';
    ctx.fillRect(0, 0, 200, 120);
    ctx.fillStyle = '#222';
    ctx.fillRect(400, 0, 200, 120);
    return Array.from(Uint8Array.from(atob(c.toDataURL().split(',')[1]), (v) => v.charCodeAt(0)));
  });
  await app.evaluate(
    async ({ clipboard, ClipboardItem }, { html, png }) => {
      await clipboard.write([
        new ClipboardItem({ 'text/html': html, 'image/png': new Blob([Uint8Array.from(png)], { type: 'image/png' }) }),
      ]);
    },
    { html, png },
  );
  await page.keyboard.press('Meta+V');
  await expect
    .poll(() => page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.type)))
    .toEqual(['image', 'shape']);
  await expect(page.getByText(/1 flattened/)).toBeVisible();
  const proof = await page.evaluate(() => {
    const P = (window as any).PM,
      c = P.renderFrameTo(2, 640, 360),
      x = c.getContext('2d');
    return {
      sizes: P.proj.layers.map((l: any) => [l.d.w, l.d.h]),
      pixels: [
        [200, 180],
        [400, 180],
      ].map(([a, b]) => Array.from(x.getImageData(a, b, 1, 1).data)),
    };
  });
  expect(proof.sizes).toEqual([
    [100, 60],
    [100, 60],
  ]);
  expect(proof.pixels).toEqual([
    [255, 64, 0, 255],
    [34, 34, 34, 255],
  ]);
  await page.keyboard.press('Meta+Z');
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(0);
  expect(await page.evaluate(() => Object.keys((window as any).PM.proj.assets).length)).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
test('clipboard GUIDs update existing animated layers', async ({ session }) => {
  const { app, page } = session;
  const paste = async (color: { r: number; g: number; b: number; a: number }) => {
    const html = await figmaHtml([figmaNode(1, 'RECTANGLE', { fillPaints: [{ type: 'SOLID', color }] })]);
    await app.evaluate(async ({ clipboard, ClipboardItem }, html) => {
      await clipboard.write([new ClipboardItem({ 'text/html': html })]);
    }, html);
    await page.keyboard.press('Meta+V');
  };
  await paste({ r: 1, g: 0, b: 0, a: 1 });
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(1);
  const id = await page.evaluate(() => {
    const P = (window as any).PM,
      l = P.proj.layers[0];
    P.setKey(l, 'rotation', 2, 0);
    P.setKey(l, 'rotation', 3, 90);
    return l.id;
  });
  await paste({ r: 0, g: 0, b: 1, a: 1 });
  await expect(page.getByText('Update from Figma', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Update Existing Layers', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers[0].d.color)).toBe('#0000FFFF');
  expect(
    await page.evaluate(() => {
      const l = (window as any).PM.proj.layers[0];
      return { id: l.id, keys: l.p.rotation.kf.length, count: (window as any).PM.proj.layers.length };
    }),
  ).toEqual({ id, keys: 2, count: 1 });
});

for (const font of ['Arial', 'Missing Figma Font For Layout Test'])
  test(`auto-width text stays on one rendered line with ${font}`, async ({ session }) => {
    const { app, page } = session;
    const html = await figmaHtml([
      figmaNode(1, 'TEXT', {
        name: 'Auto-width title',
        size: { x: 80, y: 30 },
        fontName: { family: font, style: 'Regular', postscript: 'SyntheticRegular' },
        fontSize: 24,
        lineHeight: { units: 'PIXELS', value: 30 },
        letterSpacing: { units: 'PIXELS', value: 1.5 },
        textAutoResize: 'WIDTH_AND_HEIGHT',
        textData: { characters: 'Hello Figma' },
      }),
    ]);
    await app.evaluate(async ({ clipboard, ClipboardItem }, html) => {
      await clipboard.write([new ClipboardItem({ 'text/html': html })]);
    }, html);
    await page.keyboard.press('Meta+V');
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(1);
    const proof = await page.evaluate(async () => {
      const P = (window as any).PM,
        l = P.proj.layers[0];
      await document.fonts.ready;
      const g = P.textRasterGeometry(l, 1, 2),
        r = P.raster(l, 1, 2),
        data = r.cv.getContext('2d').getImageData(0, 0, r.cv.width, r.cv.height).data;
      let top = Infinity,
        bottom = -Infinity;
      for (let y = 0; y < r.cv.height; y++)
        for (let x = 0; x < r.cv.width; x++)
          if (data[(y * r.cv.width + x) * 4 + 3] > 20) {
            top = Math.min(top, y);
            bottom = Math.max(bottom, y);
          }
      return {
        boxWidth: l.d.boxWidth,
        boxHeight: l.d.boxHeight,
        size: l.d.size,
        leading: l.d.leading,
        tracking: l.d.tracking,
        font: l.d.font,
        lines: g.lines.length,
        lineHeight: g.lh,
        renderHeight: g.h - 2 * g.pad,
        inkHeight: bottom - top + 1,
        width: g.w - 2 * g.pad,
      };
    });
    expect(proof).toMatchObject({
      boxWidth: 0,
      boxHeight: 0,
      size: 24,
      leading: 1.25,
      tracking: 1.5,
      font,
      lines: 1,
      lineHeight: 30,
      renderHeight: 30,
    });
    expect(proof.width).toBeGreaterThan(80);
    expect(proof.inkHeight).toBeGreaterThan(12);
    expect(proof.inkHeight).toBeLessThanOrEqual(proof.lineHeight);
  });
