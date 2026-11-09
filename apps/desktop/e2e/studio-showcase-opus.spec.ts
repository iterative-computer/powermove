import os from 'node:os';
import path from 'node:path';
import { mkdir, writeFile, stat } from 'node:fs/promises';
import { expect, test } from './helpers/app';

/** Opus's Studio showcase: an original ~29 s product film authored entirely
 * through Powermove's own scene APIs. Opt in so ordinary runs never start a
 * GPU job; outputs live outside the repository.
 * PM_SHOWCASE=stills|mp4; PM_SHOWCASE_ONLY=regex filters stills;
 * PM_SHOWCASE_OUTPUT=folder (default ~/Movies/powermove-studio-3d-film/opus). */
test('authors and exports the Opus Studio showcase', async ({ session }) => {
  test.skip(!process.env.PM_SHOWCASE, 'Opt-in showcase render');
  test.setTimeout(3_600_000);
  const folder = path.resolve(process.env.PM_SHOWCASE_OUTPUT || path.join(os.homedir(), 'Movies/powermove-studio-3d-film/opus'));
  const mode = process.env.PM_SHOWCASE!;
  await mkdir(path.join(folder, 'stills'), { recursive: true });
  await session.openEditor();
  const { page } = session;
  page.on('console', message => { if (message.text().startsWith('[showcase]')) console.log(message.text()); });
  let shots: { name: string; from: number; dur: number; id: string; samples: number }[];
  shots = await page.evaluate(() => {
    const PM = (window as any).PM;
    const DUR = 29;
    PM.pause(); PM.AutomaticPreview?.cancel();
    PM.replaceProject(PM.mkProject({ name: 'Powermove Studio — Opus showcase', w: 1920, h: 1080, fps: 30, dur: DUR, bg: '#000000' }));
    PM.proj.shutter = 0.5;
    PM.proj.exportDefaults = { format: 'mp4', scale: 1, fps: 30, range: 'all', quality: 'max', alpha: false, audio: false, mblur: true };
    const main = PM.Comps.active();
    const api = PM.Kernel.api('studio-showcase-opus').scene3d;
    const edit = (a: any) => { const r = api.edit(a); if (!r.ok) throw new Error(`${a.operation} ${a.target || a.object?.id || ''}: ${r.message}`); return r; };
    const check = (r: any) => { if (!r.ok) throw new Error(r.message || r.error); return r; };
    const add = (type: string, o: any) => { const l = PM.mkLayer(type, o); PM.addLayer(l, 0); return l; };
    const text = (t: string, x: number, y: number, size: number, color: string, extra: any = {}) => add('text', {
      from: 0, dur: PM.proj.dur, p: { 'position.x': x, 'position.y': y },
      d: { text: t, font: 'Inter', size, color, align: 'center', weight: 600, ...extra },
    });
    const move = (l: any, key: string, a: number, b: number, dur = PM.proj.dur) => {
      if (!PM.animate(l, key, [{ t: 0, v: a }, { t: dur, v: b }], { ease: 'easeInOut' })) throw new Error(`Missing channel ${key}`);
    };
    const build = (name: string, dur: number, bg: string, f: () => void, w = 1920, h = 1080) => {
      const id = PM.Comps.create({ name, dur, bg, w, h, fps: 30 }, { open: true });
      PM.proj.shutter = .5;
      f(); PM.touch();
      PM.hist.clear(); PM.Comps.open(main, { tab: false });
      return id;
    };
    const shots: { name: string; from: number; dur: number; id: string; samples: number }[] = [];
    const shot = (name: string, from: number, dur: number, f: () => void, bg = '#F4F3EF', samples = 8) => {
      console.log('[showcase] authoring', name);
      const id = build(name, dur, bg, () => {
        f();
        const cam = PM.proj.layers.find((l: any) => l.d?.data?.camera);
        if (cam) edit({ operation: 'set_environment', patch: { samples } });
      });
      shots.push({ name, from, dur, id, samples });
    };
    const obj = (id: string, source: any, p: any = {}, material: any = {}) => {
      edit({ operation: 'add_object', object: { id, name: id.replaceAll('_', ' '), source, p, material: { p: { color: '#B6B9BE', roughness: .45, metalness: 0, ...material } } } });
      return PM.L(id);
    };
    const prim = (id: string, primitive: string, parameters: any, p: any = {}, material: any = {}) => obj(id, { primitive, parameters }, p, material);
    // Rounded rectangle outline for extruded, bevelled parts (true chamfered edges).
    const rrect = (w: number, h: number, r: number, n = 8) => {
      const pts: [number, number][] = [];
      const corners: [number, number, number][] = [[w / 2 - r, h / 2 - r, 0], [-w / 2 + r, h / 2 - r, 90], [-w / 2 + r, -h / 2 + r, 180], [w / 2 - r, -h / 2 + r, 270]];
      for (const [cx, cy, a0] of corners) for (let i = 0; i <= n; i++) {
        const a = (a0 + 90 * i / n) * Math.PI / 180; pts.push([+(cx + r * Math.cos(a)).toFixed(5), +(cy + r * Math.sin(a)).toFixed(5)]);
      }
      return pts;
    };
    const slab = (id: string, w: number, h: number, r: number, depth: number, bevel: number, p: any, material: any) =>
      obj(id, { extrude: rrect(w, h, r), depth, bevel }, p, material);
    const light = (id: string, type: string, p: any) => { edit({ operation: 'add_light', light: { id, name: id.replaceAll('_', ' '), type, p } }); return PM.L(id); };
    const camera = (pos: number[], aim: number[], fov = 38, fStop = 8) => {
      edit({ operation: 'add_camera', id: 'camera', camera: { p: { x: pos[0], y: pos[1], z: pos[2], targetX: aim[0], targetY: aim[1], targetZ: aim[2], fov, fStop, focusDistance: 0, irisShape: 6, irisRoundness: .7 } } });
      return PM.L('camera');
    };
    const aim = (pos: number[], target: number[], fov = 38, fStop = 8) => {
      edit({ operation: 'set_camera', patch: { p: { x: pos[0], y: pos[1], z: pos[2], targetX: target[0], targetY: target[1], targetZ: target[2], fov, fStop } } });
      return PM.proj.layers.find((l: any) => l.d?.data?.camera);
    };
    // Supers: big, confident, readable on a phone. Headline at the top of frame.
    const superText = (copy: string, dark = true, at = 0) => {
      const l = text(copy, 960, 112, 72, dark ? '#1D1D1F' : '#FFFFFF', { weight: 600, tracking: -1 });
      PM.animate(l, 'opacity', [{ t: at, v: 0 }, { t: at + .45, v: 100 }], { ease: 'cubicOut' });
      PM.animate(l, 'position.y', [{ t: at, v: 132 }, { t: at + .6, v: 112 }], { ease: 'cubicOut' });
      return l;
    };
    const world = (preset: string, p: any) => edit({ operation: 'set_world', id: 'world', world: { mode: 'sky', preset, p: { metersPerUnit: 1, ...p } } });
    const env = (p: any = {}, background: string | null = null) => edit({ operation: 'set_environment', patch: { background, p: { ambient: 0, exposure: 1, ...p } } });

    // Live screen content: ordinary 2D layers in their own composition.
    const screen = build('Display — live composition', DUR, '#0B0A1E', () => {
      add('solid', { from: 0, dur: DUR, p: { 'position.x': 720, 'position.y': 427 }, d: { color: '#0B0A1E', w: 1440, h: 855 } });
      const palette = ['#FF6B4A', '#FFB347', '#7C5CFF', '#38C6F4', '#2FE0A8'];
      const a = text('Light,', 720, 300, 190, '#FFFFFF', { weight: 800, tracking: -4 });
      const b = text('shaped.', 720, 490, 190, palette[0], { weight: 800, tracking: -4 });
      PM.animate(a, 'position.x', [{ t: 0, v: 660 }, { t: DUR, v: 780 }], { ease: 'easeInOut' });
      PM.animate(b, 'position.x', [{ t: 0, v: 780 }, { t: DUR, v: 660 }], { ease: 'easeInOut' });
      const c = text('A STUDY IN MOTION', 720, 690, 40, '#9A93D8', { weight: 600, tracking: 6 });
      PM.animate(c, 'opacity', Array.from({ length: 30 }, (_, j) => ({ t: j, v: j % 2 ? 100 : 55 })), { ease: 'easeInOut' });
    }, 1440, 855);

    // An original device with real design detail: chamfered extruded frame,
    // cover glass, matte glass back, a raised camera island with coated
    // lenses, a flash, buttons and a port.
    const FINISH: Record<string, { frame: any; back: any }> = {
      anodized: { frame: { color: '#3C4A5C', metalness: 1, roughness: .3 }, back: { color: '#4A5A6E', roughness: .42, clearcoat: .7, clearcoatRoughness: .05, metalness: 0 } },
      brushed: { frame: { color: '#C4C7CC', metalness: 1, roughness: .26, anisotropy: .75 }, back: { color: '#D4D6DA', roughness: .4, clearcoat: .6, clearcoatRoughness: .05, metalness: 0 } },
      lacquer: { frame: { color: '#9E1A2C', metalness: 0, roughness: .35, clearcoat: 1, clearcoatRoughness: .02 }, back: { color: '#8E1626', roughness: .3, clearcoat: 1, clearcoatRoughness: .02, metalness: 0 } },
      film: { frame: { color: '#B8BCC4', metalness: 1, roughness: .12, iridescence: 420 }, back: { color: '#2A2C33', roughness: .18, clearcoat: 1, clearcoatRoughness: .02, metalness: 0, iridescence: 520 } },
    };
    const device = (finish = 'anodized', screenOn = true) => {
      const f = FINISH[finish], cy = 1.08, parts: string[] = [];
      const part = (l: any) => { parts.push(l.id); return l; };
      part(slab('frame', 2.84, 1.8, .16, .24, .03, { y: cy, z: -.12 }, f.frame));
      part(slab('cover_glass', 2.78, 1.74, .13, .006, .004, { y: cy, z: .152 }, { color: '#06070A', roughness: .05, clearcoat: 1, clearcoatRoughness: .01, specular: 1 }));
      part(prim('display', 'plane', { width: 2.56, height: 1.52 }, { y: cy, z: .1635 }, { color: '#0B0A1E', roughness: .6, specular: 0, clearcoat: 1, clearcoatRoughness: .02, emissiveIntensity: .9 }));
      if (screenOn) edit({ operation: 'update_object', target: 'display', patch: { material: { screen, p: { emissiveIntensity: .9 } } } });
      part(slab('back_glass', 2.7, 1.66, .11, .006, .004, { y: cy, z: -.162 }, f.back));
      // Camera island, upper corner of the back (+x is upper-left seen from behind).
      const ix = .95, iy = cy + .5;
      part(slab('camera_island', .64, .64, .17, .03, .012, { x: ix, y: iy, z: -.2 }, f.frame));
      for (const [k, dx, dy] of [['a', -.145, .145], ['b', .145, -.145]] as const) {
        part(prim(`lens_barrel_${k}`, 'cylinder', { radius: .125, height: .05, segments: 48 }, { x: ix + dx, y: iy + dy, z: -.225, rx: 90 }, { color: '#0B0C0F', roughness: .12, metalness: .2, clearcoat: 1 }));
        part(prim(`lens_ring_${k}`, 'torus', { radius: .126, tube: .011, segments: 64 }, { x: ix + dx, y: iy + dy, z: -.25 }, { color: '#D8DADF', metalness: 1, roughness: .1 }));
        part(prim(`lens_glass_${k}`, 'sphere', { radius: .085, segments: 48 }, { x: ix + dx, y: iy + dy, z: -.245, sz: .35 }, { color: '#0E1218', roughness: .02, clearcoat: 1, clearcoatRoughness: .01, iridescence: 380 }));
      }
      part(prim('flash', 'cylinder', { radius: .05, height: .02, segments: 32 }, { x: ix + .145, y: iy + .145, z: -.214, rx: 90 }, { color: '#EFE8D6', roughness: .25, clearcoat: 1 }));
      part(prim('mic', 'cylinder', { radius: .016, height: .02, segments: 16 }, { x: ix - .145, y: iy - .145, z: -.214, rx: 90 }, { color: '#050506', roughness: .8 }));
      for (const [k, x, w] of [['power', -.95, .34], ['volume', -.45, .5]] as const)
        part(prim(`button_${k}`, 'roundedBox', { width: w, height: .05, depth: .085, radius: .02, segments: 4 }, { x, y: cy + .918, z: 0 }, { ...f.frame, roughness: .18 }));
      part(prim('port', 'roundedBox', { width: .03, height: .085, depth: .2, radius: .014, segments: 4 }, { x: 1.442, y: cy, z: 0 }, { color: '#08090B', roughness: .7 }));
      for (const z of [-.06, .06]) part(prim(`speaker_${z > 0 ? 'f' : 'b'}`, 'roundedBox', { width: .03, height: .26, depth: .035, radius: .015, segments: 4 }, { x: -1.442, y: cy, z }, { color: '#08090B', roughness: .7 }));
      check(PM.Edit.apply({ type: 'group_layers', targets: parts, name: 'Slate — original device' }));
      return PM.proj.layers.find((l: any) => l.type === 'group');
    };
    const studioDevice = (finish = 'anodized', screenOn = true) => {
      const g = device(finish, screenOn);
      edit({ operation: 'auto_studio' });
      edit({ operation: 'set_environment', patch: { background: '#F4F3EF', p: { ambient: .18, exposure: 1 } } });
      return g;
    };

    // Brushed-metal extruded wordmark on a dark reflective stage.
    const metalTitle = (sweep: boolean) => {
      env({ ambient: .02, exposure: 1.15 }, '#000000');
      const title = text('Powermove Studio', 960, 560, 165, '#C8CCD2', { font: 'Arial', weight: 700 });
      check(api.setExtrusion(title.id, { enabled: true, depth: 40, bevelStyle: 'convex', bevelDepth: 5, quality: 'standard' }));
      for (const [s, roughness] of [['front', .24], ['bevel', .05], ['side', .3], ['back', .4]] as const)
        check(api.setSurfaceMaterialProperties(title.id, s, { color: s === 'front' ? '#CDD2DA' : '#E4E7EC', metallic: 1, roughness }));
      prim('stage', 'box', { width: 40, height: .1, depth: 20 }, { y: -1.5, z: -2 }, { color: '#08090B', roughness: .12, metalness: 0, clearcoat: 1, clearcoatRoughness: .05 });
      const key = light('front_sweep', 'area', { x: -7, y: 3.5, z: 9, targetX: 0, targetY: 0, targetZ: 0, intensity: 55, width: 2.4, height: 9, color: '#F2F5FF' });
      if (sweep) move(key, 'position.x', -7, 7);
      light('top_softbox', 'area', { x: 0, y: 7, z: 3, targetX: 0, targetY: 0, targetZ: 0, intensity: 14, width: 12, height: 3, color: '#FFFFFF' });
      light('rim_left', 'area', { x: -15, y: 1.5, z: -5, targetX: 0, targetY: 0, targetZ: 0, intensity: 18, width: .4, height: 5, color: '#B9C8E6' });
      light('rim_right', 'area', { x: 15, y: 1.5, z: -5, targetX: 0, targetY: 0, targetZ: 0, intensity: 18, width: .4, height: 5, color: '#E6D9C8' });
      return title;
    };

    shot('01 — Title', 0, 4, () => {
      const c = camera([2.6, .9, 7.4], [0, -.35, 0], 40, 4);
      metalTitle(true);
      move(c, 'position.x', 2.6, -.4); move(c, 'position.z', 7.4, 8.6); move(c, 'position.y', .9, .4);
    }, '#000000', 8);
    shot('02 — Slate', 4, 4, () => {
      const g = studioDevice(); move(g, 'rotation.y', 118, 172);
      const c = aim([-3.5, 3.6, 5.6], [-.05, 1.3, 0], 31, 5.6); move(c, 'position.z', 5.6, 5.0);
      superText('Every surface, path traced.', true, .2);
    });
    const labels: [string, string][] = [['anodized', 'Anodized.'], ['brushed', 'Brushed.'], ['lacquer', 'Lacquer.'], ['film', 'Thin film.']];
    labels.forEach(([finish, label], i) => {
      shot(`03${i} — ${label}`, 8 + i, 1, () => {
        const g = studioDevice(finish, false); move(g, 'rotation.y', 150 + i * 3, 156 + i * 3);
        aim([-3.6, 3.3, 5.0], [-.1, 1.32, 0], 31, 8);
        superText(label, true, 0);
      }, '#F4F3EF', 8);
    });
    shot('04 — Live screen', 12, 4, () => {
      const g = studioDevice(); move(g, 'rotation.y', 0, -14);
      const c = aim([0, 1.08, 1.45], [0, 1.08, 0], 40, 8);
      PM.setKeysOn(c.d.data.camera.p.targetY, [{ t: 0, v: 1.08, ease: 'easeInOut' }, { t: .5, v: 1.08, ease: 'easeInOut' }, { t: 3.2, v: 1.4, ease: 'easeInOut' }, { t: 4, v: 1.42, ease: 'easeInOut' }], 30);
      PM.animate(c, 'position.z', [{ t: 0, v: 1.45 }, { t: .5, v: 1.6 }, { t: 3.2, v: 5.3 }, { t: 4, v: 5.5 }], { ease: 'easeInOut' });
      PM.animate(c, 'position.y', [{ t: 0, v: 1.08 }, { t: .5, v: 1.1 }, { t: 3.2, v: 3.8 }, { t: 4, v: 3.9 }], { ease: 'easeInOut' });
      PM.animate(c, 'position.x', [{ t: 0, v: 0 }, { t: .5, v: 0 }, { t: 3.2, v: 1.9 }, { t: 4, v: 2 }], { ease: 'easeInOut' });
      superText('Any layer is a material.', true, 1.6);
    });
    shot('05 — Light that bends', 16, 4, () => {
      world('clear-noon', { metersPerUnit: .1, elevation: 30, azimuth: -55, intensity: .35, sunIntensity: 2, haze: 1.8 });
      env({ exposure: 1 });
      prim('stone', 'roundedBox', { width: 8, height: .3, depth: 6, radius: .04, segments: 4 }, { y: -.15 }, { color: '#B4AA97', roughness: .85 });
      edit({ operation: 'add_object', object: { id: 'glass', source: { lathe: [[0, 0], [.36, 0], [.395, .04], [.43, 1.13], [.425, 1.17], [.395, 1.17], [.365, .1], [0, .1]], segments: 64 }, p: { x: -.63 }, material: { p: { color: '#FFFFFF', roughness: .025, transmission: 1, ior: 1.5, metalness: 0 } } } });
      prim('water', 'cylinder', { radiusTop: .388, radiusBottom: .36, height: .77, segments: 64 }, { x: -.63, y: .49 }, { color: '#EFF9FB', roughness: .02, transmission: 1, ior: 1.333 });
      // A triangular prism splits the sun into a spectrum on the stone.
      obj('prism', { extrude: [[-.36, 0], [.36, 0], [0, .62]], depth: .9, bevel: .012 }, { x: .75, y: .012, z: -.2, ry: -28 }, { color: '#F4FCFF', transmission: 1, roughness: .02, ior: 1.6, dispersion: .06 });
      const c = camera([2.45, 1.5, 3.8], [0, .42, 0], 33, 2.2);
      move(c, 'position.x', 2.45, 1.9);
      superText('Light that bends.', true, .3);
    }, '#B4AA97', 16);
    shot('06 — Whole worlds', 20, 5, () => {
      world('golden-hour', { elevation: 8, azimuth: -65, intensity: .7, sunIntensity: 1.4, haze: 2.7, fogDensity: .003, fogDistance: 300, fogHeightFalloff: .1, fogColor: '#E2C9A9', fogAnisotropy: .35 });
      env({ exposure: 1.05 });
      edit({ operation: 'add_terrain', id: 'highlands', terrain: { p: { width: 220, depth: 220, resolution: 96, amplitude: 18, frequency: .026, octaves: 5, seed: 71 }, regions: [{ p: { minSlope: 35, color: '#8B8272', roughness: .9 } }, { p: { minHeight: 7, color: '#9E9E76' } }, { p: { maxHeight: 7, color: '#6F7956' } }] } });
      prim('conifer', 'cone', { radius: .6, height: 2.8, segments: 7 }, { x: 300, y: 1.4 }, { color: '#354B35', roughness: .9 });
      PM.L('conifer').on = false;
      edit({ operation: 'add_scatter', id: 'forest', scatter: { model: 'conifer', surface: 'highlands', p: { count: 40000, density: 1, seed: 123, scaleMin: .65, scaleMax: 1.8, minSlope: 0, maxSlope: 40, minHeight: -30, maxHeight: 30 } } });
      prim('rock', 'icosahedron', { radius: .48, detail: 0 }, { x: 300, sy: .65 }, { color: '#9A9181', roughness: .95 });
      PM.L('rock').on = false;
      edit({ operation: 'add_scatter', id: 'stone_field', scatter: { model: 'rock', surface: 'highlands', p: { count: 4500, seed: 483, scaleMin: .5, scaleMax: 2.1, minSlope: 12, maxSlope: 60 } } });
      const c = camera([23, 17, 40], [-15, 3, -35], 57, 8); move(c, 'position.z', 40, 22); move(c, 'position.x', 23, 16);
      superText('Whole worlds.', false, .3);
    }, '#DBB78D', 8);
    shot('07 — End card', 25, 4, () => {
      camera([0, .35, 9.6], [0, -.75, 0], 40, 0);
      metalTitle(false);
      PM.L('front_sweep').p['position.x'].v = -2.5;
      const line = text('Rendered in Powermove on a MacBook Pro.', 960, 930, 40, '#A7ACB4', { weight: 500 });
      PM.animate(line, 'opacity', [{ t: .4, v: 0 }, { t: 1, v: 100 }], { ease: 'cubicOut' });
    }, '#000000', 8);
    for (const s of shots) {
      const l = add('precomp', { from: s.from, dur: s.dur, d: { comp: s.id } });
      l.name = s.name; l.mblur = PM.P(true);
    }
    PM.touch(); PM.invalidate();
    return shots;
  });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.Edit.mutate('Finish showcase shutter and layer order', () => {
      for (const c of Object.values<any>(PM.proj.comps)) {
        for (const l of c.layers) l.mblur = false;
        // Supers stay above the 3D set.
        const front = c.layers.filter((l: any) => l.type === 'text' && !l.threeD);
        c.layers = [...front, ...c.layers.filter((l: any) => !front.includes(l))];
      }
      for (const l of PM.proj.layers) {
        l.mblur = PM.P(true);
        // A hard cut has no preceding exposure from this shot.
        PM.setKeysOn(l.mblur, [{ t: 0, v: false, hold: true }, { t: 1 / 30, v: true, hold: true }], 30);
      }
      PM.touch();
    });
    PM.hist.clear();
  });
  await writeFile(path.join(folder, 'shot-list.json'), JSON.stringify(shots, null, 2));
  const setDestination = (file: string) => session.app.evaluate(({ dialog }, file) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath: file });
  }, file);
  await setDestination(path.join(folder, 'powermove-studio-showcase.pmv'));
  expect(await page.evaluate(() => (window as any).PM.Export.run({ format: 'json', reportPath: true }))).toMatchObject({ cancelled: false });
  const timings: any[] = [];
  if (mode === 'stills') {
    const fraction = Number(process.env.PM_SHOWCASE_FRACTION || .6);
    for (const s of shots.filter(s => !process.env.PM_SHOWCASE_ONLY || new RegExp(process.env.PM_SHOWCASE_ONLY, 'i').test(s.name))) {
      const t = s.from + s.dur * fraction, started = Date.now();
      const f = await page.evaluate(async ({ t }) => {
        const PM = (window as any).PM; PM.agentFrameCapture = true;
        try {
          PM.Studio?.setDenoise(true); PM.Studio?.setPreview('fast');
          PM.setTime(t, { force: true });
          await PM.prepareFrame(t, { width: 960, height: 540, mblur: false });
          const c = PM.renderFrameTo(t, 960, 540, { mblur: false });
          return { png: c.toDataURL('image/png'), status: PM.Studio.status() };
        } finally { PM.agentFrameCapture = false; }
      }, { t });
      await writeFile(path.join(folder, 'stills', `${s.name.replace(/[^a-zA-Z0-9]+/g, '-')}.png`), Buffer.from(f.png.split(',')[1], 'base64'));
      timings.push({ shot: s.name, t, seconds: (Date.now() - started) / 1000, ...f.status });
      console.log('STILL', s.name, t.toFixed(2), timings.at(-1).seconds, f.status.backend);
    }
    await writeFile(path.join(folder, 'still-timings.json'), JSON.stringify(timings, null, 2));
  } else {
    const filename = 'powermove-studio-showcase.mp4';
    await setDestination(path.join(folder, filename));
    const started = Date.now();
    const progress: any[] = [];
    const timer = setInterval(() => {
      void page.evaluate(() => ({ progress: document.querySelector('.export-progress-label')?.textContent, studio: (window as any).PM.Studio?.status() }))
        .then(p => { progress.push({ seconds: (Date.now() - started) / 1000, ...p }); console.log('EXPORT', JSON.stringify(progress.at(-1))); })
        .catch(() => undefined);
    }, 15_000);
    let result: any;
    try {
      result = await page.evaluate(async () => {
        const PM = (window as any).PM;
        PM.Studio?.setDenoise(true); PM.Studio?.setPreview('fast');
        const result = await PM.Export.run({ format: 'mp4', scale: 1, fps: 30, range: 'all', quality: 'max', audio: false, alpha: false, mblur: true, mbSamples: 2, reportPath: true });
        return { result, stats: PM.Export.lastStats, studio: PM.Studio?.status() };
      });
    } finally { clearInterval(timer); }
    await writeFile(path.join(folder, 'mp4-timings.json'), JSON.stringify({ seconds: (Date.now() - started) / 1000, ...result, progress }, null, 2));
    expect(result.result).toMatchObject({ cancelled: false });
    expect((await stat(path.join(folder, filename))).size).toBeGreaterThan(10000);
    expect(result.studio.backend).toBe('native-studio');
    expect(result.studio.failure).toBeNull();
  }
  await writeFile(path.join(folder, `${mode}-diagnostics.json`), JSON.stringify(session.diagnostics, null, 2));
});
