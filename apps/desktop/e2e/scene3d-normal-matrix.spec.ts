import path from 'node:path';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { build } from 'esbuild';
import { expect, test } from './helpers/app';
import type { Proof } from './helpers/short-normal-matrix';

test('all r180 material/normal define combinations match the captured oracle', async ({ session }) => {
  test.setTimeout(180_000);
  await session.openEditor();
  const bundle = await build({
    entryPoints: [path.join(__dirname, 'helpers/short-normal-matrix.ts')],
    bundle: true, write: false, format: 'iife', platform: 'browser', target: 'chrome120',
  });
  await session.page.evaluate(bundle.outputFiles![0]!.text);
  const result: Record<string, Proof> = await session.page.evaluate(() => (window as any).__shortNormalMatrix());
  const out = '/tmp/ne-w5/threed-r8/normal-matrix';
  await mkdir(out, { recursive: true });
  for (const [key, row] of Object.entries(result)) {
    await writeFile(`${out}/${key}.png`, Buffer.from(row.png.split(',')[1]!, 'base64'));
  }
  await writeFile(`${out}/results.json`, JSON.stringify(Object.fromEntries(Object.entries(result)
    .map(([key, { png: _png, ...row }]) => [key, row])), null, 2));
  const oracle = JSON.parse(await readFile(path.join(__dirname,
    '../../../engine/crates/pm-render/tests/fixtures/short-normal-programs.json'), 'utf8'));
  expect(Object.keys(result)).toHaveLength(640);
  for (const row of oracle.cases) {
    const full = result[`${row.caseId}-full`]!;
    const short = result[`${row.caseId}-short`]!;
    expect(full.activeAttributes, row.caseId).toEqual(row.activeAttributes);
    expect(short.activeAttributes, row.caseId).toEqual(row.activeAttributes);
    expect(full.exception, row.caseId).toEqual(row.preDrawException);
    expect(short.exception, row.caseId).toEqual(row.preDrawException);
    if (row.preDrawException) {
      // Basic.normalMap is an unsupported material mutation: r180 throws
      // before either complete or short NORMAL can reach the draw.
      expect(row.kind).toBe('Basic');
      expect(row.flags.normalMap).toBe(true);
      expect(full.covered, row.caseId).toBe(0);
      expect(short.covered, row.caseId).toBe(0);
      continue;
    }
    expect(full.error, row.caseId).toBe(0);
    expect(full.covered, row.caseId).toBeGreaterThan(100);
    const consumed = row.activeAttributes.includes('normal');
    expect(short.error, row.caseId).toBe(consumed ? 1282 : 0);
    expect(short.covered, row.caseId).toBe(consumed ? 0 : full.covered);
    if (!consumed) expect(short.rgba, row.caseId).toEqual(full.rgba);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
