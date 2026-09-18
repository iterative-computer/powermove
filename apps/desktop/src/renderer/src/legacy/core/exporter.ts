import { animateExportPreview } from './export-preview';
import { prepareFrame } from './frame-preparation';
import { installRenderQueue, wavBytes } from './render-queue';
/* Ported from js/core/exporter.js — behavior-preserving. */
import {
  EXPORT_FORMAT_OPTIONS,
  EXPORT_FRAME_RATES,
  EXPORT_QUALITY_OPTIONS,
  EXPORT_RANGE_OPTIONS,
  EXPORT_SCALE_OPTIONS,
  clampExportScale,
  exportActionLabel,
  exportBitrateMbps,
  exportFieldSupport,
  normalizeExportDefaults,
  planExport
} from '../../core/export-defaults';
import type { PMRegistry } from '../registry';
import { packProjectFileBlob } from './project-file';
import { buildWebExport, inspectWebExport } from '../../player/export-web';
import { viewerService } from './services';
import { createSettingsSection } from '../ui/settings-section';
import { row as settingsRow, select as settingsSelect, numberInput as settingsNumberInput, type SettingsRow } from '../ui/project-settings';
import { mountSquircles } from '../../settings/squircle';

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
X.buildWeb = () => buildWebExport(PM);

/* The project owns its delivery settings; the legacy global store only seeds
   projects saved before Settings › Project existed. */
X.defaults = () => normalizeExportDefaults(
  PM.proj?.exportDefaults || PM.store.get('exportOpts', {}), PM.proj?.fps);

