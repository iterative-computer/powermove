import { expect, test } from './helpers/app';
import { chromium } from '@playwright/test';
import { createServer } from 'node:http';
import { readFile, writeFile } from 'node:fs/promises';
import { fixturePath } from './helpers/media';
import path from 'node:path';
import { execFileSync } from 'node:child_process';

test('web export plays independently and matches editor frames', async ({ session }, testInfo) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const videoBytes = [...await readFile(fixturePath('h264-aac.mp4'))];
  const audioBytes = [...await readFile(fixturePath('tone.wav'))];
  const exported = await session.page.evaluate(async ({ videoBytes, audioBytes }) => {
    const PM = (window as any).PM;
    PM.pause(); PM.assets.clear();
    PM.proj = PM.mkProject({ name: 'Portable motion', w: 320, h: 180, fps: 30, dur: 2, bg: '#182030' });
    PM.proj.params = { accent: { name: 'accent', label: 'Accent', control: 'color', value: '#ff5500' } };
    const box = PM.mkLayer('solid', { name: 'Animated box', d: { w: 60, h: 60, color: '#ff5500', radius: 12 } }, PM.proj);
    box.p['position.x'].kf = [PM.KF(0, 50, 'easeInOut'), PM.KF(1, 270, 'easeInOut')];
    box.p['position.y'].v = 100;
    box.d.color = PM.P('#ff5500', { expr: 'param("accent")' });
    PM.Kernel.registerEffect('generated-export-test', {
      id: 'generatedTint', label: 'Generated Tint', group: 'Generated',
      params: [{ k: 'mix', label: 'Mix', def: .5, min: 0, max: 1 }],
      frag: 'vec4 c=texture(u_tex,v_st); o=vec4(mix(c.rgb, c.bgr, u_mix),c.a);',
    });
    const generatedEffect = PM.mkEffect('generatedTint');
    generatedEffect.p.mix.kf = [PM.KF(0, .2), PM.KF(1, .9)];
    box.fx.push(generatedEffect);
    const text = PM.mkLayer('text', { name: 'Title', d: { text: 'Made in Powermove', font: 'Arial', size: 22, color: '#ffffff' } }, PM.proj);
    text.p['position.x'].v = 160; text.p['position.y'].v = 35;
    const sub = PM.mkProject({ w: 320, h: 180, dur: 2 });
    const star = PM.mkLayer('shape', { d: { shape: 'star', w: 45, h: 45, color: '#55ccff' } }, sub);
    star.p['position.x'].v = 250; star.p['position.y'].v = 100;
    star.p.rotation.kf = [PM.KF(0, 0), PM.KF(1, 90)];
    const blur = PM.mkEffect('blur');
    if (!blur) throw new Error('Built-in blur unavailable');
    star.fx.push(blur); sub.layers.push(star);
    PM.proj.comps[sub.id] = sub;
    const nested = PM.mkLayer('precomp', { d: { comp: sub.id, w: 320, h: 180 } }, PM.proj);
    const imageCanvas = document.createElement('canvas'); imageCanvas.width = imageCanvas.height = 20;
    const imageContext = imageCanvas.getContext('2d')!; imageContext.fillStyle = '#99ff55'; imageContext.fillRect(0, 0, 20, 20);
    const blob = await new Promise<Blob>(resolve => imageCanvas.toBlob(blob => resolve(blob!)));
    const asset = await PM.assets.add(new File([blob], 'square.png', { type: 'image/png' }), { silent: true });
    const picture = PM.mkLayer('image', { d: { asset: asset.id, w: 20, h: 20 } }, PM.proj);
    picture.p['position.x'].v = 25; picture.p['position.y'].v = 25;
    PM.Kernel.registerTransition('generated-export-test', {
      id: 'generatedReveal', label: 'Generated Reveal', params: [],
      frag: 'o=mix(texture(u_from,v_st),texture(u_to,v_st),smoothstep(0.,1.,u_prog));',
    });
    picture.transitionIn = PM.mkTransition('generatedReveal'); picture.transitionIn.dur = .8;
    PM.Kernel.registerLayerType('generated-export-test', {
      id: 'generated-ring', label: 'Generated Ring', version: 1, width: 40, height: 40,
      params: [{ k: 'radius', label: 'Radius', def: .3, min: .1, max: .5 }],
      renderer: { kind: 'fragment', fragment: 'void main(){float a=1.-smoothstep(.035,.05,abs(length(uv-.5)-u_radius));fragColor=vec4(vec3(.8,.3,1.)*a,a);}' },
    });
    const ring = PM.mkLayer('extension', { d: { definition: 'generated-ring', version: 1, w: 40, h: 40, data: {}, params: { radius: PM.P(.3, { kf: [PM.KF(0, .15), PM.KF(1, .4)] }) } } }, PM.proj);
    ring.p['position.x'].v = 145; ring.p['position.y'].v = 125;
    const shader = PM.mkLayer('shader', { d: {
      w: 35, h: 35,
      code: 'uniform vec3 uColor; // @param #ffcc33\nuniform float uAmount; // @param 1 0 1\nvoid main(){fragColor=vec4(uColor*(.6+.4*sin(iTime*3.))*uAmount,uAmount);}',
      uniforms: { uColor: PM.P('#ffcc33'), uAmount: PM.P(.8) },
    } }, PM.proj);
    shader.p['position.x'].v = 15; shader.p['position.y'].v = 130;
    const videoAsset = await PM.assets.add(new File([new Uint8Array(videoBytes)], 'clip.mp4', { type: 'video/mp4' }), { silent: true });
    const video = PM.mkLayer('video', { d: { asset: videoAsset.id, w: 60, h: 34, embeddedAudio: false } }, PM.proj);
    video.p['position.x'].v = 280; video.p['position.y'].v = 150;
    const audioAsset = await PM.assets.add(new File([new Uint8Array(audioBytes)], 'tone.wav', { type: 'audio/wav' }), { silent: true });
    const sound = PM.mkLayer('audio', { d: { asset: audioAsset.id } }, PM.proj);
    PM.proj.layers = [text, box, nested, picture, ring, shader, video, sound];
    PM.touch(); PM.bus.emit('project');
    // Register metadata after the layer belongs to the project: media imports
    // prune metadata for layers that are not yet present in the fixture.
    PM.UIState.setShaderMeta(shader, { udefs: PM.parseUniforms(shader.d.code) });
    const result = await PM.Export.buildWeb();
    if (!PM.GL.canvas) PM.GL.init(document.createElement('canvas'));
    const frames: number[][] = [];
    const images: string[] = [];
    const canvas = PM.GL.canvas, oldW = canvas.width, oldH = canvas.height;
    const oldCapture = PM.agentFrameCapture;
    PM.agentFrameCapture = true;
    try {
      PM.GL.resize(320, 180);
      for (const time of [0, .5, 1]) {
        await PM.prepareFrame(time);
        // Compare fully compiled export frames, not an asynchronous preview warmup.
        PM.GL.render(time, { exporting: true, mblur: true, mbSamples: 6, shutter: .5 });
        if (PM.GL.errors.size) throw new Error([...PM.GL.errors.values()].join('\n'));
        const gl = PM.GL.gl, pixels = new Uint8Array(320 * 180 * 4);
        gl.readPixels(0, 0, 320, 180, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
        frames.push([...pixels]);
        images.push(canvas.toDataURL());
      }
    } finally { PM.GL.resize(oldW, oldH); PM.agentFrameCapture = oldCapture; }
    return { files: [...result.files].map(([name, bytes]: any) => [name, [...bytes]]), zip: [...result.bytes], frames, images, textId: text.id,
      generatedEffect: result.scene.effects.some((d: any) => d.id === 'generatedTint'),
      generatedLayer: result.scene.layerTypes.some((d: any) => d.id === 'generated-ring'),
    };
  }, { videoBytes, audioBytes });
  expect(exported.generatedEffect).toBe(true);
  expect(exported.generatedLayer).toBe(true);
  const zipPath = testInfo.outputPath('animation.zip');
  await writeFile(zipPath, Buffer.from(exported.zip));
  expect(execFileSync('/usr/bin/unzip', ['-t', zipPath], { encoding: 'utf8' })).toContain('No errors detected');
  const files = new Map<string, Buffer>(exported.files.map(([name, bytes]: any) => [name, Buffer.from(bytes)]));
  const server = createServer((request, response) => {
    const name = request.url === '/' ? 'index.html' : request.url!.slice(1);
    const data = files.get(name);
    response.statusCode = data ? 200 : 404;
    response.setHeader('Content-Type', name.endsWith('.js') ? 'text/javascript' : name.endsWith('.json') ? 'application/json' : name.endsWith('.png') ? 'image/png' : name.endsWith('.mp4') ? 'video/mp4' : name.endsWith('.wav') ? 'audio/wav' : 'text/html');
    response.end(data || 'Not found');
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const port = (server.address() as any).port;
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => {
      (window as any).audioStarts = 0;
      const create = AudioContext.prototype.createBufferSource;
      AudioContext.prototype.createBufferSource = function () {
        const source = create.call(this), start = source.start;
        source.start = function (...args: Parameters<typeof start>) { (window as any).audioStarts++; return start.apply(this, args); };
        return source;
      };
    });
    await page.goto(`http://127.0.0.1:${port}`);
    await expect(page.locator('#status')).not.toHaveText('Loading animation…');
    if (await page.locator('#play').isDisabled()) throw new Error(await page.locator('#status').innerText());
    await expect(page.locator('#play')).toBeEnabled();
    await page.screenshot({ path: testInfo.outputPath('web-preview.png') });
    await page.locator('#play').click();
    await expect.poll(() => page.evaluate(() => (window as any).audioStarts)).toBeGreaterThan(0);
    await page.locator('#play').click();
    const actual = await page.evaluate(async () => {
      const { createPlayer } = await import(/* @vite-ignore */ location.origin + '/player.js');
      const canvas = document.createElement('canvas');
      canvas.style.width = '320px'; canvas.style.height = '180px';
      document.body.append(canvas);
      const player = await createPlayer({ canvas, scene: './scene.json', audio: false });
      (window as any).testPlayer = player;
      (window as any).testCanvas = canvas;
      const frames: number[][] = [];
      const images: string[] = [];
      for (const time of [0, .5, 1]) {
        await player.seek(time);
        const gl = canvas.getContext('webgl2')!, pixels = new Uint8Array(320 * 180 * 4);
        gl.readPixels(0, 0, 320, 180, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
        frames.push([...pixels]);
        images.push(canvas.toDataURL());
      }
      const alphaCanvas = document.createElement('canvas');
      const alphaPlayer = await createPlayer({ canvas: alphaCanvas, scene: './scene.json', transparent: true, audio: false });
      const alphaGL = alphaCanvas.getContext('webgl2')!, corner = new Uint8Array(4);
      alphaGL.readPixels(319, 179, 1, 1, alphaGL.RGBA, alphaGL.UNSIGNED_BYTE, corner);
      alphaPlayer.destroy();
      return { frames, images, transparent: corner[3] === 0, isolated: !(window as any).PM && !(window as any).powermove };
    });
    expect(actual.isolated).toBe(true);
    expect(actual.transparent).toBe(true);
    for (let i = 0; i < 3; i++) {
      await writeFile(testInfo.outputPath(`editor-${i}.png`), Buffer.from(exported.images[i]!.split(',')[1]!, 'base64'));
      await writeFile(testInfo.outputPath(`browser-${i}.png`), Buffer.from(actual.images[i]!.split(',')[1]!, 'base64'));
      const averageError = actual.frames[i]!.reduce((sum, value, j) => sum + Math.abs(value - exported.frames[i]![j]!), 0) / actual.frames[i]!.length;
      await testInfo.attach(`frame-${i}-comparison`, { body: JSON.stringify({ time: [0, .5, 1][i], averageChannelError: averageError, tolerance: 1 }), contentType: 'application/json' });
      expect(averageError).toBeLessThan(1);
    }
    await page.evaluate(() => {
      const canvas = (window as any).testCanvas;
      canvas.style.width = '640px'; canvas.style.height = '360px';
    });
    await expect.poll(() => page.evaluate(() => {
      const canvas = (window as any).testCanvas;
      return [canvas.width, canvas.height];
    })).toEqual([640, 360]);
    const controls = await page.evaluate(async textId => {
      const player = (window as any).testPlayer;
      await player.seek(.5);
      const before = (window as any).testCanvas.toDataURL();
      await player.setParameter('accent', '#0055ff');
      await player.setText(textId, 'Embedded in your app');
      await player.seek(.5);
      const changed = before !== (window as any).testCanvas.toDataURL();
      player.play();
      await new Promise(resolve => setTimeout(resolve, 120));
      player.pause();
      const advanced = player.currentTime > .5;
      player.loop = false;
      await player.seek(1.95); player.play();
      await new Promise(resolve => setTimeout(resolve, 200));
      const stopped = !player.playing;
      player.loop = true; await player.seek(1.95); player.play();
      await new Promise(resolve => setTimeout(resolve, 200)); player.pause();
      const looped = player.currentTime < 1;
      let rejected = false;
      try { await player.setParameter('accent', 12); } catch { rejected = true; }
      player.destroy();
      return { advanced, stopped, looped, rejected, changed };
    }, exported.textId);
    expect(controls).toEqual({ advanced: true, stopped: true, looped: true, rejected: true, changed: true });
    const retina = await browser.newPage({ deviceScaleFactor: 2 });
    await retina.goto(`http://127.0.0.1:${port}`);
    await expect(retina.locator('#play')).toBeEnabled();
    const retinaSize = await retina.locator('canvas').evaluate((canvas: HTMLCanvasElement) => ({
      actual: [canvas.width, canvas.height],
      expected: [Math.round(canvas.getBoundingClientRect().width * devicePixelRatio),
        Math.round(canvas.getBoundingClientRect().height * devicePixelRatio)],
    }));
    expect(retinaSize.actual).toEqual(retinaSize.expected);
    await retina.close();
    expect(errors).toEqual([]);
  } finally { await browser.close(); await new Promise<void>(resolve => server.close(() => resolve())); }

  // Exercise the actual UI delivery path, including native file saving.
  const output = path.join(session.userData, 'web-animation.zip');
  await session.app.evaluate(({ dialog }, file) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath: file }); }, output);
  expect(await session.page.evaluate(() => (window as any).PM.Export.run({ format: 'web' }))).toEqual({ cancelled: false });
  expect(execFileSync('/usr/bin/unzip', ['-t', output], { encoding: 'utf8' })).toContain('No errors detected');
  await session.app.evaluate(({ dialog }) => { dialog.showSaveDialog = async () => ({ canceled: true, filePath: '' }); });
  expect(await session.page.evaluate(() => (window as any).PM.Export.run({ format: 'web' }))).toEqual({ cancelled: true });
});
