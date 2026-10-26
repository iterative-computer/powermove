import { expect, test } from './helpers/app';

// These are independent arithmetic expectations, not AE reference renders.
const cases = [
  { name: 'invert RGB', type: 'invert', color: '#804020', values: {}, expected: [127, 191, 223, 255] },
  { name: 'invert red only', type: 'invert', color: '#804020', values: { channel: 1 }, expected: [127, 64, 32, 255] },
  { name: 'invert original blend', type: 'invert', color: '#804020', values: { blendWithOriginal: 100 }, expected: [128, 64, 32, 255] },
  { name: 'invert HLS lightness', type: 'invert', color: '#804020', values: { channel: 6 }, expected: [223, 159, 127, 255] },
  { name: 'invert YIQ luminance', type: 'invert', color: '#808080', values: { channel: 9 }, expected: [127, 127, 127, 255] },
  { name: 'invert alpha keeps premultiplication', type: 'invert', color: '#804020', opacity: 50, values: { channel: 12 }, expected: [64, 32, 16, 128] },
  { name: 'levels identity', type: 'levels', color: '#804020', values: {}, expected: [128, 64, 32, 255] },
  { name: 'levels red output control', type: 'levels', color: '#804020', values: { redOutBlack: .5, redOutWhite: .5 }, expected: [128, 64, 32, 255] },
  { name: 'levels inverted output range', type: 'levels', color: '#804020', values: { outBlack: 1, outWhite: 0 }, expected: [127, 191, 223, 255] },
  { name: 'levels alpha output', type: 'levels', color: '#804020', values: { alphaOutWhite: .5 }, expected: [64, 32, 16, 128] },
  { name: 'levels individual red output', type: 'levelsIndividual', color: '#804020', values: { redOutWhite: 0 }, expected: [0, 64, 32, 255] },
  { name: 'threshold dark', type: 'threshold', color: '#404040', values: {}, expected: [0, 0, 0, 255] },
  { name: 'threshold light', type: 'threshold', color: '#C0C0C0', values: {}, expected: [255, 255, 255, 255] },
  { name: 'shift identity', type: 'shiftChannels', color: '#804020', values: {}, expected: [128, 64, 32, 255] },
  { name: 'shift blue to red', type: 'shiftChannels', color: '#804020', values: { takeRedFrom: 3 }, expected: [32, 64, 32, 255] },
  { name: 'shift alpha off', type: 'shiftChannels', color: '#804020', values: { takeAlphaFrom: 9 }, expected: [0, 0, 0, 0] },
  { name: 'shift alpha from red', type: 'shiftChannels', color: '#808080', values: { takeAlphaFrom: 1 }, expected: [64, 64, 64, 128] },
  { name: 'shift full red', type: 'shiftChannels', color: '#804020', values: { takeRedFrom: 8 }, expected: [255, 64, 32, 255] },
  { name: 'exposure identity', type: 'exposure', color: '#804020', values: {}, expected: [128, 64, 32, 255] },
  { name: 'exposure raw stop', type: 'exposure', color: '#402010', values: { bypassLinearLight: true, exposure: 1 }, expected: [128, 64, 32, 255] },
  { name: 'exposure linear stop', type: 'exposure', color: '#808080', values: { exposure: 1 }, expected: [176, 176, 176, 255] },
  { name: 'exposure offset', type: 'exposure', color: '#000000', values: { bypassLinearLight: true, offset: .25 }, expected: [64, 64, 64, 255] },
  { name: 'exposure gamma', type: 'exposure', color: '#404040', values: { bypassLinearLight: true, gamma: 2 }, expected: [128, 128, 128, 255] },
  { name: 'exposure individual red', type: 'exposure', color: '#402010', values: { bypassLinearLight: true, redExposure: 1 }, expected: [128, 32, 16, 255] },

  { name: 'tint black endpoint', type: 'tint', color: '#000000', values: {}, expected: [0, 0, 0, 255] },
  { name: 'tint white endpoint', type: 'tint', color: '#FFFFFF', values: {}, expected: [255, 255, 255, 255] },
  { name: 'tint red luminance', type: 'tint', color: '#FF0000', values: {}, expected: [54, 54, 54, 255] },
  { name: 'tint green luminance', type: 'tint', color: '#00FF00', values: {}, expected: [182, 182, 182, 255] },
  { name: 'tint blue luminance', type: 'tint', color: '#0000FF', values: {}, expected: [18, 18, 18, 255] },
  { name: 'tint midpoint is linear', type: 'tint', color: '#404040', values: {}, expected: [64, 64, 64, 255] },
  { name: 'tint original bypass', type: 'tint', color: '#804020', values: { amountToTint: 0 }, expected: [128, 64, 32, 255] },
  { name: 'tint partial mix', type: 'tint', color: '#FF0000', values: { amountToTint: 50 }, expected: [155, 27, 27, 255] },
  { name: 'tint custom endpoints', type: 'tint', color: '#808080', values: { mapBlackTo: '#FF0000', mapWhiteTo: '#0000FF' }, expected: [127, 0, 128, 255] },
  { name: 'tint premultiplied edge', type: 'tint', color: '#FF0000', opacity: 50, values: {}, expected: [27, 27, 27, 128] },
  { name: 'mixer identity', type: 'channelMixer', color: '#804020', values: {}, expected: [128, 64, 32, 255] },
  { name: 'mixer channel swap', type: 'channelMixer', color: '#804020', values: { redRed: 0, redBlue: 100, blueRed: 100, blueBlue: 0 }, expected: [32, 64, 128, 255] },
  { name: 'mixer constant is a channel offset', type: 'channelMixer', color: '#000000', values: { redConst: 50 }, expected: [128, 0, 0, 255] },
  { name: 'mixer monochrome uses red output', type: 'channelMixer', color: '#804020', values: { redRed: 0, redGreen: 100, monochrome: true }, expected: [64, 64, 64, 255] },
  { name: 'mixer clamps negative and over-range channels', type: 'channelMixer', color: '#804020', values: { redRed: -200, greenGreen: 0, greenConst: 200 }, expected: [0, 255, 32, 255] },
  { name: 'mixer constant respects source alpha', type: 'channelMixer', color: '#000000', opacity: 50, values: { redConst: 100 }, expected: [128, 0, 0, 128] },

  { name: 'tritone dark map', type: 'tritone', color: '#000000', values: { shadows: '#00FF00' }, expected: [0,255,0,255] },
  { name: 'tritone bright map', type: 'tritone', color: '#FFFFFF', values: {}, expected: [255,255,255,255] },
  { name: 'tritone midpoint map', type: 'tritone', color: '#808080', values: { midtones: '#FF0000' }, expected: [255,1,1,255] },
  { name: 'tritone original blend', type: 'tritone', color: '#804020', values: { blendWithOriginal:100 }, expected: [128,64,32,255] },
  { name: 'HLS identity', type: 'colorBalanceHLS', color: '#804020', values: {}, expected: [128,64,32,255] },
  { name: 'HLS hue rotation', type: 'colorBalanceHLS', color: '#FF0000', values: {hue:120}, expected: [0,255,0,255] },
  { name: 'HLS lightness', type: 'colorBalanceHLS', color: '#000000', values: {lightness:100}, expected: [255,255,255,255] },
  { name: 'HLS grayscale', type: 'colorBalanceHLS', color: '#FF0000', values: {saturation:-100}, expected: [128,128,128,255] },
  { name: 'HLS premultiplied grayscale', type: 'colorBalanceHLS', color: '#0000FF', opacity:50, values: {saturation:-100}, expected: [64,64,64,128] },
  { name: 'brightness identity', type: 'brightnessContrast', color: '#804020', values: {}, expected: [128,64,32,255] },
  { name: 'brightness legacy offset', type: 'brightnessContrast', color: '#000000', values: {brightness:50,useLegacy:true}, expected: [128,128,128,255] },
  { name: 'contrast legacy flat', type: 'brightnessContrast', color: '#804020', values: {contrast:-100,useLegacy:true}, expected: [128,128,128,255] },
  { name: 'brightness white endpoint', type: 'brightnessContrast', color: '#000000', values: {brightness:150}, expected: [255,255,255,255] },
  { name: 'brightness black endpoint', type: 'brightnessContrast', color: '#FFFFFF', values: {brightness:-150}, expected: [0,0,0,255] },
  { name: 'blackwhite red weight', type: 'blackWhite', color: '#FF0000', values: {}, expected: [102,102,102,255] },
  { name: 'blackwhite yellow weight', type: 'blackWhite', color: '#FFFF00', values: {}, expected: [153,153,153,255] },
  { name: 'blackwhite blue weight', type: 'blackWhite', color: '#0000FF', values: {}, expected: [51,51,51,255] },
  { name: 'blackwhite neutral gray', type: 'blackWhite', color: '#808080', values: {}, expected: [128,128,128,255] },
  { name: 'blackwhite custom red weight', type: 'blackWhite', color: '#FF0000', values: {reds:100}, expected: [255,255,255,255] },
  { name: 'blackwhite tint', type: 'blackWhite', color: '#808080', values: {tint:true,tintColor:'#FF0000'}, expected: [255,1,1,255] },
];

