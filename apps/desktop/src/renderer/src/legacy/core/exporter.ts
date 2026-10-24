import { captureAgentFrame } from './agent-frame-capture';
import { animateExportPreview } from './export-preview';
import { frameFontsReady, prepareFrame } from './frame-preparation';
import { captureFramePixels, renderOpaqueFrame, writeStraightAlphaPixels } from './frame-capture';
import { trackStaticExport } from './static-export';
import { trackExportMedia } from './export-media';
import { exportEffects, type EffectRef } from './export-effects';
import { AlphaReadbackRetryError } from '../gl/half-readback';
import { installRenderQueue, wavBytes } from './render-queue';
import { WEBM_ENCODE_QUEUE_LIMIT, WEBM_VIDEO_CODECS, webmFrameTiming, webmKeyFrame, webmVideoConfig } from './webm-video';
import { openRenderQueuePanel } from './render-queue-panel';
import { muxWebM as muxEncodedWebM } from './webm-mux';
import { writeBlobChunks } from './blob-chunks';
/* Ported from js/core/exporter.js — behavior-preserving. */
import {
  EXPORT_FRAME_RATES,
  EXPORT_QUALITY_OPTIONS,
  EXPORT_RANGE_OPTIONS,
  EXPORT_SCALE_OPTIONS,
  clampExportScale,
  exportActionLabel,
  exportBitrateMbps,
  exportFieldSupport,
  exportFormatOptions,
  normalizeExportDefaults,
  planExport
} from '../../core/export-defaults';
import {
  allPresets,
  BUILT_IN_PRESETS,
  expandFileName,
  formatInfo,
  isDropFrameTimecode,
  isRenderFormat,
  legacyFormatId,
  migrateFormat,
  normalizeDelivery,
  normalizeTimecode,
  storedFormatId,
  supportsAlpha,
  type ExportPreset,
  type ExportPresetOptions,
  type SheetFormat,
  writableFormat
} from '../../../../shared/delivery';
import type { PMRegistry } from '../registry';
import { packProjectFileBlob } from './project-file';
import { buildWebExport, inspectWebExport } from '../../player/export-web';
import { viewerService } from './services';
import { createSettingsSection } from '../ui/settings-section';
import { mountSquircles, SQUIRCLE_SELECTOR } from '../../settings/squircle';
import { deliveryEngineOn, row as settingsRow, select as settingsSelect, numberInput as settingsNumberInput, type SettingsRow } from '../ui/project-settings';
import { bridge as hostBridge } from '../../kernel/bridge';
import { captionSidecars } from '../../captions/sidecar';