X.dialog = () => {
  const p: any = PM.proj;
  const opts: any = Object.assign({}, X.defaults(), { name: p.name });
  /* The dialog is the Settings › Project page in a sheet: the same group
     cards, title-and-description rows and 30px hairline controls, so export
     reads as one surface with the place its defaults come from. */
  const body: any = h('div.export-form.sg-column');
  body.addEventListener('keydown', (event: KeyboardEvent) => event.stopPropagation());
  const select = (label: string, get: any, set: any, options: any[]) => {
    const field: any = settingsSelect(label);
    field.classList.add('export-select');
    field.sync = () => {
      field.replaceChildren(...options.map((option: any) => h('option', { value: String(option.v) }, option.label)));
      field.value = String(get());
    };
    field.onchange = () => { const option = options.find(o => String(o.v) === field.value); if (option) set(option.v); field.sync(); };
    field.sync();
    return field;
  };
  /* shadcn's Switch, as on the Settings pages. */
  const toggle = (get: any, set: any, label: string) => {
    const field: any = h('button.toggle', { type: 'button', role: 'switch', 'aria-label': label }, h('i'));
    field.sync = () => { const value = !!get(); field.classList.toggle('on', value); field.setAttribute('aria-checked', String(value)); };
    field.onclick = () => { set(!get()); field.sync(); };
    field.sync();
    return field;
  };
  const category = (format: string) => format === 'web' ? 'code' : format === 'json' ? 'project' : ['png', 'still'].includes(format) ? 'images' : 'video';
  const remembered: Record<string, string> = { code: 'web', video: 'mp4', images: 'still', project: 'json' };
  remembered[category(opts.format)] = opts.format;

  const destinations = [
    ['code', 'Code', 'A live animation for apps and coding agents.'],
    ['video', 'Video', 'MP4, WebM or ProRes for delivery and editing.'],
    ['images', 'Images', 'A single frame, or every frame as a PNG sequence.'],
    ['project', 'Project', 'An editable .pmv with every layer and its media.'],
  ] as const;
  const choices = h('div.export-destinations', { role: 'group', 'aria-label': 'Export type' });
  const buttons: Record<string, HTMLButtonElement> = {};
  for (const [id, title] of destinations) {
    const button = h('button.export-destination', { type: 'button', 'aria-label': title }, title);
    button.onclick = () => { remembered[category(opts.format)] = opts.format; opts.format = remembered[id]; sync(); };
    buttons[id] = button; choices.append(button);
  }
  const destinationNote = h('p.export-destination-note');
  body.append(choices, destinationNote);

  const about = (title: string, copy: string, includes: string[], delivery?: string) => h('div.sg-group.export-about',
    h('div.export-about-copy', h('b', title), h('p', copy),
      h('div.export-code-includes', ...includes.map((item) => h('span', item))),
      delivery ? h('p.export-code-delivery', delivery) : null));
  const codeDetails = about('Put your animation in an app',
    'Export a live animation with playback controls and editable text and colors.',
    ['JavaScript player', 'React component', 'Agent handoff'],
    'One ZIP with your scene, assets and generated effects. Give it to a developer or coding agent to integrate.');
  const projectDetails = about('Keep every layer editable',
    'Save a portable Powermove project with its media, effects, keyframes and composition settings.',
    ['Editable layers', 'Original media', 'Animation & effects']);
  body.append(codeDetails, projectDetails);

  const hasWC: any = typeof window.VideoEncoder !== 'undefined';
  const plan: any = () => planExport(opts, { w: p.w, h: p.h, dur: p.dur, work: p.work });
  const rows: Record<string, SettingsRow> = {};
  const output = createSettingsSection('Output');
  const rendering = createSettingsSection('Rendering');
  const presetSection = createSettingsSection('Presets');
  body.append(output.element, rendering.element, presetSection.element);
  const mk = (section: { body: HTMLElement }, key: string, title: string, detail: string, control: HTMLElement): SettingsRow => {
    const r = settingsRow(title, detail, control);
    rows[key] = r;
    section.body.append(r.element);
    return r;
  };

  const formatOptions: any[] = [];
  const formatField = select('Format', () => opts.format, (v: any) => { opts.format = v; sync(); }, formatOptions);
  const allFormats = EXPORT_FORMAT_OPTIONS.map((o: any) => {
      if (o.v === 'mp4') return { v: o.v, label: (window as any).powermove?.render ? 'MP4 · H.264 (frame-exact)' : 'MP4 · H.264 (real-time)' };
      if (o.v === 'webm' && hasWC) return { v: o.v, label: 'WebM · VP9 (frame-exact)' };
      return { ...o };
    });
  mk(output, 'format', 'Format', 'Container and codec of the exported file.', formatField);

  /* Resolution: the common multiples of the composition, or any pixel size.
     Custom sizes keep the composition's aspect so nothing renders stretched. */
  const CUSTOM: any = '__custom__';
  const presetScales: any = EXPORT_SCALE_OPTIONS.map((o: any) => o.v);
  let customOpen: any = !presetScales.includes(opts.scale);
  const scaleField: any = select('Resolution',
    () => (customOpen ? CUSTOM : opts.scale),
    (v: any) => {
      customOpen = v === CUSTOM;
      if (!customOpen) opts.scale = v;
      sync();
    },
    [
      ...EXPORT_SCALE_OPTIONS.map((o: any) => ({
        v: o.v, label: `${o.label} · ${Math.round(p.w * o.v)}×${Math.round(p.h * o.v)}`
      })),
      { v: CUSTOM, label: 'Custom size…' },
    ]);
  mk(output, 'scale', 'Resolution', 'Rendered size relative to the composition, or any pixel size.', scaleField);
  const wIn: any = settingsNumberInput('Export width in pixels', 16, 8192, 2);
  const hIn: any = settingsNumberInput('Export height in pixels', 16, 8192, 2);
  const sizeRow = mk(output, 'size', 'Size', 'Exact pixel size to render; keeps the composition aspect.', h('div.settings-field-pair', wIn, h('span', '×'), hIn));
  const applyCustom: any = (fromWidth: any) => {
    const typed: any = Number(fromWidth ? wIn.value : hIn.value);
    if (!Number.isFinite(typed) || typed < 2) return sync();
    const base: any = fromWidth ? p.w : p.h;
    opts.scale = clampExportScale(Math.min(8192, Math.max(16, Math.round(typed))) / base, opts.scale);
    sync();
  };
  wIn.addEventListener('change', () => applyCustom(true));
  hIn.addEventListener('change', () => applyCustom(false));
  for (const inp of [wIn, hIn]) inp.addEventListener('keydown', (e: any) => {
    e.stopPropagation();
    if (e.key === 'Enter') inp.blur();
  });

  mk(output, 'fps', 'Frame rate', 'Frames written per second of exported video.', select('Frame rate', () => opts.fps, (v: any) => { opts.fps = v; sync(); }, EXPORT_FRAME_RATES.map((f: any) => ({ v: f, label: (Number.isInteger(f)?f:f.toFixed(3)) + ' fps' }))));
  const hasWork: any = !!(p.work && p.work[1] > p.work[0]);
  mk(output, 'range', 'Range', 'Export the work area or the whole composition.', select('Range', () => opts.range, (v: any) => { opts.range = v; sync(); },
    EXPORT_RANGE_OPTIONS.map((o: any) => o.v === 'work' && !hasWork
      ? { v: o.v, label: 'Work area · not set' } : { ...o })));
  mk(output, 'quality', 'Quality', 'Video bitrate used for video exports.', select('Quality', () => opts.quality, (v: any) => { opts.quality = v; sync(); }, EXPORT_QUALITY_OPTIONS.map((o: any) => ({ ...o }))));
  mk(rendering, 'mblur', 'Motion blur', 'Render motion blur for moving layers.', toggle(() => opts.mblur, (v: any) => { opts.mblur = v; sync(); }, 'Motion blur'));
  const hasAudio: any = PM.Audio.hasAudibleLayers(p);
  mk(rendering, 'audio', 'Include audio', hasAudio ? 'Mix composition audio into the video.' : 'No audio layers in this project.',
    toggle(() => opts.audio !== false, (v: any) => { opts.audio = v; sync(); }, 'Include audio'));
  mk(rendering, 'alpha', 'Transparent background', 'Keep the background see-through, with straight alpha.',
    toggle(() => !!opts.alpha, (v: any) => { opts.alpha = v; sync(); }, 'Transparent background'));

  let presets=PM.store.get('renderPresets',[]);
  let chosenPreset='';const presetOptions=[{v:'',label:'Choose preset'},...presets.map((preset:any)=>({v:preset.id,label:preset.name}))];
  const presetSelect=select('Saved preset',()=>chosenPreset,(id:any)=>{chosenPreset=id;const preset=presets.find((p:any)=>p.id===id);if(preset){Object.assign(opts,preset.options);for(const r of Object.values(rows))r.element.querySelectorAll('*').forEach((c:any)=>c.sync?.());customOpen=!presetScales.includes(opts.scale);sync();}},presetOptions);
  const presetName=h('input.settings-input.is-wide.export-preset-name',{type:'text',placeholder:'Preset name','aria-label':'Render preset name'});
  const savePreset=h('button.btn','Save');savePreset.onclick=()=>{const name=presetName.value.trim();if(!name)return;const next=presets.filter((p:any)=>p.name!==name);const {name:outputName,...settings}=opts;const id=PM.uid('preset');next.push({id,name,options:settings});presets=next;chosenPreset=id;presetOptions.splice(1,presetOptions.length-1,...next.map((p:any)=>({v:p.id,label:p.name})));presetSelect.sync?.();PM.store.set('renderPresets',next);PM.toast('Render preset saved');};
  presetName.addEventListener('keydown', (e: KeyboardEvent) => { if (e.key === 'Enter') { e.preventDefault(); savePreset.click(); } });
  const presetRow = mk(presetSection, 'preset', 'Saved preset', 'Load a set of export settings you saved earlier.', presetSelect);
  const saveRow = mk(presetSection, 'presetSave', 'Save as preset', 'Name these settings to reuse them in any project.', h('div.export-preset-save', presetName, savePreset));
  const colorNote = h('p.settings-note','Color: sRGB. ProRes uses tagged BT.709 primaries and sRGB transfer. Transparent output uses straight alpha.');
  presetSection.element.append(colorNote);
  const nfoMain: any = h('b');
  const nfoNote: any = h('span');
  const nfo: any = h('div.export-summary', nfoMain, nfoNote);
  body.appendChild(nfo);

  let m: any = null;
  function sync() {
    const kind = category(opts.format);
    for (const [id, button] of Object.entries(buttons)) {
      button.setAttribute('aria-pressed', String(id === kind));
      button.classList.toggle('on', id === kind);
    }
    destinationNote.textContent = destinations.find(([id]) => id === kind)?.[2] ?? '';
    codeDetails.hidden = kind !== 'code';
    projectDetails.hidden = kind !== 'project';
    body.querySelectorAll('select, [role=switch]').forEach((field: any) => field.sync?.());
    const settingsFree = kind === 'code' || kind === 'project';
    formatOptions.splice(0, formatOptions.length, ...allFormats.filter(o => category(o.v) === kind));
    formatField.sync?.();
    const support: any = exportFieldSupport(opts.format);
    for (const key of Object.keys(rows)) {
      if (key === 'format' || key === 'size' || key === 'preset' || key === 'presetSave') continue;
      rows[key]!.element.hidden = !support[key];
    }
    rows.format!.element.hidden = settingsFree;
    sizeRow.element.hidden = !support.scale || !customOpen;
    presetRow.element.hidden = saveRow.element.hidden = settingsFree;
    for (const section of [output, rendering, presetSection]) {
      section.element.hidden = settingsFree || ![...section.body.children].some((child) => !(child as HTMLElement).hidden);
    }
    const est: any = plan();
    if (customOpen) {
      if (window.document.activeElement !== wIn) wIn.value = String(est.width);
      if (window.document.activeElement !== hIn) hIn.value = String(est.height);
      scaleField.sync?.();
    }
    if (opts.format === 'web') {
      nfoMain.textContent = `${p.name || 'Untitled'}-web.zip`;
    } else if (opts.format === 'json') {
      nfoMain.textContent = `${p.name || 'Untitled'}.pmv`;
    } else if (opts.format === 'still') {
      nfoMain.textContent = `${est.width}×${est.height} · frame at ${PM.tc(PM.time, p.fps)}`;
    } else {
      nfoMain.textContent = `${est.frames} frames · ${PM.round(est.seconds, 2)}s · ${est.width}×${est.height}`;
    }
    nfoNote.textContent = est.note;
    if (opts.format === 'web') {
      const report = inspectWebExport(PM);
      nfoNote.textContent = report.errors.length ? report.errors.join(' · ') : est.note + (report.fonts.size ? ' · System fonts may need to be supplied in your app' : '');
    }
    const pri: any = m?.el?.querySelector('.mf .btn.pri');
    if (pri) pri.textContent = exportActionLabel(opts.format);
  }
  sync();
  let unmountSquircles = () => {};
  m = PM.modal({
    title: 'Export', body, width: 560,
    onClose: () => unmountSquircles(),
    actions: [{ label: 'Cancel' }, { label: exportActionLabel(opts.format), pri: true, run: () => { X.remember(opts); window.setTimeout(() => void run(opts), 0); } }],
  });
  m.el.classList.add('export-modal');
  sync();
  try { unmountSquircles = mountSquircles(body); } catch { /* The sheet still reads without smoothed corners. */ }
};

