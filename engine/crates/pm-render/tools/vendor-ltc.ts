/**
 * Vendors three.js r180's rectangular area light tables (LTC_MAT_1 and
 * LTC_MAT_2 from examples/jsm/lights/RectAreaLightTexturesLib.js, MIT) as
 * little-endian f32 RGBA 64 × 64 files, exactly the Float32Arrays the
 * `LTC_FLOAT_1` and `LTC_FLOAT_2` DataTextures upload (WebGL2 with
 * OES_texture_float_linear, as Chromium on Apple GPUs has).
 *
 *   bun engine/crates/pm-render/tools/vendor-ltc.ts
 */
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const here = path.dirname(new URL(import.meta.url).pathname);
const repo = path.join(here, '../../../..');
const three = path.join(repo, 'apps/desktop/node_modules/three');
const version = JSON.parse(readFileSync(path.join(three, 'package.json'), 'utf8')).version;
if (version !== '0.180.0') throw new Error(`three.js ${version} is installed; the native 3D pass ports r180`);
const { RectAreaLightTexturesLib } = await import(path.join(three, 'examples/jsm/lights/RectAreaLightTexturesLib.js'));
RectAreaLightTexturesLib.init();
const out = path.join(here, '../src/scene3d/ltc');
for (const [name, texture] of [['ltc_1', RectAreaLightTexturesLib.LTC_FLOAT_1], ['ltc_2', RectAreaLightTexturesLib.LTC_FLOAT_2]] as const) {
  const data = texture.image.data as Float32Array;
  if (data.length !== 64 * 64 * 4 || texture.image.width !== 64) throw new Error(`${name}: unexpected size`);
  writeFileSync(path.join(out, `${name}.f32`), new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
}
writeFileSync(path.join(out, 'LICENSE'), readFileSync(path.join(three, 'LICENSE'), 'utf8'));
console.log(`LTC tables from three.js ${version}`);