export function install(PM: PMRegistry): void {
const h: any = PM.h;

/* Keep the export hook scoped to this editor's application version. */
const muxWebM = (frames: any, options: any) => muxEncodedWebM(frames, options, PM.version);

/* ── export core ───────────────────────────────────────── */
// Keep a recovered decoder for the rest of this export, so every later frame
// does not have to wait for the same stalled original again.
const recoveryDecoders = new Map<HTMLVideoElement, HTMLVideoElement>();
const prepareExportFrame: typeof prepareFrame = (pm, time, project, options) =>
  prepareFrame(pm, time, project, { ...options, recoveryDecoders });
const X: any = { busy: false, cancel: false };
PM.Export = X;
X.buildWeb = () => buildWebExport(PM);

/* The project owns its delivery settings; the legacy global store only seeds
   projects saved before Settings › Project existed. */
X.defaults = () => normalizeExportDefaults(
  PM.proj?.exportDefaults || PM.store.get('exportOpts', {}), PM.proj?.fps);

/** Whether this launch runs the native engine (Use New Engine): it writes
 *  every delivery format; the current exporter writes today's. */
const engineOn = (): boolean => deliveryEngineOn();

/** The exact export frame rate when it is the composition's own. */
const RATE_LABEL = (f: number) => (Number.isInteger(f) ? String(f) : f.toFixed(3)) + ' fps';

let nextExportHold = 0;
/** Each settings sheet and export preflight owns a separate warm-job lease. */
function holdExportJob(): () => void {
  const engine = engineOn() ? hostBridge()?.nativeEngine : undefined;
  const id = `export-${++nextExportHold}`;
  engine?.exportSheet?.(id, true);
  let released = false;
  return () => {
    if (released) return;
    released = true;
    engine?.exportSheet?.(id, false);
  };
}

X.dialog = () => {
  const p: any = PM.proj;
  const engine = engineOn();
  const opts: any = Object.assign({}, X.defaults(), { name: p.name });
  opts.format = writableFormat(opts.format, engine);
  opts.delivery = normalizeDelivery(opts.delivery);
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
  const text = (label: string, get: () => string, set: (v: string) => void, placeholder = '') => {
    const field: any = h('input.settings-input.is-wide', { type: 'text', placeholder, 'aria-label': label, spellcheck: 'false' });
    field.sync = () => { if (window.document.activeElement !== field) field.value = get(); };
    field.addEventListener('change', () => { set(field.value); sync(); });
    field.addEventListener('keydown', (e: KeyboardEvent) => { e.stopPropagation(); if (e.key === 'Enter') field.blur(); });
    field.sync();
    return field;
  };
  const category = (format: string) => formatInfo(migrateFormat(format)).category;
  const remembered: Record<string, string> = { code: 'web', video: 'h264', images: 'still', project: 'json' };
  remembered[category(opts.format)] = opts.format;

  const destinations = [
    ['code', 'Code', 'A live animation for apps and coding agents.'],
    ['video', 'Video', engine ? 'ProRes, H.264, HEVC or WebM for delivery and editing.' : 'MP4, WebM or ProRes for delivery and editing.'],
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

  const plan: any = () => planExport(opts, { w: p.w, h: p.h, dur: p.dur, work: p.work });
  const rows: Record<string, SettingsRow> = {};
  const output = createSettingsSection('Output');
  const rendering = createSettingsSection('Rendering');
  const delivery = createSettingsSection('Delivery');
  const file = createSettingsSection('File');
  const presetSection = createSettingsSection('Presets');
  body.append(output.element, rendering.element, delivery.element, file.element, presetSection.element);
  const mk = (section: { body: HTMLElement }, key: string, title: string, detail: string, control: HTMLElement): SettingsRow => {
    const r = settingsRow(title, detail, control);
    rows[key] = r;
    section.body.append(r.element);
    return r;
  };

  const formatOptions: any[] = [];
  const allFormats = exportFormatOptions(engine).map((o: any) => {
    if (o.v === 'h264' && !engine) return { v: o.v, label: (hostBridge() as any)?.render ? 'MP4 · H.264 (frame-exact)' : 'MP4 · H.264 (real-time)' };
    return { ...o };
  });
  const formatField = select('Format', () => opts.format, (v: any) => { opts.format = v; sync(); }, formatOptions);
  mk(output, 'format', 'Format', 'Container and codec of the exported file.', formatField);
  mk(output, 'container', 'Container', 'MP4 for the web, or a QuickTime movie with timecode and PCM audio.',
    select('Container', () => opts.delivery.container, (v: any) => { opts.delivery.container = v; sync(); },
      [{ v: 'mp4', label: 'MP4' }, { v: 'mov', label: 'QuickTime (.mov)' }]));
  const depthOptions: any[] = [];
  mk(output, 'highDepth', 'Bit depth', 'More bits keep smooth gradients without banding.',
    select('Bit depth', () => (opts.delivery.highDepth ? 'high' : 'standard'), (v: any) => { opts.delivery.highDepth = v === 'high'; sync(); }, depthOptions));

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

  mk(output, 'fps', 'Frame rate', 'Frames written per second of exported video.', select('Frame rate', () => opts.fps, (v: any) => { opts.fps = v; sync(); }, EXPORT_FRAME_RATES.map((f: any) => ({ v: f, label: RATE_LABEL(f) }))));
  const hasWork: any = !!(p.work && p.work[1] > p.work[0]);
  mk(output, 'range', 'Range', 'Export the work area or the whole composition.', select('Range', () => opts.range, (v: any) => { opts.range = v; sync(); },
    EXPORT_RANGE_OPTIONS.map((o: any) => o.v === 'work' && !hasWork
      ? { v: o.v, label: 'Work area · not set' } : { ...o })));
  mk(output, 'quality', 'Quality', 'Video bitrate used for video exports.', select('Quality', () => opts.quality, (v: any) => { opts.quality = v; sync(); }, EXPORT_QUALITY_OPTIONS.map((o: any) => ({ ...o }))));
  mk(rendering, 'mblur', 'Motion blur', 'Render motion blur for moving layers.', toggle(() => opts.mblur, (v: any) => { opts.mblur = v; sync(); }, 'Motion blur'));
  const hasAudio: any = PM.Audio.hasAudibleLayers(p);
  mk(rendering, 'audio', 'Include audio', hasAudio ? 'Mix composition audio into the video.' : 'No audio layers in this project.',
    toggle(() => opts.audio !== false, (v: any) => { opts.audio = v; sync(); }, 'Include audio'));
  mk(rendering, 'alpha', 'Transparent background', 'Keep the background see-through.',
    toggle(() => !!opts.alpha, (v: any) => { opts.alpha = v; sync(); }, 'Transparent background'));
  mk(rendering, 'alphaMode', 'Alpha', 'Straight alpha for editing apps, or premultiplied over black.',
    select('Alpha', () => opts.delivery.alphaMode, (v: any) => { opts.delivery.alphaMode = v; sync(); },
      [{ v: 'straight', label: 'Straight' }, { v: 'premultiplied', label: 'Premultiplied' }]));

  mk(delivery, 'audioCoding', 'Audio format', 'How the soundtrack is stored in the movie.',
    select('Audio format', () => opts.delivery.audioCoding, (v: any) => { opts.delivery.audioCoding = v; sync(); }, [
      { v: 'auto', label: 'Automatic · 24-bit PCM' }, { v: 'pcm16', label: '16-bit PCM' }, { v: 'pcm24', label: '24-bit PCM' },
      { v: 'float', label: '32-bit float PCM' }, { v: 'aac', label: 'AAC' }
    ]));
  const timecodeRow = mk(delivery, 'timecode', 'Start timecode', 'The first frame’s timecode; empty uses the composition’s.',
    text('Start timecode', () => opts.delivery.timecode, (v) => { opts.delivery.timecode = normalizeTimecode(v); }, '00:00:00:00'));
  mk(delivery, 'reel', 'Reel name', 'Written with the timecode for editing apps.',
    text('Reel name', () => opts.delivery.reel, (v) => { opts.delivery.reel = v.replace(/[\u0000-\u001f]/g, '').trim().slice(0, 64); }));
  mk(delivery, 'videoColor', 'Color', 'Rec.709 for QuickTime and Safari matches the viewer; broadcast for grading.',
    select('Color', () => opts.delivery.videoColor, (v: any) => { opts.delivery.videoColor = v; sync(); },
      [{ v: 'quicktime', label: 'Rec.709 (QuickTime and Safari)' }, { v: 'broadcast', label: 'Rec.709 (broadcast)' }]));
  mk(delivery, 'imageColor', 'Color', 'The color space the image files are tagged with.',
    select('Image color', () => opts.delivery.imageColor, (v: any) => { opts.delivery.imageColor = v; sync(); },
      [{ v: 'srgb', label: 'sRGB' }, { v: 'rec709', label: 'Rec.709' }]));

  mk(file, 'fileName', 'File name', 'Use {project}, {composition}, {preset}, {size}, {fps}, {date}, and {time}.',
    text('File name', () => opts.delivery.fileName, (v) => { opts.delivery.fileName = v.trim() || '{project}'; }, '{project}'));
  mk(file, 'overwrite', 'If the file exists', 'For render queue jobs; exporting now asks.',
    select('If the file exists', () => opts.delivery.overwrite, (v: any) => { opts.delivery.overwrite = v; sync(); },
      [{ v: 'keep-both', label: 'Keep both' }, { v: 'replace', label: 'Replace' }, { v: 'stop', label: 'Stop the job' }]));

  /* Captions: burned in by default, optionally also delivered as a sidecar. */
  const captionLayers = (p.layers || []).filter((layer: any) => layer.type === 'captions');
  const captionsSection = createSettingsSection('Captions');
  if (captionLayers.length) {
    body.insertBefore(captionsSection.element, presetSection.element);
    opts.captionsBurn ??= true;
    opts.captionsSidecar ??= 'none';
    mk(captionsSection, 'captionsBurn', 'Burn in captions', 'Draw captions into the picture.',
      toggle(() => opts.captionsBurn !== false, (v: any) => { opts.captionsBurn = v; sync(); }, 'Burn in captions'));
    mk(captionsSection, 'captionsSidecar', 'Caption file', captionLayers.length > 1
      ? 'Also write one subtitle file per captions layer beside the video.'
      : 'Also write a subtitle file beside the video.',
      select('Caption file', () => opts.captionsSidecar, (v: any) => { opts.captionsSidecar = v; sync(); },
        [{ v: 'none', label: 'None' }, { v: 'srt', label: 'SubRip (.srt)' }, { v: 'vtt', label: 'WebVTT (.vtt)' }]));
  }

  /* Presets: the five built-ins, then the user's (today's saved presets migrate). */
  let presets: ExportPreset[] = allPresets(PM.store.get('renderPresets', [])).filter((preset) => engine || !formatInfo(preset.options.format).engine);
  let chosenPreset = '';
  const presetOptions = [{ v: '', label: 'Choose preset' }, ...presets.map((preset) => ({ v: preset.id, label: preset.name }))];
  const presetSelect = select('Saved preset', () => chosenPreset, (id: any) => {
    chosenPreset = id;
    const preset = presets.find((item) => item.id === id);
    if (!preset) return;
    applyPreset(opts, preset);
    customOpen = !presetScales.includes(opts.scale);
    sync();
  }, presetOptions);
  const presetName = h('input.settings-input.is-wide.export-preset-name', { type: 'text', placeholder: 'Preset name', 'aria-label': 'Render preset name' });
  const savePreset = h('button.btn', 'Save');
  savePreset.onclick = () => {
    const name = presetName.value.trim();
    if (!name) return;
    if (BUILT_IN_PRESETS.some((preset) => preset.name === name)) { PM.toast('Built-in presets keep their name; choose another.'); return; }
    const stored = (PM.store.get('renderPresets', []) as any[]).filter((preset: any) => preset?.name !== name);
    const id = PM.uid('preset');
    // Saved with today's format id where one exists (issue P1); read back through migratePreset.
    const options = presetOptionsOf(opts);
    stored.push({ id, name, options: { ...options, format: storedFormatId(options.format) } });
    PM.store.set('renderPresets', stored);
    presets = allPresets(stored).filter((preset) => engine || !formatInfo(preset.options.format).engine);
    chosenPreset = id;
    presetOptions.splice(1, presetOptions.length - 1, ...presets.map((preset) => ({ v: preset.id, label: preset.name })));
    presetSelect.sync?.();
    PM.toast('Render preset saved');
  };
  presetName.addEventListener('keydown', (e: KeyboardEvent) => { e.stopPropagation(); if (e.key === 'Enter') { e.preventDefault(); savePreset.click(); } });
  const presetRow = mk(presetSection, 'preset', 'Preset', 'Start from a preset, or one you saved earlier.', presetSelect);
  const saveRow = mk(presetSection, 'presetSave', 'Save as preset', 'Name these settings to reuse them in any project.', h('div.export-preset-save', presetName, savePreset));
  const nfoMain: any = h('b');
  const nfoNote: any = h('span');
  const nfo: any = h('div.export-summary', nfoMain, nfoNote);
  body.appendChild(nfo);

  let m: any = null;
  function sync() {
    opts.format = writableFormat(opts.format, engine);
    const kind = category(opts.format);
    for (const [id, button] of Object.entries(buttons)) {
      button.setAttribute('aria-pressed', String(id === kind));
      button.classList.toggle('on', id === kind);
    }
    destinationNote.textContent = destinations.find(([id]) => id === kind)?.[2] ?? '';
    codeDetails.hidden = kind !== 'code';
    projectDetails.hidden = kind !== 'project';
    const settingsFree = kind === 'code' || kind === 'project';
    formatOptions.splice(0, formatOptions.length, ...allFormats.filter(o => category(o.v) === kind));
    depthOptions.splice(0, depthOptions.length, ...(opts.format === 'hevc'
      ? [{ v: 'standard', label: '8-bit' }, { v: 'high', label: '10-bit' }]
      : [{ v: 'standard', label: '8-bit' }, { v: 'high', label: '16-bit' }]));
    body.querySelectorAll('select, [role=switch], input[type=text]').forEach((field: any) => field.sync?.());
    const support: any = exportFieldSupport(opts.format, opts.delivery, engine);
    const sectionFree = new Set(['format', 'size', 'preset', 'presetSave']);
    for (const key of Object.keys(rows)) {
      if (sectionFree.has(key)) continue;
      if (key === 'captionsBurn' || key === 'captionsSidecar') { rows[key]!.element.hidden = kind !== 'video' && !(key === 'captionsBurn' && kind === 'images'); continue; }
      if (key === 'reel') { rows[key]!.element.hidden = !support.timecode; continue; }
      if (key === 'fileName') { rows[key]!.element.hidden = !engine || settingsFree; continue; }
      if (key === 'overwrite') { rows[key]!.element.hidden = !engine || settingsFree; continue; }
      rows[key]!.element.hidden = !support[key];
    }
    timecodeRow.detail.textContent = opts.delivery.timecode && isDropFrameTimecode(opts.delivery.timecode)
      ? 'Drop-frame timecode; it needs 29.97 or 59.94 fps.'
      : 'The first frame’s timecode; empty uses the composition’s.';
    rows.format!.element.hidden = settingsFree;
    sizeRow.element.hidden = !support.scale || !customOpen;
    presetRow.element.hidden = saveRow.element.hidden = settingsFree;
    for (const section of [output, rendering, delivery, file, captionsSection, presetSection]) {
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
    const queue: any = m?.el?.querySelector('.mf .export-queue-action') ?? [...(m?.el?.querySelectorAll('.mf .btn') ?? [])].find((b: any) => b.textContent === 'Add to Queue');
    if (queue) { queue.classList.add('export-queue-action'); queue.hidden = settingsFree; }
  }
  sync();
  let unmountCorners: (() => void) | undefined;
  let releaseJob = () => {};
  const actions: any[] = [{ label: 'Cancel' }];
  actions.push({ label: 'Add to Queue', run: () => { X.remember(opts); window.setTimeout(() => void X.addToQueue(opts), 0); } });
  actions.push({ label: exportActionLabel(opts.format), pri: true, run: () => { X.remember(opts); window.setTimeout(() => void run(opts), 0); } });
  m = PM.modal({
    title: 'Export', body, width: 620,
    onClose: () => {
      releaseJob();
      unmountCorners?.();
    },
    actions,
  });
  if (engine) releaseJob = holdExportJob();
  m.el.classList.add('export-modal');
  unmountCorners = mountSquircles(m.el, `${SQUIRCLE_SELECTOR}, .export-destinations, .export-destination, .btn`);
  sync();
};

/** The options a preset sets (scale, rate, range, quality, switches, and
 *  delivery), applied over the sheet's. */
function applyPreset(opts: any, preset: ExportPreset): void {
  const o = preset.options;
  opts.format = o.format;
  for (const key of ['scale', 'fps', 'range', 'quality', 'mblur', 'audio', 'alpha', 'captionsBurn', 'captionsSidecar'] as const) {
    if ((o as any)[key] !== undefined) opts[key] = (o as any)[key];
  }
  opts.delivery = normalizeDelivery({ ...opts.delivery, ...(o.delivery ?? {}) });
}

/** What a saved preset keeps of the sheet's settings. */
function presetOptionsOf(opts: any): ExportPresetOptions {
  const { name: _name, reportPath: _report, ...rest } = opts;
  return {
    format: migrateFormat(rest.format), scale: rest.scale, fps: rest.fps, range: rest.range, quality: rest.quality,
    mblur: !!rest.mblur, audio: rest.audio !== false, alpha: !!rest.alpha,
    ...(rest.captionsBurn !== undefined ? { captionsBurn: rest.captionsBurn !== false } : {}),
    ...(rest.captionsSidecar ? { captionsSidecar: rest.captionsSidecar } : {}),
    delivery: normalizeDelivery(rest.delivery)
  };
}
X.applyPreset = applyPreset;
X.presetOptionsOf = presetOptionsOf;

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

function progressUI(total: any, preparingMedia = false, operation: { cancel: boolean; busy?: boolean } = X) {
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
    dismissible: false,
    onClose: () => { if (preparingMedia || operation.busy) operation.cancel = true; stopPreview(); },
    actions: [{ label: 'Cancel', run: () => {
      operation.cancel = true;
      label.textContent = 'Cancelling…'; eta.textContent = '';
      const button = mod.el.querySelector('.mf button');
      if (button) { button.disabled = true; button.textContent = 'Cancelling…'; }
      return false;
    } }],
  });
  mod.el.classList.add('export-modal');
  try { stopPreview = animateExportPreview(scan); } catch { scan.hidden = true; }
  const started: any = window.performance.now();
  /* Engine previews: JPEG frames streamed with the job's statuses. */
  let decoding = false;
  return {
    mod, prev,
    set(i: any, extra: any) {
      if (operation.cancel) return;
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
    preparing(done: number, total: number) {
      if (operation.cancel) return;
      const percent = Math.max(0, Math.min(100, done / total * 100));
      bar.parentElement.setAttribute('aria-valuenow', String(Math.round(percent)));
      bar.style.width = `${percent}%`;
      label.textContent = 'Preparing media…';
      eta.textContent = `${Math.floor(done)} / ${total}`;
    },
    /** "Waiting for media · 2 clips offline" (section 14.8). */
    waiting(message: string) {
      if (operation.cancel) return;
      label.textContent = message;
      eta.textContent = '';
    },
    preview(frame: { width: number; height: number; jpeg: Uint8Array }) {
      if (decoding || operation.cancel || typeof window.createImageBitmap !== 'function') return;
      decoding = true;
      const blob = new window.Blob([frame.jpeg as BlobPart], { type: 'image/jpeg' });
      void window.createImageBitmap(blob).then((image: ImageBitmap) => {
        if (prev.width !== frame.width || prev.height !== frame.height) { prev.width = frame.width; prev.height = frame.height; }
        prev.getContext('2d')?.drawImage(image, 0, 0, prev.width, prev.height);
        image.close();
        scan.hidden = true;
      }).catch(() => undefined).finally(() => { decoding = false; });
    },
  };
}

/** Reuses the export sheet while old media is copied into the engine store.
 * The host calls back only when there is migration work, so already-bound
 * projects proceed straight to the destination dialog. */
async function prepareMedia(project: any, operation: { cancel: boolean; busy?: boolean } = X): Promise<boolean> {
  let ui: ReturnType<typeof progressUI> | null = null;
  try {
    await PM.HostMedia?.flushBindings(project, (done: number, total: number) => {
      if (!(total > 0)) return;
      ui ??= progressUI(1, true, operation);
      ui.preparing(done, total);
    });
    return !operation.cancel;
  } finally {
    if (ui) {
      const cancelled = operation.cancel;
      (ui as ReturnType<typeof progressUI>).mod.close();
      operation.cancel = cancelled;
    }
  }
}

let destinationToken: string | null = null;
/** The file the last export wrote, for caption sidecars beside it. */
let lastOutputPath: string | null = null;
let nativeToken: string | null = null;
let sequenceDirectory: any = null;

async function run(opts: any) {
  if (X.busy) return PM.toast('Export already running');
  if (PM.agentFrameCapture) return PM.toast('Wait for the current frame capture to finish.');
  // Today's ids (`prores`, `mp4`, `webm`, `rec`) are the new formats' names now.
  opts = { ...opts, format: migrateFormat(opts?.format), delivery: normalizeDelivery(opts?.delivery) };
  // The native engine's consistency check (docs/rust-engine.md, section 6).
  PM.EngineDoc?.beforeExport?.(opts?.reportPath ? 'agent-export' : 'export');
  X.busy = true;
  X.cancel = false;
  const bridge = (hostBridge() as any);
  const releaseJob = isRenderFormat(opts.format) && !opts.reportPath ? holdExportJob() : () => {};
  try {
    const p = PM.proj;
    // With Use New Engine on, every render format runs in the job engine; a
    // project it cannot render yet comes back here and the current exporter
    // writes the file the user already chose (agent exports route in main).
    if (isRenderFormat(opts.format) && !opts.reportPath && bridge?.nativeEngine?.exportVideo) {
      const engine = await exportThroughEngine(opts);
      if (engine) return engine;
    }
    const legacy = legacyFormatId(opts.format);
    if (!legacy) {
      throw new Error(`The new engine can’t export this project yet, and the current exporter doesn’t write ${formatInfo(opts.format).label}. Choose ProRes 4444, H.264, WebM VP9, or PNG.`);
    }
    opts = { ...opts, format: legacy };
    if (opts.format !== 'json') {
      const pending = [p], checked = new Set<any>();
      while (pending.length) {
        const comp: any = pending.pop();
        if (!comp || checked.has(comp)) continue;
        checked.add(comp);
        pending.push(...Object.values(comp.comps || {}));
      }
    }
    if (opts.format === 'prores' || (opts.format === 'mp4' && bridge?.render)) {
      if (!bridge?.render) throw new Error('The native encoder requires the updated desktop runtime.');
      nativeToken = await bridge.render.start({
        width: Math.round(p.w * opts.scale / 2) * 2, height: Math.round(p.h * opts.scale / 2) * 2,
        fps: opts.fps, format: opts.format, alpha: !!opts.alpha,
        bitrateMbps: exportBitrateMbps(opts.quality), name: opts.name || p.name,
        expectsAudio: opts.audio !== false && PM.Audio.expectsOfflineMix?.(...range(opts)) === true,
      });
      if (!nativeToken) return { cancelled: true };
    } else if (bridge?.exportDestination) {
      const base = p.name || 'powermove';
      const name = opts.format === 'web' ? base + '-web.zip'
        : opts.format === 'json' ? base + '.pmv'
        : opts.format === 'still' ? `${p.name}_${PM.tc(PM.time, p.fps).replace(/:/g, '-')}.png`
        : base + (opts.format === 'mp4' ? '.mp4' : opts.format === 'png' ? '.png' : '.webm');
      destinationToken = await bridge.exportDestination.choose(name, opts.format === 'png');
      if (!destinationToken) return { cancelled: true };
    } else if (opts.format === 'png' && (window as any).showDirectoryPicker) {
      try { sequenceDirectory = await (window as any).showDirectoryPicker({ mode: 'readwrite' }); }
      catch (error: any) { if (error?.name === 'AbortError') return { cancelled: true }; throw error; }
    }
    // A clean picture for a separate caption file: the compositor skips captions.
    PM.captionsHidden = opts.captionsBurn === false;
    return await runPrepared(opts);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    PM.toast('Export failed: ' + message, 5000);
    return { error: message };
  } finally {
    try {
      if (nativeToken) await bridge.render.cancel(nativeToken).catch(() => undefined);
      if (destinationToken) await bridge.exportDestination.release(destinationToken).catch(() => undefined);
    } finally {
      for (const video of recoveryDecoders.values()) {
        video.pause(); video.removeAttribute('src'); video.load();
      }
      recoveryDecoders.clear();
      nativeToken = null;
      destinationToken = null;
      sequenceDirectory = null;
      PM.captionsHidden = false;
      X.busy = false;
      releaseJob();
    }
  }
}

/** The compatibility producers an export in `format` names. */
function compatSources(format: unknown): { compat?: string[] } {
  const render: any = PM.EngineRender;
  const sources: string[] = typeof render?.exportSources === 'function' ? render.exportSources(format) : (render?.producing ?? []);
  return sources.length ? { compat: [...sources] } : {};
}

/** The job engine's view of the sheet's settings (shared/engine-ipc.ts). */
function sheetOptions(opts: any): any {
  const p: any = PM.proj;
  const [t0, t1]: any = range(opts);
  return {
    format: opts.format, alpha: !!opts.alpha && supportsAlpha(opts.format, opts.delivery), fps: Number(opts.fps), scale: Number(opts.scale) || 1,
    start: opts.format === 'still' ? PM.time : t0, end: opts.format === 'still' ? PM.time + 1 / Math.max(1, Number(opts.fps) || p.fps || 30) : t1,
    audio: opts.audio !== false, audioIndexPlayhead: PM.Audio?.indexPlayhead?.() ?? PM.time, motionBlur: !!opts.mblur, quality: opts.quality ?? 'high',
    delivery: normalizeDelivery(opts.delivery), hideCaptions: opts.captionsBurn === false,
    // A raster override's layers are drawn by this window (kernel/raster-compat.ts),
    // and a WebM export's frames go through its compatibility context (compat-context.ts).
    ...compatSources(opts.format)
  };
}

/** The output's name from the sheet's file name template. */
function outputName(opts: any): string {
  const p: any = PM.proj;
  const est = planExport({ ...normalizeExportDefaults(opts, p.fps), format: opts.format }, { w: p.w, h: p.h, dur: p.dur, work: p.work });
  return expandFileName(opts.delivery?.fileName || '{project}', {
    project: p.name || 'Powermove', composition: p.compName || p.name || '', preset: opts.preset, format: opts.format,
    width: est.width, height: est.height, fps: Number(opts.fps) || p.fps, now: new Date()
  });
}

/** "Locate Media…" in the offline message: the Media panel's locate flow for
 *  the first offline clip. */
function locateMedia(assets: string[]): void {
  const first = assets[0];
  if (!first) return;
  PM.toast(assets.length === 1 ? 'Locate the offline clip to export at full quality' : `Locate the ${assets.length} offline clips to export at full quality`, 4000);
  PM.pickFiles?.(false, { replaceAssetId: first });
}

/** An export through the job engine (main/engine/agent-jobs.ts), with the
 *  progress sheet driven by its job statuses and live previews. Resolves null
 *  when the engine is off or routes the project to the current exporter. */
async function exportThroughEngine(opts: any): Promise<any | null> {
  const engine = (hostBridge() as any)?.nativeEngine;
  const status = await engine.status().catch(() => null);
  // The job engine starts on demand and exits when unused: a stopped one is
  // not a reason to route. Main starts it and answers `unavailable` only
  // when it cannot run.
  if (!status?.enabled) return null;
  const p: any = PM.proj;
  const [t0, t1]: any = range(opts);
  // Statuses can arrive before the start answer; keep them until the job id is known.
  const early: any[] = [];
  let listener: ((s: any) => void) | null = null;
  const off = engine.onExportStatus((s: any) => { if (listener) listener(s); else early.push(s); });
  try {
    const request = { project: JSON.stringify(p), name: outputName(opts), options: sheetOptions(opts) };
    if (!await prepareMedia(p)) return { cancelled: true };
    // Main captures this window's definitions for the job (contract 10):
    // pending registrations and extension reloads reach it first.
    await PM.EngineDefinitions?.settle?.();
    const started = await engine.exportVideo(request);
    if (started.status === 'cancelled') return { cancelled: true };
    if (started.status === 'locate') { locateMedia(started.assets); return { cancelled: true }; }
    if (started.status !== 'accepted') return null;
    const total = Math.max(1, started.frames);
    const began = window.performance.now();
    const ui = progressUI(total);
    let final: any;
    try {
      final = await new Promise<any>((resolve) => {
        const cancelPoll = window.setInterval(() => { if (X.cancel) void engine.cancelExport(started.jobId).catch(() => undefined); }, 100);
        const on = (s: any): void => {
          if (s.jobId !== started.jobId) return;
          if (s.preview) ui.preview(s.preview);
          if (s.state === 'waitingForMedia') {
            ui.waiting(`Waiting for media · ${s.waiting ?? 'offline clips'}`);
            return;
          }
          if (s.state === 'queued' || s.state === 'running') {
            ui.set(Math.min(total, Math.round(s.progress * total)), s.state === 'running' ? '' : 'Waiting');
            return;
          }
          window.clearInterval(cancelPoll);
          resolve(s);
        };
        listener = on;
        for (const s of early.splice(0)) on(s);
      });
    } finally {
      // Closing the sheet marks a running export cancelled; keep the outcome.
      const cancelled = X.cancel;
      ui.mod.close();
      X.cancel = cancelled;
    }
    if (final.routed) {
      // Never silent: the reason the engine refused goes to the console and
      // stays readable (`PM.Export.lastRouted`) until the routing is retired.
      X.lastRouted = { format: opts.format, reason: String(final.error ?? '') };
      console.warn(`[engine] the export went to the current exporter: ${final.error}`);
      if (legacyFormatId(opts.format)) return null;
      throw new Error(`The new engine can’t export this project yet (${final.error}), and the current exporter doesn’t write ${formatInfo(opts.format).label}.`);
    }
    if (final.state === 'cancelled') { PM.toast('Export cancelled'); return { cancelled: true }; }
    if (final.state === 'failed') throw new Error(final.error || 'The export failed.');
    lastOutputPath = final.path ?? started.path;
    const sidecars = await writeCaptionSidecars(opts, t0, t1);
    PM.toast(`Export finished in ${((window.performance.now() - began) / 1000).toFixed(1)}s${sidecars ? ` · ${sidecars} caption ${sidecars === 1 ? 'file' : 'files'}` : ''}`, 3400);
    return { cancelled: false };
  } finally {
    off();
  }
}

/** "Add to Queue": a render queue job in the job engine (Use New Engine), or
 *  today's queue otherwise. */
X.addToQueue = async (rawOpts: any) => {
  const opts = { ...rawOpts, format: migrateFormat(rawOpts?.format), delivery: normalizeDelivery(rawOpts?.delivery) };
  const engine = (hostBridge() as any)?.nativeEngine;
  if (!engineOn() || !engine?.queueAdd || !isRenderFormat(opts.format)) {
    const legacy = legacyFormatId(opts.format);
    if (!legacy) { PM.toast('The render queue needs the new engine for this format.', 4000); return null; }
    return X.enqueue({ ...opts, format: legacy, name: opts.name || PM.proj.name });
  }
  const [t0, t1]: any = range(opts);
  const sidecarFormat = opts.captionsSidecar;
  const sidecars = (sidecarFormat === 'srt' || sidecarFormat === 'vtt') && isRenderFormat(opts.format) && opts.format !== 'png' && opts.format !== 'still'
    ? captionSidecars(PM.proj, sidecarFormat, { from: t0, to: t1 }).map((s) => ({ extension: sidecarFormat, text: s.text, ...(s.suffix ? { suffix: s.suffix } : {}) }))
    : [];
  const project = PM.proj;
  const request = {
    project: JSON.stringify(project),
    name: outputName(opts),
    options: sheetOptions(opts),
    ...(sidecars.length ? { sidecars } : {}),
    ...(opts.preset ? { preset: String(opts.preset) } : {})
  };
  // Queue preparation can run alongside a foreground export. Its cancellation
  // belongs to this add only; the foreground export owns X.busy and X.cancel.
  const operation = { cancel: false };
  const releaseJob = holdExportJob();
  try {
    if (!await prepareMedia(project, operation)) return { status: 'cancelled' };
    // The queued job keeps this window's definitions as they are now.
    await PM.EngineDefinitions?.settle?.();
    const result = await engine.queueAdd(request);
    if (result?.status === 'added') PM.toast('Added to render queue');
    return result;
  } catch (error) {
    PM.toast('Could not add to the render queue: ' + (error instanceof Error ? error.message : String(error)), 5000);
    return null;
  } finally { releaseJob(); }
};

async function deliver(blob: Blob, name: string): Promise<boolean> {
  const bridge = (hostBridge() as any);
  let result: any;
  if (destinationToken && bridge.fileUpload && blob.size > 4 * 1024 * 1024) {
    const uploadId = await bridge.fileUpload.begin(blob.size);
    try {
      await writeBlobChunks(blob, bytes => bridge.fileUpload.chunk(uploadId, bytes));
      result = await bridge.fileUpload.finish(uploadId, { name, destinationToken });
    } finally { await bridge.fileUpload.abort(uploadId).catch(() => undefined); }
  } else {
    result = destinationToken
      ? await bridge.saveFile({ name, data: new Uint8Array(await blob.arrayBuffer()), destinationToken })
      : await PM.download(blob, name);
  }
  if (result?.cancelled) return false;
  if (result?.ok === false) throw new Error(result.error || 'Save failed');
  lastOutputPath = typeof result?.path === 'string' ? result.path : null;
  return true;
}

async function runPrepared(opts: any) {
  const p: any = PM.proj;
  const [t0, t1]: any = range(opts);
  const W: any = Math.round(p.w * opts.scale / 2) * 2, H: any = Math.round(p.h * opts.scale / 2) * 2;

  if (opts.format === 'web') {
    try {
      await PM.app?.importQueue;
      const result = await buildWebExport(PM);
      const name = (p.name || 'powermove') + '-web.zip';
      const bridge = (hostBridge() as any);
      if (bridge?.saveFile) {
        const saved = await bridge.saveFile({ name, data: result.bytes, ...(destinationToken ? { destinationToken } : {}) });
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
    }
  }

  if (opts.format === 'json') {
    try {
      await PM.app?.importQueue;
      const data = await packProjectFileBlob({ ...JSON.parse(PM.serialize()), history: PM.hist.export?.() }, PM.MediaStore);
      if (!await deliver(data, (p.name || 'powermove') + '.pmv')) return { cancelled: true };
      PM.toast('Project exported');return {cancelled:false, ...(opts.reportPath && lastOutputPath ? { path: lastOutputPath } : {})};
    } catch (error) { const message=error instanceof Error?error.message:String(error);PM.toast('Could not export project: '+message,6000);return {error:message}; }
  }
  if (opts.format === 'still') {
    const wasPlaying=PM.playing,at=PM.time;PM.pause();
    const capture: AlphaCapture | undefined = opts.alpha ? { canvas: window.document.createElement('canvas') } : undefined;
    try{await prepareExportFrame(PM,at,PM.proj,{mblur:opts.mblur!==false,mbSamples:opts.alpha?20:16,shutter:PM.proj.shutter||.5});
      const cv: any = opts.alpha ? alphaFrame(at, W, H, opts.mblur, capture, fastAlphaCapture(W, H)) : PM.renderFrameTo(at, W, H);
      const blob=await new Promise<Blob>((resolve,reject)=>cv.toBlob((b:Blob|null)=>b?resolve(b):reject(new Error('Could not encode frame'))));
      if (!await deliver(blob, `${p.name}_${PM.tc(at, p.fps).replace(/:/g, '-')}.png`)) return { cancelled: true };
      PM.toast('Frame exported');return {cancelled:false};
    }catch(error){const message=(error as Error).message;PM.toast(message,6000);return {error:message};}
    finally {
      try {
        releaseAlphaReadback(capture);
        if (capture) { capture.image = undefined; capture.canvas.width = 0; capture.canvas.height = 0; }
      } finally { PM.preparedVideoFrames=null;PM.setTime(at,{force:true});viewerService(PM)?.layout();if(wasPlaying)PM.play(); }
    }
  }

  const wasPlaying: any = PM.playing;
  const oldT: any = PM.time, oldQ: any = PM.quality;
  const total: any = Math.max(1, Math.round((t1 - t0) * opts.fps));
  let ui: any;
  const bitrate: any = exportBitrateMbps(opts.quality) * 1e6;
  const t: any = window.performance.now();
  lastOutputPath = null;

  try {
    PM.pause();
    ui = progressUI(total);
    const pctx = ui.prev.getContext('2d');
    if (!pctx) throw new Error('Could not create export preview');
    ui.prev.width = 640; ui.prev.height = Math.round(640 * H / W);
    const wantsAudio: any = opts.audio !== false && PM.Audio.hasAudibleLayers(PM.proj);
    const needsRecorderAudio: any = opts.format === 'webm' && wantsAudio && !(await PM.Audio.supportsOpus());
    if(opts.format==='prores'||(opts.format==='mp4'&&(hostBridge() as any)?.render)){
      await exportNative({opts,W,H,t0,t1,total,ui,pctx});
    } else if (opts.format === 'mp4' || opts.format === 'rec' || (opts.format === 'webm' && (typeof window.VideoEncoder === 'undefined' || needsRecorderAudio))) {
      await exportRecorder({ opts, W, H, t0, t1, total, ui, pctx, bitrate });
    } else if (opts.format === 'png') {
      await exportPNGs({ opts, W, H, t0, total, ui, pctx });
    } else {
      await exportWebCodecs({ opts, W, H, t0, t1, total, ui, pctx, bitrate });
    }
    if (X.cancel) PM.toast('Export cancelled');
    else {
      const sidecars = await writeCaptionSidecars(opts, t0, t1);
      PM.toast(`Export finished in ${((window.performance.now() - t) / 1000).toFixed(1)}s${sidecars ? ` · ${sidecars} caption ${sidecars === 1 ? 'file' : 'files'}` : ''}`, 3400);
    }
    return {cancelled:X.cancel, ...(opts.reportPath && !X.cancel && lastOutputPath ? { path: lastOutputPath } : {})};
  } catch (e: any) {
    if (X.cancel && (e?.name === 'AbortError' || e?.name === 'EffectInputError' && /cancelled/.test(e.message))) { PM.toast('Export cancelled'); return { cancelled: true }; }
    window.console.error(e);
    const message = e instanceof Error ? e.message : String(e);
    PM.toast('Export failed: ' + message, 5000);
    return {error:message};
  } finally {
    // Closing completed progress must not mark a successful export as cancelled.
    const cancelled = X.cancel;
    ui?.mod.close();
    X.cancel = cancelled;
    PM.preparedVideoFrames=null;
    PM.quality = oldQ;
    PM.setTime(oldT, { force: true });
    viewerService(PM)?.layout();
    if (wasPlaying) PM.play();
  }
}

const isVideoExport = (format: string) => !['web', 'json', 'png', 'still'].includes(format);

/** Subtitle files beside the exported video, one per captions layer. */
async function writeCaptionSidecars(opts: any, t0: number, t1: number): Promise<number> {
  const format = opts.captionsSidecar;
  // Only beside a video: the choice survives switching to a still, an image
  // sequence or a code/project export, where a subtitle file means nothing.
  if ((format !== 'srt' && format !== 'vtt') || !lastOutputPath || !isVideoExport(opts.format)) return 0;
  const bridge = (hostBridge() as any);
  const files = captionSidecars(PM.proj, format, { from: t0, to: t1 });
  if (!bridge?.exportSidecar) {
    for (const file of files) await PM.download(new Blob([file.text], { type: 'text/plain' }), `${PM.proj.name || 'Captions'}${file.suffix ? `.${file.suffix}` : ''}.${format}`);
    return files.length;
  }
  let written = 0;
  for (const file of files) {
    const result = await bridge.exportSidecar({ path: lastOutputPath, extension: format, ...(file.suffix ? { suffix: file.suffix } : {}), text: file.text });
    if (result?.ok) written++;
    else PM.toast(`Could not write the caption file: ${result?.error || 'unknown error'}`, 5000, { error: true });
  }
  return written;
}

function renderInto(T: any, W: any, H: any, mblur: any) {
  PM.quality = 1;
  PM.GL.resize(W, H);
  PM.GL.render(T, { mblur, mbSamples: 20, shutter: PM.proj.shutter || .5 });
  return PM.GL.canvas;
}

/* Transparent frames: read back the composited FBO (premultiplied, bottom-up)
   and convert to straight-alpha top-down ImageData. */
type AlphaReadback = { capture(time: number, options: Record<string, unknown>): Uint8Array<ArrayBuffer> | null; dispose(): void };
type AlphaCapture = { canvas: HTMLCanvasElement; image?: ImageData;
  readback?: { width: number; height: number; depth: 16 | 32; project: any; gl: any; session: AlphaReadback | null } };
function releaseAlphaReadback(target?: AlphaCapture) {
  target?.readback?.session?.dispose();
  if (target) target.readback = undefined;
}
function alphaFrame(T: any, W: any, H: any, mblur: any, target?: AlphaCapture, fast = false) {
  const depth = PM.proj.colorDepth === 32 ? 32 : 16;
  if (target?.readback && (!fast || target.readback.width !== W || target.readback.height !== H || target.readback.depth !== depth
      || target.readback.project !== PM.proj || target.readback.gl !== PM.GL.gl)) releaseAlphaReadback(target);
  if (target && fast && !target.readback) target.readback = { width: W, height: H, depth, project: PM.proj, gl: PM.GL.gl,
    session: (depth === 32 ? PM.GL.createFloatReadback?.(W, H) : PM.GL.createHalfReadback?.(W, H)) || null };
  const session = target?.readback?.session;
  const px = captureFramePixels(PM, T, W, H, {
    transparent: true, mblur: mblur !== false, mbSamples: 20, shutter: PM.proj.shutter || .5,
  }, session ? options => session.capture(T, options) : undefined);
  return finishAlphaFrame(px, W, H, target);
}
function finishAlphaFrame(px: Uint8Array, W: number, H: number, target?: AlphaCapture) {
  const surface = target || { canvas: window.document.createElement('canvas') };
  const cv = surface.canvas;
  if (cv.width !== W || cv.height !== H) { cv.width = W; cv.height = H; surface.image = undefined; }
  const c: any = cv.getContext('2d');
  const img = surface.image ||= c.createImageData(W, H);
  writeStraightAlphaPixels(px, W, H, img.data);
  c.putImageData(img, 0, 0);
  return cv;
}
function fastAlphaCapture(width: number, height: number, uncachedFrames = 0): boolean {
  // Small stills cannot amortize the compatibility probe. Longer per-frame
  // sequences benefit even at 360p; cached artwork keeps the larger cutoff.
  const area = width * height;
  return PM.proj.colorDepth !== 8 && (area >= 1920 * 1080 || uncachedFrames >= 12 && area >= 640 * 360);
}

async function exportNative({opts,W,H,t0,t1,total,ui,pctx}:any) {
  const bridge = (hostBridge() as any)?.render;
  if (!bridge) throw new Error('The native encoder requires the updated desktop runtime.');
  const token = nativeToken;
  if (!token) throw new Error('Export destination is unavailable');
  const chunks = async (bytes: Uint8Array, audio = false, retain = false) => {
    const limit = 4 * 1024 * 1024;
    for (let at = 0; at < bytes.length; at += limit) {
      // A view of a large buffer can serialize its entire backing store through
      // Electron. Keep bounded, owned chunks; avoid copying an already-owned
      // complete frame/audio buffer that fits in one message.
      const chunk = bytes.length <= limit && bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength
        ? bytes : bytes.slice(at, at + limit);
      await bridge.write(token, chunk, audio, retain);
    }
  };
  type ReadbackTicket = { readInto(target: Uint8Array, cancelled: () => boolean): Promise<boolean>; dispose(): void };
  type Readback = { readonly byteLength?: number; begin(time: number, options: any): ReadbackTicket | null; dispose(): void };
  type WriteResult = { ok: true } | { ok: false; error: unknown };
  const preview = window.document.createElement('canvas');
  const alphaCapture: AlphaCapture | undefined = opts.alpha ? { canvas: preview } : undefined;
  const retention = trackStaticExport(PM, opts.mblur !== false);
  const media = trackExportMedia(PM);
  let readback: Readback | null = null, ticket: ReadbackTicket | null = null;
  let readbackUnavailable = false, pixels: Uint8Array<ArrayBuffer> | null = null, image: ImageData | null = null;
  let pending: { frame: number; image?: ImageData; result: Promise<WriteResult> } | null = null;
  let alphaDescriptor: { project: any; gl: any; depth: unknown } | undefined;
  const expressionFree = (value: any, seen = new WeakSet<object>()): boolean => {
    if (!value || typeof value !== 'object' || seen.has(value)) return true;
    if (value.expr) return false;
    seen.add(value);
    return Object.values(value).every(child => expressionFree(child, seen));
  };
  let eligibility: { project: any; depth: unknown; version: unknown; revision: number; layers: any; count: number; background: any; mediaLayers: any[]; hasVideo: boolean; effectIds: EffectRef[]; allowed: boolean } | null = null;
  const artworkEligible = () => {
    // Callers refresh retention.key(time) first. Its revision covers renderer,
    // font and project events; ordinary edits also advance animVersion.
    const project = PM.proj, version = PM.animVersion?.(), revision = retention.revision;
    const depth = opts.alpha ? project.colorDepth : undefined;
    if (!eligibility || eligibility.project !== project || eligibility.depth !== depth || eligibility.version !== version || eligibility.revision !== revision
        || eligibility.layers !== project.layers || eligibility.count !== project.layers.length || eligibility.background !== project.backgroundFill) {
      const mediaLayers = project.layers.filter((layer: any) => layer.type === 'image' || layer.type === 'video');
      const effectIds: EffectRef[] = [];
      const knownEffects = (layer: any) => (!layer.fx?.length || Array.isArray(layer.fx) && layer.fx.every((effect: any) => {
        if (!exportEffects.supports(effect?.type)) return false;
        effectIds.push(effect);
        return true;
      }));
      const record = (value: any) => value && typeof value === 'object' && !Array.isArray(value);
      const builtinPath = (path: any) => record(path) && record(path.p) && Array.isArray(path.vertices)
        && path.vertices.every((vertex: any) => record(vertex) && record(vertex.p));
      const builtinMasks = (layer: any) => !layer.masks?.length || Array.isArray(layer.masks) && layer.masks.every((mask: any) =>
        mask && ['rect', 'ellipse'].includes(mask.shape) && (!mask.path || builtinPath(mask.path)) && mask.p && typeof mask.p === 'object');
      eligibility = { project, depth, version, revision, layers: project.layers, count: project.layers.length, background: project.backgroundFill,
        mediaLayers, hasVideo: mediaLayers.some((layer: any) => layer.type === 'video'), effectIds,
        allowed: (!opts.alpha || opts.format === 'prores' && W * H >= 1920 * 1080 && total >= 12
          && W * (H + 1) * 4 <= 32 * 1024 * 1024 && depth !== 8 && !mediaLayers.length)
          && W * H * 4 <= 32 * 1024 * 1024 && expressionFree(project.backgroundFill)
          && (!mediaLayers.length || opts.mblur === false
            || project.layers.every((layer: any) => layer.type === 'audio' || !layer.mblur))
          && project.layers.every((layer: any) => layer.type === 'audio' ||
            ['solid', 'shape', 'text', 'null', 'group', 'image', 'video'].includes(layer.type)
            && knownEffects(layer) && builtinMasks(layer) && !layer.threeD && !layer.matteSource
            && !layer.transitionIn && !layer.transitionOut && !layer.d?.animators?.length && expressionFree(layer)) };
    }
    // Provider overrides can change without a project edit, even inside a
    // registry batch. Cache structure only and check active providers live.
    return eligibility.allowed && (!opts.alpha || !retention.hasStaticPlan)
      && (!eligibility.effectIds.length || exportEffects.allow(PM, eligibility.effectIds));
  };
  const eligible = () => artworkEligible() && frameFontsReady(PM);
  let failedWrite = false;
  const releaseReadback = () => {
    ticket?.dispose(); ticket = null;
    readback?.dispose(); readback = null; pixels = null; image = null;
  };
  try {
    preview.width = W; preview.height = H;
    const previewContext = preview.getContext('2d');
    if (!previewContext) throw new Error('Could not create export preview');
    const join = async () => {
      const writing = pending;
      if (!writing) return;
      pending = null;
      const result = await writing.result;
      if (!result.ok) { failedWrite = true; throw result.error; }
      // The preview copies the shared CPU pixels before the next readback may
      // reuse them. Progress still follows the encoder's acknowledgement.
      // Alpha already passed through Canvas2D once before delivery. Reusing
      // its rounded getImageData output here would round the pixels again.
      if (writing.image) previewContext.putImageData(writing.image, 0, 0);
      pctx.drawImage(preview, 0, 0, ui.prev.width, ui.prev.height);
      ui.set(writing.frame + 1, opts.format === 'prores' ? 'ProRes 4444' : 'H.264');
      await new Promise(resolve => setTimeout(resolve, 0));
    };
    const prepareNativeFrame = async (time: number) => {
      retention.key(time); let allowed = artworkEligible();
      // A video seek can take seconds. Keep it behind the previous write so
      // an earlier error or cancellation never waits for a speculative seek.
      if (eligibility!.hasVideo) {
        await join();
        if (X.cancel) return { stamp: null, changed: false };
        // A project or layer edit can arrive during the prior write. Source
        // stamps must describe the scene that preparation will actually use.
        retention.key(time); allowed = artworkEligible();
      }
      if (X.cancel) return { stamp: null, changed: false };
      const layers = eligibility!.mediaLayers;
      if (!allowed || !layers.length) {
        await prepareExportFrame(PM, time, PM.proj, {mblur:opts.mblur!==false,mbSamples:opts.alpha?20:16,shutter:PM.proj.shutter||.5});
        return { stamp: undefined, changed: false };
      }
      const sources = media.sources(layers);
      if (sources === null) await join();
      if (X.cancel) return { stamp: null, changed: false };
      await prepareExportFrame(PM, time, PM.proj, {mblur:opts.mblur!==false,mbSamples:opts.alpha?20:16,shutter:PM.proj.shutter||.5});
      const changed = !!sources && !media.sameSources(sources);
      return { stamp: sources && !changed ? media.prepared(layers, time, sources) : sources === undefined ? undefined : null, changed };
    };
    try {
      let lastKey: string | null = null, retained: Uint8Array | null = null;
      for (let i = 0; i < total; i++) {
        if (X.cancel) break;
        const T = t0 + i / opts.fps;
        try {
          if (opts.alpha && (!alphaDescriptor || alphaDescriptor.project !== PM.proj
              || alphaDescriptor.gl !== PM.GL.gl || alphaDescriptor.depth !== PM.proj.colorDepth)) {
            releaseReadback(); readbackUnavailable = false;
            alphaDescriptor = { project: PM.proj, gl: PM.GL.gl, depth: PM.proj.colorDepth };
          }
          if (PM.GL.contextLost || PM.GL.gl?.isContextLost?.()) throw new Error('The GPU could not capture the export frame');
          let key = retention.key(T);
          if (key !== null || !eligible()) {
            await join(); releaseReadback();
            // A bitmap can be replaced or closed while the prior write joins.
            key = retention.key(T);
          }
          if (X.cancel) break;
          if (key !== null && key === lastKey && retained) {
            let count = 1;
            if (bridge.repeat) {
              // Small batches bound backpressure and keep Cancel responsive.
              while (count < retention.maxRepeat && i + count < total && retention.key(t0 + (i + count) / opts.fps) === key) count++;
              try { await bridge.repeat(token, count); } catch (error) { failedWrite = true; throw error; }
            } else {
              try { await chunks(retained); } catch (error) { failedWrite = true; throw error; }
            }
            i += count - 1;
            ui.set(i + 1, opts.format === 'prores' ? 'ProRes 4444' : 'H.264');
            await new Promise(resolve => setTimeout(resolve, 0));
            continue;
          }
          let preparedMedia = await prepareNativeFrame(T);
          if (X.cancel) break;
          if (preparedMedia.changed) {
            await join(); releaseReadback();
            preparedMedia = await prepareNativeFrame(T);
            if (X.cancel) break;
            if (preparedMedia.changed) throw new Error('Media changed while preparing the export frame');
          }
          if (preparedMedia.stamp !== null && retention.key(T) === null && eligible()
              && (!eligibility!.mediaLayers.length || preparedMedia.stamp !== undefined) && !readbackUnavailable) {
            const mediaStamp = preparedMedia.stamp;
            const effectsStamp = eligibility!.effectIds.length ? exportEffects.capture(PM, eligibility!.effectIds) : undefined;
            if (effectsStamp !== null && !readback) {
              if (opts.alpha) releaseAlphaReadback(alphaCapture);
              readback = (opts.alpha ? PM.GL.createAlphaReadback?.(W, H) : PM.GL.createOpaqueReadback?.(W, H)) || null;
              readbackUnavailable = !readback;
            }
            if (effectsStamp !== null && readback && (effectsStamp === undefined || exportEffects.valid(PM, effectsStamp))) {
              const quality = PM.quality, depth = PM.proj.colorDepth, motionBlur = opts.mblur !== false;
              try {
                if (!opts.alpha) PM.quality = 1;
                ticket = readback.begin(T, opts.alpha
                  ? { transparent: true, mblur: motionBlur, mbSamples: 20, shutter: PM.proj.shutter || .5 }
                  : { mblur: motionBlur, mbSamples: motionBlur ? 16 : 1,
                    shutter: PM.proj.shutter || .5, opaque: true, topDownOpaque: true });
              } finally { if (!opts.alpha) PM.quality = quality; }
              // Rendering itself may invalidate a font or renderer cache. Stamp
              // its completed submission, then reject any later source changes.
              const project = PM.proj, capturedLayers = project.layers, version = PM.animVersion?.(), gl = PM.GL.gl;
              const capturedKey = retention.key(T), revision = retention.revision, fontsReady = frameFontsReady(PM);
              const fresh = () => retention.key(T) === capturedKey && retention.revision === revision
                && project === PM.proj && capturedLayers === PM.proj.layers && version === PM.animVersion?.() && gl === PM.GL.gl
                && frameFontsReady(PM) === fontsReady && artworkEligible() && document.fonts?.status !== 'loading'
                && (!opts.alpha || quality === PM.quality && depth === PM.proj.colorDepth)
                && (mediaStamp === undefined || media.valid(mediaStamp))
                && (effectsStamp === undefined || exportEffects.valid(PM, effectsStamp));
              const sourcesUnchanged = (mediaStamp === undefined || media.valid(mediaStamp))
                && (effectsStamp === undefined || exportEffects.valid(PM, effectsStamp));
              await join();
              if (X.cancel) break;
              if (PM.GL.contextLost || PM.GL.gl?.isContextLost?.()) throw new Error('The GPU could not capture the export frame');
              if (ticket && sourcesUnchanged && fresh()) {
                // One reusable raw readback buffer and one PBO. Fill it only
                // after all prior chunks have been acknowledged.
                if (!pixels) {
                  retained = null; lastKey = null;
                  const length = opts.alpha ? W * (H + 1) * 4 : W * H * 4;
                  if (opts.alpha && readback.byteLength !== length) throw new Error('The GPU capture target has the wrong size or type');
                  pixels = new Uint8Array(length);
                  if (!opts.alpha) image = new ImageData(new Uint8ClampedArray(pixels.buffer), W, H);
                }
                let copied = false, retry = false;
                try { copied = await ticket.readInto(pixels, () => X.cancel); }
                catch (error) {
                  if (!opts.alpha || !(error instanceof AlphaReadbackRetryError)
                      || PM.GL.contextLost || PM.GL.gl?.isContextLost?.()) throw error;
                  retry = true; readbackUnavailable = true;
                }
                if (X.cancel || !retry && !copied) { X.cancel = true; ticket.dispose(); ticket = null; break; }
                if (PM.GL.contextLost || PM.GL.gl?.isContextLost?.()) throw new Error('The GPU could not capture the export frame');
                if (!retry && fresh()) {
                  ticket.dispose(); ticket = null;
                  retained = null; lastKey = null;
                  let bytes: Uint8Array = pixels;
                  if (opts.alpha) {
                    const visible = captureFramePixels(PM, T, W, H, {}, () => pixels!.subarray(0, W * H * 4));
                    finishAlphaFrame(visible, W, H, alphaCapture);
                    const data = previewContext.getImageData(0, 0, W, H).data;
                    bytes = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
                  }
                  // Attach rejection handling before preparing another frame,
                  // including a rejected promise whose reason is undefined.
                  pending = { frame: i, image: image || undefined, result: chunks(bytes).then<WriteResult, WriteResult>(
                    () => ({ ok: true }), error => ({ ok: false, error })) };
                  continue;
                }
              }
              if (!ticket) readbackUnavailable = true;
              // A source changed, the deferred reader was unavailable, or its
              // guard failed. Recapture this timestamp through the serial path.
              releaseReadback(); retained = null; lastKey = null;
              preparedMedia = await prepareNativeFrame(T);
              if (X.cancel) break;
              if (preparedMedia.changed) throw new Error('Media changed while preparing the export frame');
            }
          }
          await join();
          releaseReadback();
          if (X.cancel) break;
          // Alpha preserves Canvas2D's established rounding. Static artwork and
          // unsupported contexts keep their existing serial/repeat path.
          const captured = opts.alpha ? null : renderOpaqueFrame(PM, T, W, H, { mblur: opts.mblur });
          const cv = captured ? preview : alphaFrame(T, W, H, opts.mblur, alphaCapture,
            fastAlphaCapture(W, H, retention.key(T) === null ? total : 0));
          // Cached static bytes own their pixels; repeats no longer need the
          // raw readback or its GPU target. Keep unavailable factories cached.
          if (key !== null && (i + 1 >= total || retention.key(t0 + (i + 1) / opts.fps) === key)
              && alphaCapture?.readback?.session) releaseAlphaReadback(alphaCapture);
          const data = captured ? captured.data : previewContext.getImageData(0, 0, W, H).data;
          const bytes = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
          try { await chunks(bytes, false, key !== null && !!bridge.repeat); }
          catch (error) { failedWrite = true; throw error; }
          retained = key !== null ? bytes : null; lastKey = key;
          if (captured) previewContext.putImageData(captured, 0, 0);
          pctx.drawImage(cv, 0, 0, ui.prev.width, ui.prev.height);
          ui.set(i + 1, opts.format === 'prores' ? 'ProRes 4444' : 'H.264');
          await new Promise(resolve => setTimeout(resolve, 0));
        } catch (error) {
          // An earlier frame's write takes precedence over speculative capture
          // errors. Drain it before cancellation or preview restoration.
          await join();
          if (X.cancel && !failedWrite) return;
          throw error;
        }
      }
      await join();
      if (X.cancel) return;
      if (opts.audio !== false && PM.Audio.hasAudibleLayers(PM.proj)) {
        const mix = await PM.Audio.renderOffline(t0, t1);
        if (mix) await chunks(wavBytes(mix), true);
      }
      const result = await bridge.finish(token);
      nativeToken = null;
      if (result.cancelled) X.cancel = true;
      else if (!result.path) throw new Error('Could not save video');
      lastOutputPath = result.path || null;
    } finally { await join(); }
  } finally {
    releaseReadback(); retention.dispose(); media.dispose();
    preview.width = 0; preview.height = 0;
    releaseAlphaReadback(alphaCapture);
    if (alphaCapture) alphaCapture.image = undefined;
  }
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
  const retention = trackStaticExport(PM, opts.mblur !== false);
  let retained: VideoFrame | null = null, lastKey: string | null = null;
  try {
  let chosen: any = null;
  for (const c of WEBM_VIDEO_CODECS) {
    const cfg: any = webmVideoConfig(c.codec, W, H, bitrate, opts.fps);
    const sup: any = await window.VideoEncoder.isConfigSupported(cfg).catch(() => null);
    if (sup && sup.supported) { enc.configure(cfg); chosen = c; configured = true; break; }
  }
  if (!configured) throw new Error('No supported VP9/VP8 encoder — use realtime capture');

  for (let i: any = 0; i < total; i++) {
    if (X.cancel) break;
    const T: any = t0 + i / opts.fps;
    if (PM.GL.contextLost || PM.GL.gl?.isContextLost?.()) throw new Error('The GPU could not capture the export frame');
    const key = retention.key(T);
    let cv: any;
    if (key === null || key !== lastKey || !retained) {
      retained?.close(); retained = null;
      await prepareExportFrame(PM,T,PM.proj,{mblur:opts.mblur!==false,mbSamples:opts.format==='png'&&!opts.alpha?16:20,shutter:PM.proj.shutter||.5});
      if (X.cancel) break;
      cv = renderInto(T, W, H, opts.mblur);
    }
    const frame: VideoFrame = new window.VideoFrame(retained || cv, webmFrameTiming(i, opts.fps));
    try {
      if (key !== null && !retained) { retained = frame.clone(); lastKey = key; }
      if (encoderError) throw encoderError;
      enc.encode(frame, { keyFrame: webmKeyFrame(i, opts.fps) });
    } finally { frame.close(); }
    if (i % 3 === 0) {
      pctx.drawImage(retained || cv, 0, 0, ui.prev.width, ui.prev.height);
      ui.set(i + 1, 'Rendering');
      await new Promise((r: any) => window.setTimeout(r, 0));
    }
    while (enc.encodeQueueSize > WEBM_ENCODE_QUEUE_LIMIT && !X.cancel && !encoderError) {
      await new Promise((r: any) => window.setTimeout(r, 6));
    }
    if (encoderError) throw encoderError;
  }
  if (X.cancel) return;
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
  } finally {
    retained?.close(); retention.dispose();
    if (enc.state !== 'closed') enc.close();
  }
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
    await prepareExportFrame(PM,T,PM.proj,{mblur:opts.mblur!==false,mbSamples:opts.format==='png'&&!opts.alpha?16:20,shutter:PM.proj.shutter||.5});
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
  const dir = sequenceDirectory;
  const retention = trackStaticExport(PM, opts.mblur !== false);
  const captureCanvas = window.document.createElement('canvas');
  const alphaCapture: AlphaCapture | undefined = opts.alpha ? { canvas: captureCanvas } : undefined;
  let lastKey: string | null = null, retained: Blob | null = null;
  type DeliveryResult = { status: 'saved' | 'cancelled' } | { status: 'failed'; error: unknown };
  type PendingPNG = { frame: number; settled: boolean; result: Promise<DeliveryResult> };
  let pending: PendingPNG | null = null;
  const save = async (blob: Blob, name: string) => {
    if (!dir) return await deliver(blob, name);
    const fh: any = await dir.getFileHandle(name, { create: true });
    const writer: any = await fh.createWritable();
    try { await writer.write(blob); await writer.close(); }
    catch (error) {
      try { await writer.abort?.(); } catch { /* Keep the original write error. */ }
      throw error;
    }
    return true;
  };
  const join = async (): Promise<boolean> => {
    if (!pending) return true;
    const delivery = pending;
    const outcome = await delivery.result; pending = null;
    if (outcome.status === 'failed') throw outcome.error;
    if (outcome.status === 'cancelled') { X.cancel = true; return false; }
    ui.set(delivery.frame + 1, dir ? 'writing to folder' : 'downloading');
    return true;
  };
  try {
  for (let i: any = 0; i < total; i++) {
    if (X.cancel) break;
    if (pending?.settled && !await join()) return;
    const T: any = t0 + i / opts.fps;
    let cv: any;
    let blob: Blob | null = null;
    let speculative = false, reused = false, capturedKey: string | null = null, capturedRevision = 0, capturedFontsReady = false;
    try {
      // Bound the two PNGs' memory and avoid starting a video seek or font wait.
      // Recheck the live project so edits cannot retain an older eligibility.
      if ((W * H * 4 > 32 * 1024 * 1024 || !frameFontsReady(PM)
          || PM.proj.layers.some((layer: any) => layer.type === 'video' || layer.type === 'precomp'))
          && !await join()) return;
      if (X.cancel) break;
      if (PM.GL.contextLost || PM.GL.gl?.isContextLost?.()) throw new Error('The GPU could not capture the export frame');
      const key = retention.key(T);
      speculative = pending !== null;
      blob = key !== null && key === lastKey ? retained : null;
      reused = blob !== null;
      if (!blob) {
        await prepareExportFrame(PM,T,PM.proj,{mblur:opts.mblur!==false,mbSamples:opts.format==='png'&&!opts.alpha?16:20,shutter:PM.proj.shutter||.5});
        if (X.cancel) break;
        const cacheKey = retention.key(T);
        if (opts.alpha) cv = alphaFrame(T, W, H, opts.mblur, alphaCapture, fastAlphaCapture(W, H, cacheKey === null ? total : 0));
        if (cacheKey !== null && (i + 1 >= total || retention.key(t0 + (i + 1) / opts.fps) === cacheKey)
            && alphaCapture?.readback?.session) releaseAlphaReadback(alphaCapture);
        if (!cv) cv = PM.renderFrameTo(T, W, H, { mblur: opts.mblur }, captureCanvas);
        capturedKey = retention.key(T); capturedRevision = retention.revision;
        capturedFontsReady = frameFontsReady(PM);
        blob = await new Promise<Blob>((resolve, reject) => cv.toBlob((encoded: Blob | null) =>
          encoded ? resolve(encoded) : reject(new Error('Could not encode frame')), 'image/png'));
        retained = cacheKey !== null ? blob : null; lastKey = cacheKey;
      } else { capturedKey = key; capturedRevision = retention.revision; capturedFontsReady = frameFontsReady(PM); }
    } catch (error) {
      // The earlier durable write's error/cancellation takes precedence over a
      // speculative capture failure and must settle before restoring preview.
      if (pending && (!await join() || X.cancel)) return;
      throw error;
    }
    if (!await join()) return;
    if (X.cancel) break;
    if ((speculative || reused) && (PM.GL.contextLost || PM.GL.gl?.isContextLost?.())) throw new Error('The GPU could not capture the export frame');
    if ((speculative || reused) && (retention.key(T) !== capturedKey || retention.revision !== capturedRevision || frameFontsReady(PM) !== capturedFontsReady)) {
      // Edits while the preceding file was saving belong to this frame, just
      // as on the serial path. Retry without a pending write or another retry.
      retained = null; lastKey = null; i--; continue;
    }
    const name: any = `${PM.proj.name || 'frame'}_${String(i).padStart(5, '0')}.png`;
    // The preview keeps its own copied pixels while this surface captures the
    // next frame. Progress advances only after the corresponding save succeeds.
    if (cv) pctx.drawImage(cv, 0, 0, ui.prev.width, ui.prev.height);
    const next: PendingPNG = { frame: i, settled: false, result: save(blob!, name).then<DeliveryResult, DeliveryResult>(
      saved => { next.settled = true; return { status: saved ? 'saved' : 'cancelled' }; },
      error => { next.settled = true; return { status: 'failed', error }; }) };
    pending = next;
    await new Promise((r: any) => window.setTimeout(r, 0));
  }
  await join();
  } finally {
    try { await join(); }
    finally {
      retention.dispose();
      captureCanvas.width = 0; captureCanvas.height = 0;
      releaseAlphaReadback(alphaCapture);
      if (alphaCapture) alphaCapture.image = undefined;
    }
  }
}

/* Programmatic export — used by tests and the native menu. */
installRenderQueue(PM,X);
/* With Use New Engine on, the Render Queue panel shows the job engine's queue
   (render-queue-panel.ts); today's queue dialog stays for the current exporter. */
const legacyQueueDialog = X.queueDialog;
X.queueDialog = () => (engineOn() && openRenderQueuePanel(PM)) || legacyQueueDialog();
/* "Locate Media…" from the offline message reaches this window too when the
   export started elsewhere (an agent, the queue). */
(hostBridge() as any)?.nativeEngine?.onLocateMedia?.((assets: string[]) => { if (!X.busy) locateMedia(assets); });
X.run = (opts: any) => run(Object.assign({}, X.defaults(), { name: PM.proj.name }, opts || {}));

/* Test hooks: the muxer is deterministic pure JS, so it is verifiable headlessly. */
X.muxWebM = muxWebM;

/* Await source decoding before an agent reviews a video frame. */
X.snapshotAsync = (time: number, maxWidth = 480, options: { signal?: AbortSignal } = {}) => {
  // "Use New Engine": the engine draws it (engine-connection/app-captures.ts).
  if (PM.EngineCaptures) return PM.EngineCaptures.snapshotAsync(time, maxWidth, options);
  return captureAgentFrame(PM, X, time, maxWidth, options);
};

/* Fast still capture used by the agent's `look` tool. */
X.snapshot = (T: any, maxW: any = 480) => {
  if (PM.EngineCaptures) return PM.EngineCaptures.snapshot(T, maxW);
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
