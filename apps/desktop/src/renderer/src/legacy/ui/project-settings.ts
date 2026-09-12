import {
  EXPORT_FORMAT_OPTIONS,
  EXPORT_QUALITY_OPTIONS,
  EXPORT_RANGE_OPTIONS,
  EXPORT_SCALE_OPTIONS,
  clampExportScale,
  exportFieldSupport,
  type ExportDefaults
} from '../../core/export-defaults';
import { createSettingsSection } from './settings-tabs';

/** Composition fields the Project tab and the New project dialog both edit. */
export interface CompositionValues {
  name: string;
  w: number;
  h: number;
  fps: number;
  dur: number;
}

/** Field names accepted by the `set_composition` edit command. */
export interface CompositionPatch {
  name?: string;
  width?: number;
  height?: number;
  fps?: number;
  duration?: number;
}

export interface ProjectSettingsBridge {
  /** Current composition, re-read on every refresh. */
  composition(): CompositionValues;
  applyComposition(patch: CompositionPatch): void;
  exportDefaults(): ExportDefaults;
  applyExportDefaults(patch: Partial<ExportDefaults>): void;
  /**
   * Optional richer background control (the app passes the gradient-capable
   * fill field). Without it the tab falls back to a plain colour swatch.
   */
  backgroundField?(): HTMLElement;
  background?(): string;
  applyBackground?(color: string): void;
}

export interface ProjectSettingsControl {
  element: HTMLElement;
  focus(): void;
  /** Re-read the project after an edit made elsewhere (undo, agent, viewer). */
  refresh(): void;
  destroy(): void;
}

export interface ResolutionPreset {
  w: number;
  h: number;
  label: string;
}

export const RESOLUTION_PRESETS: ResolutionPreset[] = [
  { w: 3840, h: 2160, label: '4K UHD · 3840×2160' },
  { w: 1920, h: 1080, label: 'HD 1080p · 1920×1080' },
  { w: 1280, h: 720, label: 'HD 720p · 1280×720' },
  { w: 1080, h: 1920, label: 'Vertical HD · 1080×1920' },
  { w: 1080, h: 1350, label: 'Portrait 4:5 · 1080×1350' },
  { w: 1080, h: 1080, label: 'Square · 1080×1080' }
];

export const MIN_DIMENSION = 16;
export const MAX_DIMENSION = 8192;
export const MIN_FRAME_RATE = 1;
export const MAX_FRAME_RATE = 240;
export const MIN_DURATION = 0.1;
export const MAX_DURATION = 3600;

const CUSTOM = 'custom';

