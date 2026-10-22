import path from 'node:path';
import { buildSync } from 'esbuild';
import { expect, test } from './helpers/app';

test('native surface presentation preserves F16 HDR, alpha and orientation in sRGB and P3 canvases', async ({ session }) => {
  await session.openEditor();
  const code = buildSync({ entryPoints: [path.join(__dirname, 'tools/frame-painter-probe.ts')], bundle: true, format: 'iife', globalName: 'FramePainterProbe', write: false }).outputFiles[0]!.text;
  const rows = await session.page.evaluate(async code => {
    return new Function(`${code}; return FramePainterProbe.run();`)();
  }, code);
  expect(rows).toHaveLength(14);
  for (const row of rows) expect(row.actual, JSON.stringify({ space: row.space, color: row.color })).toEqual(row.expected);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