/** Keep the dialog's last choices as this project's export settings. */
X.remember = (opts: any) => {
  const next: any = normalizeExportDefaults(opts, PM.proj?.fps);
  PM.store.set('exportOpts', next);
  PM.exportDefaults?.write?.(next);
};

function range(opts: any) {
  const p: any = PM.proj;
  return opts.range === 'work' && p.work && p.work[1] > p.work[0] ? [p.work[0], p.work[1]] : [0, p.dur];
}

function progressUI(total: any) {
  const bar: any = h('i', { style: { width: '0%' } });
  const label: any = h('span', 'Preparing…');
  const eta: any = h('span');
  const prev: any = h('canvas.export-preview');
  const scan = h('canvas.export-scan', { 'aria-hidden': 'true' });
  let stopPreview = () => {};
  const body: any = h('div.export-progress',
    h('div.export-preview-stage', prev, scan), h('div.bar', { role: 'progressbar', 'aria-label': 'Export progress', 'aria-valuemin': '0', 'aria-valuemax': '100', 'aria-valuenow': '0', style: { height: '4px' } }, bar), h('div.export-progress-label', label, eta));
  const mod: any = PM.modal({
    title: 'Exporting ' + (PM.proj.name || 'Untitled'), body, width: 580,
    onClose: () => { X.cancel = true; stopPreview(); },
    actions: [{ label: 'Cancel', run: () => { X.cancel = true; } }],
  });
  mod.el.classList.add('export-modal');
  try { stopPreview = animateExportPreview(scan); } catch { scan.hidden = true; }
  const started: any = window.performance.now();
  return {
    mod, prev,
    set(i: any, extra: any) {
      bar.parentElement.setAttribute('aria-valuenow', String(Math.round(i / total * 100)));
      bar.style.width = (i / total * 100).toFixed(1) + '%';
      label.textContent = `Frame ${i} / ${total}  ·  ${(i / total * 100).toFixed(0)}%` + (extra ? '  ·  ' + extra : '');
      const elapsed: any = (window.performance.now() - started) / 1000;
      /* ETA appears once there is enough signal for it not to jitter. */
      if (i >= Math.min(total, 8) && i < total && elapsed > 1) {
        const left: any = elapsed / i * (total - i);
        eta.textContent = left >= 90 ? `~${Math.round(left / 60)} min left` : `~${Math.max(1, Math.round(left))}s left`;
      } else if (i >= total) {
        eta.textContent = 'Finishing…';
      }
    },
  };
}

