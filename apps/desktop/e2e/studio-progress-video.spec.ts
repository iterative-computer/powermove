import os from 'node:os';
import path from 'node:path';
import { existsSync } from 'node:fs';
import { mkdir, readFile, stat, writeFile } from 'node:fs/promises';
import { expect, repoRoot, test } from './helpers/app';

/** Opus's Studio progress film: imported evidence footage and stills, 2D
 * layout and one native Final 3D title still, authored through Powermove's
 * own project APIs and exported with the ordinary exporter. Opt in only;
 * every input and output lives outside the repository.
 * PM_PROGRESS=stills|mp4; PM_PROGRESS_ONLY=regex filters stills;
 * PM_PROGRESS_SOURCES=folder holding the lane evidence (default /tmp/pm-shots/studio3d);
 * PM_PROGRESS_OUTPUT=folder (default ~/Movies/powermove-studio-3d-film/progress). */
test('authors and exports the Studio progress film', async ({ session }) => {
  test.skip(!process.env.PM_PROGRESS, 'Opt-in progress film render');
  test.setTimeout(3_600_000);
  const folder = path.resolve(process.env.PM_PROGRESS_OUTPUT || path.join(os.homedir(), 'Movies/powermove-studio-3d-film/progress'));
  const sources = path.resolve(process.env.PM_PROGRESS_SOURCES || '/tmp/pm-shots/studio3d');
  const mode = process.env.PM_PROGRESS!;
  await mkdir(path.join(folder, 'stills'), { recursive: true });
  const titlePng = path.join(folder, 'title-3d.png');
  const media: Record<string, [string, number, number]> = {
    glassBefore: ['laneDN/glass-before.mp4', 1920, 1080],
    glassAfter: ['laneDN/glass-after.mp4', 1920, 1080],
    landBefore: ['laneI/before/env-default-landscape-before-final128.png', 960, 540],
    landNow: ['laneI/ui/default-landscape-app-final.png', 960, 540],
    closeup: ['laneI/after/env-nature-closeup-final128.png', 960, 540],
    forest: ['laneI/after/env-landscape-100k-final128.png', 960, 540],
    world: ['laneJ/after-01-world-empty-comp.png', 2880, 1800],
    addMenu: ['laneJ/after-07-add-menu.png', 2880, 1800],
    materialOptions: ['laneD/after-10-material-options.png', 2880, 1800],
    fourViews: ['laneJ/after-09-four-views-landscape.png', 2880, 1800],
  };
  for (const [file] of Object.values(media)) expect(existsSync(path.join(sources, file)), file).toBe(true);
  await session.openEditor();
  const { page } = session;
  page.on('console', message => { if (message.text().startsWith('[progress]')) console.log(message.text()); });
  const timings: Record<string, number> = {};

  // 1. The 3D title: Inter, extruded and bevelled, polished metal on black.
  // v2: the key softbox sweeps left to right so its reflection travels
  // across the metal letters, rendered frame by frame by Studio at Preview quality and
  // encoded to a clip reused by the title and end card.
  const titleClip = path.join(folder, 'title-3d.mp4');
  const titleFrames = Number(process.env.PM_PROGRESS_TITLE_FRAMES || 120);
  const titleQuality = process.env.PM_PROGRESS_TITLE_QUALITY || 'preview';
  if (!existsSync(titleClip) || process.env.PM_PROGRESS_TITLE) {
    const started = Date.now();
    const frameDir = path.join(os.tmpdir(), 'pm-progress-title-frames');
    await mkdir(frameDir, { recursive: true });
    const inter = [...await readFile(path.join(repoRoot, 'assets/fonts/Inter-Variable.ttf'))];
    await page.evaluate(async ({ inter, quality, frames }) => {
      const PM = (window as any).PM;
      const bytes = new Uint8Array(inter).buffer;
      PM.fontSource = (name: string) => name === 'Inter' ? bytes : undefined;
      PM.pause(); PM.AutomaticPreview?.cancel();
      PM.replaceProject(PM.mkProject({ name: 'Studio title', w: 1920, h: 1080, fps: 30, dur: frames / 30, bg: '#000000' }));
      const api = PM.Kernel.api('studio-progress-video').scene3d;
      const edit = (a: any) => { const r = api.edit(a); if (!r.ok) throw new Error(`${a.operation}: ${r.message}`); return r; };
      const check = (r: any) => { if (!r.ok) throw new Error(r.message || r.error); return r; };
      const add = (type: string, o: any) => { const l = PM.mkLayer(type, o); PM.addLayer(l, 0); return l; };
      const light = (id: string, type: string, p: any) => edit({ operation: 'add_light', light: { id, name: id.replaceAll('_', ' '), type, p } });
      edit({ operation: 'add_camera', id: 'camera', camera: { p: { x: -.25, y: .45, z: 8.9, targetX: 0, targetY: -.55, targetZ: 0, fov: 40, fStop: 0, focusDistance: 0 } } });
      edit({ operation: 'set_environment', patch: { background: '#000000', p: { ambient: .02, exposure: 1.15 } } });
      const t = add('text', { from: 0, dur: frames / 30, p: { 'position.x': 960, 'position.y': 520 }, d: { text: 'Powermove Studio', font: 'Inter', size: 165, color: '#C8CCD2', align: 'center', weight: 700, tracking: -3 } });
      check(api.setExtrusion(t.id, { enabled: true, depth: 40, bevelStyle: 'convex', bevelDepth: 5, quality: 'standard' }));
      for (const [s, roughness] of [['front', .24], ['bevel', .05], ['side', .3], ['back', .4]] as const)
        check(api.setSurfaceMaterialProperties(t.id, s, { color: s === 'front' ? '#CDD2DA' : '#E4E7EC', metallic: 1, roughness }));
      light('front_key', 'area', { x: -2.2, y: 3.5, z: 9, targetX: 0, targetY: 0, targetZ: 0, intensity: 55, width: 2.4, height: 9, color: '#F2F5FF' });
      light('top_softbox', 'area', { x: 0, y: 7, z: 3, targetX: 0, targetY: 0, targetZ: 0, intensity: 14, width: 12, height: 3, color: '#FFFFFF' });
      light('rim_left', 'area', { x: -15, y: 1.5, z: -5, targetX: 0, targetY: 0, targetZ: 0, intensity: 18, width: .4, height: 5, color: '#B9C8E6' });
      light('rim_right', 'area', { x: 15, y: 1.5, z: -5, targetX: 0, targetY: 0, targetZ: 0, intensity: 18, width: .4, height: 5, color: '#E6D9C8' });
      const dur = frames / 30;
      check(api.renderQuality.set(quality));
      for (const l of PM.proj.layers) l.mblur = false;
      PM.touch(); PM.invalidate(); PM.hist.clear();
      PM.Studio?.setDenoise(true);
    }, { inter, quality: titleQuality, frames: titleFrames });
    for (let i = 0; i < titleFrames; i++) {
      const f0 = Date.now();
      const png = await page.evaluate(async ({ i, frames }) => {
        const PM = (window as any).PM;
        const t = i / 30, u = i / Math.max(1, frames - 1), e = u * u * (3 - 2 * u);
        const r = PM.Kernel.api('studio-progress-video').scene3d.edit({ operation: 'update_light', target: 'front_key', patch: { p: { x: -7 + 13 * e } } });
        if (!r.ok) throw new Error(r.message);
        PM.agentFrameCapture = true;
        try {
          PM.setTime(t, { force: true });
          await PM.prepareFrame(t, { width: 1920, height: 1080, mblur: false });
          return PM.renderFrameTo(t, 1920, 1080, { mblur: false }).toDataURL('image/png');
        } finally { PM.agentFrameCapture = false; }
      }, { i, frames: titleFrames });
      await writeFile(path.join(frameDir, `f${String(i).padStart(4, '0')}.png`), Buffer.from(png.split(',')[1], 'base64'));
      console.log('TITLE FRAME', i, (Date.now() - f0) / 1000);
    }
    const { execFileSync } = await import('node:child_process');
    execFileSync('ffmpeg', ['-y', '-loglevel', 'error', '-framerate', '30', '-i', path.join(frameDir, 'f%04d.png'), '-c:v', 'libx264', '-crf', '12', '-pix_fmt', 'yuv420p', titleClip]);
    execFileSync('rm', ['-rf', frameDir]);
    timings.title3dSeconds = (Date.now() - started) / 1000;
    console.log('TITLE', timings.title3dSeconds);
  }

  // 2. Import every source through the ordinary media importer.
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause(); PM.AutomaticPreview?.cancel();
    PM.replaceProject(PM.mkProject({ name: 'Powermove Studio — progress', w: 1920, h: 1080, fps: 30, dur: 67, bg: '#000000' }));
    const input = document.createElement('input'); input.type = 'file'; input.multiple = true; input.id = 'progress-sources'; document.body.appendChild(input);
  });
  await page.locator('#progress-sources').setInputFiles([...Object.values(media).map(([file]) => path.join(sources, file)), titleClip]);
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.importFiles([...((document.querySelector('#progress-sources') as HTMLInputElement).files!)], { placement: null });
  });

  const shots = await page.evaluate(({ media }) => {
    const PM = (window as any).PM;
    const DUR = 67;
    PM.proj.exportDefaults = { format: 'mp4', scale: 1, fps: 30, range: 'all', quality: 'max', alpha: false, audio: false, mblur: false };
    const main = PM.Comps.active();
    const assets: any[] = Object.values(PM.proj.assets);
    const asset = (key: string) => {
      const [file, w, h] = key === 'title' ? ['title-3d.mp4', 1920, 1080] : media[key];
      const name = file.split('/').pop();
      const a = assets.find(a => a.name === name);
      if (!a) throw new Error(`Missing imported ${name}: ${assets.map(a => a.name).join(', ')}`);
      return { id: a.id, w: a.w || w, h: a.h || h };
    };
    const add = (type: string, o: any) => { const l = PM.mkLayer(type, o); PM.addLayer(l, 0); return l; };
    const ORANGE = '#FF6B2C', WHITE = '#F5F5F7', GREY = '#8E8E93';
    const text = (t: string, x: number, y: number, size: number, color: string, extra: any = {}, from = 0, dur = PM.proj.dur) => add('text', {
      from, dur, p: { 'position.x': x, 'position.y': y },
      d: { text: t, font: 'Inter', size, color, align: 'center', weight: 600, ...extra },
    });
    // Times below are composition times; keyframes are stored layer-local.
    const keys = (l: any, key: string, k: [number, number][], ease = 'easeInOut') => {
      if (!PM.animate(l, key, k.map(([t, v]) => ({ t: t - (l.from || 0), v })), { ease })) throw new Error(`Missing channel ${key}`);
    };
    // Fade + small rise, the first film's super motion; optional fade out.
    const reveal = (l: any, at: number, out?: number, rise = 18) => {
      const y = l.p['position.y'].v;
      keys(l, 'opacity', out == null ? [[at, 0], [at + .45, 100]] : [[at, 0], [at + .45, 100], [out - .35, 100], [out, 0]], 'cubicOut');
      keys(l, 'position.y', [[at, y + rise], [at + .6, y]], 'cubicOut');
      return l;
    };
    const eyebrow = (copy: string, at = 0) => reveal(text(copy.toUpperCase(), 960, 86, 26, ORANGE, { weight: 600, tracking: 5 }), at);
    const headline = (copy: string, at = .15, out?: number, y = 150) => reveal(text(copy, 960, y, 64, WHITE, { weight: 600, tracking: -1 }), at, out);
    const image = (key: string, x: number, y: number, scale: number, from = 0, dur = PM.proj.dur) => {
      const a = asset(key);
      return add('image', { from, dur, p: { 'position.x': x, 'position.y': y, 'scale.x': scale, 'scale.y': scale }, d: { asset: a.id, w: a.w, h: a.h } });
    };
    const video = (key: string, x: number, y: number, scale: number, from = 0, speed = 1) => {
      const a = asset(key);
      return add('video', { from, dur: 4 / speed, p: { 'position.x': x, 'position.y': y, 'scale.x': scale, 'scale.y': scale }, d: { asset: a.id, trim: 0, speed, embeddedAudio: false, w: a.w, h: a.h } });
    };
    const scaleTo = (l: any, k: [number, number][], ease = 'easeInOut') => { keys(l, 'scale.x', k, ease); keys(l, 'scale.y', k, ease); };
    const fade = (l: any, k: [number, number][]) => keys(l, 'opacity', k, 'easeInOut');
    // Solids and precomps have a top-left origin and mkLayer resets their
    // position, so place them after creation. (x, y) is the centre here.
    const place = (l: any, x: number, y: number) => { l.p['position.x'].v = x - l.d.w / 2; l.p['position.y'].v = y - l.d.h / 2; return l; };
    const solid = (x: number, y: number, w: number, h: number, color: string, from = 0, dur = PM.proj.dur) =>
      place(add('solid', { from, dur, d: { color, w, h } }), x, y);
    const build = (name: string, dur: number, f: () => void, w = 1920, h = 1080) => {
      const id = PM.Comps.create({ name, dur, bg: '#000000', w, h, fps: 30 }, { open: true });
      f(); for (const l of PM.proj.layers) l.mblur = false;
      // Type always sits above pictures.
      const texts = PM.proj.layers.filter((l: any) => l.type === 'text');
      PM.proj.layers = [...texts, ...PM.proj.layers.filter((l: any) => !texts.includes(l))];
      PM.ProjectIndex?.invalidate?.(); PM.touch();
      PM.hist.clear(); PM.Comps.open(main, { tab: false });
      return id;
    };
    const shots: { name: string; from: number; dur: number; id: string }[] = [];
    const section = (name: string, from: number, dur: number, f: () => void) => {
      console.log('[progress] authoring', name);
      shots.push({ name, from, dur, id: build(name, dur, f) });
    };

    section('01 Title', 0, 4, () => {
      const t = video('title', 960, 540, 100); scaleTo(t, [[0, 100], [4, 104]]); fade(t, [[0, 0], [1, 100]]);
      reveal(text('Progress since the first film', 960, 690, 44, GREY, { weight: 500 }), 1.4);
    });

    // Glass: the same 4 s shot from the first film, before and after the
    // denoising work, side by side; then the same pair at 1:1 pixels.
    // The 4 s clip plays at 0.85× so it fills the 4.6 s close-up.
    const glassPanel = (key: string) => build(`Glass 1:1 — ${key}`, 4.7, () => {
      // Centre source pixel (1300, 600) of the 1920×1080 clip (prism and the
      // glass's caustic) in a 900×600 window.
      video(key, 450 - 340, 300 - 60, 100, 0, .85);
    }, 900, 600);
    const crop = { before: glassPanel('glassBefore'), after: glassPanel('glassAfter') };
    section('02 Glass denoised', 4, 9, () => {
      eyebrow('Denoising'); headline('The glass shot, denoised.');
      for (const [key, x] of [['glassBefore', 480], ['glassAfter', 1440]] as const) {
        const v = video(key, x, 555, 46.5, .2); fade(v, [[.2, 0], [.6, 100], [4, 100], [4.2, 0]]);
      }
      for (const [copy, x] of [['Before', 480], ['After', 1440]] as const) {
        const l = text(copy, x, 885, 36, copy === 'After' ? WHITE : GREY, { weight: 600 }, 0, 9); reveal(l, .5);
      }
      const stats = text('Colour error vs a high-sample reference  1.005 → 0.528      Similarity (SSIM)  0.978 → 0.991', 960, 975, 32, GREY, { weight: 500 }, 0, 4.2);
      reveal(stats, .9, 4.2);
      for (const [key, x] of [['before', 480], ['after', 1440]] as const) {
        const l = place(add('precomp', { from: 4.4, dur: 4.6, d: { comp: (crop as any)[key], w: 900, h: 600 } }), x, 545);
        l.name = `Glass 1:1 ${key}`; fade(l, [[4.4, 0], [4.8, 100]]);
      }
      reveal(text('Actual pixels. Before: blotchy caustic light under the glass. After: smooth, with sharper refraction.', 960, 975, 32, GREY, { weight: 500 }, 4.4, 4.6), 4.9);
    });

    section('03 Landscapes', 13, 14.5, () => {
      eyebrow('Landscapes'); headline('Places, not props.');
      const at = (key: string, from: number, dur: number, s0: number, s1: number, x0 = 960, x1 = 960) => {
        const l = image(key, x0, 615, s0, from, dur);
        scaleTo(l, [[from, s0], [from + dur, s1]]);
        if (x0 !== x1) keys(l, 'position.x', [[from, x0], [from + dur, x1]]);
        fade(l, [[from, 0], [from + .5, 100]]);
        return l;
      };
      const caption = (copy: string, from: number, dur: number) => reveal(text(copy, 960, 1045, 32, GREY, { weight: 500 }, from, dur), from + .3, from + dur);
      at('landBefore', 0, 4.3, 140, 144); caption('Add ▸ Terrain, before: faceted ground, no plants', 0, 4.3);
      at('landNow', 4, 4.3, 140, 145); caption('Add ▸ Terrain, now: eroded hills, trees, meadow, rocks, haze', 4, 4.3);
      at('closeup', 8, 3.3, 141, 146, 980, 940); caption('Branching trees, rocks and grass blades, all procedural', 8, 3.3);
      at('forest', 11, 3.5, 140, 145); caption('100,000 trees and 50,000 grass tufts in one scene', 11, 3.5);
    });

    // New tools: each screenshot starts whole, then eases into the feature.
    const tool = (key: string, copy: string, rx: number, ry: number, s1: number) => (from: number, dur: number) => {
      const s0 = 50, c = [1440, 900], target = [960, 600];
      const l = image(key, 960, 600, s0, from, dur);
      const end = [target[0] - (rx - c[0]) * s1 / 100, target[1] - (ry - c[1]) * s1 / 100];
      scaleTo(l, [[from + .3, s0], [from + dur, s1]]);
      keys(l, 'position.x', [[from + .3, 960], [from + dur, end[0]]]);
      keys(l, 'position.y', [[from + .3, 600], [from + dur, end[1]]]);
      fade(l, [[from, 0], [from + .35, 100]]);
      headline(copy, from + .1, from + dur, 140);
    };
    section('04 New tools', 27.5, 17.5, () => {
      eyebrow('New tools');
      // Black band keeps the headline clear of the screenshot as it grows.
      tool('world', 'World: sky presets and a sun you can drag.', 2420, 560, 120)(0, 3.5);
      tool('addMenu', 'New layers: Terrain, Scatter, Fog, Environment light.', 1585, 880, 140)(3.5, 3.5);
      tool('materialOptions', 'Material Options, in After Effects’ terms.', 2560, 620, 160)(7, 3.5);
      tool('fourViews', 'One, two or four views.', 1440, 600, 92)(10.5, 3.5);
      const band = solid(960, 100, 1920, 200, '#000000', 0, 14); band.name = 'Headline band';
      headline('Also new', 14.1, 17.5, 200);
      const lines = ['FBX import, with animation', 'glTF: KTX2 textures and UV transforms', '3D text in variable fonts such as Inter', 'Edits survive a 3D renderer crash'];
      lines.forEach((copy, i) => reveal(text(copy, 960, 360 + i * 110, 52, WHITE, { weight: 500 }, 14, 3.5), 14.35 + i * .18));
      // The band sits above the growing screenshots (type goes on top in build()).
      PM.proj.layers = [band, ...PM.proj.layers.filter((l: any) => l !== band)];
    });

    section('05 Cycles', 45, 7, () => {
      eyebrow('Checked against Blender Cycles'); headline('Closer to the reference.');
      const unit = 140, left = 560;
      const row = (label: string, y: number, before: number, now: number, at: number) => {
        reveal(text(label, 300, y, 44, WHITE, { weight: 600, align: 'right' }), at);
        for (const [value, dy, color, delay] of [[before, -34, '#3A3A3C', 0], [now, 34, ORANGE, .5]] as const) {
          const w = value * unit, bar = solid(left + w / 2, y + dy, w, 46, color);
          keys(bar, 'scale.x', [[at + delay, 0], [at + delay + .9, 100]], 'cubicOut');
          reveal(text(value.toFixed(2), left + w + 70, y + dy, 34, color === ORANGE ? ORANGE : GREY, { weight: 600 }), at + delay + .7, undefined, 0);
        }
      };
      row('Materials', 430, 7.04, 2.12, .6);
      row('Caustics', 680, 3.62, 1.98, 1.2);
      reveal(text('Before', 1790, 396, 30, GREY, { weight: 500, align: 'left' }), .9, undefined, 0);
      reveal(text('Now', 1790, 464, 30, ORANGE, { weight: 500, align: 'left' }), 1.4, undefined, 0);
      reveal(text('Mean colour difference (ΔE2000) from Cycles renders of the same scenes. Lower is closer.', 960, 900, 32, GREY, { weight: 500 }), 2);
      reveal(text('The other nine test scenes moved by less than 0.5. Wax-like subsurface still differs.', 960, 950, 32, GREY, { weight: 500 }), 2.2);
    });

    section('06 Tested', 52, 5, () => {
      eyebrow('Tested'); headline('Every suite green.');
      const stat = (n: string, label: string, x: number, at: number) => {
        reveal(text(n, x, 490, 140, WHITE, { weight: 700, tracking: -4 }), at);
        reveal(text(label, x, 660, 32, GREY, { weight: 500 }), at + .15);
      };
      stat('1,000', 'end-to-end tests passed, 0 failed', 400, .4);
      stat('7,486', 'unit tests passed', 960, .6);
      stat('115', 'Rust and GPU tests passed', 1520, .8);
      reveal(text('The packaged app also passed launch, crash recovery, save and reopen, and export checks.', 960, 900, 32, GREY, { weight: 500 }), 1.3);
    });

    section('07 Next', 57, 6, () => {
      eyebrow('Next'); headline('Speed.');
      const lines = [
        'Final export takes about 54 s per frame at the new quality default.',
        'Editing at a steady 60 fps is not there yet.',
        'Memory still grows over long sessions.',
      ];
      lines.forEach((copy, i) => reveal(text(copy, 960, 430 + i * 120, 48, WHITE, { weight: 500 }), .5 + i * .35));
      reveal(text('All three are being worked on now.', 960, 900, 32, GREY, { weight: 500 }), 1.8);
    });

    section('08 End card', 63, 4, () => {
      const t = video('title', 960, 540, 103); scaleTo(t, [[0, 103], [4, 100]]); fade(t, [[0, 0], [.8, 100], [3.4, 100], [4, 0]]);
      const line = text('Made in Powermove.', 960, 690, 40, GREY, { weight: 500 }); reveal(line, .8, 4);
    });

    for (const s of shots) {
      const l = add('precomp', { from: s.from, dur: s.dur, d: { comp: s.id } });
      l.name = s.name; l.mblur = false;
      // Dip through black between sections.
      const end = s.from + s.dur;
      keys(l, 'opacity', [[s.from, s.from ? 0 : 100], [s.from + .3, 100], [end - .3, 100], [end, end >= DUR ? 100 : 0]]);
    }
    PM.touch(); PM.invalidate(); PM.hist.clear();
    return shots;
  }, { media });

  await writeFile(path.join(folder, 'shot-list.json'), JSON.stringify(shots, null, 2));
  const setDestination = (file: string) => session.app.evaluate(({ dialog }, file) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath: file });
  }, file);
  await setDestination(path.join(folder, process.env.PM_PROGRESS_PMV || 'powermove-studio-progress.pmv'));
  expect(await page.evaluate(() => (window as any).PM.Export.run({ format: 'json', reportPath: true }))).toMatchObject({ cancelled: false });

  if (mode === 'stills') {
    const times = (process.env.PM_PROGRESS_TIMES || '').split(',').filter(Boolean).map(Number);
    const picks = times.length ? times.map(t => ({ name: `t${t.toFixed(2)}`, t }))
      : shots.filter(s => !process.env.PM_PROGRESS_ONLY || new RegExp(process.env.PM_PROGRESS_ONLY, 'i').test(s.name))
        .map(s => ({ name: s.name, t: s.from + s.dur * Number(process.env.PM_PROGRESS_FRACTION || .6) }));
    for (const s of picks) {
      const f = await page.evaluate(async ({ t }) => {
        const PM = (window as any).PM;
        PM.setTime(t, { force: true });
        await PM.prepareFrame(t, { width: 960, height: 540, mblur: false });
        return PM.renderFrameTo(t, 960, 540, { mblur: false }).toDataURL('image/png');
      }, { t: s.t });
      await writeFile(path.join(folder, 'stills', `${s.name.replace(/[^a-zA-Z0-9.]+/g, '-')}.png`), Buffer.from(f.split(',')[1], 'base64'));
      console.log('STILL', s.name, s.t.toFixed(2));
    }
  } else {
    const filename = process.env.PM_PROGRESS_FILENAME || 'powermove-studio-progress.mp4';
    await setDestination(path.join(folder, filename));
    const started = Date.now();
    const result = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const result = await PM.Export.run({ format: 'mp4', scale: 1, fps: 30, range: 'all', quality: 'max', audio: false, alpha: false, mblur: false, reportPath: true });
      return { result, stats: PM.Export.lastStats };
    });
    timings.exportSeconds = (Date.now() - started) / 1000;
    await writeFile(path.join(folder, 'mp4-timings.json'), JSON.stringify({ ...timings, ...result }, null, 2));
    expect(result.result).toMatchObject({ cancelled: false });
    expect((await stat(path.join(folder, filename))).size).toBeGreaterThan(10000);
  }
  await writeFile(path.join(folder, `${mode}-diagnostics.json`), JSON.stringify(session.diagnostics, null, 2));
});
