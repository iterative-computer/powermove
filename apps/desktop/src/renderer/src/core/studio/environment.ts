/** Evaluated environment contract shared with pm-studio's environment.rs.
 * File decoding is outside the render boundary; pixels here are linear HDR. */
import type { StudioVec3 } from '../scene3d/studio-backend';
export type StudioSky = Partial<{
  elevation: number;
  azimuth: number;
  turbidity: number;
  ground_albedo: StudioVec3;
  intensity: number;
  sun_intensity: number;
  sun_radius: number;
  overcast: number;
  visible: boolean;
}>;
export type StudioEnvironmentMap = {
  width: number;
  height: number;
  pixels: StudioVec3[];
  rotation?: number;
  intensity?: number;
  visible?: boolean;
};
export type StudioAtmosphere = Partial<{
  density: number;
  color: StudioVec3;
  anisotropy: number;
  base_height: number;
  height_falloff: number;
  distance: number;
}>;
export type StudioVolume = {
  shape: 'box' | 'sphere';
  center: StudioVec3;
  size: StudioVec3;
  density: number;
  color: StudioVec3;
  anisotropy: number;
};
export type StudioWorld = {
  sky?: StudioSky | null;
  hdri?: StudioEnvironmentMap | null;
  atmosphere?: StudioAtmosphere;
  volumes?: StudioVolume[];
  adaptive?: Partial<{ enabled: boolean; min_samples: number; relative_error: number; absolute_error: number }>;
};
type V = [number, number, number];
const pi = Math.PI,
  tau = 2 * pi,
  earth = 6360000;