async function deliver(blob: Blob, name: string): Promise<boolean> {
  const result = await PM.download(blob, name);
  if (result?.cancelled) return false;
  if (result?.ok === false) throw new Error(result.error || 'Save failed');
  return true;
}

async function run(opts: any) {
  if (X.busy) return PM.toast('Export already running');
  const p: any = PM.proj;
  const [t0, t1]: any = range(opts);
  const W: any = Math.round(p.w * opts.scale / 2) * 2, H: any = Math.round(p.h * opts.scale / 2) * 2;

  if (opts.format === 'web') {
    X.busy = true;
    try {
      await PM.app?.importQueue;
      const result = await buildWebExport(PM);
      const name = (p.name || 'powermove') + '-web.zip';
      const bridge = (window as any).powermove;
      if (bridge?.saveFile) {
        const saved = await bridge.saveFile({ name, data: result.bytes });
        if (!saved.ok) {
          if (saved.cancelled) return { cancelled: true };
          throw new Error(saved.error || 'Could not save web animation');
        }
      } else if (!await deliver(new Blob([result.bytes], { type: 'application/zip' }), name)) return { cancelled: true };
      PM.toast(result.scene.warnings.length ? 'Web animation exported · see README for compatibility notes' : 'Web animation exported');
      return { cancelled: false };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      PM.toast('Could not export web animation: ' + message, 8000);
      return { error: message };
    } finally { X.busy = false; }
  }

  if (opts.format === 'json') {
    X.busy = true;
    try {
      await PM.app?.importQueue;
      const data = await packProjectFileBlob({ ...JSON.parse(PM.serialize()), history: PM.hist.export?.() }, PM.MediaStore);
      if (!await deliver(data, (p.name || 'powermove') + '.pmv')) return { cancelled: true };
      PM.toast('Project exported');return {cancelled:false};
    } catch (error) { const message=error instanceof Error?error.message:String(error);PM.toast('Could not export project: '+message,6000);return {error:message}; }
    finally { X.busy = false; }
  }
  if (opts.format === 'still') {
    const wasPlaying=PM.playing,at=PM.time;PM.pause();X.busy=true;
    try{await prepareFrame(PM,at);
      const cv: any = opts.alpha ? alphaFrame(at, W, H, opts.mblur) : PM.renderFrameTo(at, W, H);
      const blob=await new Promise<Blob>((resolve,reject)=>cv.toBlob((b:Blob|null)=>b?resolve(b):reject(new Error('Could not encode frame'))));
      if (!await deliver(blob, `${p.name}_${PM.tc(at, p.fps).replace(/:/g, '-')}.png`)) return { cancelled: true };
      PM.toast('Frame exported');return {cancelled:false};
    }catch(error){const message=(error as Error).message;PM.toast(message,6000);return {error:message};}
    finally{X.busy=false;PM.preparedVideoFrames=null;PM.setTime(at,{force:true});viewerService(PM)?.layout();if(wasPlaying)PM.play();}
  }

  X.busy = true; X.cancel = false;
  const wasPlaying: any = PM.playing;
  const oldT: any = PM.time, oldQ: any = PM.quality;
  const total: any = Math.max(1, Math.round((t1 - t0) * opts.fps));
  let ui: any;
  const bitrate: any = exportBitrateMbps(opts.quality) * 1e6;
  const t: any = window.performance.now();

  try {
    PM.pause();
    ui = progressUI(total);
    const pctx = ui.prev.getContext('2d');
    if (!pctx) throw new Error('Could not create export preview');
    ui.prev.width = 640; ui.prev.height = Math.round(640 * H / W);
    const wantsAudio: any = opts.audio !== false && PM.Audio.hasAudibleLayers(PM.proj);
    const needsRecorderAudio: any = opts.format === 'webm' && wantsAudio && !(await PM.Audio.supportsOpus());
    if(opts.format==='prores'||(opts.format==='mp4'&&(window as any).powermove?.render)){
      await exportNative({opts,W,H,t0,t1,total,ui,pctx});
    } else if (opts.format === 'mp4' || opts.format === 'rec' || (opts.format === 'webm' && (typeof window.VideoEncoder === 'undefined' || needsRecorderAudio))) {
      await exportRecorder({ opts, W, H, t0, t1, total, ui, pctx, bitrate });
    } else if (opts.format === 'png') {
      await exportPNGs({ opts, W, H, t0, total, ui, pctx });
    } else {
      await exportWebCodecs({ opts, W, H, t0, t1, total, ui, pctx, bitrate });
    }
    if (X.cancel) PM.toast('Export cancelled');
    else PM.toast(`Export finished in ${((window.performance.now() - t) / 1000).toFixed(1)}s`, 3400);
    return {cancelled:X.cancel};
  } catch (e: any) {
    window.console.error(e);
    PM.toast('Export failed: ' + e.message, 5000);
    return {error:e.message};
  } finally {
    X.busy = false;
    ui?.mod.close();
    PM.preparedVideoFrames=null;
    PM.quality = oldQ;
    PM.setTime(oldT, { force: true });
    viewerService(PM)?.layout();
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

async function exportNative({opts,W,H,t0,t1,total,ui,pctx}:any) {
  const bridge=(window as any).powermove?.render;if(!bridge)throw new Error('The native encoder requires the updated desktop runtime.');
  const token=await bridge.start({width:W,height:H,fps:opts.fps,format:opts.format,alpha:!!opts.alpha,bitrateMbps:exportBitrateMbps(opts.quality),name:opts.name||PM.proj.name});let released=false;
  const chunks=async(bytes:Uint8Array,audio=false)=>{for(let at=0;at<bytes.length;at+=4*1024*1024)await bridge.write(token,bytes.slice(at,at+4*1024*1024),audio);};
  try{for(let i=0;i<total;i++){if(X.cancel)break;const T=t0+i/opts.fps;await prepareFrame(PM,T);const cv=opts.alpha?alphaFrame(T,W,H,opts.mblur):PM.renderFrameTo(T,W,H,{mblur:opts.mblur});const bytes=new Uint8Array(cv.getContext('2d').getImageData(0,0,W,H).data);await chunks(bytes);pctx.drawImage(cv,0,0,ui.prev.width,ui.prev.height);ui.set(i+1,opts.format==='prores'?'ProRes 4444':'H.264');await new Promise(r=>setTimeout(r,0));}
    if(X.cancel)return;if(opts.audio!==false&&PM.Audio.hasAudibleLayers(PM.proj)){const mix=await PM.Audio.renderOffline(t0,t1);if(mix)await chunks(wavBytes(mix),true);}const result=await bridge.finish(token);released=true;if(result.cancelled)X.cancel=true;else if(!result.path)throw new Error('Could not save video');
  }finally{if(!released)await bridge.cancel(token).catch(()=>undefined);}
}

async function exportWebCodecs({ opts, W, H, t0, t1, total, ui, pctx, bitrate }: any) {
  const frames: any = [];
  let configured: any = false;
  let encoderError: Error | null = null;
  const enc: any = new window.VideoEncoder({
    output: (chunk: any) => {
      const data: any = new Uint8Array(chunk.byteLength);
      chunk.copyTo(data);
      frames.push({ ts: chunk.timestamp, key: chunk.type === 'key', data });
    },
    error: (e: any) => { encoderError = e; },
  });
  try {
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
    await prepareFrame(PM,T);
    const cv: any = renderInto(T, W, H, opts.mblur);
    const frame: any = new window.VideoFrame(cv, { timestamp: Math.round(i / opts.fps * 1e6), duration: Math.round(1e6 / opts.fps) });
    try { if (encoderError) throw encoderError; enc.encode(frame, { keyFrame: i % Math.round(opts.fps * 2) === 0 }); } finally { frame.close(); }
    if (i % 3 === 0) {
      pctx.drawImage(cv, 0, 0, ui.prev.width, ui.prev.height);
      ui.set(i + 1, 'Rendering');
      await new Promise((r: any) => window.setTimeout(r, 0));
    }
    if (enc.encodeQueueSize > 12) await new Promise((r: any) => window.setTimeout(r, 6));
  }
  await enc.flush();
  if (encoderError) throw encoderError;
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
  if (!await deliver(blob, `${PM.proj.name || 'powermove'}.webm`)) X.cancel = true;
  } finally { if (enc.state !== 'closed') enc.close(); }
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
  const isMp4: any = opts.format === 'mp4';
  const types: any = isMp4
    ? (audioMix
      ? ['video/mp4;codecs=avc1.42001f,mp4a.40.2', 'video/mp4;codecs=avc1.42001f', 'video/mp4']
      : ['video/mp4;codecs=avc1.42001f', 'video/mp4'])
    : (audioMix
      ? ['video/webm;codecs=vp9,opus', 'video/webm;codecs=vp8,opus', 'video/webm']
      : ['video/webm;codecs=vp9', 'video/webm;codecs=vp8', 'video/webm']);
  const mime: any = types.find((m: any) => window.MediaRecorder.isTypeSupported(m));
  if (!mime) {
    audioMix && audioMix.stop();
    throw new Error(isMp4 ? 'H.264 MP4 export is unavailable on this Mac' : 'No supported realtime WebM recorder');
  }
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
    await prepareFrame(PM,T);
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
    track?.stop?.();
  }
  if (X.cancel) return;
  const extension: any = isMp4 ? 'mp4' : 'webm';
  const type: any = isMp4 ? 'video/mp4' : 'video/webm';
  if (!await deliver(new window.Blob(chunks, { type }), `${PM.proj.name || 'powermove'}.${extension}`)) X.cancel = true;
}

