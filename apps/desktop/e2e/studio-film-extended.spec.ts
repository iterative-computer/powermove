import path from 'node:path';
import { copyFileSync, statSync } from 'node:fs';
import { expect, test } from './helpers/app';

/* The extended acceptance film: an original product film for an original
   tablet ("Slate"), covering every shot type of the reference analysis
   (docs/3d-studio-redesign.md, 2a) and made and exported entirely in
   Powermove. Each 3D shot is its own composition; the main composition cuts
   between them and the kinetic-type and graphic cards.

     0.0  Thin.                kinetic type, blur in by word
     2.0  Edge-on reveal       the sliver turns to show its face
     4.5  Light. Bright.       kinetic type, rise by word, two colors
     6.0  Flat lay             hero tilt
     8.5  Pull back            from the full-frame screen to the device
    11.0  Macro                rack focus across the display corner
    13.5  Flip                 back (camera module) to front
    16.0  Stylus               slides in beside a line of type
    18.5  Wet                  droplets on the display glass
    20.5  Glow                 a graphic orb, blurred and glowing
    22.0  End                  turntable on the set, wordmark, fine print */
test('an extended original product film covers every shot type', async ({ session }, info) => {
  test.setTimeout(1_800_000);
  await session.openEditor();
  const { page } = session;
  const output = path.join(session.userData, 'slate-film-extended.mp4');
  await session.app.evaluate(({ dialog }, file) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath: file }); }, output);

  const built = await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-film-extended');
    const edit = (a: any) => { const r = api.scene3d.edit(a); if (!r.ok) throw new Error(`${a.operation} ${a.target ?? a.object?.id ?? ''}: ${r.message}`); return r; };
    const W = 1280, H = 720, FPS = 30;
    PM.proj.w = W; PM.proj.h = H; PM.proj.fps = FPS; PM.proj.dur = 25; PM.proj.bg = '#F6F5F8';
    PM.proj.backgroundFill = PM.normalizeFill({ type: 'solid', color: '#F6F5F8' }, '#F6F5F8');
    const mainId = PM.Comps.active();
    // Each composition is opened to build it (edits go to the open one).
    const build = (settings: any, f: () => void): string => {
      const id = PM.Comps.create({ w: W, h: H, fps: FPS, bg: '#F6F5F8', ...settings }, { open: true });
      f();
      PM.Comps.open(mainId, { tab: false });
      return id;
    };
    const add = (type: string, opts: any) => { const l = PM.mkLayer(type, opts); PM.addLayer(l, 0); return l; };
    const effect = (l: any, id: string, parameters: Record<string, number> = {}) => {
      const r = PM.Edit.apply({ type: 'add_effect', target: l.id, effect: id, parameters }, { origin: 'agent' });
      if (!r.ok) throw new Error(`${id}: ${r.message ?? r.error}`);
    };
    const text = (t: string, from: number, dur: number, x: number, y: number, size: number, color: string, extra: any = {}) =>
      add('text', { from, dur, p: { 'position.x': x, 'position.y': y }, d: { text: t, size, color, align: 'center', weight: 600, ...extra } });
    // Text animators (as the inspector's presets build them): by word, in.
    const animate = (l: any, props: Record<string, number>, duration = 0.9, stagger = 0.12, easing = 'expoOut') => {
      l.d.animators = [{ id: `a${l.id}`, name: 'In', enabled: true, mode: 'stagger', unit: 'words', order: 'forward', direction: 'in', easing,
        p: Object.fromEntries(Object.entries({ amount: 100, delay: 0, duration, stagger, ...props }).map(([k, v]) => [k, PM.P(v)])) }];
    };

    // The screen: an original wallpaper of drifting colour fields and a clock.
    const screenId = build({ name: 'Slate screen', w: 1024, h: 1400, dur: 25, bg: '#07070D' }, () => {
      const fields: [string, number, number, number][] = [['#4B5BFF', 200, 300, 1], ['#9A4BFF', 760, 900, 1.6], ['#2BB3FF', 420, 1200, 2.2], ['#FF5FA2', 820, 260, 0.6]];
      for (const [color, x, y, phase] of fields) {
        const l = add('solid', { from: 0, dur: 25, d: { color, w: 560, h: 560 } });
        PM.animate(l, 'position.x', [{ t: 0, v: x }, { t: 25, v: 1024 - x }]);
        PM.animate(l, 'position.y', [{ t: 0, v: y }, { t: 25, v: y + 300 * Math.sin(phase) }]);
        PM.animate(l, 'opacity', [{ t: 0, v: 50 }, { t: 12, v: 85 }, { t: 25, v: 50 }]);
        // Soft colour fields, not squares.
        effect(l, 'blur', { amount: 140 });
      }
      text('10:08', 0, 25, 512, 300, 190, '#FFFFFF', { weight: 300 });
    });

    /** The product in the open composition: body, display, camera module; grouped. */
    const slate = (p: string, wet = false) => {
      edit({ operation: 'add_object', object: { id: `${p}_body`, name: 'Slate body', source: { primitive: 'roundedBox', parameters: { width: 1.35, height: 0.07, depth: 1.95, radius: 0.03 } } } });
      edit({ operation: 'apply_material', target: `${p}_body`, preset: 'anodized', p: { color: '#5B5E66' } });
      edit({ operation: 'add_object', object: { id: `${p}_display`, name: 'Slate display', source: { primitive: 'plane', parameters: { width: 1.22, height: 1.82 } }, p: { y: 0.0355, rx: -90 },
        material: { screen: screenId, p: { color: '#050507', roughness: 0.6, clearcoat: 1, clearcoatRoughness: 0.012, specular: 0, emissiveIntensity: 1.3, ...(wet ? { droplets: 0.6 } : {}) } } } });
      edit({ operation: 'add_object', object: { id: `${p}_ring`, name: 'Camera ring', source: { primitive: 'cylinder', parameters: { radiusTop: 0.07, radiusBottom: 0.07, height: 0.02, segments: 64 } }, p: { x: 0.48, y: -0.043, z: -0.78 } } });
      edit({ operation: 'apply_material', target: `${p}_ring`, preset: 'polished' });
      edit({ operation: 'add_object', object: { id: `${p}_lens`, name: 'Lens', source: { primitive: 'cylinder', parameters: { radiusTop: 0.045, radiusBottom: 0.045, height: 0.022, segments: 64 } }, p: { x: 0.48, y: -0.044, z: -0.78 },
        material: { p: { color: '#050507', roughness: 0.2, clearcoat: 1, clearcoatRoughness: 0.01 } } } });
      PM.Edit.apply({ type: 'group_layers', targets: [`${p}_body`, `${p}_display`, `${p}_ring`, `${p}_lens`], name: 'Slate' });
      return PM.proj.layers.find((l: any) => l.type === 'group');
    };
    const camera = () => PM.proj.layers.find((l: any) => l.d?.definition === 'powermove.3d.camera');
    const aim = (pos: number[], target: number[], extra: Record<string, number> = {}) => {
      const cam = camera();
      ['x', 'y', 'z'].forEach((k, i) => { cam.p[`position.${k}`].v = pos[i]; });
      const p = cam.d.data.camera.p;
      ['targetX', 'targetY', 'targetZ'].forEach((k, i) => { p[k].v = target[i]; });
      for (const [k, v] of Object.entries(extra)) p[k].v = v;
      return cam;
    };
    const shots: [number, number, string][] = [];
    // Final samples per frame: macro and wet shots (shallow focus) get more.
    const shot = (name: string, from: number, dur: number, f: () => void, samples = 32) => {
      shots.push([from, dur, build({ name, dur }, () => { f(); edit({ operation: 'set_environment', patch: { samples } }); PM.touch(); })]);
    };

    // 2.0 Edge-on reveal: standing upright, the sliver turns to show its face.
    shot('Edge-on reveal', 2, 2.5, () => {
      const g = slate('e');
      edit({ operation: 'auto_studio' });
      g.p['rotation.x'].v = 90;
      PM.animate(g, 'rotation.y', [{ t: 0, v: 88 }, { t: 2.5, v: 0 }]);
      aim([0, 0.75, 7.2], [0, 0.75, 0]);
    });
    // 6.0 Flat lay, tilting toward the camera.
    shot('Flat lay', 6, 2.5, () => {
      const g = slate('f');
      edit({ operation: 'auto_studio' });
      g.p['rotation.y'].v = 25;
      edit({ operation: 'shot', shot: 'hero_tilt', start: 0, duration: 2.5 });
    });
    // 8.5 Pull back: the screen fills the frame, then the device appears around it.
    shot('Pull back', 8.5, 2.5, () => {
      slate('p');
      edit({ operation: 'auto_studio' });
      const cam = aim([0, 1.55, 0.001], [0, 0, 0]);
      PM.animate(cam, 'position.y', [{ t: 0, v: 1.55 }, { t: 2.5, v: 3.4 }]);
      PM.animate(cam, 'position.z', [{ t: 0, v: 0.001 }, { t: 2.5, v: 1.1 }]);
    });
    // 11.0 Macro: shallow focus racks across the display corner.
    shot('Macro', 11, 2.5, () => {
      slate('m');
      edit({ operation: 'auto_studio' });
      aim([0.95, 0.32, 1.45], [0.5, 0.07, 0.8], { fStop: 2, focusDistance: 0 });
      edit({ operation: 'shot', shot: 'rack_focus', start: 0, duration: 2.5 });
    }, 96);
    // 13.5 Flip: the back with its camera module turns over to the front.
    shot('Flip', 13.5, 2.5, () => {
      const g = slate('b');
      edit({ operation: 'auto_studio' });
      PM.animate(g, 'rotation.z', [{ t: 0, v: 180 }, { t: 2.5, v: 360 }]);
      g.p['position.y'].v = 0.45;
      aim([0.4, 1.9, 2.6], [0, 0.45, 0]);
    });
    // 16.0 Stylus: slides in beside the device and a line of type.
    shot('Stylus', 16, 2.5, () => {
      slate('s');
      edit({ operation: 'add_object', object: { id: 's_stylus', name: 'Stylus', source: { primitive: 'capsule', parameters: { radius: 0.035, height: 1.55, segments: 32 } }, p: { x: 0.82, y: 0.04, rx: 90 } } });
      edit({ operation: 'apply_material', target: 's_stylus', preset: 'lacquer', p: { color: '#F4F4F2' } });
      edit({ operation: 'auto_studio' });
      PM.animate(PM.L('s_stylus'), 'position.z', [{ t: 0, v: 2.6 }, { t: 1.2, v: 0 }]);
      aim([-0.3, 2.9, 2.2], [-0.75, 0, 0]);
      animate(text('Draws like ink.', 0.6, 1.9, 270, 360, 46, '#16161A'), { opacity: 0, x: -40 });
    });
    // 18.5 Wet: droplets bead on the display glass, close up.
    shot('Wet', 18.5, 2, () => {
      slate('w', true);
      edit({ operation: 'auto_studio' });
      const cam = aim([0.55, 0.75, 1.25], [0.15, 0.04, 0.35], { fStop: 2.8 });
      PM.animate(cam, 'position.x', [{ t: 0, v: 0.55 }, { t: 2, v: 0.25 }]);
    }, 96);
    // 22.0 End: a turntable on the set.
    shot('End', 22, 3, () => {
      const g = slate('t');
      edit({ operation: 'auto_studio' });
      edit({ operation: 'shot', shot: 'turntable', target: g.id, start: 0, duration: 3, degrees: 120 });
    });
    // A graphic card on black: a glowing orb that swells, fringes and blurs away.
    const effects = Object.keys(PM.FX || {}).filter(e => ['blur', 'glow', 'chroma'].includes(e));
    const glowId = build({ name: 'Glow', dur: 1.5, bg: '#000000' }, () => {
      add('solid', { from: 0, dur: 1.5, d: { color: '#000000', w: W, h: H } });
      // A glyph's position is its baseline: lift it so the orb is centred.
      const orb = text('●', 0, 1.5, W / 2, H / 2 - 150, 420, '#8A7CFF', { weight: 400 });
      PM.animate(orb, 'scale.x', [{ t: 0, v: 35 }, { t: 1.5, v: 140 }]);
      PM.animate(orb, 'scale.y', [{ t: 0, v: 35 }, { t: 1.5, v: 140 }]);
      effect(orb, 'glow');
      effect(orb, 'chroma');
      effect(orb, 'blur', { amount: 18 });
      PM.touch();
    });

    // The main composition: shots and cards, cut on the beat.
    for (const [from, dur, id] of shots) add('precomp', { from, dur, d: { comp: id } });
    add('precomp', { from: 20.5, dur: 1.5, d: { comp: glowId } });
    animate(text('Thin.', 0.1, 1.9, 640, 380, 120, '#16161A'), { opacity: 0, blur: 30 });
    animate(text('Light. Bright.', 4.6, 1.4, 640, 380, 110, '#5B5BEF'), { opacity: 0, y: 70 });
    animate(text('Slate', 22.3, 2.7, 640, 120, 84, '#16161A', { weight: 700 }), { opacity: 0, blur: 20 });
    text('An original product film, made and rendered entirely in Powermove Studio.', 23, 2, 640, 680, 16, '#86868B', { weight: 400 });
    PM.touch(); PM.invalidate();
    return { shots: shots.length, layers: PM.proj.layers.length, effects, main: PM.Comps.active() === mainId };
  });
  expect(built.shots).toBe(8);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);

  // Stills from every shot, then the whole film through Final.
  const stills = [1, 3.5, 5.2, 7.5, 10.5, 12.5, 15.2, 17.6, 19.8, 21.2, 23.8];
  const frames = await page.evaluate((times: number[]) => {
    const PM = (window as any).PM;
    PM.agentFrameCapture = true;
    const out = times.map(t => {
      const px = PM.GL.renderToPixels(t, 640, 360, { transparent: false, mblur: false });
      const c = document.createElement('canvas'); c.width = 640; c.height = 360;
      const flipped = new Uint8ClampedArray(px.length);
      for (let y = 0; y < 360; y++) flipped.set(px.subarray((359 - y) * 640 * 4, (360 - y) * 640 * 4), y * 640 * 4);
      c.getContext('2d')!.putImageData(new ImageData(flipped, 640, 360), 0, 0);
      let varied = 0;
      for (let i = 0; i < px.length; i += 4) if (Math.abs(px[i]! - 246) + Math.abs(px[i + 1]! - 245) + Math.abs(px[i + 2]! - 248) > 24) varied++;
      return { t, url: c.toDataURL(), varied };
    });
    PM.agentFrameCapture = false;
    return out;
  }, stills);
  for (const f of frames) {
    await info.attach(`still-${f.t}`, { body: Buffer.from(f.url.split(',')[1]!, 'base64'), contentType: 'image/png' });
    // Every shot shows something besides the empty set.
    if (process.env.PM_FILM_STILLS) require('node:fs').writeFileSync(path.join(process.env.PM_FILM_STILLS, `still-${f.t}.png`), Buffer.from(f.url.split(',')[1]!, 'base64'));
    expect(f.varied, `still at ${f.t}s`).toBeGreaterThan(1500);
  }
  const started = Date.now();
  const result = await page.evaluate(() => (window as any).PM.Export.run({ format: 'mp4', alpha: false, scale: 1, fps: 30, range: 'all', quality: 'high', audio: false, mblur: false }));
  expect(result).toEqual({ cancelled: false });
  expect(statSync(output).size).toBeGreaterThan(500_000);
  copyFileSync(output, info.outputPath('slate-film-extended.mp4'));
  await info.attach('export-seconds', { body: String((Date.now() - started) / 1000), contentType: 'text/plain' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
