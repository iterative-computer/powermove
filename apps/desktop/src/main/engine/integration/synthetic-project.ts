/*
 * The synthetic projects of the Phase 2 performance rows (plan section 25,
 * "Layer drag, synthetic 500-layer project of slice features"; "Inspector
 * edit during playback, 1,000 layers"; docs/engine/tracer.md).
 *
 * "Slice features" are what the Phase 2 renderer draws: solids, images,
 * transforms (static and keyframed position, rotation, scale, anchor),
 * opacity (static and keyframed), normal blending, groups, precomps, and
 * parenting through nulls. Video layers are measured on their own (the
 * scrub rows). Every layer is one the engine draws, so the whole project
 * routes to it.
 *
 * `buildSyntheticProject` runs in the editor's page and is self-contained
 * (no imports, no outer references), so the measurement harness can bundle
 * it and Playwright can pass it to `page.evaluate`. It is deterministic for
 * a seed. `layers` counts every layer in every composition.
 */

export type SyntheticOptions = {
  layers: number;
  fps?: number;
  /** Frames per second as a rational (23.976 is 24000/1001). */
  fpsRational?: [number, number] | null;
  w?: number;
  h?: number;
  dur?: number;
  seed?: number;
  /** Image assets to import and use (0: solids only). */
  images?: number;
};

export type SyntheticSummary = {
  layers: number;
  byType: Record<string, number>;
  groups: number;
  precomps: number;
  compositions: number;
  keyframed: number;
  images: number;
  fps: number;
  w: number;
  h: number;
};

