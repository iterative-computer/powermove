import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, launchApp, test, type LaunchedApp } from './helpers/app';

type Case = 'plain' | 'color-map' | 'jpeg-color-map' | 'webp-color-map' | 'texture-transform' |
  'multi-material' | 'bump-map' | 'large-bump-map';
const cases: Case[] = ['plain', 'color-map', 'jpeg-color-map', 'webp-color-map',
  'texture-transform', 'multi-material', 'bump-map', 'large-bump-map'];

async function capture(session: LaunchedApp, choice: Case): Promise<Buffer> {
  const encoded = await session.page.evaluate(async choice => {
    const PM = (window as any).PM;
    const api = PM.Kernel.api('import-byte-parity');
    const multi = choice === 'multi-material';
    const source = 'mtllib materials.mtl\no Painted Thing\nv -1 -1 0\nv 1 -1 0\nv 0 1 0\n' +
      (multi ? 'v 1 1 0\n' : '') +
      'vt 0 0\nvt 1 0\nvt .5 1\n' + (multi ? 'vt 1 1\n' : '') +
      'vn 0 0 1\nusemtl Painted Surface\nf 1/1/1 2/2/1 3/3/1\n' +
      (multi ? 'usemtl Second Surface\nf 2/2/1 4/4/1 3/3/1\n' : '');
    const imageName = choice === 'jpeg-color-map' ? 'paint.jpg' : choice === 'webp-color-map' ? 'paint.webp' : 'paint.png';
    const colorMap = choice.includes('color-map') || choice === 'texture-transform';
    const mtl = `newmtl Painted Surface\nKd 0.8 0.6 0.4\nNs 42\n` +
      (colorMap ? `map_Kd ${choice === 'texture-transform' ? '-o 0.2 0.3 0 -s 2 1.5 1 ' : ''}${imageName}\n` : '') +
      (choice.includes('bump') ? 'map_bump -bm 0.6 height.png\n' : '') +
      (multi ? 'newmtl Second Surface\nKd 0.1 0.3 0.9\nNs 7\n' : '');
    const files = [new File([source], 'painted.obj', { type: 'model/obj' }), new File([mtl], 'materials.mtl', { type: 'text/plain' })];
    if (choice !== 'plain' && !multi) {
      const large = choice === 'large-bump-map';
      const canvas = document.createElement('canvas');
      canvas.width = large ? 2051 : 13;
      canvas.height = large ? 9 : 11;
      const context = canvas.getContext('2d')!;
      const image = context.createImageData(canvas.width, canvas.height);
      for (let y = 0; y < canvas.height; y++) for (let x = 0; x < canvas.width; x++) {
        const i = (y * canvas.width + x) * 4;
        image.data.set([(x * 19 + y * 7) & 255, (x * 3 + y * 31) & 255, (x * 11 + y * 13) & 255, 255], i);
      }
      context.putImageData(image, 0, 0);
      const mime = choice === 'jpeg-color-map' ? 'image/jpeg' : choice === 'webp-color-map' ? 'image/webp' : 'image/png';
      const blob = await new Promise<Blob>(resolve => canvas.toBlob(value => resolve(value!), mime, .85));
      files.push(new File([blob], colorMap ? imageName : 'height.png', { type: mime }));
    }
    const result = await api.scene3d.prepareImport(files);
    if (result.name !== 'painted.glb' || result.type !== 'model/gltf-binary') throw new Error('Unexpected prepared model identity');
    const bytes = new Uint8Array(await result.arrayBuffer());
    let text = '';
    for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(text);
  }, choice);
  return Buffer.from(encoded, 'base64');
}

test('engine prepareImport emits the exact legacy GLB bytes for selected OBJ bundles', async () => {
  test.setTimeout(180_000);
  const selected = cases.filter(choice => !process.env.PM_IMPORT_PARITY_CASE || process.env.PM_IMPORT_PARITY_CASE === choice);
  expect(selected.length).toBeGreaterThan(0);
  const results = new Map<Case, Buffer>();
  let session: LaunchedApp | undefined;
  try {
    session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
    await session.openEditor();
    for (const choice of selected) results.set(choice, await capture(session, choice));
    expect(session.diagnostics.pageErrors).toEqual([]);
    await session.close(); session = undefined;

    session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
    await session.openEditor();
    for (const choice of selected) {
      const expected = results.get(choice)!;
      const actual = await capture(session, choice);
      const evidence = process.env.PM_IMPORT_PARITY_EVIDENCE;
      if (evidence) {
        await mkdir(evidence, { recursive: true });
        await Promise.all([
          writeFile(path.join(evidence, `${choice}-legacy.glb`), expected),
          writeFile(path.join(evidence, `${choice}-engine.glb`), actual),
        ]);
      }
      const firstDifference = actual.findIndex((byte, index) => byte !== expected[index]);
      const detail = `${choice}: legacy=${expected.length} bytes sha256=${createHash('sha256').update(expected).digest('hex')}, ` +
        `engine=${actual.length} bytes sha256=${createHash('sha256').update(actual).digest('hex')}, first differing byte=${firstDifference}`;
      expect(actual.equals(expected), detail).toBe(true);
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session?.close();
  }
});