test('color correction uses real GPU programs with correct mapping, mixing, alpha and depth', async ({ session }, testInfo) => {
  await session.openEditor();
  const results = await session.page.evaluate((cases) => {
    const PM = (window as any).PM;
    PM.pause(); PM.agentFrameCapture = true; PM.quality = 1; PM.AutomaticPreview?.cancel();
    const results: Array<{ name: string; depth: number; actual: number[]; expected: number[] }> = [];
    for (const depth of [8, 16, 32]) {
      if (!PM.GL.supportedColorDepths.includes(depth)) continue;
      for (const fixture of cases) {
        const project = PM.mkProject({ name: fixture.name, w: 32, h: 32, fps: 30, dur: 2 });
        project.colorDepth = depth; project.backgroundFill = { type: 'none', stops: [], angle: 0 };
        const layer = PM.mkLayer('shape', { name: 'Color test', d: { shape: 'rect', w: 24, h: 24, radius: 0, color: fixture.color },
          p: { 'position.x': 16, 'position.y': 16, opacity: fixture.opacity ?? 100 } }, project);
        const fx = PM.mkEffect(fixture.type);
        for (const [key, value] of Object.entries(fixture.values)) fx.p[key].v = value;
        layer.fx = [fx]; project.layers = [layer]; PM.replaceProject(project);
        const pixels = PM.GL.renderToPixels(0, 32, 32, { transparent: true, mblur: false });
        if (!PM.GL.progs.get('fx:' + fixture.type)?.pr || PM.GL.errors.size) throw new Error('Effect did not compile');
        const at = (16 * 32 + 16) * 4;
        results.push({ name: fixture.name, depth, actual: [...pixels.slice(at, at + 4)], expected: fixture.expected });
        if (pixels.slice(0, 4).some((value: number) => value !== 0)) throw new Error('Effect filled transparent bounds');
      }
    }
    return results;
  }, cases);
  expect(results.filter(result => result.depth === 8)).toHaveLength(cases.length);
  for (const result of results) for (let channel = 0; channel < 4; channel++) {
    expect(Math.abs(result.actual[channel]! - result.expected[channel]!), `${result.name}, ${result.depth}-bit, channel ${channel}`).toBeLessThanOrEqual(1);
  }
  await testInfo.attach('arithmetic-pixel-results', { body: JSON.stringify(results, null, 2), contentType: 'application/json' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('tint animation and ordered color stacks survive serialization in the native Inspector', async ({ session }, testInfo) => {
  await session.openEditor();
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.agentFrameCapture = true; PM.quality = 1; PM.AutomaticPreview?.cancel();
    const project = PM.mkProject({ name: 'Tint and Channel Mixer', w: 640, h: 360, fps: 30, dur: 2 });
    const layer = PM.mkLayer('shape', { name: 'Color correction', from: 0, dur: 2,
      d: { shape: 'rect', w: 360, h: 220, radius: 28, color: '#E95A2B' },
      p: { 'position.x': 320, 'position.y': 180 } }, project);
    const tint = PM.mkEffect('tint'), mixer = PM.mkEffect('channelMixer');
    tint.p.mapBlackTo.v = '#142A5C'; tint.p.mapWhiteTo.v = '#FFE0A0';
    layer.fx = [tint, mixer]; project.layers = [layer]; PM.replaceProject(project);
    PM.animate(layer, 'effect.amountToTint', [{ t: 0, v: 0 }, { t: 1, v: 100 }], { prop: tint.p.amountToTint });
    const sample = (time: number) => [...PM.GL.renderToPixels(time, 32, 18, { mblur: false })];
    const before = [sample(0), sample(1)];
    const snapshot = JSON.parse(JSON.stringify(PM.proj));
    PM.replaceProject(snapshot); PM.selectLayers(snapshot.layers[0].id);
    const after = [sample(0), sample(1)];
    const loaded = PM.proj.layers[0];
    PM.UIState.setFxOpen(loaded.fx[0], true); PM.UIState.setFxOpen(loaded.fx[1], true);
    PM.agentFrameCapture = false; PM.setTime(1, { force: true }); PM.bus.emit('layers'); PM.invalidate();
    return { before, after, types: loaded.fx.map((fx: any) => fx.type), id: loaded.fx[0].id,
      keyCount: loaded.fx[0].p.amountToTint.kf.length };
  });
  expect(result.after).toEqual(result.before);
  expect(result.before[0]).not.toEqual(result.before[1]);
  expect(result.types).toEqual(['tint', 'channelMixer']);
  expect(result.keyCount).toBe(2);
  const tint = session.page.locator(`[data-effect-id="${result.id}"]`);
  await expect(tint).toBeVisible();
  await expect(session.page.getByText('Map Black To', { exact: true })).toBeVisible();
  await expect(session.page.getByText('Monochrome', { exact: true })).toBeVisible();
  // Captures suppress the normal paint loop. Resume it and wait for actual
  // colored viewer pixels before recording UI evidence.
  await session.page.waitForFunction(() => {
    const GL = (window as any).PM.GL, gl = GL.gl;
    const pixel = new Uint8Array(4);
    gl.readPixels(Math.floor(GL.canvas.width / 2), Math.floor(GL.canvas.height / 2), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    return pixel[0]! > 30 && pixel[1]! > 20;
  });
  for (const theme of ['light', 'dark']) {
    await session.page.evaluate(theme => {
      const PM = (window as any).PM;
      PM.theme.apply(theme); PM.invalidate('render');
    }, theme);
    await session.page.waitForFunction(() => {
      const GL = (window as any).PM.GL, gl = GL.gl, pixel = new Uint8Array(4);
      gl.readPixels(Math.floor(GL.canvas.width / 2), Math.floor(GL.canvas.height / 2), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
      return pixel[0]! > 30 && pixel[1]! > 20;
    });
    await tint.scrollIntoViewIfNeeded();
    await session.page.screenshot({ path: testInfo.outputPath(`color-correction-tint-${theme}.png`) });
    await session.page.getByText('Monochrome', { exact: true }).scrollIntoViewIfNeeded();
    await session.page.screenshot({ path: testInfo.outputPath(`color-correction-mixer-${theme}.png`) });
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('old and current Invert revisions render together and survive real project hydration', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.agentFrameCapture = true; PM.quality = 1; PM.AutomaticPreview?.cancel();
    const project = PM.mkProject({ name: 'Effect revisions', w: 64, h: 32, fps: 30, dur: 2 });
    project.colorDepth = 16; project.backgroundFill = { type: 'none', stops: [], angle: 0 };
    const layer = (x: number) => PM.mkLayer('shape', { d: { shape: 'rect', w: 24, h: 24, radius: 0, color: '#FF0000' }, p: { 'position.x': x, 'position.y': 16 } }, project);
    const old = layer(16), current = layer(48);
    old.fx = [{ id: 'saved-original', type: 'invert', on: true, p: { amount: PM.P(25) } }];
    const revised = PM.mkEffect('invert'); revised.p.channel.v = 3; current.fx = [revised];
    project.layers = [old, current];
    const capture = () => {
      const pixels = PM.GL.renderToPixels(0, 64, 32, { transparent: true, mblur: false });
      return [16, 48].map(x => [...pixels.slice((16 * 64 + x) * 4, (16 * 64 + x) * 4 + 4)]);
    };
    PM.replaceProject(project); const before = capture();
    PM.replaceProject(PM.hydrateProject(JSON.parse(JSON.stringify(PM.proj)))); const after = capture();
    const versions = PM.proj.layers.map((layer: any) => layer.fx[0].version);
    const parameters = PM.proj.layers.map((layer: any) => Object.keys(layer.fx[0].p));
    for (const layer of PM.proj.layers) layer.d.color = '#808080';
    PM.proj.layers[0].fx = [{ id: 'saved-levels', type: 'levels', on: true, p: { inBlack: PM.P(0), inWhite: PM.P(50), gamma: PM.P(1) } }];
    PM.proj.layers[1].fx = [PM.mkEffect('levels')];
    PM.replaceProject(PM.hydrateProject(JSON.parse(JSON.stringify(PM.proj))));
    const levels = capture();
    return { before, after, levels, levelVersions: PM.proj.layers.map((layer: any) => layer.fx[0].version), versions,
      parameters,
      programs: [...PM.GL.progs.keys()].filter((key: string) => key.startsWith('fx:invert')) };
  });
  expect(result.after).toEqual(result.before);
  expect(result.versions).toEqual([1, 2]);
  expect(result.levelVersions).toEqual([1, 2]);
  expect(result.levels).toEqual([[255, 255, 255, 255], [128, 128, 128, 255]]);
  expect(result.parameters).toEqual([['amount'], ['channel', 'blendWithOriginal']]);
  expect(result.programs).toEqual(expect.arrayContaining(['fx:invert@1', 'fx:invert']));
  for (const [index, expected] of [[0, [191, 64, 64, 255]], [1, [255, 0, 255, 255]]] as const)
    for (let c = 0; c < 4; c++) expect(Math.abs(result.after[index]![c]! - expected[c]!)).toBeLessThanOrEqual(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Tint Swap Colors is one undoable native action and channel controls use disclosures', async ({ session }) => {
  await session.openEditor();
  const id = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Swap colors', w: 640, h: 360 });
    const layer = PM.mkLayer('solid', { name: 'Tint controls' }, project);
    const tint = PM.mkEffect('tint'), mixer = PM.mkEffect('channelMixer');
    tint.p.mapBlackTo.v = '#142A5C'; tint.p.mapWhiteTo.v = '#FFE0A0';
    layer.fx = [tint, mixer]; project.layers = [layer]; PM.replaceProject(project); PM.selectLayers(layer.id);
    PM.UIState.setFxOpen(tint, true); PM.UIState.setFxOpen(mixer, true);
    PM.hist.clear(); PM.bus.emit('layers'); PM.invalidate(); return layer.id;
  });
  const colors = () => session.page.evaluate(id => {
    const effect = (window as any).PM.L(id).fx[0]; return [effect.p.mapBlackTo.v, effect.p.mapWhiteTo.v];
  }, id);
  await session.page.getByRole('button', { name: 'Swap Colors', exact: true }).click();
  expect(await colors()).toEqual(['#FFE0A0', '#142A5C']);
  await session.page.keyboard.press('Meta+z');
  expect(await colors()).toEqual(['#142A5C', '#FFE0A0']);
  const group = session.page.locator('details.control-disclosure').filter({ has: session.page.locator('summary', { hasText: /^Red/ }) });
  await group.locator('summary').click();
  await expect(group.getByText('Red-Const', { exact: true })).toBeVisible();
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('float effects preserve signed HDR values and deliberate Levels clipping', async ({ session }) => {
  await session.openEditor();
  const results = await session.page.evaluate(() => {
    const PM = (window as any).PM, gl = PM.GL.gl;
    PM.agentFrameCapture = true; PM.quality = 1; PM.AutomaticPreview?.cancel();
    const results: Array<{ depth: number; name: string; actual: number[]; expected: number[] }> = [];
    for (const depth of [16, 32]) {
      if (!PM.GL.supportedColorDepths.includes(depth)) continue;
      const fixtures = [
        { name: 'HDR brightness neutral', type: 'brightnessContrast', values: {}, expected: [-.25,2,.5,1] },
        { name: 'HDR HLS neutral', type: 'colorBalanceHLS', values: {}, expected: [-.25,2,.5,1] },
        { name: 'raw exposure stop', type: 'exposure', values: { bypassLinearLight: true, exposure: 1 }, expected: [-.5, 4, 1, 1] },
        { name: 'signed exposure gamma', type: 'exposure', values: { bypassLinearLight: true, gamma: 2 }, expected: [-.5, Math.SQRT2, Math.SQRT1_2, 1] },
        { name: 'unclipped levels', type: 'levels', values: { clipBlack: false, clipWhite: false }, expected: [-.25, 2, .5, 1] },
        { name: 'clipped levels', type: 'levels', values: {}, expected: [0, 1, .5, 1] },
        { name: 'HDR invert', type: 'invert', values: {}, expected: [1.25, -1, .5, 1] },
        { name: 'HDR tint', type: 'tint', values: {}, expected: [1.41335, 1.41335, 1.41335, 1] },
      ];
      for (const fixture of fixtures) {
        const project = PM.mkProject({ name: fixture.name, w: 16, h: 16, dur: 2 });
        project.colorDepth = depth; project.backgroundFill = { type: 'none', stops: [], angle: 0 };
        const layer = PM.mkLayer('shader', { d: { w: 16, h: 16, code: 'void main(){fragColor=vec4(-.25,2.,.5,1.);}' }, p: { 'position.x': 0, 'position.y': 0 } }, project);
        const effect = PM.mkEffect(fixture.type);
        for (const [key, value] of Object.entries(fixture.values)) effect.p[key].v = value;
        layer.fx = [effect]; project.layers = [layer]; PM.replaceProject(project);
        const read = gl.readPixels.bind(gl); let sample: number[] | undefined;
        gl.readPixels = (...args: any[]) => {
          read(...args);
          if (args[5] === gl.FLOAT && args[6] instanceof Float32Array) sample = [...args[6].slice((8 * 16 + 8) * 4, (8 * 16 + 8) * 4 + 4)];
        };
        try { PM.GL.renderToPixels(0, 16, 16, { transparent: true, mblur: false }); }
        finally { gl.readPixels = read; }
        if (!sample || PM.GL.errors.size) throw new Error('Actual float effect output was unavailable');
        results.push({ depth, name: fixture.name, actual: sample, expected: fixture.expected });
      }
    }
    return results;
  });
  expect(results.length).toBeGreaterThanOrEqual(6);
  for (const result of results) for (let channel = 0; channel < 4; channel++)
    expect(Math.abs(result.actual[channel]! - result.expected[channel]!), `${result.name}, ${result.depth}-bit, channel ${channel}`).toBeLessThan(.003);
  expect(session.diagnostics.pageErrors).toEqual([]);
});


test('tonal controls keep native layout and animated ordered stacks survive hydration',async({session},testInfo)=>{
  await session.openEditor();
  const result=await session.page.evaluate(()=>{
    const PM=(window as any).PM;PM.agentFrameCapture=true;PM.quality=1;PM.AutomaticPreview?.cancel();
    const project=PM.mkProject({name:'Tonal controls',w:640,h:360,dur:2});
    const layer=PM.mkLayer('shape',{d:{w:360,h:220,radius:28,color:'#E95A2B'},p:{'position.x':320,'position.y':180}},project);
    const tritone=PM.mkEffect('tritone');tritone.p.midtones.v='#1560DD';
    layer.fx=[tritone,PM.mkEffect('colorBalanceHLS'),PM.mkEffect('brightnessContrast')];project.layers=[layer];PM.replaceProject(project);
    PM.animate(layer,'effect.blendWithOriginal',[{t:0,v:100},{t:1,v:0}],{prop:tritone.p.blendWithOriginal});
    const sample=(time:number)=>[...PM.GL.renderToPixels(time,32,18,{mblur:false})];
    const before=[sample(0),sample(1)];PM.replaceProject(PM.hydrateProject(JSON.parse(JSON.stringify(PM.proj))));
    const after=[sample(0),sample(1)];
    PM.agentFrameCapture=false;PM.selectLayers(PM.proj.layers[0].id);PM.setTime(1,{force:true});PM.invalidate();
    return{before,after,keys:PM.proj.layers[0].fx[0].p.blendWithOriginal.kf.length};
  });
  expect(result.before).toEqual(result.after);expect(result.before[0]).not.toEqual(result.before[1]);expect(result.keys).toBe(2);
  for(const type of ['tritone','colorBalanceHLS','brightnessContrast','blackWhite']){
    const id=await session.page.evaluate(type=>{const PM=(window as any).PM,layer=PM.proj.layers[0],fx=PM.mkEffect(type);layer.fx=[fx];PM.UIState.setFxOpen(fx,true);PM.touch();PM.bus.emit('layers');PM.invalidate();return fx.id;},type);
    const section=session.page.locator(`[data-effect-id="${id}"]`);await expect(section).toBeVisible();
    if(type==='blackWhite'){
      const group=session.page.locator('details.control-disclosure').filter({has:session.page.locator('summary',{hasText:/^Color Weights/})});
      await group.locator('summary').click();await expect(group.getByText('Reds',{exact:true})).toBeVisible();
    }
    for(const theme of ['light','dark']){
      await session.page.evaluate(theme=>{const PM=(window as any).PM;PM.theme.apply(theme);PM.invalidate('render');},theme);
      await session.page.waitForFunction(()=>{const PM=(window as any).PM;return !PM.GL.compiling&&PM.GL.errors.size===0;});
      await session.page.screenshot({path:testInfo.outputPath(`tonal-${type}-${theme}.png`)});
    }
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