/* eslint-disable @typescript-eslint/no-explicit-any */
export async function buildSyntheticProject(options: SyntheticOptions): Promise<SyntheticSummary> {
  const PM = (globalThis as any).PM;
  const total = Math.max(1, Math.floor(options.layers));
  const fps = options.fpsRational ? options.fpsRational[0] / options.fpsRational[1] : (options.fps ?? 30);
  const w = options.w ?? 1920;
  const h = options.h ?? 1080;
  const dur = options.dur ?? 10;
  let seed = (options.seed ?? 1) >>> 0;
  const random = (): number => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  const pick = <T,>(xs: readonly T[]): T => xs[Math.floor(random() * xs.length)]!;
  const snap = (t: number): number => Math.round(t * fps) / fps;
  const palette = ['#E8553A', '#F2A541', '#3FA7D6', '#59CD90', '#EE6352', '#FAC05E', '#8C5383', '#2E86AB', '#F6F5AE', '#D1D1D1'];

  const project = PM.mkProject({ name: `Synthetic ${total} layers`, w, h, fps, dur, bg: '#141821' });
  if (options.fpsRational) project.fpsRational = { num: options.fpsRational[0], den: options.fpsRational[1] };
  project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 'bg', color: '#141821', position: 0 }] };
  PM.replaceProject(project);
  const root = PM.proj;

  // Image assets: small generated PNGs imported as media (no layers).
  const imageIds: string[] = [];
  const imageCount = Math.max(0, Math.floor(options.images ?? 2));
  if (imageCount && typeof OffscreenCanvas !== 'undefined' && PM.importFiles) {
    const files: File[] = [];
    for (let i = 0; i < imageCount; i++) {
      const canvas = new OffscreenCanvas(256, 256);
      const g = canvas.getContext('2d')!;
      const grad = g.createLinearGradient(0, 0, 256, 256);
      grad.addColorStop(0, palette[i % palette.length]!);
      grad.addColorStop(1, palette[(i + 3) % palette.length]!);
      g.fillStyle = grad;
      g.fillRect(0, 0, 256, 256);
      g.fillStyle = 'rgba(255,255,255,0.8)';
      g.beginPath();
      g.arc(128, 128, 64 + i * 8, 0, Math.PI * 2);
      g.fill();
      const blob = await canvas.convertToBlob({ type: 'image/png' });
      // Letters, not numbers: numbered stills would be offered as an image sequence.
      files.push(new File([blob], `synthetic image ${String.fromCharCode(97 + (i % 26))}.png`, { type: 'image/png' }));
    }
    const before = new Set(Object.keys(root.assets ?? {}));
    await PM.importFiles(files, { project: root, placement: null });
    for (const [id, asset] of Object.entries<any>(root.assets ?? {})) if (!before.has(id) && (asset?.kind ?? asset?.type) === 'image') imageIds.push(id);
  }

  const byType: Record<string, number> = {};
  let keyframed = 0;
  const count = (type: string): void => {
    byType[type] = (byType[type] ?? 0) + 1;
  };
  const animate = (layer: any, path: string, values: number[]): void => {
    const prop = layer.p[path];
    if (!prop) return;
    prop.kf = values.map((v, i) => PM.KF(snap((i * dur) / Math.max(1, values.length - 1)), v, pick(['linear', 'easeOut', 'easeInOut', 'power'])));
    keyframed++;
  };
  /** A slice-feature leaf layer (solid or image) placed in a `cw` × `ch` composition. */
  const leaf = (target: any, cw: number, ch: number, index: number): any => {
    const useImage = imageIds.length > 0 && random() < 0.06;
    const size = 24 + Math.floor(random() * 120);
    const layer = useImage
      ? PM.mkLayer('image', { name: `Image ${index}`, from: 0, dur, d: { asset: pick(imageIds), fit: 'contain', w: size, h: size } }, target)
      : PM.mkLayer('solid', { name: `Solid ${index}`, from: 0, dur, d: { color: pick(palette), w: size, h: Math.max(8, Math.floor(size * (0.4 + random()))) } }, target);
    count(layer.type);
    layer.p['position.x'].v = Math.round(random() * cw);
    layer.p['position.y'].v = Math.round(random() * ch);
    layer.p.opacity.v = 40 + Math.round(random() * 60);
    if (layer.p.rotation) layer.p.rotation.v = Math.round(random() * 360 - 180);
    const roll = random();
    if (roll < 0.25) animate(layer, 'position.x', [layer.p['position.x'].v, Math.round(random() * cw), layer.p['position.x'].v]);
    else if (roll < 0.4) animate(layer, 'rotation', [0, Math.round(random() * 360)]);
    else if (roll < 0.5) animate(layer, 'opacity', [100, 20, 100]);
    else if (roll < 0.58) {
      animate(layer, 'scale.x', [100, 60 + Math.round(random() * 80)]);
      animate(layer, 'scale.y', [100, 60 + Math.round(random() * 80)]);
    }
    return layer;
  };

  let made = 0;
  const layers: any[] = [];
  // A full-frame background solid.
  const backdrop = PM.mkLayer('solid', { name: 'Backdrop', from: 0, dur, d: { color: '#1E2430', w, h } }, root);
  backdrop.p['anchor.x'].v = 0;
  backdrop.p['anchor.y'].v = 0;
  backdrop.p['position.x'].v = 0;
  backdrop.p['position.y'].v = 0;
  count('solid');
  layers.push(backdrop);
  made++;

  // Precomps: one composition of 10 layers per 100 layers, each used twice.
  const compositions: string[] = [];
  const precompCount = Math.floor(total / 100);
  for (let c = 0; c < precompCount && made + 12 <= total; c++) {
    const sub = PM.mkProject({ name: `Nested ${c + 1}`, w: 480, h: 270, fps, dur, bg: '#000000' });
    if (options.fpsRational) sub.fpsRational = { num: options.fpsRational[0], den: options.fpsRational[1] };
    sub.backgroundFill = { type: 'none', stops: [], angle: 0 };
    const inner: any[] = [];
    for (let i = 0; i < 10; i++) inner.push(leaf(sub, 480, 270, made + i));
    sub.layers = inner;
    root.comps[sub.id] = sub;
    compositions.push(sub.id);
    made += 10;
    for (let k = 0; k < 2; k++) {
      const pre = PM.mkLayer('precomp', { name: `Nested ${c + 1}.${k + 1}`, from: 0, dur, d: { comp: sub.id, w: 480, h: 270 } }, root);
      pre.p['position.x'].v = Math.round(240 + random() * (w - 480));
      pre.p['position.y'].v = Math.round(135 + random() * (h - 270));
      pre.p.opacity.v = 70 + Math.round(random() * 30);
      if (k === 1) animate(pre, 'rotation', [0, 30]);
      count('precomp');
      layers.push(pre);
      made++;
    }
  }

  // Nulls that parent a few layers each (parenting chains).
  const nulls: any[] = [];
  const nullCount = Math.max(1, Math.floor(total / 125));
  for (let i = 0; i < nullCount && made < total; i++) {
    const n = PM.mkLayer('null', { name: `Null ${i + 1}`, from: 0, dur }, root);
    n.p['position.x'].v = Math.round(random() * w);
    n.p['position.y'].v = Math.round(random() * h);
    animate(n, 'rotation', [0, pick([-20, 20, 45])]);
    count('null');
    nulls.push(n);
    layers.push(n);
    made++;
  }

  // Groups of 20 leaves; the rest at the root.
  let groups = 0;
  while (made < total) {
    const room = total - made;
    if (room >= 21 && random() < 0.5) {
      const group = PM.mkLayer('group', { name: `Group ${groups + 1}`, from: 0, dur }, root);
      group.p.opacity.v = 80 + Math.round(random() * 20);
      count('group');
      layers.push(group);
      made++;
      groups++;
      for (let i = 0; i < 20; i++) {
        const child = leaf(root, w, h, made);
        child.group = group.id;
        layers.push(child);
        made++;
      }
    } else {
      const layer = leaf(root, w, h, made);
      if (nulls.length && random() < 0.08) layer.parent = pick(nulls).id;
      layers.push(layer);
      made++;
    }
  }
  // The first layer in the list is on top (a group precedes its members,
  // as the editor orders them): the backdrop goes last.
  root.layers = layers.slice(1).concat([backdrop]);
  PM.touch?.();
  PM.bus?.emit?.('project');
  return {
    layers: made,
    byType,
    groups,
    precomps: byType['precomp'] ?? 0,
    compositions: compositions.length,
    keyframed,
    images: imageIds.length,
    fps,
    w,
    h
  };
}
