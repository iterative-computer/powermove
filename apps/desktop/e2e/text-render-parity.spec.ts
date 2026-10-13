import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { expect, test } from './helpers/app';

// Optional exact comparison against an unchanged build. Keep machine-specific
// captures outside the repository, as with rendering-performance.spec.ts.
test('multiline animated text preserves glyph geometry and rendered pixels', async ({ session }, info) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true;
    await document.fonts.ready;
    const project = PM.mkProject({ name: 'Multiline text parity', w: 640, h: 480, dur: 3, bg: '#253141' });
    const samples = [
      { text: 'AVATAR AV\nMultiline tracking\nAVATAR AV', font: 'Arial', align: 'center', tracking: -1.5, size: 24 },
      { text: 'café e\u0301 👨‍👩‍👧‍👦\n世界 · مرحبا · 😀', font: 'Georgia', align: 'left', tracking: 2, size: 22 },
      { text: 'Paragraph text wraps AVATAR and keeps every Unicode glyph 👋 in its line.',
        font: 'Arial', align: 'right', tracking: 1, size: 20, paragraph: true, boxWidth: 260, boxHeight: 160 },
    ];
    project.layers = samples.map((d, index) => {
      const layer = PM.mkLayer('text', { d: { ...d, color: '#ffffff', leading: 1.2 },
        p: { 'position.x': index === 0 ? 320 : index === 1 ? 35 : 570, 'position.y': 35 + index * 130 } }, project);
      layer.d.animators = [{ id: 'wave', name: 'Tracking wave', mode: 'wave', unit: index === 1 ? 'words' : 'characters',
        p: Object.fromEntries(Object.entries({ amount: 100, speed: 0.8, spread: 17, tracking: 3.5, rotation: 8, y: 5 })
          .map(([key, value]) => [key, PM.P(value)])) }];
      return layer;
    });
    PM.proj = project; PM.touch(); PM.rasterClear();
    const images: Record<string, number[]> = {};
    const layouts: Record<string, any> = {};
    for (const align of ['left', 'center', 'right']) {
      for (const layer of project.layers) layer.d.align = align;
      PM.touch(); PM.rasterClear();
      project.layers.forEach((layer: any, index: number) => { layouts[`${align}-${index}`] = PM.textLayout(layer, 0); });
      for (const time of [0, 0.45, 1.25]) {
        images[`${align}-${time}`] = Array.from(PM.GL.renderToPixels(time, 320, 240, { opaque: true, mblur: false }));
      }
    }
    return { images, layouts, errors: [...PM.GL.errors], glError: PM.GL.gl.getError() };
  });
  const hash = (value: Uint8Array | string) => createHash('sha256').update(value).digest('hex');
  const evidence = {
    images: Object.fromEntries(Object.entries(result.images).map(([name, bytes]) => [name, hash(Uint8Array.from(bytes))])),
    layouts: Object.fromEntries(Object.entries(result.layouts).map(([name, layout]) => [name, hash(JSON.stringify(layout))])),
  };
  writeFileSync(info.outputPath('text-render-parity.json'), JSON.stringify(evidence, null, 2));
  const baseline = process.env.PM_TEXT_RENDER_BASELINE;
  if (baseline) {
    if (process.env.PM_RECORD_TEXT_RENDER_BASELINE === '1') writeFileSync(baseline, JSON.stringify(evidence, null, 2));
    else expect(evidence).toEqual(JSON.parse(readFileSync(baseline, 'utf8')));
  }
  expect(new Set(Object.values(evidence.images)).size).toBeGreaterThan(3);
  expect(result.layouts['left-0'].lines).toHaveLength(3);
  expect(result.layouts['left-1'].characters.some((glyph: any) => glyph.text === '👨‍👩‍👧‍👦')).toBe(true);
  expect(result.layouts['left-2'].lines.length).toBeGreaterThan(1);
  expect(result.errors).toEqual([]);
  expect(result.glError).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
