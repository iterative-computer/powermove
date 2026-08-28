/* Ported from js/core/exporter.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { packProjectFile } from './project-file';

export function install(PM: PMRegistry): void {
const h: any = PM.h;

/* ── minimal EBML / WebM muxer ─────────────────────────── */
function vint(n: any, len?: any) {
  if (!len) { len = 1; while (n >= 2 ** (7 * len) - 1) len++; }
  const b: any = new Uint8Array(len);
  for (let i: any = len - 1; i >= 0; i--) { b[i] = n & 0xff; n = Math.floor(n / 256); }
  b[0] |= 1 << (8 - len);
  return b;
}
function uintBytes(n: any) {
  if (n === 0) return new Uint8Array([0]);
  const out: any = [];
  while (n > 0) { out.unshift(n & 0xff); n = Math.floor(n / 256); }
  return new Uint8Array(out);
}
function floatBytes(f: any) {
  const b: any = new Uint8Array(8); new DataView(b.buffer).setFloat64(0, f); return b;
}
function idBytes(id: any) {
  const out: any = [];
  let n: any = id;
  while (n > 0) { out.unshift(n & 0xff); n = Math.floor(n / 256); }
  return new Uint8Array(out);
}
function el(id: any, payload: any) {
  /* ArrayBuffer.isView is realm-safe (unlike instanceof), keeping the muxer testable headlessly */
  const data: any = ArrayBuffer.isView(payload) ? payload : concat(payload);
  return concat([idBytes(id), vint(data.length ?? data.byteLength), data]);
}
function elUnknownSize(id: any) { return concat([idBytes(id), new Uint8Array([0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])]); }
function concat(arrs: any) {
  let n: any = 0; arrs.forEach((a: any) => n += a.length);
  const out: any = new Uint8Array(n); let o: any = 0;
  arrs.forEach((a: any) => { out.set(a, o); o += a.length; });
  return out;
}
const str: any = (s: any) => new window.TextEncoder().encode(s);