/** Blank fields fall back to the previous value; anything else is clamped. */
function parsed(value: unknown): number | null {
  if (typeof value === 'string' && !value.trim()) return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

export function clampDimension(value: unknown, fallback: number): number {
  const number = parsed(value);
  if (number == null) return fallback;
  return Math.min(MAX_DIMENSION, Math.max(MIN_DIMENSION, Math.round(number)));
}

export function clampFrameRate(value: unknown, fallback: number): number {
  const number = parsed(value);
  if (number == null) return fallback;
  return Math.min(MAX_FRAME_RATE, Math.max(MIN_FRAME_RATE, Math.round(number)));
}

export function clampDuration(value: unknown, fallback: number): number {
  const number = parsed(value);
  if (number == null) return fallback;
  return Math.min(MAX_DURATION, Math.max(MIN_DURATION, Math.round(number * 1000) / 1000));
}

interface SettingsRow {
  element: HTMLElement;
  /** Muted description line; rewritten live for context-dependent rows. */
  detail: HTMLElement;
  /** Dim the row and lock its controls when the current format ignores it. */
  setEnabled(enabled: boolean): void;
}

function row(title: string, detail: string, control: HTMLElement): SettingsRow {
  const element = document.createElement('div');
  element.className = 'settings-row';
  const copy = document.createElement('div');
  copy.className = 'settings-copy';
  const label = document.createElement('b');
  label.textContent = title;
  const description = document.createElement('span');
  description.textContent = detail;
  copy.append(label, description);
  element.append(copy, control);
  const controls = [control, ...control.querySelectorAll('input, select, button')];
  return {
    element,
    detail: description,
    setEnabled(enabled: boolean) {
      element.classList.toggle('is-dim', !enabled);
      for (const item of controls) {
        if ('disabled' in item) (item as HTMLInputElement).disabled = !enabled;
      }
    }
  };
}

function select(label: string): HTMLSelectElement {
  const element = document.createElement('select');
  element.className = 'settings-select';
  element.setAttribute('aria-label', label);
  return element;
}

function option(value: string, label: string): HTMLOptionElement {
  const element = document.createElement('option');
  element.value = value;
  element.textContent = label;
  return element;
}

function fillOptions<T>(
  element: HTMLSelectElement,
  options: Array<{ v: T; label: string }>,
  extra?: Array<{ v: string; label: string }>
): void {
  element.textContent = '';
  for (const item of options) element.append(option(String(item.v), item.label));
  for (const item of extra ?? []) element.append(option(item.v, item.label));
}

function numberInput(label: string, min: number, max: number, step: number): HTMLInputElement {
  const element = document.createElement('input');
  element.type = 'number';
  element.className = 'settings-input';
  element.setAttribute('aria-label', label);
  element.min = String(min);
  element.max = String(max);
  element.step = String(step);
  return element;
}

function checkbox(label: string): HTMLInputElement {
  const element = document.createElement('input');
  element.type = 'checkbox';
  element.className = 'settings-check';
  element.setAttribute('aria-label', label);
  return element;
}

function presetKey(w: number, h: number): string {
  const preset = RESOLUTION_PRESETS.find((item) => item.w === w && item.h === h);
  return preset ? `${preset.w}x${preset.h}` : CUSTOM;
}

/** "16:9" for tidy ratios, "1.85:1" otherwise. */
export function aspectLabel(w: number, h: number): string {
  const gcd = (a: number, b: number): number => (b ? gcd(b, a % b) : a);
  const g = gcd(Math.round(w), Math.round(h)) || 1;
  const a = Math.round(w) / g;
  const b = Math.round(h) / g;
  if (a <= 32 && b <= 32) return `${a}:${b}`;
  return `${Math.round((w / h) * 100) / 100}:1`;
}

/** Even-rounded output size for a scale factor, matching the renderer. */
function scaledSize(w: number, h: number, scale: number): [number, number] {
  return [Math.max(2, Math.round(w * scale / 2) * 2), Math.max(2, Math.round(h * scale / 2) * 2)];
}

/**
 * The Project tab: composition size, timing and background on top, the export
 * settings this project starts from below.
 */
export function createProjectSettingsControl(bridge: ProjectSettingsBridge): ProjectSettingsControl {
  const element = document.createElement('div');
  /* The Project tab is taller than the modal, so this page scrolls rather than
     letting the flex column clip its last rows. */
  element.className = 'settings-sections is-scroll';
  let current = bridge.composition();

  const composition = createSettingsSection('Composition');
  const name = document.createElement('input');
  name.type = 'text';
  name.className = 'settings-input is-wide';
  name.setAttribute('aria-label', 'Project name');

  const preset = select('Resolution preset');
  const width = numberInput('Width in pixels', MIN_DIMENSION, MAX_DIMENSION, 2);
  const height = numberInput('Height in pixels', MIN_DIMENSION, MAX_DIMENSION, 2);
  const size = document.createElement('div');
  size.className = 'settings-field-pair';
  const times = document.createElement('span');
  times.textContent = '×';
  size.append(width, times, height);

  const frameRate = numberInput('Frame rate', MIN_FRAME_RATE, MAX_FRAME_RATE, 1);
  const duration = numberInput('Duration in seconds', MIN_DURATION, MAX_DURATION, 0.5);

  const sizeRow = row('Size', 'Width and height of every frame, in pixels.', size);
  const durationRow = row('Duration', 'Total length of the composition, in seconds.', duration);
  composition.body.append(
    row('Name', 'Shown on the project tab and used for exported file names.', name).element,
    row('Resolution', 'Pick a common size, or set your own pixel dimensions.', preset).element,
    sizeRow.element,
    row('Frame rate', 'Frames rendered per second of the composition — any rate from 1 to 240.', frameRate).element,
    durationRow.element
  );

  let backgroundControl: HTMLElement | null = null;
  let backgroundSwatch: HTMLInputElement | null = null;
  if (bridge.backgroundField) {
    backgroundControl = bridge.backgroundField();
  } else if (bridge.background && bridge.applyBackground) {
    backgroundSwatch = document.createElement('input');
    backgroundSwatch.type = 'color';
    backgroundSwatch.className = 'settings-swatch';
    backgroundSwatch.setAttribute('aria-label', 'Background colour');
    backgroundSwatch.addEventListener('change', () => {
      bridge.applyBackground?.(backgroundSwatch!.value.toUpperCase());
    });
    backgroundControl = backgroundSwatch;
  }
  if (backgroundControl) {
    composition.body.append(
      row('Background', 'Colour shown behind every layer of the composition.', backgroundControl).element
    );
  }

  const exports = createSettingsSection('Export');
  const format = select('Export format');
  fillOptions(format, EXPORT_FORMAT_OPTIONS);
  const scale = select('Export resolution');
  const exportWidth = numberInput('Export width in pixels', MIN_DIMENSION, MAX_DIMENSION, 2);
  const exportHeight = numberInput('Export height in pixels', MIN_DIMENSION, MAX_DIMENSION, 2);
  const exportSize = document.createElement('div');
  exportSize.className = 'settings-field-pair';
  const exportTimes = document.createElement('span');
  exportTimes.textContent = '×';
  exportSize.append(exportWidth, exportTimes, exportHeight);
  const exportFps = numberInput('Export frame rate', MIN_FRAME_RATE, MAX_FRAME_RATE, 1);
  const range = select('Export range');
  fillOptions(range, EXPORT_RANGE_OPTIONS);
  const quality = select('Export quality');
  fillOptions(quality, EXPORT_QUALITY_OPTIONS);
  const mblur = checkbox('Motion blur');
  const audio = checkbox('Include audio');
  const alpha = checkbox('Transparent background');

  const scaleRow = row('Resolution', 'Rendered size relative to the composition, or any pixel size.', scale);
  const exportSizeRow = row('Output size', 'Exact pixel size to render; keeps the composition aspect.', exportSize);
  const fpsRow = row('Frame rate', 'Frames written per second of exported video.', exportFps);
  const rangeRow = row('Range', 'Export the work area or the whole composition.', range);
  const qualityRow = row('Quality', 'Video bitrate used for video exports.', quality);
  const mblurRow = row('Motion blur', 'Render motion blur for moving layers.', mblur);
  const audioRow = row('Include audio', 'Mix composition audio into video exports.', audio);
  const alphaRow = row('Transparent background', 'Keep the background see-through in PNG and still exports.', alpha);
  const gatedRows = {
    scale: scaleRow, fps: fpsRow, range: rangeRow, quality: qualityRow,
    mblur: mblurRow, audio: audioRow, alpha: alphaRow
  } as const;

  exports.body.append(
    row('Format', 'What the Export dialog produces by default.', format).element,
    scaleRow.element,
    exportSizeRow.element,
    fpsRow.element,
    rangeRow.element,
    qualityRow.element,
    mblurRow.element,
    audioRow.element,
    alphaRow.element
  );
  const note = document.createElement('p');
  note.className = 'settings-note';
  note.textContent = 'The Export dialog opens with these settings, and remembers whatever you change there.';
  exports.element.append(note);

  element.append(composition.element, exports.element);

  const editing = (field: HTMLElement): boolean => document.activeElement === field;

  const syncComposition = (): void => {
    current = bridge.composition();
    if (!editing(name)) name.value = current.name;
    fillOptions(
      preset,
      RESOLUTION_PRESETS.map((item) => ({ v: `${item.w}x${item.h}`, label: item.label })),
      [{ v: CUSTOM, label: `Custom · ${current.w}×${current.h}` }]
    );
    preset.value = presetKey(current.w, current.h);
    if (!editing(width)) width.value = String(current.w);
    if (!editing(height)) height.value = String(current.h);
    if (!editing(frameRate)) frameRate.value = String(current.fps);
    if (!editing(duration)) duration.value = String(current.dur);
    if (backgroundSwatch && bridge.background && !editing(backgroundSwatch)) {
      backgroundSwatch.value = bridge.background().toLowerCase();
    }
    (backgroundControl as { sync?: () => void } | null)?.sync?.();
    sizeRow.detail.textContent =
      `Width and height of every frame, in pixels · ${aspectLabel(current.w, current.h)}.`;
    durationRow.detail.textContent =
      `Total length of the composition — ${Math.max(1, Math.round(current.dur * current.fps))} frames at ${current.fps} fps.`;
  };

  let customScaleOpen = false;
  const syncExport = (): void => {
    const defaults = bridge.exportDefaults();
    const isPreset = EXPORT_SCALE_OPTIONS.some((item) => item.v === defaults.scale) && !customScaleOpen;
    const [outW, outH] = scaledSize(current.w, current.h, defaults.scale);
    fillOptions(
      scale,
      EXPORT_SCALE_OPTIONS.map((item) => ({
        v: item.v,
        label: `${item.label} · ${Math.round(current.w * item.v)}×${Math.round(current.h * item.v)}`
      })),
      [{ v: CUSTOM, label: isPreset ? 'Custom size…' : `Custom · ${outW}×${outH}` }]
    );
    format.value = defaults.format;
    scale.value = isPreset ? String(defaults.scale) : CUSTOM;
    exportSizeRow.element.hidden = isPreset;
    if (!editing(exportWidth)) exportWidth.value = String(outW);
    if (!editing(exportHeight)) exportHeight.value = String(outH);
    if (!editing(exportFps)) exportFps.value = String(defaults.fps);
    range.value = defaults.range;
    quality.value = defaults.quality;
    mblur.checked = defaults.mblur;
    audio.checked = defaults.audio;
    alpha.checked = defaults.alpha;
    const support = exportFieldSupport(defaults.format);
    for (const [key, gated] of Object.entries(gatedRows)) {
      gated.setEnabled(support[key as keyof typeof support]);
    }
    exportSizeRow.setEnabled(support.scale);
    const formatLabel = EXPORT_FORMAT_OPTIONS.find((item) => item.v === defaults.format)?.label ?? '';
    exports.summary.textContent = defaults.format === 'web' ? 'Web animation · full composition · code and assets' : defaults.format === 'json'
      ? formatLabel
      : `${formatLabel} · ${outW}×${outH}${support.fps ? ` · ${defaults.fps} fps` : ''}`;
  };

  const refresh = (): void => { syncComposition(); syncExport(); };

  name.addEventListener('change', () => {
    const next = name.value.trim();
    if (!next || next === current.name) return syncComposition();
    bridge.applyComposition({ name: next });
    refresh();
  });
  preset.addEventListener('change', () => {
    if (preset.value === CUSTOM) return syncComposition();
    const [w, h] = preset.value.split('x').map(Number);
    if (!w || !h || (w === current.w && h === current.h)) return syncComposition();
    bridge.applyComposition({ width: w, height: h });
    refresh();
  });
  width.addEventListener('change', () => {
    const next = clampDimension(width.value, current.w);
    if (next !== current.w) bridge.applyComposition({ width: next });
    refresh();
  });
  height.addEventListener('change', () => {
    const next = clampDimension(height.value, current.h);
    if (next !== current.h) bridge.applyComposition({ height: next });
    refresh();
  });
  frameRate.addEventListener('change', () => {
    const next = clampFrameRate(frameRate.value, current.fps);
    if (next !== current.fps) bridge.applyComposition({ fps: next });
    refresh();
  });
  duration.addEventListener('change', () => {
    const next = clampDuration(duration.value, current.dur);
    if (next !== current.dur) bridge.applyComposition({ duration: next });
    refresh();
  });

  format.addEventListener('change', () => {
    bridge.applyExportDefaults({ format: format.value as ExportDefaults['format'] });
    syncExport();
  });
  scale.addEventListener('change', () => {
    if (scale.value === CUSTOM) {
      customScaleOpen = true;
      syncExport();
      exportWidth.focus();
      exportWidth.select();
      return;
    }
    customScaleOpen = false;
    bridge.applyExportDefaults({ scale: Number(scale.value) });
    syncExport();
  });
  const applyExportSize = (fromWidth: boolean): void => {
    const input = fromWidth ? exportWidth : exportHeight;
    const base = fromWidth ? current.w : current.h;
    const typed = parsed(input.value);
    if (typed == null || typed <= 0) return syncExport();
    const pixels = Math.min(MAX_DIMENSION, Math.max(MIN_DIMENSION, Math.round(typed)));
    bridge.applyExportDefaults({ scale: clampExportScale(pixels / base, bridge.exportDefaults().scale) });
    customScaleOpen = true;
    syncExport();
  };
  exportWidth.addEventListener('change', () => applyExportSize(true));
  exportHeight.addEventListener('change', () => applyExportSize(false));
  exportFps.addEventListener('change', () => {
    bridge.applyExportDefaults({ fps: clampFrameRate(exportFps.value, bridge.exportDefaults().fps) });
    syncExport();
  });
  range.addEventListener('change', () => {
    bridge.applyExportDefaults({ range: range.value as ExportDefaults['range'] });
    syncExport();
  });
  quality.addEventListener('change', () => {
    bridge.applyExportDefaults({ quality: quality.value as ExportDefaults['quality'] });
    syncExport();
  });
  for (const [field, key] of [[mblur, 'mblur'], [audio, 'audio'], [alpha, 'alpha']] as const) {
    field.addEventListener('change', () => {
      bridge.applyExportDefaults({ [key]: field.checked });
      syncExport();
    });
  }

  refresh();

  return {
    element,
    focus: () => name.focus(),
    refresh,
    destroy: () => { (backgroundControl as { remove?: () => void } | null)?.remove?.(); }
  };
}

export interface NewProjectValues extends CompositionValues {
  bg: string;
}

export interface NewProjectForm {
  element: HTMLElement;
  values(): NewProjectValues;
  focus(): void;
}

export interface NewProjectFormOptions {
  /**
   * The app supplies its in-window picker here. Keeping the form's colour as
   * local draft state avoids the native macOS colour panel, whose focus can
   * bounce back to an input inside a modal after it is dismissed.
   */
  backgroundField?(get: () => string, set: (value: string) => void): HTMLElement;
}

const NEW_PROJECT_DEFAULTS: NewProjectValues = {
  name: 'Untitled', w: 1920, h: 1080, fps: 30, dur: 10, bg: '#09090A'
};

/** The New project dialog body: the settings worth deciding before you start. */
export function createNewProjectForm(
  initial: Partial<NewProjectValues> = {},
  options: NewProjectFormOptions = {}
): NewProjectForm {
  const seed: NewProjectValues = { ...NEW_PROJECT_DEFAULTS, ...initial };
  let backgroundValue = /^#[0-9a-f]{6}$/i.test(seed.bg) ? seed.bg.toUpperCase() : NEW_PROJECT_DEFAULTS.bg;
  const element = document.createElement('div');
  element.className = 'new-project-form';

  const field = (label: string, control: HTMLElement | HTMLElement[]): HTMLElement => {
    const wrap = document.createElement('label');
    wrap.className = 'new-project-field';
    const text = document.createElement('span');
    text.textContent = label;
    wrap.append(text, ...([] as HTMLElement[]).concat(control));
    return wrap;
  };

  const name = document.createElement('input');
  name.type = 'text';
  name.className = 'settings-input is-wide';
  name.value = seed.name;
  name.setAttribute('aria-label', 'Project name');

  const preset = select('Resolution preset');
  fillOptions(
    preset,
    RESOLUTION_PRESETS.map((item) => ({ v: `${item.w}x${item.h}`, label: item.label })),
    [{ v: CUSTOM, label: 'Custom' }]
  );
  preset.value = presetKey(seed.w, seed.h);

  const width = numberInput('Width in pixels', MIN_DIMENSION, MAX_DIMENSION, 2);
  const height = numberInput('Height in pixels', MIN_DIMENSION, MAX_DIMENSION, 2);
  width.value = String(seed.w);
  height.value = String(seed.h);
  const size = document.createElement('div');
  size.className = 'settings-field-pair';
  const times = document.createElement('span');
  times.textContent = '×';
  size.append(width, times, height);

  const frameRate = numberInput('Frame rate', MIN_FRAME_RATE, MAX_FRAME_RATE, 1);
  frameRate.value = String(seed.fps);

  const duration = numberInput('Duration in seconds', MIN_DURATION, MAX_DURATION, 0.5);
  duration.value = String(seed.dur);

  const setBackground = (value: string): void => {
    if (/^#[0-9a-f]{6}$/i.test(value)) backgroundValue = value.toUpperCase();
  };
  const background = options.backgroundField?.(() => backgroundValue, setBackground)
    ?? document.createElement('input');
  if (background instanceof HTMLInputElement) {
    /* A text fallback is deliberate: type=color opens a separate native panel
       whose dismissal races the surrounding modal's focus restoration. */
    background.type = 'text';
    background.className = 'settings-input';
    background.value = backgroundValue;
    background.spellcheck = false;
    background.setAttribute('aria-label', 'Background colour');
    background.addEventListener('change', () => {
      setBackground(background.value);
      background.value = backgroundValue;
    });
  }

  preset.addEventListener('change', () => {
    if (preset.value === CUSTOM) return;
    const [w, h] = preset.value.split('x').map(Number);
    if (!w || !h) return;
    width.value = String(w);
    height.value = String(h);
  });
  const syncPreset = (): void => {
    preset.value = presetKey(clampDimension(width.value, seed.w), clampDimension(height.value, seed.h));
  };
  width.addEventListener('change', syncPreset);
  height.addEventListener('change', syncPreset);

  element.append(
    field('Name', name),
    field('Resolution', preset),
    field('Size', size),
    field('Frame rate', frameRate),
    field('Duration', duration),
    field('Background', background)
  );

  return {
    element,
    values: () => ({
      name: name.value.trim() || NEW_PROJECT_DEFAULTS.name,
      w: clampDimension(width.value, seed.w),
      h: clampDimension(height.value, seed.h),
      fps: clampFrameRate(frameRate.value, seed.fps),
      dur: clampDuration(duration.value, seed.dur),
      bg: backgroundValue
    }),
    focus: () => { name.focus(); name.select(); }
  };
}
