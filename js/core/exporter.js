/* Powermove — export. WebCodecs VP9 → WebM (deterministic, frame-exact),
   MediaRecorder fallback, PNG sequence, stills, project JSON. */
(() => {
const PM = window.PM, h = PM.h;

/* ── minimal EBML / WebM muxer ─────────────────────────── */
function vint(n, len) {
  if (!len) { len = 1; while (n >= 2 ** (7 * len) - 1) len++; }
  const b = new Uint8Array(len);
  for (let i = len - 1; i >= 0; i--) { b[i] = n & 0xff; n = Math.floor(n / 256); }
  b[0] |= 1 << (8 - len);
  return b;
}
function uintBytes(n) {
  if (n === 0) return new Uint8Array([0]);
  const out = [];
  while (n > 0) { out.unshift(n & 0xff); n = Math.floor(n / 256); }
  return new Uint8Array(out);
}
function floatBytes(f) {
  const b = new Uint8Array(8); new DataView(b.buffer).setFloat64(0, f); return b;
}
function idBytes(id) {
  const out = [];
  let n = id;
  while (n > 0) { out.unshift(n & 0xff); n = Math.floor(n / 256); }
  return new Uint8Array(out);
}
function el(id, payload) {
  /* ArrayBuffer.isView is realm-safe (unlike instanceof), keeping the muxer testable headlessly */
  const data = ArrayBuffer.isView(payload) ? payload : concat(payload);
  return concat([idBytes(id), vint(data.length ?? data.byteLength), data]);
}
function elUnknownSize(id) { return concat([idBytes(id), new Uint8Array([0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])]); }
function concat(arrs) {
  let n = 0; arrs.forEach(a => n += a.length);
  const out = new Uint8Array(n); let o = 0;
  arrs.forEach(a => { out.set(a, o); o += a.length; });
  return out;
}
const str = (s) => new TextEncoder().encode(s);

function muxWebM(frames, { width, height, fps, codecId = 'V_VP9', audio }) {
  const scale = 1e6; // 1ms ticks
  const header = el(0x1A45DFA3, [
    el(0x4286, uintBytes(1)), el(0x42F7, uintBytes(1)),
    el(0x42F2, uintBytes(4)), el(0x42F3, uintBytes(8)),
    el(0x4282, str('webm')), el(0x4287, uintBytes(2)), el(0x4285, uintBytes(2)),
  ]);
  /* merged, time-ordered block stream so clusters interleave video + audio */
  const blocks = frames.map(f => ({ ms: Math.round(f.ts / 1000), track: 1, key: f.key, data: f.data }));
  if (audio && audio.chunks.length) {
    audio.chunks.forEach(b => blocks.push({ ms: Math.round(b.ts / 1000), track: 2, key: false, data: b.data }));
    blocks.sort((a, b) => a.ms - b.ms || a.track - b.track);
  }
  const lastMs = blocks.length ? blocks[blocks.length - 1].ms : 0;
  const durMs = lastMs + 1000 / fps;
  const info = el(0x1549A966, [
    el(0x2AD7B1, uintBytes(scale)),
    el(0x4D80, str('Powermove')), el(0x5741, str('Powermove ' + PM.version)),
    el(0x4489, floatBytes(durMs)),
  ]);
  const tracks = [
    el(0xAE, [
      el(0xD7, uintBytes(1)), el(0x73C5, uintBytes(1)), el(0x83, uintBytes(1)),
      el(0x536E, str('Video')), el(0x86, str(codecId)),
      el(0xE0, [el(0xB0, uintBytes(width)), el(0xBA, uintBytes(height))]),
    ]),
  ];
  if (audio && audio.chunks.length) {
    tracks.push(el(0xAE, [
      el(0xD7, uintBytes(2)), el(0x73C5, uintBytes(2)), el(0x83, uintBytes(2)),
      el(0x536E, str('Audio')), el(0x86, str('A_OPUS')),
      ...(audio.priv ? [el(0x63A2, audio.priv)] : []),
      el(0xE1, [
        el(0xB5, floatBytes(audio.rate || 48000)),
        el(0x9F, uintBytes(audio.channels || 2)),
      ]),
    ]));
  }
  const tracksEl = el(0x1654AE6B, tracks);
  /* one cluster per ~2s keeps block-relative timecodes inside their vint */
  const clusters = [];
  let cur = null, curStart = 0;
  for (const b of blocks) {
    if (!cur || b.ms - curStart > 2000) {
      if (cur) clusters.push(el(0x1F43B675, [el(0xE7, uintBytes(curStart)), ...cur]));
      cur = []; curStart = b.ms;
    }
    const rel = b.ms - curStart;
    const bh = new Uint8Array(4);
    bh[0] = 0x80 | b.track;          // track number, single-byte vint
    bh[1] = (rel >> 8) & 0xff; bh[2] = rel & 0xff;
    bh[3] = b.key ? 0x80 : 0x00;
    cur.push(el(0xA3, concat([bh, b.data])));
  }
  if (cur) clusters.push(el(0x1F43B675, [el(0xE7, uintBytes(curStart)), ...cur]));
  const segment = el(0x18538067, [info, tracksEl, ...clusters]);
  return new Blob([header, segment], { type: 'video/webm' });
}

/* ── export core ───────────────────────────────────────── */
const X = { busy: false, cancel: false };
PM.Export = X;

X.dialog = () => {
  const p = PM.proj;
  const st = PM.store.get('exportOpts', {});
  const opts = Object.assign({ format: 'webm', scale: 1, fps: p.fps, range: 'work', quality: 'high', mblur: true, name: p.name }, st);
  const body = h('div', { style: { display: 'flex', flexDirection: 'column', gap: '4px' } });
  const mk = (label, ctl) => body.appendChild(PM.row(label, ctl));
  const hasWC = typeof VideoEncoder !== 'undefined';

  mk('Format', PM.selectField(() => opts.format, v => { opts.format = v; info(); },
    [{ v: 'webm', label: hasWC ? 'WebM · VP9 (frame-exact)' : 'WebM · VP9' },
     { v: 'rec', label: 'WebM · realtime capture' },
     { v: 'png', label: 'PNG sequence' },
     { v: 'still', label: 'Still frame (PNG)' },
     { v: 'json', label: 'Project file (.pmv)' }]));
  mk('Resolution', PM.selectField(() => opts.scale, v => { opts.scale = v; info(); },
    [{ v: .5, label: 'Half · ' + (p.w / 2 | 0) + '×' + (p.h / 2 | 0) },
     { v: 1, label: 'Full · ' + p.w + '×' + p.h },
     { v: 2, label: '2× · ' + p.w * 2 + '×' + p.h * 2 }]));
  mk('Frame rate', PM.selectField(() => opts.fps, v => { opts.fps = v; info(); }, [24, 25, 30, 50, 60].map(f => ({ v: f, label: f + ' fps' }))));
  mk('Range', PM.selectField(() => opts.range, v => { opts.range = v; info(); },
    [{ v: 'work', label: 'Work area' }, { v: 'all', label: 'Full composition' }]));
  mk('Quality', PM.selectField(() => opts.quality, v => { opts.quality = v; info(); },
    [{ v: 'draft', label: 'Draft · 4 Mbps' }, { v: 'high', label: 'High · 16 Mbps' }, { v: 'max', label: 'Max · 40 Mbps' }]));
  mk('Motion blur', PM.toggleField(() => opts.mblur, v => { opts.mblur = v; }));
  const hasAudio = PM.Audio.hasAudibleLayers(p);
  mk('Include audio', PM.toggleField(() => opts.audio !== false, v => { opts.audio = v; }, { label: hasAudio ? 'Mixes composition audio (WebM)' : 'No audio layers in this project' }));
  mk('Transparent background', PM.toggleField(() => !!opts.alpha, v => { opts.alpha = v; }, { label: 'PNG / still only' }));
  const nfo = h('div', { style: { fontSize: '11px', color: 'var(--tx-3)', padding: '10px 4px 0', lineHeight: 1.7, fontVariantNumeric: 'tabular-nums' } });
  body.appendChild(nfo);
  function info() {
    const [a, b] = range(opts);
    const n = Math.max(1, Math.round((b - a) * opts.fps));
    const mbps = opts.quality === 'draft' ? 4 : opts.quality === 'high' ? 16 : 40;
    nfo.textContent = `${n} frames · ${PM.round(b - a, 2)}s · ${Math.round(p.w * opts.scale)}×${Math.round(p.h * opts.scale)}\n` +
      (opts.format === 'webm' || opts.format === 'rec' ? `≈ ${PM.round(mbps * (b - a) / 8, 1)} MB` : opts.format === 'png' ? `${n} PNG files` : '');
  }
  info();
  const m = PM.modal({
    title: 'Export', body, width: 460,
    actions: [{ label: 'Cancel' }, { label: 'Export', pri: true, run: () => { PM.store.set('exportOpts', opts); run(opts); } }],
  });
};

function range(opts) {
  const p = PM.proj;
  return opts.range === 'work' && p.work && p.work[1] > p.work[0] ? [p.work[0], p.work[1]] : [0, p.dur];
}

function progressUI(total) {
  const bar = h('i', { style: { width: '0%' } });
  const label = h('div', { style: { fontSize: '11.5px', color: 'var(--tx-2)', fontVariantNumeric: 'tabular-nums' } }, 'Preparing…');
  const prev = h('canvas', { style: { width: '100%', borderRadius: '8px', background: '#000', display: 'block' } });
  const body = h('div', { style: { display: 'flex', flexDirection: 'column', gap: '10px' } },
    prev, h('div.bar', { style: { height: '4px' } }, bar), label);
  const mod = PM.modal({
    title: 'Exporting', body, width: 460,
    actions: [{ label: 'Cancel', run: () => { X.cancel = true; } }],
  });
  return {
    mod, prev,
    set(i, extra) {
      bar.style.width = (i / total * 100).toFixed(1) + '%';
      label.textContent = `Frame ${i} / ${total}  ·  ${(i / total * 100).toFixed(0)}%` + (extra ? '  ·  ' + extra : '');
    },
  };
}

async function run(opts) {
  if (X.busy) return PM.toast('Export already running');
  const p = PM.proj;
  const [t0, t1] = range(opts);
  const W = Math.round(p.w * opts.scale / 2) * 2, H = Math.round(p.h * opts.scale / 2) * 2;

  if (opts.format === 'json') {
    PM.download(new Blob([PM.serialize()], { type: 'application/json' }), (p.name || 'powermove') + '.pmv');
    return PM.toast('Project exported');
  }
  if (opts.format === 'still') {
    const cv = opts.alpha ? alphaFrame(PM.time, W, H, opts.mblur) : PM.renderFrameTo(PM.time, W, H);
    cv.toBlob(b => PM.download(b, `${p.name}_${PM.tc(PM.time, p.fps).replace(/:/g, '-')}.png`));
    return PM.toast('Frame exported');
  }

  X.busy = true; X.cancel = false;
  const wasPlaying = PM.playing; PM.pause();
  const oldT = PM.time, oldQ = PM.quality;
  const total = Math.max(1, Math.round((t1 - t0) * opts.fps));
  const ui = progressUI(total);
  const pctx = ui.prev.getContext('2d');
  ui.prev.width = 320; ui.prev.height = Math.round(320 * H / W);
  const bitrate = (opts.quality === 'draft' ? 4 : opts.quality === 'high' ? 16 : 40) * 1e6;
  const t = performance.now();

  try {
    if (opts.format === 'rec' || (opts.format === 'webm' && typeof VideoEncoder === 'undefined')) {
      await exportRecorder({ opts, W, H, t0, t1, total, ui, pctx, bitrate });
    } else if (opts.format === 'png') {
      await exportPNGs({ opts, W, H, t0, total, ui, pctx });
    } else {
      await exportWebCodecs({ opts, W, H, t0, t1, total, ui, pctx, bitrate });
    }
    if (!X.cancel) PM.toast(`Export finished in ${((performance.now() - t) / 1000).toFixed(1)}s`, 3400);
  } catch (e) {
    console.error(e);
    PM.toast('Export failed: ' + e.message, 5000);
  } finally {
    X.busy = false;
    ui.mod.close();
    PM.quality = oldQ;
    PM.setTime(oldT, { force: true });
    PM.Viewer.layout();
    if (wasPlaying) PM.play();
  }
}

function renderInto(T, W, H, mblur) {
  PM.quality = 1;
  PM.GL.resize(W, H);
  PM.GL.render(T, { mblur, mbSamples: 20, shutter: PM.proj.shutter || .5 });
  return PM.GL.canvas;
}

/* Transparent frames: read back the composited FBO (premultiplied, bottom-up)
   and convert to straight-alpha top-down ImageData. */
function alphaFrame(T, W, H, mblur) {
  const px = PM.GL.renderToPixels(T, W, H, {
    transparent: true, mblur: mblur !== false, mbSamples: 20, shutter: PM.proj.shutter || .5,
  });
  if (!px) return null;
  const cv = document.createElement('canvas');
  cv.width = W; cv.height = H;
  const c = cv.getContext('2d');
  const img = c.createImageData(W, H);
  const d = img.data;
  for (let y = 0; y < H; y++) {
    let si = ((H - 1 - y) * W) * 4, di = y * W * 4;
    for (let x = 0; x < W; x++, si += 4, di += 4) {
      const a = px[si + 3];
      if (a === 255) { d[di] = px[si]; d[di + 1] = px[si + 1]; d[di + 2] = px[si + 2]; d[di + 3] = 255; }
      else if (a === 0) { d[di] = d[di + 1] = d[di + 2] = d[di + 3] = 0; }
      else {
        d[di] = Math.min(255, (px[si] * 255 / a) | 0);
        d[di + 1] = Math.min(255, (px[si + 1] * 255 / a) | 0);
        d[di + 2] = Math.min(255, (px[si + 2] * 255 / a) | 0);
        d[di + 3] = a;
      }
    }
  }
  c.putImageData(img, 0, 0);
  return cv;
}

async function exportWebCodecs({ opts, W, H, t0, t1, total, ui, pctx, bitrate }) {
  const frames = [];
  let configured = false;
  const enc = new VideoEncoder({
    output: (chunk) => {
      const data = new Uint8Array(chunk.byteLength);
      chunk.copyTo(data);
      frames.push({ ts: chunk.timestamp, key: chunk.type === 'key', data });
    },
    error: (e) => { throw e; },
  });
  const cfgs = [
    { codec: 'vp09.00.31.08', id: 'V_VP9' },
    { codec: 'vp8', id: 'V_VP8' },
  ];
  let chosen = null;
  for (const c of cfgs) {
    const cfg = { codec: c.codec, width: W, height: H, bitrate, framerate: opts.fps, latencyMode: 'quality' };
    const sup = await VideoEncoder.isConfigSupported(cfg).catch(() => null);
    if (sup && sup.supported) { enc.configure(cfg); chosen = c; configured = true; break; }
  }
  if (!configured) throw new Error('No supported VP9/VP8 encoder — use realtime capture');

  for (let i = 0; i < total; i++) {
    if (X.cancel) break;
    const T = t0 + i / opts.fps;
    const cv = renderInto(T, W, H, opts.mblur);
    const frame = new VideoFrame(cv, { timestamp: Math.round(i / opts.fps * 1e6), duration: Math.round(1e6 / opts.fps) });
    enc.encode(frame, { keyFrame: i % Math.round(opts.fps * 2) === 0 });
    frame.close();
    if (i % 3 === 0) {
      pctx.drawImage(cv, 0, 0, ui.prev.width, ui.prev.height);
      ui.set(i + 1, chosen.id + ' · queue ' + enc.encodeQueueSize);
      await new Promise(r => setTimeout(r, 0));
    }
    if (enc.encodeQueueSize > 12) await new Promise(r => setTimeout(r, 6));
  }
  await enc.flush();
  enc.close();
  if (X.cancel) return;
  ui.set(total, 'muxing…');
  await new Promise(r => setTimeout(r, 16));
  let audioPayload = null, audioNote = '';
  if (opts.audio !== false) {
    ui.set(total, 'mixing audio…');
    await new Promise(r => setTimeout(r, 16));
    const mix = await PM.Audio.renderOffline(t0, t1);
    if (mix) {
      audioPayload = await PM.Audio.encodeOpus(mix);
      if (!audioPayload) audioNote = ' · no system opus encoder';
    }
  }
  const blob = muxWebM(frames, { width: W, height: H, fps: opts.fps, codecId: chosen.id, audio: audioPayload });
  PM.download(blob, `${PM.proj.name || 'powermove'}.webm`);
  if (audioNote) PM.toast(audioNote, 4000);
}

async function exportRecorder({ opts, W, H, t0, t1, total, ui, pctx, bitrate }) {
  PM.GL.resize(W, H);
  const stream = PM.GL.canvas.captureStream(0);
  const track = stream.getVideoTracks()[0];
  const mime = ['video/webm;codecs=vp9', 'video/webm;codecs=vp8', 'video/webm'].find(m => MediaRecorder.isTypeSupported(m));
  const rec = new MediaRecorder(stream, { mimeType: mime, videoBitsPerSecond: bitrate });
  const chunks = [];
  rec.ondataavailable = (e) => e.data.size && chunks.push(e.data);
  const done = new Promise(r => (rec.onstop = r));
  rec.start();
  const frameDur = 1000 / opts.fps;
  const start = performance.now();
  for (let i = 0; i < total; i++) {
    if (X.cancel) break;
    const T = t0 + i / opts.fps;
    const cv = renderInto(T, W, H, opts.mblur);
    if (track.requestFrame) track.requestFrame();
    const target = start + i * frameDur;
    const wait = target - performance.now();
    if (wait > 0) await new Promise(r => setTimeout(r, wait));
    if (i % 3 === 0) { pctx.drawImage(cv, 0, 0, ui.prev.width, ui.prev.height); ui.set(i + 1, 'realtime'); }
  }
  await new Promise(r => setTimeout(r, 120));
  rec.stop();
  await done;
  if (X.cancel) return;
  PM.download(new Blob(chunks, { type: 'video/webm' }), `${PM.proj.name || 'powermove'}.webm`);
}

async function exportPNGs({ opts, W, H, t0, total, ui, pctx }) {
  let dir = null;
  if (window.showDirectoryPicker) {
    try { dir = await window.showDirectoryPicker({ mode: 'readwrite' }); } catch (e) { }
  }
  for (let i = 0; i < total; i++) {
    if (X.cancel) break;
    const T = t0 + i / opts.fps;
    let cv;
    if (opts.alpha) cv = alphaFrame(T, W, H, opts.mblur);
    if (!cv) cv = renderInto(T, W, H, opts.mblur);
    const blob = await new Promise(r => cv.toBlob(r, 'image/png'));
    const name = `${PM.proj.name || 'frame'}_${String(i).padStart(5, '0')}.png`;
    if (dir) {
      const fh = await dir.getFileHandle(name, { create: true });
      const w = await fh.createWritable();
      await w.write(blob); await w.close();
    } else PM.download(blob, name);
    pctx.drawImage(cv, 0, 0, ui.prev.width, ui.prev.height);
    ui.set(i + 1, dir ? 'writing to folder' : 'downloading');
    await new Promise(r => setTimeout(r, 0));
  }
}

/* Programmatic export — used by tests and the native menu. */
X.run = (opts) => run(Object.assign({ format: 'webm', scale: 1, fps: PM.proj.fps, range: 'work', quality: 'high', mblur: true, alpha: false, audio: true, name: PM.proj.name }, opts || {}));

/* Test hooks: the muxer is deterministic pure JS, so it is verifiable headlessly. */
X.muxWebM = muxWebM;

/* Fast still capture used by the agent's `look` tool. */
X.snapshot = (T, maxW = 480) => {
  const p = PM.proj;
  const s = Math.min(1, maxW / p.w);
  /* W/H are composition coordinates, not merely output size. Rendering at
     thumbnail width crops layers positioned in a larger composition. */
  const full = PM.renderFrameTo(T, p.w, p.h);
  if (s === 1) return full.toDataURL('image/jpeg', .74);
  const cv = document.createElement('canvas');
  cv.width = Math.max(2, Math.round(p.w * s));
  cv.height = Math.max(2, Math.round(p.h * s));
  cv.getContext('2d').drawImage(full, 0, 0, cv.width, cv.height);
  return cv.toDataURL('image/jpeg', .74);
};
})();
