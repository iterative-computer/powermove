/* Opt-in measurement harness for 3D material maps. Run explicitly under
   pm-electron-lock with PM_THREED_MAPS_EVIDENCE set to a directory outside
   Git (playwright.threed-maps.config.ts).

   Today's runtime (core/scene3d/runtime.ts texture()) gives three.js the
   asset's bitmap, the same Chromium ImageBitmap 2D layers draw. three.js
   r180 uploads it as WebGLTextures.uploadTexture does for an image source:
   UNPACK_FLIP_Y true (ignored for an ImageBitmap), UNPACK_PREMULTIPLY_ALPHA
   false, alignment 4, colorspace conversion NONE (sRGB maps share the
   working primaries), texStorage2D with log2(max(w, h)) + 1 levels in
   SRGB8_ALPHA8 (color and emissive maps) or RGBA8 (the others),
   texSubImage2D(RGBA, UNSIGNED_BYTE), then generateMipmap. This records
   the stored texels of levels 0 and 1 and any GL error, per file and per
   internal format. pm-render's tests/threed_maps.rs consumes manifest.json
   (hashes only are committed). A successful capture is not a parity pass. */
import { readdir, mkdir, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { test, expect, launchApp } from '../helpers/app';

const STILLS = path.resolve(__dirname, '../../../../engine/crates/pm-render/tests/fixtures/stills2');
const FIXTURES = path.resolve(__dirname, '../fixtures');
const MIME: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', avif: 'image/avif', tiff: 'image/tiff', webp: 'image/webp', heic: 'image/heic' };

test('record three.js map uploads of still classes through the shipped importer', async () => {
  test.setTimeout(900_000);
  const output = process.env.PM_THREED_MAPS_EVIDENCE;
  expect(output, 'set an evidence directory outside Git').toBeTruthy();
  await mkdir(output!, { recursive: true });
  const cases: Array<{ name: string; file: string; mime: string }> = [];
  for (const name of (await readdir(STILLS)).sort()) {
    const ext = name.split('.').pop()!.toLowerCase();
    if (MIME[ext]) cases.push({ name, file: path.join(STILLS, name), mime: MIME[ext]! });
  }
  for (const name of ['still-red.png', 'still-alpha.png', 'still.webp', 'still.avif', 'still.heic', 'still.tiff']) {
    cases.push({ name: `fixture-${name}`, file: path.join(FIXTURES, name), mime: MIME[name.split('.').pop()!]! });
  }
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
  const rows: unknown[] = [];
  try {
    await session.openEditor();
    await session.page.evaluate(() => {
      const input = document.createElement('input'); input.type = 'file'; input.id = 'threed-maps-source'; input.hidden = true; document.body.append(input);
    });
    for (const c of cases) {
      await session.page.locator('#threed-maps-source').setInputFiles(c.file);
      const capture = await session.page.evaluate(async input => {
        const PM = (window as any).PM;
        PM.replaceProject(PM.mkProject({ w: 256, h: 256, fps: 30, dur: 1, name: input.name }));
        const file = (document.querySelector('#threed-maps-source') as HTMLInputElement).files![0]!;
        let a: any;
        try { a = await PM.assets.add(file, { silent: true }); } catch (error) { return { reachable: false, error: String(error) }; }
        const source = a?.el || a?.bitmap;
        if (!source || a.kind !== 'image') return { reachable: false, error: 'no bitmap' };
        const w = source.width || source.naturalWidth, h = source.height || source.naturalHeight;
        const sourceType = Object.prototype.toString.call(source);
        const uploads: Record<string, unknown> = {};
        for (const srgb of [true, false]) {
          const gl = new OffscreenCanvas(4, 4).getContext('webgl2', { antialias: false })!;
          const t = gl.createTexture();
          gl.activeTexture(gl.TEXTURE0);
          gl.bindTexture(gl.TEXTURE_2D, t);
          gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
          gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
          gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
          gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.REPEAT);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
          gl.texStorage2D(gl.TEXTURE_2D, Math.log2(Math.max(w, h)) + 1, srgb ? gl.SRGB8_ALPHA8 : gl.RGBA8, w, h);
          gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, gl.RGBA, gl.UNSIGNED_BYTE, source);
          const uploadError = gl.getError();
          gl.generateMipmap(gl.TEXTURE_2D);
          const mipError = gl.getError();
          const levels: number[][] = [];
          const fb = gl.createFramebuffer();
          gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
          for (const level of [0, 1]) {
            const lw = Math.max(1, w >> level), lh = Math.max(1, h >> level);
            if (level > 0 && w === 1 && h === 1) break;
            gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t, level);
            const pixels = new Uint8Array(lw * lh * 4);
            gl.readPixels(0, 0, lw, lh, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
            levels.push([...pixels]);
          }
          const readError = gl.getError();
          gl.deleteFramebuffer(fb); gl.deleteTexture(t); gl.getExtension('WEBGL_lose_context')?.loseContext();
          uploads[srgb ? 'srgb' : 'linear'] = { uploadError, mipError, readError, levels };
        }
        return { reachable: true, w, h, sourceType, uploads };
      }, c);
      const facts: any = { ...c, file: path.basename(c.file), ...(capture as any) };
      if (facts.uploads) {
        for (const [space, u] of Object.entries<any>(facts.uploads)) {
          u.sha256 = u.levels.map((l: number[]) => createHash('sha256').update(Buffer.from(l)).digest('hex'));
          for (const [i, l] of (u.levels as number[][]).entries()) await writeFile(path.join(output!, `${c.name}.${space}.level${i}.rgba8`), Buffer.from(l));
          delete u.levels;
        }
      }
      rows.push(facts);
      await writeFile(path.join(output!, 'manifest.json'), JSON.stringify(rows, null, 2));
    }
    expect(rows).toHaveLength(cases.length);
    // The level 0 bytes are what the engine's decode must produce.
    expect((await readFile(path.join(output!, 'manifest.json'), 'utf8')).length).toBeGreaterThan(0);
  } finally { await session.close(); }
});
