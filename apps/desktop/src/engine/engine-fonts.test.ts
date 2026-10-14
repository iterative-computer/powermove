// The headless host's text layout through pm-wasm with the native font
// registry in process. Needs the pm-bridge addon (engine/crates/pm-bridge/
// scripts/build.sh): PM_BRIDGE_ADDON or POWERMOVE_PM_BRIDGE names it.
import { existsSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { makePM } from '../renderer/src/legacy/__tests__/make-pm';
import { parseMode } from '../renderer/src/engine-doc/flags';
import { installEngineDoc } from '../renderer/src/engine-doc/install';

const MODULE = path.resolve('src/renderer/public/engine/pm_wasm.wasm');
const ADDON = process.env.PM_BRIDGE_ADDON ?? process.env.POWERMOVE_PM_BRIDGE ?? path.resolve('../../engine/target/pm-bridge/pm_bridge.node');

describe.skipIf(!existsSync(MODULE) || !existsSync(ADDON))('text layout in pm-wasm with the native registry', () => {
  it('loads answers and subsets on demand and answers natively', async () => {
    const { FontRegistry } = createRequire(import.meta.url)(ADDON);
    const registry = new FontRegistry();
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    const doc = installEngineDoc(PM, {
      mode: parseMode('on'), loadModule: () => WebAssembly.compile(readFileSync(MODULE)), sink: () => {}, headless: true,
      fonts: { sync: true, answer: (q: string) => registry.answer(q), subset: (b: string, i: number) => registry.subset(b, i) },
    });
    PM.proj = PM.mkProject({ w: 1280, h: 720 });
    expect(await doc.whenReady).toBe(true);
    const L = PM.mkLayer('text', { d: { text: 'Hello Powermove\nSecond line', font: 'Helvetica', size: 64 } });
    PM.hist.do('Add', () => PM.addLayer(L));
    const live = PM.proj.layers[0];
    const layout = doc.api!.textLayout(live, 0);
    expect(layout.lines.map((l: any) => l.text)).toEqual(['Hello Powermove', 'Second line']);
    expect(layout.lines[0].width).toBeGreaterThan(300);
    expect(doc.api!.stats.fallbacks['textLayout:fonts']).toBeUndefined();
    expect(doc.api!.fonts!.rounds).toBeGreaterThan(0);
    // Bounds depend on glyph ink, which subsets cannot give: the renderer's
    // answer stands (none here, headless).
    doc.api!.bounds(live, 0);
    expect(doc.api!.stats.fallbacks['gl.bounds:ink']).toBe(1);
  });
});
