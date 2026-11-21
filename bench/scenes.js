// Scene definitions shared by the Electron scene builder (runs inside the Powermove v1 renderer,
// using the app's own factories) and mirrored by ae/build_scenes.jsx. 1920x1080, 30 fps, 10 s.
// Deterministic: no Math.random.
window.__benchScenes = (() => {
  const W = 1920, H = 1080, FPS = 30, DUR = 10;
  const PAL = ['#FF6B1A', '#4C8DFF', '#3FCF8E', '#F2C14E', '#E8E2CF', '#C04CFF', '#FF4C7A', '#2FD3E0'];
  const key = (t, v) => ({ t, v, i: PM.uid('k'), inInterp: 'linear', outInterp: 'linear',
    inEase: { speed: 0, influence: 100 / 3 }, outEase: { speed: 0, influence: 100 / 3 }, autoBezier: false, continuous: false });
  const anim = (L, ch, keys) => { L.p[ch].v = keys[0][1]; L.p[ch].kf = keys.map(([t, v]) => key(t, v)); };
  const set = (L, ch, v) => { L.p[ch].v = v; };
  const fx = (L, type, values) => { const f = PM.mkEffect(type, undefined, L); if (!f) throw new Error('no effect ' + type);
    for (const [k, v] of Object.entries(values)) { if (!f.p[k]) throw new Error(type + ' has no ' + k); f.p[k].v = v; } L.fx.push(f); return f; };
  const blankComp = (id, name, w, h) => ({ id, name, w, h, fps: FPS, dur: DUR, bg: '#000000',
    backgroundFill: PM.normalizeFill({ type: 'none' }, '#000000'), layers: [], assets: {}, markers: [], work: [0, DUR], params: {}, revision: 0, edits: [], shutter: .5 });
  const add = (comp, L) => { comp.layers.unshift(L); return L; };
  // A motion-graphics element: i drives a deterministic layout and timing.
  const element = (comp, i, kind, cw, ch) => {
    const col = i % 6, row = Math.floor(i / 6) % 4;
    const x = cw * (.12 + col * .152), y = ch * (.18 + row * .21);
    const t0 = (i % 10) * .25, color = PAL[i % PAL.length];
    const L = kind === 'text'
      ? PM.mkLayer('text', { name: `Text ${i + 1}`, d: { text: `MOTION ${i + 1}`, size: 64, weight: 700, font: 'Helvetica', color, align: 'center', tracking: 0 } }, comp)
      : PM.mkLayer('shape', { name: `Shape ${i + 1}`, d: { shape: i % 3 === 0 ? 'ellipse' : i % 3 === 1 ? 'rect' : 'star', color, w: 160, h: 160, radius: 18, points: 5 } }, comp);
    anim(L, 'position.x', [[t0, x - 220], [t0 + 2, x], [8, x + 60], [10, x - 40]]);
    anim(L, 'position.y', [[t0, y + 90], [t0 + 2, y], [10, y - 30]]);
    anim(L, 'scale.x', [[t0, 40], [t0 + 1.5, 110], [t0 + 3, 100], [10, 85]]);
    anim(L, 'scale.y', [[t0, 40], [t0 + 1.5, 110], [t0 + 3, 100], [10, 85]]);
    anim(L, 'opacity', [[t0, 0], [t0 + 1, 100], [9, 100], [10, 20]]);
    return add(comp, L);
  };

  function sceneA() {
    const root = PM.mkProject({ name: 'Bench A motion graphics', w: W, h: H, fps: FPS, dur: DUR, bg: '#0C0D12' });
    const pre = [0, 1].map(n => { const c = blankComp(PM.uid('C'), `Precomp ${n + 1}`, 960, 540); root.comps[c.id] = c; return c; });
    // 24 elements in the root, 8 in each precomp: 40 text+shape layers in total, all keyframed.
    let n = 0;
    for (let i = 0; i < 24; i++) element(root, n++, i % 2 ? 'text' : 'shape', W, H);
    for (const c of pre) for (let i = 0; i < 8; i++) element(c, n++, i % 2 ? 'text' : 'shape', 960, 540);
    pre.forEach((c, j) => {
      const L = PM.mkLayer('precomp', { name: c.name, d: { comp: c.id, w: 960, h: 540 } }, root);
      anim(L, 'position.x', [[0, j ? 900 : 60], [10, j ? 960 : 0]]);
      set(L, 'position.y', j ? 500 : 40);
      anim(L, 'scale.x', [[0, 90], [10, 100]]); anim(L, 'scale.y', [[0, 90], [10, 100]]);
      anim(L, 'opacity', [[0, 60], [2, 100]]);
      add(root, L);
    });
    // Gaussian blur on 4 root elements (2 shapes, 2 text).
    const blurred = root.layers.filter(L => L.type !== 'precomp').slice(-4);
    blurred.forEach((L, j) => fx(L, 'blur', { amount: 6 + j * 4 }));
    return root;
  }

  // Edit: 6 clips, cuts at 2.0 / 5.4 / 8.5, crossfades (0.5 s) into clip 3 at 3.2 and clip 5 at 6.5; one audio bed.
  const EDIT = [
    { file: 'clip1.mp4', from: 0, end: 2.0 },
    { file: 'clip2.mp4', from: 2.0, end: 3.7 },
    { file: 'clip3.mp4', from: 3.2, end: 5.4, xfade: .5 },
    { file: 'clip4.mp4', from: 5.4, end: 7.0 },
    { file: 'clip5.mp4', from: 6.5, end: 8.5, xfade: .5 },
    { file: 'clip6.mp4', from: 8.5, end: 10 },
  ];
  function sceneB() {
    const root = PM.mkProject({ name: 'Bench B edit', w: W, h: H, fps: FPS, dur: DUR, bg: '#000000' });
    EDIT.forEach((c, i) => {
      const L = PM.mkLayer('video', { name: `Clip ${i + 1}`, from: c.from, dur: c.end - c.from, d: { asset: 'media:' + c.file, fit: 'cover', w: W, h: H, trim: .5 } }, root);
      if (c.xfade) { const t = PM.mkTransition('crossfade'); t.dur = c.xfade; L.transitionIn = t; }
      add(root, L);
    });
    root.layers.push(PM.mkLayer('audio', { name: 'Music', d: { asset: 'media:music.wav', gain: 1 } }, root));
    return root;
  }

  function sceneC() {
    const root = PM.mkProject({ name: 'Bench C effects stress', w: W, h: H, fps: FPS, dur: DUR, bg: '#101014' });
    for (let i = 0; i < 10; i++) {
      const col = i % 5, row = Math.floor(i / 5);
      const x = 230 + col * 365, y = 330 + row * 420;
      const L = PM.mkLayer('shape', { name: `Solid ${i + 1}`, d: { shape: i % 2 ? 'ellipse' : 'rect', color: PAL[i % PAL.length], w: 300, h: 300, radius: 0 } }, root);
      anim(L, 'position.x', [[0, x - 40], [10, x + 40]]);
      set(L, 'position.y', y);
      anim(L, 'rotation', [[0, 0], [10, 90 + i * 9]]);
      fx(L, 'blur', { amount: 8 });
      fx(L, 'glow', { threshold: 40, radius: 40, intensity: 150 });
      fx(L, 'levels', { inBlack: .1, inWhite: .9, gamma: 1.2 }); // levels uses a 0–1 scale
      add(root, L);
    }
    const adj = PM.mkLayer('adjustment', { name: 'Grade' }, root);
    fx(adj, 'color', { saturation: 30, contrast: 20 });
    add(root, adj);
    return root;
  }
  return { sceneA, sceneB, sceneC, EDIT };
})();