async function exportPNGs({ opts, W, H, t0, total, ui, pctx }: any) {
  let dir: any = null;
  if ((window as any).showDirectoryPicker) {
    try { dir = await (window as any).showDirectoryPicker({ mode: 'readwrite' }); }
    catch (e: any) {
      if (e?.name === 'AbortError') { X.cancel = true; return; }
      throw e;
    }
  }
  for (let i: any = 0; i < total; i++) {
    if (X.cancel) break;
    const T: any = t0 + i / opts.fps;
    await prepareFrame(PM,T);
    let cv: any;
    if (opts.alpha) cv = alphaFrame(T, W, H, opts.mblur);
    if (!cv) cv = PM.renderFrameTo(T, W, H, { mblur: opts.mblur });
    const blob: Blob = await new Promise((resolve, reject) => cv.toBlob((encoded: Blob | null) =>
      encoded ? resolve(encoded) : reject(new Error('Could not encode frame')), 'image/png'));
    const name: any = `${PM.proj.name || 'frame'}_${String(i).padStart(5, '0')}.png`;
    if (dir) {
      const fh: any = await dir.getFileHandle(name, { create: true });
      const w: any = await fh.createWritable();
      try { await w.write(blob); await w.close(); }
      catch (error) {
        try { await w.abort?.(); } catch { /* Keep the original write error. */ }
        throw error;
      }
    } else if (!await deliver(blob, name)) { X.cancel = true; return; }
    pctx.drawImage(cv, 0, 0, ui.prev.width, ui.prev.height);
    ui.set(i + 1, dir ? 'writing to folder' : 'downloading');
    await new Promise((r: any) => window.setTimeout(r, 0));
  }
}