function muxWebM(frames: any, { width, height, fps, codecId = 'V_VP9', audio }: any) {
  const scale: any = 1e6; // 1ms ticks
  const header: any = el(0x1A45DFA3, [
    el(0x4286, uintBytes(1)), el(0x42F7, uintBytes(1)),
    el(0x42F2, uintBytes(4)), el(0x42F3, uintBytes(8)),
    el(0x4282, str('webm')), el(0x4287, uintBytes(2)), el(0x4285, uintBytes(2)),
  ]);
  /* merged, time-ordered block stream so clusters interleave video + audio */
  const blocks: any = frames.map((f: any) => ({ ms: Math.round(f.ts / 1000), track: 1, key: f.key, data: f.data }));
  if (audio && audio.chunks.length) {
    audio.chunks.forEach((b: any) => blocks.push({ ms: Math.round(b.ts / 1000), track: 2, key: false, data: b.data }));
    blocks.sort((a: any, b: any) => a.ms - b.ms || a.track - b.track);
  }
  const lastMs: any = blocks.length ? blocks[blocks.length - 1].ms : 0;
  const durMs: any = lastMs + 1000 / fps;
  const info: any = el(0x1549A966, [
    el(0x2AD7B1, uintBytes(scale)),
    el(0x4D80, str('Powermove')), el(0x5741, str('Powermove ' + PM.version)),
    el(0x4489, floatBytes(durMs)),
  ]);
  const tracks: any = [
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
  const tracksEl: any = el(0x1654AE6B, tracks);
  /* one cluster per ~2s keeps block-relative timecodes inside their vint */
  const clusters: any = [];
  let cur: any = null, curStart: any = 0;
  for (const b of blocks) {
    if (!cur || b.ms - curStart > 2000) {
      if (cur) clusters.push(el(0x1F43B675, [el(0xE7, uintBytes(curStart)), ...cur]));
      cur = []; curStart = b.ms;
    }
    const rel: any = b.ms - curStart;
    const bh: any = new Uint8Array(4);
    bh[0] = 0x80 | b.track;          // track number, single-byte vint
    bh[1] = (rel >> 8) & 0xff; bh[2] = rel & 0xff;
    bh[3] = b.key ? 0x80 : 0x00;
    cur.push(el(0xA3, concat([bh, b.data])));
  }
  if (cur) clusters.push(el(0x1F43B675, [el(0xE7, uintBytes(curStart)), ...cur]));
  const segment: any = el(0x18538067, [info, tracksEl, ...clusters]);
  return new window.Blob([header, segment], { type: 'video/webm' });
}

/* ── export core ───────────────────────────────────────── */
const X: any = { busy: false, cancel: false };
PM.Export = X;

X.dialog = () => {
  const p: any = PM.proj;
  const st: any = PM.store.get('exportOpts', {});
  const opts: any = Object.assign({ format: 'webm', scale: 1, fps: p.fps, range: 'work', quality: 'high', mblur: true, name: p.name }, st);
  const body: any = h('div', { style: { display: 'flex', flexDirection: 'column', gap: '4px' } });
  const mk: any = (label: any, ctl: any) => body.appendChild(PM.row(label, ctl));
  const hasWC: any = typeof window.VideoEncoder !== 'undefined';

  mk('Format', PM.selectField(() => opts.format, (v: any) => { opts.format = v; info(); },
    [{ v: 'webm', label: hasWC ? 'WebM · VP9 (frame-exact)' : 'WebM · VP9' },
     { v: 'rec', label: 'WebM · realtime capture' },
     { v: 'png', label: 'PNG sequence' },
     { v: 'still', label: 'Still frame (PNG)' },
     { v: 'json', label: 'Project file (.pmv)' }]));
  mk('Resolution', PM.selectField(() => opts.scale, (v: any) => { opts.scale = v; info(); },
    [{ v: .5, label: 'Half · ' + (p.w / 2 | 0) + '×' + (p.h / 2 | 0) },
     { v: 1, label: 'Full · ' + p.w + '×' + p.h },
     { v: 2, label: '2× · ' + p.w * 2 + '×' + p.h * 2 }]));
  mk('Frame rate', PM.selectField(() => opts.fps, (v: any) => { opts.fps = v; info(); }, [24, 25, 30, 50, 60].map((f: any) => ({ v: f, label: f + ' fps' }))));
  mk('Range', PM.selectField(() => opts.range, (v: any) => { opts.range = v; info(); },
    [{ v: 'work', label: 'Work area' }, { v: 'all', label: 'Full composition' }]));
  mk('Quality', PM.selectField(() => opts.quality, (v: any) => { opts.quality = v; info(); },
    [{ v: 'draft', label: 'Draft · 4 Mbps' }, { v: 'high', label: 'High · 16 Mbps' }, { v: 'max', label: 'Max · 40 Mbps' }]));
  mk('Motion blur', PM.toggleField(() => opts.mblur, (v: any) => { opts.mblur = v; }));
  const hasAudio: any = PM.Audio.hasAudibleLayers(p);
  mk('Include audio', PM.toggleField(() => opts.audio !== false, (v: any) => { opts.audio = v; }, { label: hasAudio ? 'Mixes composition audio (WebM)' : 'No audio layers in this project' }));
  mk('Transparent background', PM.toggleField(() => !!opts.alpha, (v: any) => { opts.alpha = v; }, { label: 'PNG / still only' }));
  const nfo: any = h('div', { style: { fontSize: '11px', color: 'var(--tx-3)', padding: '10px 4px 0', lineHeight: 1.7, fontVariantNumeric: 'tabular-nums' } });
  body.appendChild(nfo);
  function info() {
    const [a, b]: any = range(opts);
    const n: any = Math.max(1, Math.round((b - a) * opts.fps));
    const mbps: any = opts.quality === 'draft' ? 4 : opts.quality === 'high' ? 16 : 40;
    nfo.textContent = `${n} frames · ${PM.round(b - a, 2)}s · ${Math.round(p.w * opts.scale)}×${Math.round(p.h * opts.scale)}\n` +
      (opts.format === 'webm' || opts.format === 'rec' ? `≈ ${PM.round(mbps * (b - a) / 8, 1)} MB` : opts.format === 'png' ? `${n} PNG files` : '');
  }
  info();
  const m: any = PM.modal({
    title: 'Export', body, width: 460,
    actions: [{ label: 'Cancel' }, { label: 'Export', pri: true, run: () => { PM.store.set('exportOpts', opts); run(opts); } }],
  });
};

function range(opts: any) {
  const p: any = PM.proj;
  return opts.range === 'work' && p.work && p.work[1] > p.work[0] ? [p.work[0], p.work[1]] : [0, p.dur];
}

function progressUI(total: any) {
  const bar: any = h('i', { style: { width: '0%' } });
  const label: any = h('div', { style: { fontSize: '11.5px', color: 'var(--tx-2)', fontVariantNumeric: 'tabular-nums' } }, 'Preparing…');
  const prev: any = h('canvas', { style: { width: '100%', borderRadius: '8px', background: '#000', display: 'block' } });
  const body: any = h('div', { style: { display: 'flex', flexDirection: 'column', gap: '10px' } },
    prev, h('div.bar', { style: { height: '4px' } }, bar), label);
  const mod: any = PM.modal({
    title: 'Exporting', body, width: 460,
    actions: [{ label: 'Cancel', run: () => { X.cancel = true; } }],
  });
  return {
    mod, prev,
    set(i: any, extra: any) {
      bar.style.width = (i / total * 100).toFixed(1) + '%';
      label.textContent = `Frame ${i} / ${total}  ·  ${(i / total * 100).toFixed(0)}%` + (extra ? '  ·  ' + extra : '');
    },
  };
}

async function run(opts: any) {
  if (X.busy) return PM.toast('Export already running');
  const p: any = PM.proj;
  const [t0, t1]: any = range(opts);
  const W: any = Math.round(p.w * opts.scale / 2) * 2, H: any = Math.round(p.h * opts.scale / 2) * 2;

  if (opts.format === 'json') {
    try {
      await PM.app?.importQueue;
      const text = await packProjectFile(PM.serialize(), PM.MediaStore);
      PM.download(new window.Blob([text], { type: 'application/json' }), (p.name || 'powermove') + '.pmv');
      return PM.toast('Project exported');
    } catch (error) { return PM.toast('Could not export project: ' + (error instanceof Error ? error.message : String(error)), 6000); }
  }
  if (opts.format === 'still') {
    const cv: any = opts.alpha ? alphaFrame(PM.time, W, H, opts.mblur) : PM.renderFrameTo(PM.time, W, H);
    cv.toBlob((b: any) => PM.download(b, `${p.name}_${PM.tc(PM.time, p.fps).replace(/:/g, '-')}.png`));
    return PM.toast('Frame exported');
  }

  X.busy = true; X.cancel = false;
  const wasPlaying: any = PM.playing; PM.pause();
  const oldT: any = PM.time, oldQ: any = PM.quality;
  const total: any = Math.max(1, Math.round((t1 - t0) * opts.fps));
  const ui: any = progressUI(total);
  const pctx: any = ui.prev.getContext('2d');
  ui.prev.width = 320; ui.prev.height = Math.round(320 * H / W);
  const bitrate: any = (opts.quality === 'draft' ? 4 : opts.quality === 'high' ? 16 : 40) * 1e6;
  const t: any = window.performance.now();

  try {
    const wantsAudio: any = opts.audio !== false && PM.Audio.hasAudibleLayers(PM.proj);
    const needsRecorderAudio: any = opts.format === 'webm' && wantsAudio && !(await PM.Audio.supportsOpus());
    if (opts.format === 'rec' || (opts.format === 'webm' && (typeof window.VideoEncoder === 'undefined' || needsRecorderAudio))) {
      await exportRecorder({ opts, W, H, t0, t1, total, ui, pctx, bitrate });
    } else if (opts.format === 'png') {
      await exportPNGs({ opts, W, H, t0, total, ui, pctx });
    } else {
      await exportWebCodecs({ opts, W, H, t0, t1, total, ui, pctx, bitrate });
    }
    if (!X.cancel) PM.toast(`Export finished in ${((window.performance.now() - t) / 1000).toFixed(1)}s`, 3400);
  } catch (e: any) {
    window.console.error(e);
    PM.toast('Export failed: ' + e.message, 5000);
  } finally {
    X.busy = false;
    ui.mod.close();
    PM.quality = oldQ;
    PM.setTime(oldT, { force: true });
    PM.Viewer?.layout?.();
    if (wasPlaying) PM.play();
  }
}

function renderInto(T: any, W: any, H: any, mblur: any) {
  PM.quality = 1;
  PM.GL.resize(W, H);
  PM.GL.render(T, { mblur, mbSamples: 20, shutter: PM.proj.shutter || .5 });
  return PM.GL.canvas;
}

/* Transparent frames: read back the composited FBO (premultiplied, bottom-up)
   and convert to straight-alpha top-down ImageData. */
function alphaFrame(T: any, W: any, H: any, mblur: any) {
  const px: any = PM.GL.renderToPixels(T, W, H, {
    transparent: true, mblur: mblur !== false, mbSamples: 20, shutter: PM.proj.shutter || .5,
  });
  if (!px) return null;
  const cv: any = window.document.createElement('canvas');
  cv.width = W; cv.height = H;
  const c: any = cv.getContext('2d');
  const img: any = c.createImageData(W, H);
  const d: any = img.data;
  for (let y: any = 0; y < H; y++) {
    let si: any = ((H - 1 - y) * W) * 4, di: any = y * W * 4;
    for (let x: any = 0; x < W; x++, si += 4, di += 4) {
      const a: any = px[si + 3];
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

async function exportWebCodecs({ opts, W, H, t0, t1, total, ui, pctx, bitrate }: any) {
  const frames: any = [];
  let configured: any = false;
  const enc: any = new window.VideoEncoder({
    output: (chunk: any) => {
      const data: any = new Uint8Array(chunk.byteLength);
      chunk.copyTo(data);
      frames.push({ ts: chunk.timestamp, key: chunk.type === 'key', data });
    },
    error: (e: any) => { throw e; },
  });
  const cfgs: any = [
    { codec: 'vp09.00.31.08', id: 'V_VP9' },
    { codec: 'vp8', id: 'V_VP8' },
  ];
  let chosen: any = null;
  for (const c of cfgs) {
    const cfg: any = { codec: c.codec, width: W, height: H, bitrate, framerate: opts.fps, latencyMode: 'quality' };
    const sup: any = await window.VideoEncoder.isConfigSupported(cfg).catch(() => null);
    if (sup && sup.supported) { enc.configure(cfg); chosen = c; configured = true; break; }
  }
  if (!configured) throw new Error('No supported VP9/VP8 encoder — use realtime capture');

  for (let i: any = 0; i < total; i++) {
    if (X.cancel) break;
    const T: any = t0 + i / opts.fps;
    const cv: any = renderInto(T, W, H, opts.mblur);
    const frame: any = new window.VideoFrame(cv, { timestamp: Math.round(i / opts.fps * 1e6), duration: Math.round(1e6 / opts.fps) });
    enc.encode(frame, { keyFrame: i % Math.round(opts.fps * 2) === 0 });
    frame.close();
    if (i % 3 === 0) {
      pctx.drawImage(cv, 0, 0, ui.prev.width, ui.prev.height);
      ui.set(i + 1, chosen.id + ' · queue ' + enc.encodeQueueSize);
      await new Promise((r: any) => window.setTimeout(r, 0));
    }
    if (enc.encodeQueueSize > 12) await new Promise((r: any) => window.setTimeout(r, 6));
  }
  await enc.flush();
  enc.close();
  if (X.cancel) return;
  ui.set(total, 'muxing…');
  await new Promise((r: any) => window.setTimeout(r, 16));
  let audioPayload: any = null;
  if (opts.audio !== false) {
    ui.set(total, 'mixing audio…');
    await new Promise((r: any) => window.setTimeout(r, 16));
    const mix: any = await PM.Audio.renderOffline(t0, t1);
    if (mix) {
      audioPayload = await PM.Audio.encodeOpus(mix);
      if (!audioPayload) throw new Error('Could not encode the audio track · use realtime capture');
    }
  }
  const blob: any = muxWebM(frames, { width: W, height: H, fps: opts.fps, codecId: chosen.id, audio: audioPayload });
  PM.download(blob, `${PM.proj.name || 'powermove'}.webm`);
}

async function exportRecorder({ opts, W, H, t0, t1, total, ui, pctx, bitrate }: any) {
  PM.GL.resize(W, H);
  const stream: any = PM.GL.canvas.captureStream(0);
  const track: any = stream.getVideoTracks()[0];
  const audioMix: any = opts.audio !== false ? await PM.Audio.createRealtimeMix(t0, t1) : null;
  if (audioMix) {
    const audioTrack: any = audioMix.stream && audioMix.stream.getAudioTracks && audioMix.stream.getAudioTracks()[0];
    if (!audioTrack) { audioMix.stop(); throw new Error('Realtime audio export could not create an audio track'); }
    stream.addTrack(audioTrack);
  }
  const types: any = audioMix
    ? ['video/webm;codecs=vp9,opus', 'video/webm;codecs=vp8,opus', 'video/webm']
    : ['video/webm;codecs=vp9', 'video/webm;codecs=vp8', 'video/webm'];
  const mime: any = types.find((m: any) => window.MediaRecorder.isTypeSupported(m));
  if (!mime) { audioMix && audioMix.stop(); throw new Error('No supported realtime WebM recorder'); }
  const rec: any = new window.MediaRecorder(stream, { mimeType: mime, videoBitsPerSecond: bitrate });
  const chunks: any = [];
  rec.ondataavailable = (e: any) => e.data.size && chunks.push(e.data);
  const done: any = new Promise((r: any) => (rec.onstop = r));
  let recorderStopped: any = false;
  try {
    rec.start();
    const lead: any = audioMix ? audioMix.start(.06) : 0;
    if (lead > 0) await new Promise((r: any) => window.setTimeout(r, lead * 1000));
    const frameDur: any = 1000 / opts.fps;
    const start: any = window.performance.now();
    for (let i: any = 0; i < total; i++) {
      if (X.cancel) break;
      const T: any = t0 + i / opts.fps;
      const cv: any = renderInto(T, W, H, opts.mblur);
      if (track.requestFrame) track.requestFrame();
      const target: any = start + i * frameDur;
      const wait: any = target - window.performance.now();
      if (wait > 0) await new Promise((r: any) => window.setTimeout(r, wait));
      if (i % 3 === 0) { pctx.drawImage(cv, 0, 0, ui.prev.width, ui.prev.height); ui.set(i + 1, audioMix ? 'realtime · audio' : 'realtime'); }
    }
    await new Promise((r: any) => window.setTimeout(r, 120));
    rec.stop();
    await done;
    recorderStopped = true;
  } finally {
    if (!recorderStopped) {
      try { rec.stop(); await done; } catch (error: any) { }
    }
    audioMix && audioMix.stop();
  }
  if (X.cancel) return;
  PM.download(new window.Blob(chunks, { type: 'video/webm' }), `${PM.proj.name || 'powermove'}.webm`);
}

async function exportPNGs({ opts, W, H, t0, total, ui, pctx }: any) {
  let dir: any = null;
  if ((window as any).showDirectoryPicker) {
    try { dir = await (window as any).showDirectoryPicker({ mode: 'readwrite' }); } catch (e: any) { }
  }
  for (let i: any = 0; i < total; i++) {
    if (X.cancel) break;
    const T: any = t0 + i / opts.fps;
    let cv: any;
    if (opts.alpha) cv = alphaFrame(T, W, H, opts.mblur);
    if (!cv) cv = renderInto(T, W, H, opts.mblur);
    const blob: any = await new Promise((r: any) => cv.toBlob(r, 'image/png'));
    const name: any = `${PM.proj.name || 'frame'}_${String(i).padStart(5, '0')}.png`;
    if (dir) {
      const fh: any = await dir.getFileHandle(name, { create: true });
      const w: any = await fh.createWritable();
      await w.write(blob); await w.close();
    } else PM.download(blob, name);
    pctx.drawImage(cv, 0, 0, ui.prev.width, ui.prev.height);
    ui.set(i + 1, dir ? 'writing to folder' : 'downloading');
    await new Promise((r: any) => window.setTimeout(r, 0));
  }
}

/* Programmatic export — used by tests and the native menu. */
X.run = (opts: any) => run(Object.assign({ format: 'webm', scale: 1, fps: PM.proj.fps, range: 'work', quality: 'high', mblur: true, alpha: false, audio: true, name: PM.proj.name }, opts || {}));

/* Test hooks: the muxer is deterministic pure JS, so it is verifiable headlessly. */
X.muxWebM = muxWebM;

/* Fast still capture used by the agent's `look` tool. */
X.snapshot = (T: any, maxW: any = 480) => {
  const p: any = PM.proj;
  const s: any = Math.min(1, maxW / p.w);
  /* W/H are composition coordinates, not merely output size. Rendering at
     thumbnail width crops layers positioned in a larger composition. */
  const full: any = PM.renderFrameTo(T, p.w, p.h);
  if (s === 1) return full.toDataURL('image/jpeg', .74);
  const cv: any = window.document.createElement('canvas');
  cv.width = Math.max(2, Math.round(p.w * s));
  cv.height = Math.max(2, Math.round(p.h * s));
  cv.getContext('2d').drawImage(full, 0, 0, cv.width, cv.height);
  return cv.toDataURL('image/jpeg', .74);
};
}