const tuple = (v: StudioVec3): V => [v.x, v.y, v.z];
const add = (a: V, b: V): V => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const mul = (a: V, s: number): V => [a[0] * s, a[1] * s, a[2] * s];
const product = (a: V, b: V): V => [a[0] * b[0], a[1] * b[1], a[2] * b[2]];
const dot = (a: V, b: V) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const exp = (a: V): V => [Math.exp(a[0]), Math.exp(a[1]), Math.exp(a[2])];
const vec = (a: V): StudioVec3 => ({ x: a[0], y: a[1], z: a[2] });
const clamp = (x: number, a: number, b: number) => Math.max(a, Math.min(b, x));
export function skySettings(input: StudioSky = {}) {
  return {
    elevation: 45,
    azimuth: 0,
    turbidity: 2,
    ground_albedo: { x: 0.2, y: 0.2, z: 0.2 },
    intensity: 1,
    sun_intensity: 1,
    sun_radius: 0.266,
    overcast: 0,
    visible: true,
    ...input,
  };
}
function sunDirection(s: ReturnType<typeof skySettings>): V {
  const e = (s.elevation * pi) / 180,
    a = (s.azimuth * pi) / 180;
  return [Math.sin(a) * Math.cos(e), Math.sin(e), Math.cos(a) * Math.cos(e)];
}
function exit(p: V, d: V) {
  const b = dot(p, d),
    c = dot(p, p) - (earth + 80000) ** 2;
  return Math.max(0, -b + Math.sqrt(Math.max(0, b * b - c)));
}
function sunTransmittance(p: V, d: V, turbidity: number): V {
  const b = dot(p, d),
    c = dot(p, p) - earth * earth;
  if (b < 0 && b * b > c) return [0, 0, 0];
  const ds = exit(p, d) / 12;
  let r = 0,
    m = 0;
  for (let i = 0; i < 12; i++) {
    const q = add(p, mul(d, (i + 0.5) * ds)),
      h = Math.max(0, Math.sqrt(dot(q, q)) - earth);
    r += Math.exp(-h / 8000) * ds;
    m += Math.exp(-h / 1200) * ds;
  }
  return exp(add(mul([5.8e-6, 13.5e-6, 33.1e-6], -r), mul([1, 1, 1], -2e-6 * Math.max(1, turbidity) * 1.1 * m)));
}
export function bakeSky(input: StudioSky, width = 128, height = 64): StudioEnvironmentMap {
  const s = skySettings(input),
    sun = sunDirection(s),
    ro: V = [0, earth + 2, 0],
    br: V = [5.8e-6, 13.5e-6, 33.1e-6],
    bm = 2e-6 * Math.max(1, s.turbidity);
  const pixels: StudioVec3[] = [];
  for (let y = 0; y < height; y++)
    for (let x = 0; x < width; x++) {
      const theta = (pi * (y + 0.5)) / height,
        phi = (tau * (x + 0.5)) / width,
        d: V = [Math.sin(theta) * Math.sin(phi), Math.cos(theta), Math.sin(theta) * Math.cos(phi)];
      if (d[1] < 0) {
        pixels.push(vec(mul(tuple(s.ground_albedo), (0.06 + 0.24 * Math.max(0, sun[1])) * s.intensity)));
        continue;
      }
      const ds = exit(ro, d) / 24,
        mu = dot(d, sun),
        pr = (3 * (1 + mu * mu)) / (16 * pi),
        g = 0.76,
        pm = (1 - g * g) / (4 * pi * (1 + g * g - 2 * g * mu) ** 1.5);
      let odr = 0,
        odm = 0,
        out: V = [0, 0, 0];
      for (let i = 0; i < 24; i++) {
        const p = add(ro, mul(d, (i + 0.5) * ds)),
          h = Math.max(0, Math.sqrt(dot(p, p)) - earth),
          dr = Math.exp(-h / 8000),
          dm = Math.exp(-h / 1200);
        odr += dr * ds * 0.5;
        odm += dm * ds * 0.5;
        const eye = exp(mul(add(mul(br, odr), mul([1, 1, 1], bm * 1.1 * odm)), -1));
        out = add(
          out,
          mul(
            product(
              product(eye, sunTransmittance(p, sun, s.turbidity)),
              add(mul(br, dr * pr), mul([1, 1, 1], bm * dm * pm)),
            ),
            ds * 20,
          ),
        );
        odr += dr * ds * 0.5;
        odm += dm * ds * 0.5;
      }
      const day = clamp((sun[1] + 0.12) / 0.25, 0, 1),
        diffuse = mul([0.018, 0.035, 0.075], day * (1 + dot(tuple(s.ground_albedo), [1 / 3, 1 / 3, 1 / 3]))),
        clear = add(add(out, diffuse), [0.0002, 0.00035, 0.0008]);
      const cloud = ((0.15 + 0.6 * day) * (1 + 2 * d[1])) / 3;
      pixels.push(vec(mul(add(mul(clear, 1 - s.overcast), mul([1, 1, 1], cloud * s.overcast)), s.intensity)));
    }
  return { width, height, pixels, visible: s.visible };
}
export function worldSun(world: StudioWorld): number[] {
  if (!world.sky) return [];
  const s = skySettings(world.sky);
  if (s.elevation <= -0.8 || s.sun_intensity <= 0) return [];
  const d = sunDirection(s),
    radius = (s.sun_radius * pi) / 180,
    omega = tau * (1 - Math.cos(radius)),
    r = mul(
      sunTransmittance([0, earth + 2, 0], d, s.turbidity),
      (20 * s.intensity * s.sun_intensity * (1 - s.overcast)) / Math.max(1e-8, omega),
    );
  return [...d, 3, Math.cos(radius), 0, 0, 0, 0, 0, 0, 0, ...r, 0];
}
export function packWorld(world: StudioWorld, lights: ArrayLike<number>): Float32Array {
  const map = world.hdri ?? (world.sky ? bakeSky(world.sky) : null),
    a = {
      density: 0,
      color: { x: 0.9, y: 0.9, z: 0.9 },
      anisotropy: 0,
      base_height: 0,
      height_falloff: 0,
      distance: 10000,
      ...world.atmosphere,
    };
  const colorValid = (p: StudioVec3) => [p.x, p.y, p.z].every((x) => Number.isFinite(x) && x >= 0 && x <= 1);
  if (
    a.density < 0 ||
    a.height_falloff < 0 ||
    a.distance <= 0 ||
    Math.abs(a.anisotropy) > 0.95 ||
    !colorValid(a.color) ||
    [a.density, a.height_falloff, a.distance, a.anisotropy, a.base_height].some((x) => !Number.isFinite(x))
  )
    throw new Error('Invalid global atmosphere');
  if (world.sky) {
    const s = skySettings(world.sky);
    if (
      [s.elevation, s.azimuth, s.turbidity, s.intensity, s.sun_intensity, s.sun_radius, s.overcast].some(
        (x) => !Number.isFinite(x),
      ) ||
      s.elevation < -90 ||
      s.elevation > 90 ||
      s.turbidity < 1 ||
      s.turbidity > 10 ||
      s.intensity < 0 ||
      s.sun_intensity < 0 ||
      s.sun_radius < 0.05 ||
      s.sun_radius > 10 ||
      s.overcast < 0 ||
      s.overcast > 1 ||
      !colorValid(s.ground_albedo)
    )
      throw new Error('Invalid physical sky');
  }
  if (
    map &&
    (!Number.isInteger(map.width) ||
      !Number.isInteger(map.height) ||
      !Number.isFinite(map.intensity ?? 1) ||
      (map.intensity ?? 1) < 0 ||
      !Number.isFinite(map.rotation ?? 0))
  )
    throw new Error('Invalid environment map');
  for (const volume of world.volumes ?? []) {
    if (
      !colorValid(volume.color) ||
      Math.abs(volume.anisotropy) > 0.95 ||
      volume.density < 0 ||
      [volume.density, volume.anisotropy, ...tuple(volume.center), ...tuple(volume.size)].some(
        (x) => !Number.isFinite(x),
      ) ||
      tuple(volume.size).some((x) => x <= 0)
    )
      throw new Error('Invalid local fog volume');
  }
  if (
    map &&
    (map.width < 1 ||
      map.height < 1 ||
      map.width > 4096 ||
      map.height > 2048 ||
      map.pixels.length !== map.width * map.height)
  )
    throw new Error('Invalid environment map size');
  if ((world.volumes?.length ?? 0) > 32) throw new Error('At most 32 fog volumes');
  const data: number[][] = [
    [735031, 1, map?.width ?? 0, map?.height ?? 0],
    [((map?.rotation ?? 0) * pi) / 180, map?.intensity ?? 1, map?.visible === false ? 0 : map ? 1 : 0, 6],
    [...tuple(a.color), a.density],
    [a.anisotropy, a.base_height, a.height_falloff, a.distance],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
  ];
  if (map) {
    const weights: number[] = [],
      pdf: number[] = [];
    let sum = 0;
    for (let i = 0; i < map.pixels.length; i++) {
      const p = map.pixels[i]!,
        row = Math.floor(i / map.width),
        solid = (tau / map.width) * (Math.cos((pi * row) / map.height) - Math.cos((pi * (row + 1)) / map.height)),
        lum = dot(tuple(p), [0.2126, 0.7152, 0.0722]);
      if (!Number.isFinite(lum) || p.x < 0 || p.y < 0 || p.z < 0)
        throw new Error('Environment pixels must be finite nonnegative linear RGB');
      const weight = lum * solid;
      sum += weight;
      weights.push(weight);
      pdf.push(solid);
      data.push([...tuple(p), 0]);
    }
    let cdf = 0;
    weights.forEach((w, i) => {
      const probability = sum > 0 ? (0.99 * w) / sum + (0.01 * pdf[i]!) / (4 * pi) : pdf[i]! / (4 * pi);
      cdf += probability;
      data.push([i + 1 === weights.length ? 1 : cdf, probability / pdf[i]!, 0, 0]);
    });
  }
  data[4] = [data.length, world.volumes?.length ?? 0, 0, 0];
  for (const v of world.volumes ?? [])
    data.push(
      [...tuple(v.center), v.shape === 'box' ? 0 : 1],
      [...tuple(v.size), v.density],
      [...tuple(v.color), v.anisotropy],
    );
  const count = lights.length / 16,
    cdfOffset = data.length;
  data[4]![2] = cdfOffset;
  const power = (i: number) => {
    const at = i * 16,
      l = lights,
      lum = Math.max(0, l[at + 12]! * 0.2126 + l[at + 13]! * 0.7152 + l[at + 14]! * 0.0722),
      area =
        l[at + 3] === 0 ? Math.abs(l[at + 7]!) : l[at + 3] === 3 ? tau * (1 - l[at + 4]!) : 4 * pi * l[at + 4]! ** 2;
    return Math.max(1e-12, lum * area);
  };
  const weights = Array.from({ length: count }, (_, i) => power(i)),
    sum = weights.reduce((a, b) => a + b, 0);
  let cdf = 0;
  weights.forEach((w, i) => {
    cdf += w / sum;
    data.push([i + 1 === count ? 1 : cdf, w / sum, 0, 0]);
  });
  if (count > 8) {
    const base = data.length;
    data[4]![3] = base;
    const bounds = (i: number): [V, V] => {
      const o = i * 16,
        l = lights;
      if (l[o + 3] === 3)
        return [
          [0, 0, 0],
          [0, 0, 0],
        ];
      const p: V = [l[o]!, l[o + 1]!, l[o + 2]!],
        r: V =
          l[o + 3] === 0
            ? [
                Math.abs(l[o + 4]!) + Math.abs(l[o + 8]!),
                Math.abs(l[o + 5]!) + Math.abs(l[o + 9]!),
                Math.abs(l[o + 6]!) + Math.abs(l[o + 10]!),
              ]
            : [l[o + 4]!, l[o + 4]!, l[o + 4]!];
      return [add(p, mul(r, -1)), add(p, r)];
    };
    const build = (indices: number[], parent: number): number => {
      const index = (data.length - base) / 3;
      data.push([0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]);
      let lo: V = [Infinity, Infinity, Infinity],
        hi: V = [-Infinity, -Infinity, -Infinity],
        finite = 0,
        infinite = 0;
      for (const i of indices) {
        const [a, b] = bounds(i);
        lo = lo.map((v, k) => Math.min(v, a[k]!)) as V;
        hi = hi.map((v, k) => Math.max(v, b[k]!)) as V;
        if (lights[i * 16 + 3] === 3) infinite += power(i);
        else finite += power(i);
      }
      let left: number, right: number;
      if (indices.length === 1) {
        left = indices[0]!;
        right = -1;
        data[cdfOffset + left]![2] = index;
      } else {
        const extent = add(hi, mul(lo, -1)),
          axis = extent[0] >= extent[1] && extent[0] >= extent[2] ? 0 : extent[1] >= extent[2] ? 1 : 2;
        indices.sort((a, b) => lights[a * 16 + axis]! - lights[b * 16 + axis]! || a - b);
        const mid = Math.floor(indices.length / 2);
        left = build(indices.slice(0, mid), index);
        right = build(indices.slice(mid), index);
      }
      data[base + index * 3] = [...lo, left];
      data[base + index * 3 + 1] = [...hi, right];
      data[base + index * 3 + 2] = [finite, infinite, parent, 0];
      return index;
    };
    build(
      Array.from({ length: count }, (_, i) => i),
      0,
    );
  }
  const adaptive = { enabled: false, min_samples: 64, relative_error: 0.025, absolute_error: 0.001, ...world.adaptive };
  data[5] = [adaptive.enabled ? 1 : 0, adaptive.min_samples, adaptive.relative_error, adaptive.absolute_error];
  if (adaptive.min_samples < 4 || adaptive.relative_error < 0 || adaptive.absolute_error < 0)
    throw new Error('Invalid adaptive sampling settings');
  const result = new Float32Array(data.flat());
  if (result.some((v) => !Number.isFinite(v))) throw new Error('Invalid environment data');
  return result;
}