/* Programmatic export — used by tests and the native menu. */
installRenderQueue(PM,X);
X.run = (opts: any) => run(Object.assign({}, X.defaults(), { name: PM.proj.name }, opts || {}));

/* Test hooks: the muxer is deterministic pure JS, so it is verifiable headlessly. */
X.muxWebM = muxWebM;

/* Await source decoding before an agent reviews a video frame. */
X.snapshotAsync = async (T: number, maxW = 480) => {
  if (X.busy || PM.agentFrameCapture) throw new Error('Wait for the current frame capture or export to finish.');
  const wasPlaying = PM.playing;
  PM.pause();
  PM.agentFrameCapture = true;
  try {
    await prepareFrame(PM, T);
    return X.snapshot(T, maxW);
  } finally {
    PM.agentFrameCapture = false;
    PM.preparedVideoFrames = null;
    PM.invalidate('render');
    if (wasPlaying) PM.play();
  }
};

/* Fast still capture used by the agent's `look` tool. */
X.snapshot = (T: any, maxW: any = 480) => {
  const p: any = PM.proj;
  const s: any = Math.min(1, maxW / p.w);
  const cv: any = PM.renderFrameTo(
    T,
    Math.max(2, Math.round(p.w * s)),
    Math.max(2, Math.round(p.h * s)),
    { mblur: false, mbSamples: 1 },
  );
  return cv.toDataURL('image/jpeg', .74);
};
}
