import { appendFile } from 'node:fs/promises';
import { expect, test } from '@playwright/test';
import { launchApp } from './helpers/app';

test('census and canvas-first font seeds match this renderer’s Canvas2D advances', async () => {
  test.setTimeout(180_000);
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await session.openEditor();
    await session.page.waitForFunction(() => (window as any).PM?.EngineDoc?.ready);
    const first = await session.page.evaluate(async () => {
      const w = window as any;
      const bridge = w.__powermoveTestBridge;
      const before = JSON.parse((await bridge.engineFonts.seeds()).value);
      const canvas = document.createElement('canvas').getContext('2d')!;
      canvas.font = '400 30px system-ui';
      const width = canvas.measureText('First creator AVATAR 123').width;
      const menu = document.createElement('div');
      menu.id = 'font-seed-menu';
      menu.style.cssText = 'font:400 15px system-ui';
      menu.textContent = 'Menu that appears after a canvas';
      document.body.append(menu);
      menu.getBoundingClientRect();
      return { before, width, dpr: devicePixelRatio };
    });
    await session.page.waitForTimeout(1500);
    const seeds = await session.page.evaluate(async () => JSON.parse((await (window as any).__powermoveTestBridge.engineFonts.seeds()).value));
    const existing = first.before.seeds.find((s: any) => s.size === 30 && s.weight === 400 && !s.italic);
    expect(seeds.seeds.find((s: any) => s.size === 30 && s.weight === 400 && !s.italic)?.specified).toBe(existing?.specified ?? 30);
    const cases = seeds.seeds.filter((s: any) => s.family === 'system-ui').map((s: any) => ({ size: s.size, weight: s.weight, italic: s.italic }));
    cases.push({ size: 30, weight: 400, italic: false });
    const rows = [];
    for (const d of cases) {
      const id = await session.page.evaluate((d) => {
        const PM = (window as any).PM;
        const p = PM.mkProject({ name: 'Font cache order', w: 640, h: 360, fps: 30, dur: 5 });
        const layer = PM.mkLayer('text', { d: { ...d, font: 'system-ui', text: 'First creator AVATAR 123', tracking: 0, align: 'left' } }, p);
        p.layers = [layer]; PM.replaceProject(p); PM.invalidate();
        return layer.id;
      }, d);
      await expect.poll(() => session.page.evaluate(id => {
        const PM = (window as any).PM;
        PM.EngineDoc.api.textLayout(PM.L(id), 0);
        return !!PM.EngineDoc.wasm.query({ op: 'glyphLayout', comp: PM.proj.compId, open: PM.proj.compId, layer: 0, t: 0 }).ok;
      }, id), { timeout: 30_000 }).toBe(true);
      const row = await session.page.evaluate((d) => {
        const PM = (window as any).PM;
        const c = document.createElement('canvas').getContext('2d')!;
        c.font = `${d.italic ? 'italic ' : ''}${d.weight} ${d.size}px system-ui`;
        const legacy = c.measureText('First creator AVATAR 123').width;
        const reply = PM.EngineDoc.wasm.query({ op: 'glyphLayout', comp: PM.proj.compId, open: PM.proj.compId, layer: 0, t: 0 });
        return { ...d, legacy, native: reply.ok?.lines[0]?.w, reply, seeds: PM.EngineDoc.stats().fontCensus };
      }, d);
      expect(typeof row.native, JSON.stringify(row)).toBe('number');
      expect(Math.abs(row.legacy - row.native), JSON.stringify(row)).toBeLessThanOrEqual(0.25);
      rows.push(row);
    }
    await appendFile(process.env.PM_FONT_SEEDS_OUT ?? '/tmp/powermove-font-seeds.jsonl', JSON.stringify({ first, seeds, rows }) + '\n');
  } finally { await session.close(); }
});
