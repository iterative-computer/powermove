import path from 'node:path';
import { mkdir, writeFile } from 'node:fs/promises';
import { build } from 'esbuild';
import { expect, test } from './helpers/app';

// pm-electron-lock bunx playwright test e2e/scene3d-short-attributes.spec.ts --workers=1
test('Chromium binds only final-program attributes and packs short morphs in JS', async ({ session }) => {
  await session.openEditor();
  const bundle = await build({
    entryPoints: [path.join(__dirname, 'helpers/gltf-short-attributes.ts')],
    bundle: true, write: false, format: 'iife', platform: 'browser', target: 'chrome120',
  });
  await session.page.evaluate(bundle.outputFiles![0]!.text);
  const result = await session.page.evaluate(() => (window as any).__gltfShortAttributeProof());
  const out = '/tmp/ne-w5/gltf-r7-attributes';
  await mkdir(out, { recursive: true });
  for (const [name, proof] of Object.entries(result) as Array<[string, { rgb: number[]; error: number; exception: string | null;
    activeAttributes: string[]; png: string }]>) {
    await writeFile(path.join(out, `${name}.png`), Buffer.from(proof.png.split(',')[1]!, 'base64'));
  }
  await writeFile(path.join(out, 'results.json'), JSON.stringify(Object.fromEntries(
    Object.entries(result).map(([name, { png: _png, ...proof }]) => [name, proof])), null, 2));
  for (const name of ['uvUnused', 'uvUsedIndexedTail', 'uv1Unused', 'uv2Unused', 'uv3Unused',
    'normalUnlit', 'tangentUnused', 'shortMorph'] as const) {
    expect(result[name].exception, name).toBeNull();
    expect(result[name].error, name).toBe(0);
    expect(result[name].rgb[0], name).toBeGreaterThan(100);
  }
  for (const name of ['uvUsed', 'uv1Used', 'uv2Used', 'uv3Used',
    'normalLit', 'normalLitNoLights', 'normalLitAmbientOnly',
    'normalOverride', 'tangentUsed', 'badIndex'] as const) {
    expect(result[name].exception, name).toBeNull();
    expect(result[name].error, name).toBe(1282); // INVALID_OPERATION
    expect(result[name].rgb).toEqual([0, 0, 0]);
  }
  for (const name of ['normalOnlyMorph', 'colorOnlyMorph'] as const) {
    expect(result[name].exception, name).toContain("reading 'count'");
  }
  // Chromium r180 keeps NORMAL active even with no lights or ambient only.
  for (const name of ['normalLitNoLights', 'normalLitAmbientOnly'] as const)
    expect(result[name].activeAttributes, name).toContain('normal');
  // The depth cases are recorded with active attributes and pixels before
  // assigning expected verdicts for r180's optimized map/normal shader.
  for (const name of ['uvDepthNoMap', 'uvDepthMapOpaque',
    'uvDepthMapAlphaTest', 'normalDepth', 'colorDepth'] as const)
    expect(result[name].activeAttributes, name).toBeDefined();
  expect(session.diagnostics.pageErrors).toEqual([]);
});
