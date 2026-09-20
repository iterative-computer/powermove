import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

test('compact history preserves pixels, every action, and pending redo through pressure and relaunch', async ({ session }) => {
  await session.openEditor();
  await session.page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  const before = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.hist.clear();
    const hashes: number[] = [];
    const hash = () => {
      let h = 2166136261;
      for (const byte of PM.GL.renderToPixels(0, 160, 90, { transparent: true, mblur: false })) h = Math.imul(h ^ byte, 16777619);
      return h;
    };
    hashes.push(hash());
    for (let i = 0; i < 12; i++) {
      PM.hist.do(`Add shape ${i}`, () => PM.proj.layers.push(PM.mkLayer('shape', {
        p: { 'position.x': 100 + i * 60, 'position.y': 200 },
        d: { w: 100, h: 100, fill: i % 2 ? '#ff0000' : '#00ff00' },
      })));
      hashes.push(hash());
    }
    PM.hist.undo(); PM.hist.undo();
    const saved = JSON.stringify(PM.hist.export());
    PM.Memory.pressure('critical');
    if (JSON.stringify(PM.hist.export()) !== saved) throw new Error('Pressure discarded history');
    await PM.flushProject();
    return { id: PM.proj.id, hashes, saved, current: hash() };
  });
  const disk = JSON.parse(await readFile(path.join(session.userData, 'store', `projectHistory.${before.id}.json`), 'utf8'));
  expect(disk.format).toBe('powermove-history-dictionary');
  expect(disk.history.index).toBe(9);
  expect(disk.records.length).toBeLessThan(24);
  await session.relaunch();
  await session.page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  const reopened = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const hash = () => {
      let h = 2166136261;
      for (const byte of PM.GL.renderToPixels(0, 160, 90, { transparent: true, mblur: false })) h = Math.imul(h ^ byte, 16777619);
      return h;
    };
    const saved = JSON.stringify(PM.hist.export()), current = hash();
    const undo: number[] = [], redo: number[] = [];
    while (PM.hist.undo()) undo.push(hash());
    while (PM.hist.redo()) redo.push(hash());
    return { id: PM.proj.id, saved, current, undo, redo };
  });
  expect(reopened.id).toBe(before.id);
  expect(reopened.saved).toBe(before.saved);
  expect(reopened.current).toBe(before.current);
  expect(reopened.undo).toEqual(before.hashes.slice(0, 10).reverse());
  expect(reopened.redo).toEqual(before.hashes.slice(1));
  expect(new Set(before.hashes).size).toBeGreaterThan(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
