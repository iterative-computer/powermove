import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { mkdir, writeFile, stat, readFile } from 'node:fs/promises';
import { expect, repoRoot, test } from './helpers/app';

/** An original, editable Studio showcase. Opt in so ordinary test runs never
 * start a long GPU job. Outputs always live outside the repository.
 * PM_SHOWCASE=stills|benchmark|mp4|prores; PM_SHOWCASE_ONLY=regex filters stills.
 * PM_SHOWCASE_STILL_MBLUR=1 checks the same two shutter samples as the film.
 * Run with --global-timeout=7200000 for a full-resolution export. */
test('authors and exports the original Studio showcase', async ({ session }) => {
  test.skip(!process.env.PM_SHOWCASE, 'Opt-in showcase render');
  test.setTimeout(7_200_000);
  const folder = path.resolve(process.env.PM_SHOWCASE_OUTPUT || path.join(os.homedir(), 'Movies/powermove-studio-3d-film'));
  const mode = process.env.PM_SHOWCASE!;
  await mkdir(path.join(folder, 'stills'), { recursive: true });
  await session.openEditor();
  const { page } = session;
  page.on('console', message => { if (message.text().startsWith('[showcase]')) console.log(message.text()); });
  let shots: { name: string; from: number; dur: number; id: string; samples: number }[];
  if (process.env.PM_SHOWCASE_REUSE) {
    await session.app.evaluate(({ dialog }, file) => {
      dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [file] });
      dialog.showMessageBox = async () => ({ response: 1, checkboxChecked: false });
    }, process.env.PM_SHOWCASE_PROJECT || path.join(folder, 'powermove-studio-showcase.pmv'));
    await page.evaluate(() => (window as any).PM.openProject());
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const main = Object.values<any>(PM.proj.comps).find((c: any) => c.dur === 50 && c.layers.some((l: any) => l.name === '01 — Studio in metal'));
      if (main) PM.Comps.open(main.id, { tab: false });
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(17);
    shots = JSON.parse(await readFile(path.join(folder, 'shot-list.json'), 'utf8'));
  } else {
  shots = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause(); PM.AutomaticPreview?.cancel();
    PM.replaceProject(PM.mkProject({ name: 'Powermove Studio — A study in light', w: 1920, h: 1080, fps: 30, dur: 50, bg: '#000000' }));
    PM.proj.shutter = 0.5;
    PM.proj.exportDefaults = { format: 'mp4', scale: 1, fps: 30, range: 'all', quality: 'max', alpha: false, audio: false, mblur: true };
    const main = PM.Comps.active();
    const api = PM.Kernel.api('studio-showcase').scene3d;
    const edit = (a: any) => { const r = api.edit(a); if (!r.ok) throw new Error(`${a.operation} ${a.target || a.object?.id || ''}: ${r.message}`); return r; };
    const check = (r: any) => { if (!r.ok) throw new Error(r.message || r.error); return r; };
    const add = (type: string, o: any) => { const l = PM.mkLayer(type, o); PM.addLayer(l, 0); return l; };
    const text = (t: string, x: number, y: number, size: number, color: string, extra: any = {}) => add('text', {
      from: 0, dur: PM.proj.dur, p: { 'position.x': x, 'position.y': y },
      d: { text: t, font: 'Arial', size, color, align: 'center', weight: 400, ...extra },
    });
    const move = (l: any, key: string, a: number, b: number, dur = PM.proj.dur) => {
      if (!PM.animate(l, key, [{ t: 0, v: a }, { t: dur, v: b }], { ease: 'easeInOut' })) throw new Error(`Missing channel ${key}`);
    };
    const dataMove = (l: any, key: string, a: number, b: number, dur = PM.proj.dur) => {
      const prop = l.d.data.camera.p[key];
      PM.setKeysOn(prop, [{ t: 0, v: a, ease: 'easeInOut' }, { t: dur, v: b, ease: 'easeInOut' }], 30);
    };
    const build = (name: string, dur: number, bg: string, f: () => void, w = 1920, h = 1080) => {
      const id = PM.Comps.create({ name, dur, bg, w, h, fps: 30 }, { open: true });
      PM.proj.shutter = .5;
      f(); PM.touch();
      // This reproducible authored artifact needs its editable source, not
      // hundreds of megabytes of intermediate construction undo records.
      PM.hist.clear(); PM.Comps.open(main, { tab: false });
      return id;
    };
    const shots: { name: string; from: number; dur: number; id: string; samples: number }[] = [];
    const shot = (name: string, from: number, dur: number, f: () => void, bg = '#F4F3EF', samples = 64) => {
      console.log('[showcase] authoring', name);
      const id = build(name, dur, bg, () => {
        f();
        const cam = PM.proj.layers.find((l: any) => l.d?.data?.camera);
        if (cam && cam.d.data.environment.samples !== samples) edit({ operation: 'set_environment', patch: { samples } });
        // The main precomp samples the entire scene across the shutter. Its
        // children must not sample the shutter again (16 × 16 is redundant).
      });
      shots.push({ name, from, dur, id, samples });
    };
    const obj = (id: string, primitive: string, parameters: any, p: any = {}, material: any = {}) => {
      id = id.replace(/[^a-zA-Z0-9_-]/g, '_');
      edit({ operation: 'add_object', object: { id, name: id.replaceAll('_', ' '), source: { primitive, parameters }, p, material: { p: { color: '#B6B9BE', roughness: .45, metalness: 0, ...material } } } });
      return PM.L(id);
    };
    const box = (id: string, size: number[], pos: number[], color: string, radius = 0, extra: any = {}) =>
      obj(id, radius ? 'roundedBox' : 'box', { width: size[0], height: size[1], depth: size[2], ...(radius ? { radius, segments: 4 } : {}) }, { x: pos[0], y: pos[1], z: pos[2] }, { color, ...extra });
    const light = (id: string, type: string, p: any) => {
      edit({ operation: 'add_light', light: { id, name: id.replaceAll('_', ' '), type, p } }); return PM.L(id);
    };
    const camera = (pos: number[], aim: number[], fov = 38, fStop = 8) => {
      edit({ operation: 'add_camera', id: 'camera', camera: { p: { x: pos[0], y: pos[1], z: pos[2], targetX: aim[0], targetY: aim[1], targetZ: aim[2], fov, fStop, focusDistance: 0, irisShape: 6, irisRoundness: .7 } } });
      return PM.L('camera');
    };
    const aim = (pos: number[], target: number[], fov = 38, fStop = 8) => {
      edit({ operation: 'set_camera', patch: { p: { x: pos[0], y: pos[1], z: pos[2], targetX: target[0], targetY: target[1], targetZ: target[2], fov, fStop } } });
      return PM.proj.layers.find((l: any) => l.d?.data?.camera);
    };
    const superText = (copy: string, dark = true, eyebrow = '') => {
      const color = dark ? '#202225' : '#FFFFFF';
      if (eyebrow) text(eyebrow, 960, 130, 19, dark ? '#777A7C' : '#A9B1C0', { tracking: 4, weight: 600 });
      const l = text(copy, 960, 962, 40, color);
      PM.animate(l, 'opacity', [{ t: 0, v: 0 }, { t: .4, v: 100 }], { ease: 'cubicOut' });
      return l;
    };
    const world = (preset: string, p: any) => edit({ operation: 'set_world', id: 'world', world: { mode: 'sky', preset, p: { metersPerUnit: 1, ...p } } });
    const env = (p: any = {}, background: string | null = null) => edit({ operation: 'set_environment', patch: { background, p: { ambient: 0, exposure: 1, ...p } } });

    // Every pixel on the display comes from ordinary editable 2D layers.
    const screen = build('Signal — live kinetic typography', 50, '#091F26', () => {
      add('solid', { from: 0, dur: 50, p: { 'position.x': 720, 'position.y': 420 }, d: { color: '#091F26', w: 1440, h: 840 } });
      for (let i = 0; i < 9; i++) {
        const l = add('solid', { from: 0, dur: 50, p: { 'position.x': 120 + i * 154, 'position.y': 410 }, d: { color: i % 2 ? '#7EE6C0' : '#2F9B91', w: 45, h: 180 + (i % 4) * 70 } });
        PM.animate(l, 'scale.y', Array.from({ length: 26 }, (_, j) => ({ t: j * 2, v: 30 + 70 * (.5 + .5 * Math.sin(j * 1.1 + i)) })), { ease: 'easeInOut' });
        PM.animate(l, 'position.y', [{ t: 0, v: 480 }, { t: 25, v: 370 }, { t: 50, v: 480 }], { ease: 'easeInOut' });
      }
      text('MAKE', 720, 300, 174, '#F0F8F2', { weight: 700 });
      const word = text('WAVES.', 720, 490, 174, '#F0F8F2', { weight: 700 });
      PM.animate(word, 'position.x', [{ t: 0, v: 700 }, { t: 25, v: 740 }, { t: 50, v: 700 }], { ease: 'easeInOut' });
      text('A STUDY IN MOTION  /  01', 720, 745, 26, '#B6D4CC', { tracking: 3 });
    }, 1440, 840);

    // A deliberately simple original device, not an imported product model.
    const device = (finish = 'anodized', screenOn = true) => {
      box('device_shell', [2.9, 1.86, .34], [0, 1.08, 0], '#B4BCC4', .12);
      edit({ operation: 'apply_material', target: 'device_shell', preset: finish,
        p: finish === 'anodized' ? { color: '#768994', roughness: .28 } : finish === 'lacquer' ? { color: '#7B252E', roughness: .3, clearcoat: 1, clearcoatRoughness: .04 } : finish === 'brushed' ? { color: '#BFC2C8', anisotropy: .65, roughness: .32 } : {} });
      box('display_surround', [2.68, 1.64, .022], [0, 1.08, .179], '#0B1013', .075, { roughness: .33 });
      obj('display', 'plane', { width: 2.5, height: 1.458 }, { y: 1.08, z: .193 }, { color: '#091F26', roughness: .6, specular: 0, clearcoat: 1, clearcoatRoughness: .03, emissiveIntensity: .8 });
      if (screenOn) edit({ operation: 'update_object', target: 'display', patch: { material: { screen, p: { emissiveIntensity: .8 } } } });
      box('quiet_button', [.04, .22, .07], [1.456, 1.29, -.02], '#E1E4E7', .018, { metalness: 1, roughness: .16 });
      obj('status_light', 'sphere', { radius: .018, segments: 12 }, { x: 1.33, y: 1.08, z: .194 }, { color: '#B3E8D2', emissive: '#B3E8D2', emissiveIntensity: 1 });
      check(PM.Edit.apply({ type: 'group_layers', targets: ['device_shell', 'display_surround', 'display', 'quiet_button', 'status_light'], name: 'Study 01 — original device' }));
      return PM.proj.layers.find((l: any) => l.type === 'group');
    };
    const studioDevice = (finish = 'anodized') => {
      const g = device(finish);
      edit({ operation: 'auto_studio' });
      edit({ operation: 'set_environment', patch: { background: '#F4F3EF', p: { ambient: .18, exposure: 1 } } });
      return g;
    };

    shot('01 — Studio in metal', 0, 4, () => {
      camera([.15, .1, 8.4], [0, 0, 0], 40, 5.6);
      env({ ambient: .025, exposure: 1.1 }, '#000000');
      const title = text('Powermove Studio', 960, 585, 139, '#C8CCD2', { weight: 700 });
      check(api.setExtrusion(title.id, { enabled: true, depth: 34, bevelStyle: 'convex', bevelDepth: 3, quality: 'standard' }));
      for (const [s, roughness] of [['front', .3], ['bevel', .055], ['side', .22], ['back', .35]] as const)
        check(api.setSurfaceMaterialProperties(title.id, s, { color: '#D0D4DC', metallic: 1, roughness }));
      move(title, 'rotation.y', -9, 2);
      const key = light('sweeping_softbox', 'area', { x: -4, y: 2.8, z: 4, targetX: 0, targetY: 0, targetZ: 0, intensity: 40, width: 2, height: 4, color: '#EFF3FF' });
      move(key, 'position.x', -4, 4);
      light('silver_rim', 'area', { x: 2, y: -1, z: 2, targetX: 0, targetY: 0, targetZ: 0, intensity: 7, width: 5, height: .3, color: '#9BAFCE' });
      text('A STUDY IN LIGHT', 960, 758, 22, '#929CA9', { tracking: 5 });
    }, '#000000', 64);
    shot('02a — Device hero', 4, 3, () => {
      const g = studioDevice(); move(g, 'rotation.y', -20, 8);
      const c = aim([3.5, 2.2, 5.8], [0, 1.02, 0], 35, 8); move(c, 'position.x', 3.5, 2.8);
      superText('Path traced. Live.', true, 'STUDY 01');
    });
    shot('02b — Edge study', 7, 3, () => {
      const g = studioDevice(); move(g, 'rotation.y', 3, -8);
      const c = aim([-3.4, 1.7, 3.4], [-.45, 1.12, 0], 31, 5.6); move(c, 'position.z', 3.4, 3.9);
      superText('Path traced. Live.');
    });
    for (const [i, finish, label] of [[0, 'anodized', '01  /  ANODIZED ALUMINUM'], [1, 'frosted', '02  /  FROSTED GLASS'], [2, 'lacquer', '03  /  CAR PAINT'], [3, 'brushed', '04  /  BRUSHED METAL']] as const) {
      shot(`03${i} — ${label}`, 10 + i * 1.25, 1.25, () => {
        const g = studioDevice(finish); g.p['rotation.y'].v = 157;
        aim([-3.7, 2.3, 4.6], [-.12, 1.08, 0], 33, 8);
        superText('Materials that behave like the real thing.', true, label);
      }, '#F4F3EF', 96);
    }
    shot('04a — Any layer', 15, 3, () => {
      studioDevice();
      const c = aim([0, 1.08, 1.52], [0, 1.08, 0], 40, 8);
      move(c, 'position.z', 1.52, 4.5); move(c, 'position.y', 1.08, 1.8); move(c, 'position.x', 0, 1.2);
      superText('Any layer is a material.');
    });
    shot('04b — Live screen', 18, 3, () => {
      const g = studioDevice(); move(g, 'rotation.y', -10, 10);
      const c = aim([-2.6, 2.1, 5.5], [0, 1.08, 0], 36, 6.3); move(c, 'position.x', -2.6, -1.8);
      superText('Any layer is a material.', true, 'LIVE COMPOSITION → SURFACE');
    });

    shot('05 — Light that bends', 21, 5, () => {
      world('clear-noon', { metersPerUnit: .1, elevation: 42, azimuth: -55, intensity: .35, sunIntensity: 2, haze: 1.8 });
      env({ exposure: 1 });
      box('stone', [8, .3, 6], [0, -.15, 0], '#B4AA97', .04, { roughness: .85 });
      // Closed lathed glass with a thick base and a thin, open rim.
      edit({ operation: 'add_object', object: { id: 'glass', source: { lathe: [[0, 0], [.36, 0], [.395, .04], [.43, 1.13], [.425, 1.17], [.395, 1.17], [.365, .1], [0, .1]], segments: 64 }, p: { x: -.63 }, material: { p: { color: '#FFFFFF', roughness: .025, transmission: 1, ior: 1.5, metalness: 0 } } } });
      obj('water', 'cylinder', { radiusTop: .388, radiusBottom: .36, height: .77, segments: 64 }, { x: -.63, y: .49 }, { color: '#EFF9FB', roughness: .02, transmission: 1, ior: 1.333 });
      obj('crystal', 'icosahedron', { radius: .63, detail: 0 }, { x: .7, y: .48, z: .17, rx: 12, ry: 23, sx: .8, sy: 1.25 }, { color: '#F1FBFF', transmission: 1, roughness: .02, ior: 1.65, dispersion: .04 });
      const c = camera([2.45, 1.7, 3.8], [0, .48, 0], 33, 1.8);
      move(c, 'position.x', 2.45, 2.08); dataMove(c, 'focusDistance', 4.6, 3.8);
      superText('Light that bends.', true, 'GLASS / WATER / CRYSTAL');
    }, '#B4AA97', 128);

    const room = () => {
      world('golden-hour', { elevation: 22, azimuth: -70, intensity: .3, sunIntensity: 1.5, haze: 2.1, fogDensity: .002, fogDistance: 30, fogColor: '#E9DBBF' });
      env({ exposure: 1.15 });
      box('floor', [8, .16, 7], [0, -.08, 0], '#AE9473');
      box('ceiling', [8, .14, 7], [0, 3.6, 0], '#D6CEC0');
      box('back_wall', [8, 3.6, .16], [0, 1.8, -3.5], '#DAD2C3');
      box('front_wall', [8, 3.6, .16], [0, 1.8, 3.5], '#DAD2C3');
      box('right_wall', [.16, 3.6, 7], [4, 1.8, 0], '#DAD2C3');
      box('window_sill', [.18, .65, 7], [-4, .325, 0], '#DAD2C3');
      box('window_header', [.18, .55, 7], [-4, 3.325, 0], '#DAD2C3');
      for (const z of [-3.2, -.6, 2]) box(`window_pier_${z}`, [.22, 2.4, .23], [-4, 1.85, z], '#DAD2C3');
      box('sofa_seat', [2.6, .45, .95], [1.2, .48, -2.4], '#65766C', .14, { sheen: .4, roughness: .8 });
      box('sofa_back', [2.6, .8, .22], [1.2, .95, -2.77], '#65766C', .1, { sheen: .4, roughness: .8 });
      for (const x of [-.07, 2.47]) box(`arm_${x}`, [.19, .55, 1], [x, .74, -2.4], '#65766C', .07);
      box('low_table', [1.8, .12, .85], [.15, .63, -.3], '#563B2B', .08);
      for (const x of [-.53, .83]) for (const z of [-.58, -.02]) box(`leg_${x}_${z}`, [.09, .56, .09], [x, .28, z], '#563B2B');
      obj('vase', 'sphere', { radius: .22, segments: 32 }, { x: -.3, y: .93, z: -.32, sy: 1.4 }, { color: '#E2D6BF', roughness: .25, clearcoat: .6 });
      box('art_panel', [1.6, 1.15, .06], [.9, 2.3, -3.38], '#B96F49', .01);
      obj('art_disc', 'cylinder', { radius: .33, height: .035, segments: 48 }, { x: .88, y: 2.28, z: -3.32, rx: 90 }, { color: '#E5CDA0', roughness: .9 });
      edit({ operation: 'add_fog', id: 'window_haze', fog: { p: { width: 7.7, height: 3.5, depth: 6.7, density: .035, anisotropy: .55, color: '#F4E9D6' } } });
      edit({ operation: 'update_object', target: 'window_haze', patch: { p: { y: 1.75 } } });
    };
    shot('06a — Whole sets', 26, 3, () => {
      room(); const c = camera([3, 1.75, 2.5], [-1, 1.35, -1.2], 65, 8); move(c, 'position.x', 3, 2.5);
      superText('Whole sets.', false);
    }, '#DAD2C3', 64);
    shot('06b — Afternoon interior', 29, 3, () => {
      room(); const c = camera([2.9, 1.35, .9], [-.5, 1.05, -1.7], 53, 4); move(c, 'position.z', .9, .4);
      superText('Whole sets.', false);
    }, '#DAD2C3', 64);

    const street = () => {
      world('blue-hour', { elevation: -6, azimuth: -75, intensity: .45, sunIntensity: .05, haze: 3, fogDensity: .012, fogDistance: 110, fogHeightFalloff: .08, fogColor: '#6C7FAD', fogAnisotropy: .3 });
      env({ exposure: 1.4 });
      box('street', [8, .15, 65], [0, -.15, -19], '#202630', 0, { roughness: .28 });
      for (const side of [-1, 1]) {
        box(`pavement_${side}`, [3.8, .18, 65], [side * 5.8, -.04, -19], '#6A707B');
        for (let i = 0; i < 12; i++) {
          const z = 9 - i * 5, x = side * 3.7;
          obj(`post_${side}_${i}`, 'cylinder', { radius: .045, height: 3.5, segments: 8 }, { x, y: 1.75, z }, { color: '#272F3B', metalness: .7, roughness: .35 });
          obj(`lamp_${side}_${i}`, 'sphere', { radius: .16, segments: 12 }, { x, y: 3.55, z }, { color: '#FFDDB3', emissive: '#FFDDB3', emissiveIntensity: 5 });
          light(`practical_${side}_${i}`, 'point', { x, y: 3.48, z, color: '#FFD6A1', intensity: 13, distance: 9, decay: 2 });
          const h = 5 + (i % 4) * 1.4;
          box(`building_${side}_${i}`, [4.2, h, 4.6], [side * 8, h / 2, z], i % 2 ? '#525D70' : '#737C8B');
          for (let row = 0; row < 3; row++) for (let col = 0; col < 3; col++) {
            const wz = z - 1.3 + col * 1.3, wy = 1.4 + row * 1.35;
            box(`window_${side}_${i}_${row}_${col}`, [.025, .65, .62], [side * 5.886, wy, wz], '#B9A289', 0, { emissive: '#F7BC78', emissiveIntensity: (i + row + col) % 4 ? .7 : .05, roughness: .25 });
            // Each glowing window is also a real analytic emitter: 216 windows
            // plus 24 street lamps, all editable ordinary Studio light layers.
            light(`window_light_${side}_${i}_${row}_${col}`, 'point', { x: side * 5.7, y: wy, z: wz, color: '#FFC58A', intensity: .65, distance: 5, decay: 2 });
          }
        }
      }
      for (let i = 0; i < 16; i++) box(`lane_mark_${i}`, [.08, .007, 1.4], [0, -.068, 8 - i * 4], '#A9B0BB');
    };
    shot('07a — Hundreds of lights', 32, 3, () => {
      street(); const c = camera([.8, 1.9, 10], [0, 2, -15], 53, 4); move(c, 'position.y', 1.9, 3.1); move(c, 'position.z', 10, 7.5);
      superText('Hundreds of lights.', false);
    }, '#101B33', 64);
    shot('07b — Blue hour crane', 35, 3, () => {
      street(); const c = camera([-1.5, 4.7, 5], [0, 1.4, -13], 53, 4); move(c, 'position.y', 4.7, 6.4); move(c, 'position.z', 5, 2);
      superText('Hundreds of lights.', false, '240 PRACTICAL LIGHTS');
    }, '#101B33', 64);

    const landscape = () => {
      world('golden-hour', { elevation: 8, azimuth: -65, intensity: .7, sunIntensity: 1.4, haze: 2.7, fogDensity: .003, fogDistance: 300, fogHeightFalloff: .1, fogColor: '#E2C9A9', fogAnisotropy: .35 });
      env({ exposure: 1.05 });
      edit({ operation: 'add_terrain', id: 'highlands', terrain: { p: { width: 220, depth: 220, resolution: 96, amplitude: 18, frequency: .026, octaves: 5, seed: 71 }, regions: [{ p: { minSlope: 35, color: '#8B8272', roughness: .9 } }, { p: { minHeight: 7, color: '#9E9E76' } }, { p: { maxHeight: 7, color: '#6F7956' } }] } });
      obj('conifer', 'cone', { radius: .6, height: 2.8, segments: 7 }, { x: 300, y: 1.4 }, { color: '#354B35', roughness: .9 });
      PM.L('conifer').on = false;
      edit({ operation: 'add_scatter', id: 'forest', scatter: { model: 'conifer', surface: 'highlands', p: { count: 26000, density: 1, seed: 123, scaleMin: .65, scaleMax: 1.8, minSlope: 0, maxSlope: 40, minHeight: -30, maxHeight: 30 } } });
      obj('rock', 'icosahedron', { radius: .48, detail: 0 }, { x: 300, sy: .65 }, { color: '#9A9181', roughness: .95 });
      PM.L('rock').on = false;
      edit({ operation: 'add_scatter', id: 'stone_field', scatter: { model: 'rock', surface: 'highlands', p: { count: 4500, seed: 483, scaleMin: .5, scaleMax: 2.1, minSlope: 12, maxSlope: 60 } } });
    };
    shot('08a — Entire worlds', 38, 3.5, () => {
      landscape(); const c = camera([55, 30, 75], [-8, 2, -18], 54, 8); move(c, 'position.z', 75, 63); move(c, 'position.y', 30, 25);
      superText('Entire worlds.', false);
    }, '#DBB78D', 64);
    shot('08b — Low flight', 41.5, 3.5, () => {
      landscape(); const c = camera([23, 17, 40], [-15, 3, -35], 57, 8); move(c, 'position.z', 40, 24); move(c, 'position.x', 23, 17);
      superText('Entire worlds.', false, '30,500 SEEDED SCATTER CANDIDATES');
    }, '#DBB78D', 64);
    shot('09 — End card', 45, 5, () => {
      // The hardware claim is verified again by the native-backend assertions.
      text('Powermove Studio', 960, 531, 114, '#F3F4F6', { weight: 600 });
      text('Rendered on a MacBook Pro.', 960, 635, 31, '#9EA5AF');
      text('MADE ENTIRELY IN POWERMOVE', 960, 923, 18, '#727C89', { tracking: 4 });
    }, '#000000', 64);
    for (const s of shots) {
      const l = add('precomp', { from: s.from, dur: s.dur, d: { comp: s.id } });
      l.name = s.name; l.mblur = PM.P(true);
    }
    PM.touch(); PM.invalidate();
    return shots;
  });
  await writeFile(path.join(folder, 'shot-list.json'), JSON.stringify(shots, null, 2));
  }
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.Edit.mutate('Finish showcase camera and shutter settings', () => {
      for (const comp of Object.values<any>(PM.proj.comps)) {
        for (const l of comp.layers) l.mblur = false;
        for (const l of comp.layers) if (l.d?.extrusion) l.d.extrusion.p.quality.v = 'standard';
        const cam = comp.layers.find((l: any) => l.d?.data?.camera);
        if (/Device hero/.test(comp.name)) cam.p['position.y'].v = 3.65;
        if (/Edge study/.test(comp.name)) cam.p['position.y'].v = 3.1;
        if (/Edge study/.test(comp.name)) {
          cam.p['position.z'].v = 4.2;
          cam.p['position.z'].kf = [];
          cam.d.data.camera.p.fov.v = 35;
        }
        if (/03[0-3] —/.test(comp.name)) {
          cam.p['position.y'].v = 3.5;
          comp.layers.find((l: any) => l.type === 'group').p['rotation.y'].v = 157;
        }
        if (/04a —/.test(comp.name)) cam.p['position.y'].kf.at(-1).v = 3.4;
        if (/04a —/.test(comp.name)) for (const l of comp.layers) if (l.d?.text === 'Any layer is a material.') { l.from = 1.2; l.dur = 1.8; }
        if (/04b —/.test(comp.name)) cam.p['position.y'].v = 3.5;
        if (/FROSTED GLASS/.test(comp.name)) for (const l of comp.layers) if (['display', 'display_surround', 'status_light'].includes(l.id)) l.on = false;
        if (/Light that bends/.test(comp.name)) {
          cam.d.data.camera.p.fStop.v = 1.8;
          comp.layers.find((l: any) => l.d?.data?.object?.world).d.data.object.world.p.metersPerUnit.v = .1;
        }
        if (/Signal —/.test(comp.name)) for (const l of comp.layers) if (['MAKE','WAVES.'].includes(l.d?.text)) {
          l.d.animators = [{ id: `in_${l.id}`, name: 'Signal in', enabled: true, mode: 'stagger', unit: 'characters', order: 'forward', direction: 'in', easing: 'expoOut',
            p: Object.fromEntries(Object.entries({ amount:100, delay:l.d.text==='MAKE'?0:.1, duration:.7, stagger:.025, opacity:0, y:65 }).map(([k,v])=>[k,PM.P(v)])) }];
        }
        if (/09 —/.test(comp.name)) for (const l of comp.layers) {
          if(l.d?.text==='Powermove Studio')l.p['position.y'].v=440;
          if(l.d?.text==='Rendered on a MacBook Pro.')l.p['position.y'].v=600;
        }
        for(const l of comp.layers)if(l.d?.text==='30,500 SEEDED SCATTER CANDIDATES')l.d.text='LIGHT / LAND / ATMOSPHERE';
        if(cam)cam.d.data.environment.samples=/Light that bends/.test(comp.name)?64:16;
      }
      for (const l of PM.proj.layers) {
        l.mblur = PM.P(true);
        // A hard cut has no preceding exposure from this shot.
        PM.setKeysOn(l.mblur,[{t:0,v:false,hold:true},{t:1/30,v:true,hold:true}],30);
      }
      PM.touch();
    });
    PM.hist.clear();
  });
  // Pack repeated architecture into a few material meshes using Powermove's
  // own editable mesh source. Analytic lights remain individual light layers.
  await page.evaluate(() => {
    const PM = (window as any).PM, main = PM.Comps.active(), api = PM.Kernel.api('studio-showcase').scene3d;
    const vertex = (p: number[], n: number[], out: any) => { out.positions.push(...p); out.normals.push(...n); };
    const boxMesh = (w: number, h: number, d: number) => {
      const out = { positions: [] as number[], normals: [] as number[] };
      const faces = [
        [[1,0,0], [[1,-1,-1],[1,1,-1],[1,1,1],[1,-1,1]]],
        [[-1,0,0], [[-1,-1,1],[-1,1,1],[-1,1,-1],[-1,-1,-1]]],
        [[0,1,0], [[-1,1,-1],[-1,1,1],[1,1,1],[1,1,-1]]],
        [[0,-1,0], [[-1,-1,1],[-1,-1,-1],[1,-1,-1],[1,-1,1]]],
        [[0,0,1], [[-1,-1,1],[1,-1,1],[1,1,1],[-1,1,1]]],
        [[0,0,-1], [[1,-1,-1],[-1,-1,-1],[-1,1,-1],[1,1,-1]]],
      ];
      for (const [n, ps] of faces as any) for (const i of [0,1,2,0,2,3]) vertex([ps[i][0]*w/2,ps[i][1]*h/2,ps[i][2]*d/2],n,out);
      return out;
    };
    const radialMesh = (primitive: string, radius: number, height: number, segments: number) => {
      const out = { positions: [] as number[], normals: [] as number[] };
      const point = (u: number, v: number): number[] => primitive === 'sphere'
        ? [radius*Math.cos(u)*Math.sin(v),radius*Math.cos(v),radius*Math.sin(u)*Math.sin(v)]
        : [radius*Math.cos(u),v,radius*Math.sin(u)];
      const normal = (p: number[], cap = 0) => cap ? [0,cap,0] : primitive === 'sphere' ? p.map(v=>v/radius) : [p[0]/radius,0,p[2]/radius];
      const triangle = (a: number[], b: number[], c: number[], cap = 0) => { for(const p of [a,b,c]) vertex(p,normal(p,cap),out); };
      const rows = primitive === 'sphere' ? Math.max(8,Math.floor(segments/2)) : 1;
      for(let y=0;y<rows;y++) for(let i=0;i<segments;i++) {
        const u=i/segments*Math.PI*2, u1=(i+1)/segments*Math.PI*2;
        const v=primitive==='sphere'?y/rows*Math.PI:-height/2, v1=primitive==='sphere'?(y+1)/rows*Math.PI:height/2;
        const a=point(u,v),b=point(u1,v),c=point(u1,v1),d=point(u,v1);
        triangle(a,c,b);triangle(a,d,c);
        if(primitive==='cylinder'){triangle([0,height/2,0],point(u1,height/2),point(u,height/2),1);triangle([0,-height/2,0],point(u,-height/2),point(u1,-height/2),-1);}
      }
      return out;
    };
    for (const comp of Object.values<any>(PM.proj.comps).filter((c: any) => /07[ab] —|06[ab] —/.test(c.name))) {
      PM.Comps.open(comp.id, { tab: false });
      if (/07[ab] —/.test(comp.name)) {
        const w = PM.proj.layers.find((l: any) => l.d?.data?.object?.world)?.d.data.object.world;
        if(w?.p.elevation.v!==-3) {
          const r=api.edit({operation:'set_world',patch:{p:{elevation:-3,intensity:1.5,sunIntensity:0,fogColor:'#728BC0'}}});
          if(!r.ok)throw new Error(r.message);
        }
        if(!PM.L('blue_hour_fill')) {
          const r=api.edit({operation:'add_light',light:{id:'blue_hour_fill',type:'ambient',p:{color:'#6E8ECA',intensity:.18}}});
          if(!r.ok)throw new Error(r.message);
        }
      }
      const sources = PM.proj.layers.filter((l: any) => {
        const primitive = l.d?.data?.object?.source?.primitive;
        return primitive === 'box' || /07[ab] —/.test(comp.name) && ['sphere','cylinder'].includes(primitive);
      });
      if (!sources.length) { PM.Comps.open(main, { tab: false }); continue; }
      const groups = new Map<string, any>();
      for (const l of sources) {
        const o = l.d.data.object, key = JSON.stringify(o.material);
        let batch = groups.get(key);
        if (!batch) { batch = { material: o.material, positions: [], normals: [] }; groups.set(key, batch); }
        const p = o.source.parameters, mesh = o.source.primitive === 'box' ? boxMesh(p.width, p.height, p.depth)
          : radialMesh(o.source.primitive,p.radius || p.radiusTop,p.height || 1,p.segments || 12);
        const pos = ['x','y','z'].map(k => l.p[`position.${k}`].v);
        for (let i = 0; i < mesh.positions.length; i += 3) batch.positions.push(mesh.positions[i]+pos[0],mesh.positions[i+1]+pos[1],mesh.positions[i+2]+pos[2]);
        batch.normals.push(...mesh.normals);
      }
      const removed = PM.Edit.apply({ type: 'delete_layers', targets: sources.map((l: any) => l.id) });
      if (!removed.ok) throw new Error(removed.message);
      let index = 0;
      for (const g of groups.values()) {
        const r = api.edit({ operation: 'add_object', object: { id: `architecture_${index++}`, name: 'Architecture material mesh', source: { mesh: { positions: g.positions, normals: g.normals } }, material: g.material } });
        if (!r.ok) throw new Error(r.message);
      }
      PM.hist.clear(); PM.Comps.open(main, { tab: false });
    }
    PM.Edit.mutate('Keep supers above Studio sets',()=>{
      for(const c of Object.values<any>(PM.proj.comps)) {
        const front=c.layers.filter((l:any)=>l.type==='text'&&!l.threeD);
        c.layers=[...front,...c.layers.filter((l:any)=>!front.includes(l))];
      }
      PM.touch();
    });
    PM.hist.clear();
  });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.Edit.mutate('Balance the world-shot render budget', () => {
      for (const comp of Object.values<any>(PM.proj.comps)) if (/^0[78][ab] —/.test(comp.name)) {
        const camera = comp.layers.find((l: any) => l.d?.data?.camera);
        if (camera) camera.d.data.environment.samples = 8;
      }
      PM.touch();
    });
    PM.hist.clear();
  });
  shots = await page.evaluate(shots => {
    const PM = (window as any).PM;
    return shots.map(s => ({ ...s, samples: PM.proj.comps[s.id]?.layers.find((l: any) => l.d?.data?.camera)?.d.data.environment.samples || 0 }));
  }, shots);
  await writeFile(path.join(folder, 'shot-list.json'), JSON.stringify(shots, null, 2));
  const setDestination = (file: string) => session.app.evaluate(({ dialog }, file) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath: file });
  }, file);
  await setDestination(path.join(folder, 'powermove-studio-showcase.pmv'));
  expect(await page.evaluate(() => (window as any).PM.Export.run({ format: 'json', reportPath: true }))).toMatchObject({ cancelled: false });
  const timings: any[] = [];
  if (mode === 'stills') {
    for (const s of shots.filter(s => !process.env.PM_SHOWCASE_ONLY || new RegExp(process.env.PM_SHOWCASE_ONLY, 'i').test(s.name))) {
      for (const fraction of [.2, .72]) {
        const t = s.from + s.dur * fraction, started = Date.now();
        const f = await page.evaluate(async ({ t, mblur }) => {
          const PM = (window as any).PM; PM.agentFrameCapture = true;
          try {
            PM.Studio?.setDenoise(true); PM.Studio?.setPreview('fast');
            PM.setTime(t, { force: true });
            try { await PM.prepareFrame(t, { width: 960, height: 540, mblur, mbSamples: 2 }); }
            catch (error: any) { throw new Error(JSON.stringify(error.issues || { message:error.message,stack:error.stack })); }
            const c = PM.renderFrameTo(t, 960, 540, { mblur, mbSamples: 2 });
            const status = PM.Studio.status();
            const px = c.getContext('2d').getImageData(0, 0, 960, 540).data;
            let varied = 0;
            for (let i = 0; i < px.length; i += 4) if (Math.abs(px[i] - px[0]) + Math.abs(px[i + 1] - px[1]) + Math.abs(px[i + 2] - px[2]) > 24) varied++;
            return { png: c.toDataURL('image/png'), status, varied, project: { name: PM.proj.name, layers: PM.proj.layers.length, active: PM.proj.layers.filter((l: any) => PM.active(l, t)).map((l: any) => ({ name: l.name, type: l.type, sub: PM.compOf(l)?.name })) } };
          } finally { PM.agentFrameCapture = false; }
        }, { t, mblur: !!process.env.PM_SHOWCASE_STILL_MBLUR });
        const file = `${s.name.replace(/[^a-zA-Z0-9]+/g, '-')}-${fraction}.png`;
        await writeFile(path.join(folder, 'stills', file), Buffer.from(f.png.split(',')[1], 'base64'));
        await writeFile(path.join(folder, 'capture-diagnostics.json'), JSON.stringify(session.diagnostics, null, 2));
        console.log('CAPTURE', JSON.stringify({ status: f.status, project: f.project }));
        if (s.from < 45) expect(f.status.backend, JSON.stringify(f.status)).toBe('native-studio');
        expect(f.status.failure).toBeNull();
        expect(f.status.declined).toBe('');
        expect(f.varied, `Blank frame at ${t}`).toBeGreaterThan(1000);
        timings.push({ shot: s.name, t, seconds: (Date.now() - started) / 1000, ...f.status });
        console.log('STILL', s.name, t.toFixed(2), timings.at(-1).seconds);
        await writeFile(path.join(folder, 'still-timings.json'), JSON.stringify(timings, null, 2));
      }
    }
  } else {
    const format = mode === 'prores' ? 'prores' : 'mp4';
    const filename = mode === 'benchmark' ? 'benchmark.mp4' : format === 'prores' ? 'powermove-studio-showcase-master.mov' : 'powermove-studio-showcase.mp4';
    await setDestination(path.join(folder, filename));
    if (mode === 'benchmark') await page.evaluate(range => { const PM = (window as any).PM; PM.proj.work = range; }, JSON.parse(process.env.PM_SHOWCASE_BENCHMARK_RANGE || '[4,4.5]'));
    const started = Date.now();
    // Heartbeats make long unattended renders measurable without saving frames.
    const progress: any[] = [];
    const timer = setInterval(() => {
      void page.evaluate(() => ({ progress: document.querySelector('.export-progress-label')?.textContent, studio: (window as any).PM.Studio?.status() }))
        .then(p => { progress.push({ seconds: (Date.now() - started) / 1000, ...p }); console.log('EXPORT', JSON.stringify(progress.at(-1))); })
        .catch(() => undefined);
    }, 15_000);
    let result: any;
    try {
      result = await page.evaluate(async ({ format, benchmark }) => {
        const PM = (window as any).PM;
        PM.Studio?.setDenoise(true); PM.Studio?.setPreview('fast');
        const started = performance.now(), frameTimes: { frame: number; seconds: number }[] = [];
        const observer = new MutationObserver(() => {
          const label = document.querySelector('.export-progress-label');
          const frame = Number(label?.textContent?.match(/Frame (\d+) \/ /)?.[1]);
          if (frame && frame !== frameTimes.at(-1)?.frame) frameTimes.push({ frame, seconds: (performance.now() - started) / 1000 });
        });
        observer.observe(document.body, { childList: true, subtree: true, characterData: true });
        try {
          const result = await PM.Export.run({ format, scale: 1, fps: 30, range: benchmark ? 'work' : 'all', quality: 'max', audio: false, alpha: false, mblur: true, mbSamples: 2, reportPath: true });
          return { result, stats: PM.Export.lastStats, studio: PM.Studio?.status(), frameTimes };
        } finally { observer.disconnect(); }
      }, { format, benchmark: mode === 'benchmark' });
    } finally { clearInterval(timer); }
    await writeFile(path.join(folder, `${mode}-timings.json`), JSON.stringify({ seconds: (Date.now() - started) / 1000, ...result, progress }, null, 2));
    expect(result.result).toMatchObject({ cancelled: false });
    expect((await stat(path.join(folder, filename))).size).toBeGreaterThan(10000);
    expect(result.studio.backend).toBe('native-studio');
    expect(result.studio.failure).toBeNull();
    expect(result.studio.declined).toBe('');
    for (const p of progress) {
      expect(p.studio?.failure).toBeNull();
      expect(p.studio?.declined).toBe('');
      expect(p.studio?.backend, JSON.stringify(p)).toBe('native-studio');
    }
    if(mode==='prores' && process.env.PM_SHOWCASE_DELIVERY) {
      // Encode the app-rendered master once; this changes no authored pixels
      // and avoids tracing the same 1,500 frames twice on a shared Mac.
      const at=Date.now(), file=path.join(folder,'powermove-studio-showcase.mp4');
      execFileSync(path.join(repoRoot,'node_modules/ffmpeg-static/ffmpeg'),[
        '-hide_banner','-loglevel','error','-i',path.join(folder,filename),'-an',
        '-c:v','h264_videotoolbox','-allow_sw','0','-profile:v','high','-pix_fmt','yuv420p','-b:v','40M',
        '-bsf:v','h264_metadata=colour_primaries=1:transfer_characteristics=13:matrix_coefficients=1:video_full_range_flag=0',
        '-color_primaries','bt709','-color_trc','iec61966-2-1','-colorspace','bt709','-color_range','tv','-movflags','+faststart','-y',file,
      ],{stdio:['ignore','pipe','pipe'],maxBuffer:2*1024*1024});
      await writeFile(path.join(folder,'delivery-encode.json'),JSON.stringify({seconds:(Date.now()-at)/1000,source:filename,codec:'H.264',bytes:(await stat(file)).size},null,2));
    }
  }
  // Persist diagnostics only next to the film, never in source control.
  await writeFile(path.join(folder, `${mode}-diagnostics.json`), JSON.stringify(session.diagnostics, null, 2));
  expect(session.diagnostics.pageErrors).toEqual([]);
});
