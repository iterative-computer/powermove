import { appendFile, mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
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
      menu.style.cssText = 'position:fixed;top:20px;left:20px;z-index:99999;font:400 15px system-ui';
      menu.textContent = 'Menu that appears after a canvas';
      document.body.append(menu);
      menu.getBoundingClientRect();
      // Also verify UI-first keys: these descriptions were parked offscreen
      // at startup, and only now create their visible 2x strike.
      for (const weight of [500, 600]) {
        const visible = menu.cloneNode(true) as HTMLElement;
        visible.id = `font-seed-ui-${weight}`;
        visible.style.top = `${40 + (weight-500)/5}px`;
        visible.style.fontWeight = String(weight);
        document.body.append(visible);
        visible.getBoundingClientRect();
      }
      const origins = Array.from(document.querySelectorAll('*')).flatMap(el => {
        if (!Array.from(el.childNodes).some(n => n.nodeType === 3 && /\S/.test(n.textContent ?? ''))) return [];
        const s = getComputedStyle(el), r = el.getBoundingClientRect();
        if (![10, 15].includes(parseFloat(s.fontSize))) return [];
        return [{ tag:el.tagName, cls:el.className, text:el.textContent?.slice(0,80), rect:{x:r.x,y:r.y,w:r.width,h:r.height}, family:s.fontFamily, size:s.fontSize, weight:s.fontWeight, optical:s.fontOpticalSizing, variation:s.fontVariationSettings, rendering:s.textRendering, synthesis:s.fontSynthesis, transform:s.transform, visibility:s.visibility }];
      });
      return { before, width, dpr: devicePixelRatio, origins };
    });
    await session.page.waitForTimeout(1500);
    const seeds = await session.page.evaluate(async () => JSON.parse((await (window as any).__powermoveTestBridge.engineFonts.seeds()).value));
    const existing = first.before.seeds.find((s: any) => s.size === 30 && s.weight === 400 && !s.italic);
    expect(seeds.seeds.find((s: any) => s.size === 30 && s.weight === 400 && !s.italic)?.specified).toBe(existing?.specified ?? 30);
    const cases = seeds.seeds.filter((s: any) => s.family === 'system-ui').map((s: any) => ({ size: s.size, weight: s.weight, italic: s.italic }));
    // Keep the four formerly poisoned descriptions even when culling
    // correctly leaves them out of the census. No regression case escapes.
    cases.push(...[{ size:20, weight:400 }, { size:24, weight:560 }, { size:30, weight:500 }, { size:30, weight:600 }, { size:30, weight:400 }].map(d => ({ ...d, italic:false })));
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
        const query = () => PM.EngineDoc.wasm.query({ op: 'glyphLayout', comp: PM.proj.compId, open: PM.proj.compId, layer: 0, t: 0 });
        const reply = query();
        if (reply.fonts) PM.EngineDoc.api.fonts.resolve(PM.EngineDoc.wasm, query, (r: any) => !!r.fonts);
        return !!reply.ok;
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
      // Diagnose an erroneous UI seed separately from native font matching:
      // a fresh resolver for the same font description should agree when
      // this renderer never created the proposed UI cache entry.
      await expect.poll(() => session.page.evaluate(id => {
        const PM = (window as any).PM;
        PM.EngineDoc.wasm.setFontSeeds('{"seeds":[]}');
        const query = () => PM.EngineDoc.wasm.query({ op: 'glyphLayout', comp: PM.proj.compId, open: PM.proj.compId, layer: 0, t: 0 });
        const reply = query();
        if (reply.fonts) PM.EngineDoc.api.fonts.resolve(PM.EngineDoc.wasm, query, (r: any) => !!r.fonts);
        return !!reply.ok;
      }, id), { timeout: 30_000 }).toBe(true);
      const fresh = await session.page.evaluate(seeds => {
        const PM = (window as any).PM;
        const reply = PM.EngineDoc.wasm.query({ op: 'glyphLayout', comp: PM.proj.compId, open: PM.proj.compId, layer: 0, t: 0 });
        PM.EngineDoc.wasm.setFontSeeds(JSON.stringify(seeds));
        return reply.ok?.lines[0]?.w;
      }, seeds);
      rows.push({ ...row, fresh });
    }
    await appendFile(process.env.PM_FONT_SEEDS_OUT ?? '/tmp/powermove-font-seeds.jsonl', JSON.stringify({ first, seeds, rows }) + '\n');
    if (process.env.PM_FONT_SEEDS_CAPTURE) {
      const root = process.env.PM_FONT_SEEDS_CAPTURE;
      await mkdir(root, { recursive: true });
      const cases = [], shots = [];
      for (const [index, row] of rows.entries()) {
        if (Math.abs(row.legacy - row.native) <= 0.001) continue;
        const d = { text: 'First creator AVATAR 123', font: 'system-ui', size: row.size, weight: row.weight, italic: row.italic, tracking: 0, leading: 1.15, align: 'left', color: '#FFFFFF' };
        const shot = await session.page.evaluate(d => {
          const cv = document.createElement('canvas');
          const ctx = cv.getContext('2d')!;
          const font = `${d.italic ? 'italic ' : ''}${d.weight} ${d.size}px system-ui`;
          ctx.font = font;
          const width = ctx.measureText(d.text).width;
          const pad = Math.ceil(d.size * 0.6) + 24;
          cv.width = Math.ceil(Math.max(width,1)) + pad*2;
          cv.height = Math.ceil(d.size*1.15) + pad*2;
          ctx.font = font; ctx.fillStyle = '#FFFFFF';
          ctx.fillText(d.text, pad, pad + d.size*0.82);
          const bytes = ctx.getImageData(0,0,cv.width,cv.height).data;
          for (let i=0;i<bytes.length;i+=4) for (let c=0;c<3;c++) bytes[i+c] = Math.round(bytes[i+c]! * bytes[i+3]! / 255);
          return { width: cv.width, height: cv.height, rgba: Array.from(bytes) };
        }, d);
        const id = `font-seed-${index}`;
        cases.push({ id, group: 'font-seeds', kind: 'text', d, scale: 1 });
        shots.push({ id, width: shot.width, height: shot.height });
        await writeFile(path.join(root, `${id}.rgba`), Buffer.from(shot.rgba));
      }
      await writeFile(path.join(root, 'cases.json'), JSON.stringify({ cases, seeds: seeds.seeds, projectFonts: [], variableSources: {} }));
      await writeFile(path.join(root, 'manifest.json'), JSON.stringify({ cases: shots }));
    }
    for (const row of rows) expect(Math.abs(row.legacy - row.native), JSON.stringify(row)).toBeLessThanOrEqual(0.001);
  } finally { await session.close(); }
});
