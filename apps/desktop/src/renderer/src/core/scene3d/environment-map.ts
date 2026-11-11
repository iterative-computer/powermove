import * as THREE from 'three';
import { EXRLoader } from 'three/addons/loaders/EXRLoader.js';
import { HDRLoader } from 'three/addons/loaders/HDRLoader.js';

export const isEnvironmentImage = (name: string): boolean => /\.(hdr|exr)$/i.test(name);

/** Keep linear HDR pixels for lighting; the small display preview is independent.
 * Three caches the roughness-filtered environment per texture and GPU renderer. */
export async function loadEnvironmentImage(blob: Blob, name: string) {
  if (blob.size > 256 * 1024 * 1024) throw new Error('Environment images must be smaller than 256 MB');
  const format = /\.exr$/i.test(name) ? 'exr' : 'hdr';
  let decoded: ReturnType<EXRLoader['parse']> | ReturnType<HDRLoader['parse']>;
  try {
    const bytes = await blob.arrayBuffer();
    decoded = format === 'exr' ? new EXRLoader().setDataType(THREE.HalfFloatType).parse(bytes)
      : new HDRLoader().setDataType(THREE.HalfFloatType).parse(bytes);
  } catch { throw new Error(`Could not read this ${format.toUpperCase()} image. Choose an RGB equirectangular environment map.`); }
  if (!decoded?.data || !decoded.width || !decoded.height) throw new Error('Environment image has no RGB pixels');
  const { width: w, height: h } = decoded;
  if (w * h > 64 * 1024 * 1024) throw new Error('Environment images must be at most 64 megapixels');
  // A 4K panorama is ample for the filtered lighting texture. Preserve the
  // original file in media storage, and avoid uploading enormous panoramas.
  const scale = Math.min(1, 4096 / w, 2048 / h), width = Math.max(1, Math.round(w * scale)), height = Math.max(1, Math.round(h * scale));
  const channels = decoded.data.length / (w * h);
  if (channels !== 3 && channels !== 4) throw new Error('Choose an RGB environment image');
  const pixels = new Uint16Array(width * height * 4);
  const data = decoded.data as Uint16Array;
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const dest = (y * width + x) * 4;
    if (width === w && height === h) {
      const source = (y * w + x) * channels;
      pixels[dest] = data[source]!; pixels[dest + 1] = data[source + 1]!; pixels[dest + 2] = data[source + 2]!;
    } else {
      // Average in linear light so a small, bright sun retains its energy.
      const x0 = Math.floor(x * w / width), x1 = Math.floor((x + 1) * w / width);
      const y0 = Math.floor(y * h / height), y1 = Math.floor((y + 1) * h / height);
      const count = (x1 - x0) * (y1 - y0);
      for (let c = 0; c < 3; c++) {
        let sum = 0;
        for (let sy = y0; sy < y1; sy++) for (let sx = x0; sx < x1; sx++) sum += THREE.DataUtils.fromHalfFloat(data[(sy * w + sx) * channels + c]!);
        pixels[dest + c] = THREE.DataUtils.toHalfFloat(Math.min(65504, Math.max(0, sum / count)));
      }
    }
    pixels[dest + 3] = 0x3c00;
  }
  const texture = new THREE.DataTexture(pixels, width, height, THREE.RGBAFormat, THREE.HalfFloatType);
  texture.mapping = THREE.EquirectangularReflectionMapping;
  texture.colorSpace = THREE.LinearSRGBColorSpace;
  texture.minFilter = texture.magFilter = THREE.LinearFilter;
  texture.generateMipmaps = false; texture.needsUpdate = true;
  const el = document.createElement('canvas');
  el.width = Math.min(1024, width); el.height = Math.max(1, Math.round(height * el.width / width));
  const context = el.getContext('2d');
  if (context) {
    const preview = context.createImageData(el.width, el.height);
    for (let y = 0; y < el.height; y++) for (let x = 0; x < el.width; x++) {
      const source = (Math.floor(y * height / el.height) * width + Math.floor(x * width / el.width)) * 4, dest = (y * el.width + x) * 4;
      for (let c = 0; c < 3; c++) {
        const linear = Math.max(0, THREE.DataUtils.fromHalfFloat(pixels[source + c]!));
        const mapped = Number.isFinite(linear) ? linear / (1 + linear) : 1;
        preview.data[dest + c] = Math.round((mapped <= .0031308 ? mapped * 12.92 : 1.055 * mapped ** (1 / 2.4) - .055) * 255);
      }
      preview.data[dest + 3] = 255;
    }
    context.putImageData(preview, 0, 0);
  }
  return { format, w, h, el, environmentTexture: texture };
}

/** Original neutral studio lighting used by a new Environment Light. */
export function defaultEnvironmentTexture(): THREE.DataTexture {
  const width=128,height=64,pixels=new Uint16Array(width*height*4);
  for(let y=0;y<height;y++)for(let x=0;x<width;x++){
    const panel=(x>16&&x<38&&y>12&&y<32)?4:(x>80&&x<104&&y>18&&y<38)?2:.12;
    const i=(y*width+x)*4;
    for(let c=0;c<3;c++)pixels[i+c]=THREE.DataUtils.toHalfFloat(panel);
    pixels[i+3]=0x3c00;
  }
  const texture=new THREE.DataTexture(pixels,width,height,THREE.RGBAFormat,THREE.HalfFloatType);
  texture.mapping=THREE.EquirectangularReflectionMapping;texture.colorSpace=THREE.LinearSRGBColorSpace;
  texture.minFilter=texture.magFilter=THREE.LinearFilter;texture.needsUpdate=true;
  return texture;
}
