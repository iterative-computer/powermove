import type { EditCommand, JsonValue } from './types/commands';
import type {
  GeneratedButtonControl,
  GeneratedControl,
  GeneratedReadoutSource
} from './types/workspace';

type LegacyPM = Record<string, any>;
type ToolState = Record<string, unknown>;

export interface GeneratedSourceBinding {
  get(): unknown;
  command(value: unknown): EditCommand;
}

export interface SceneParameter {
  name: string;
  label: string;
  control: string;
  value: unknown;
  min?: number;
  max?: number;
  options?: unknown[];
  [key: string]: unknown;
}

export interface PreviewChange {
  description?: string;
  name?: string;
  path?: string;
  before?: unknown;
  after?: unknown;
}

export interface GeneratedResult {
  ok?: boolean;
  message?: string;
  changes?: PreviewChange[];
  preview?: { changes?: PreviewChange[] };
  [key: string]: unknown;
}

export interface PreviewItem {
  description?: string;
  name?: string;
  difference?: string;
  more?: boolean;
}

export interface PreviewModel {
  ok: boolean;
  title: string;
  message: string;
  items: PreviewItem[];
}

export type ButtonResolution =
  | { kind: 'preview'; result: GeneratedResult; applied: boolean; toast?: string }
  | { kind: 'reset' }
  | { kind: 'commands' }
  | { kind: 'command' }
  | { kind: 'prompt' }
  | { kind: 'none' };

const controlValue = (control: GeneratedControl, key: string): unknown =>
  (control as unknown as Record<string, unknown>)[key];

const clamp = (PM: LegacyPM, value: number, min: number, max: number): number =>
  typeof PM.clamp === 'function' ? PM.clamp(value, min, max) : Math.max(min, Math.min(max, value));

const json = (value: unknown): JsonValue => value as JsonValue;

/** Pure TypeScript port of legacy workspace.ts sourceBinding. */
export function sourceBinding(PM: LegacyPM, control: GeneratedControl): GeneratedSourceBinding | null {
  const authored = control as unknown as Record<string, any>;
  const target = authored.target || authored.binding?.target;
  const path = authored.path || authored.binding?.path;
  if (!target || !path) return null;

  if (target === '$composition' || target === 'composition') {
    const direct: Record<string, [string, () => unknown]> = {
      'composition.name': ['name', () => PM.proj.name],
      'composition.width': ['width', () => PM.proj.w],
      'composition.height': ['height', () => PM.proj.h],
      'composition.fps': ['fps', () => PM.proj.fps],
      'composition.duration': ['duration', () => PM.proj.dur],
      'composition.shutter': ['shutter', () => PM.proj.shutter ?? .5],
      'composition.background': ['background', () => PM.proj.bg],
      'composition.backgroundFill': ['backgroundFill', () => PM.normalizeFill(PM.proj.backgroundFill, PM.proj.bg)]
    };
    const found = direct[path];
    if (found) {
      const [key, get] = found;
      return {
        get,
        command: (value) => ({ type: 'set_composition', patch: { [key]: json(value) } }) as EditCommand
      };
    }
    if (path === 'composition.workArea.start' || path === 'composition.workArea.end') {
      const index = path.endsWith('.start') ? 0 : 1;
      return {
        get: () => PM.proj.work?.[index] ?? (index ? PM.proj.dur : 0),
        command: (value) => {
          const source = PM.proj.work || [0, PM.proj.dur];
          const workArea: [number, number] = [source[0], source[1]];
          const frame = 1 / Math.max(1, PM.proj.fps || 30);
          workArea[index] = index === 0
            ? clamp(PM, Number(value), 0, Math.max(0, workArea[1] - frame))
            : clamp(PM, Number(value), Math.min(PM.proj.dur, workArea[0] + frame), PM.proj.dur);
          return { type: 'set_composition', patch: { workArea } };
        }
      };
    }
  }

  if ((target === '$composition' || target === 'composition') && path.startsWith('composition.background.')) {
    const key = path.slice('composition.background.'.length);
    return {
      get: () => {
        const fill = PM.normalizeFill(PM.proj.backgroundFill, PM.proj.bg);
        if (key === 'type') return fill.type;
        if (key === 'startColor') return fill.stops[0]?.color || controlValue(control, 'def');
        if (key === 'endColor') return fill.stops[1]?.color || fill.stops[0]?.color || controlValue(control, 'def');
        if (key === 'angle') return fill.angle;
        if (key === 'midpoint') return fill.stops[1]?.position ?? 100;
        return controlValue(control, 'def');
      },
      command: (next) => {
        let fill = PM.normalizeFill(PM.proj.backgroundFill, PM.proj.bg);
        if (key === 'type') {
          fill = PM.normalizeFill({
            ...fill,
            type: ['solid', 'linear', 'radial', 'none'].includes(String(next)) ? next : 'solid'
          }, PM.proj.bg);
        } else {
          // Gradient-stop fields always materialize a real two-stop fill.
          if (!['linear', 'radial'].includes(fill.type)) {
            fill = PM.normalizeFill({ ...fill, type: 'linear' }, PM.proj.bg);
          }
          if (key === 'startColor') fill.stops[0].color = next;
          else if (key === 'endColor') fill.stops[1].color = next;
          else if (key === 'angle') fill.angle = next;
          else if (key === 'midpoint') fill.stops[1].position = next;
        }
        return { type: 'set_composition', patch: { backgroundFill: fill } } as EditCommand;
      }
    };
  }

  // Kept inside the getter: `$selection` must resolve at read time, not mount time.
  const layer = (): any => target === '$selection' || target === 'selection'
    ? PM.firstSel()
    : (PM.L(target) || PM.byName(target));
  const fallback = controlValue(control, 'def');

  if (path.startsWith('content.')) {
    const key = path.slice('content.'.length);
    return {
      get: () => {
        const current = layer();
        return current && current.d[key] !== undefined ? current.d[key] : fallback;
      },
      command: (value) => ({ type: 'set_content', target, patch: { [key]: json(value) } })
    };
  }

  if (path.startsWith('properties.') || path.startsWith('transform.')) {
    const channel = path.replace(/^properties\./, '').replace(/^transform\./, '');
    return {
      get: () => {
        const current = layer();
        const property = current && PM.findProp(current, channel);
        return current && property ? PM.evP(current, property, PM.time, channel) : fallback;
      },
      command: (value) => ({
        type: 'set_property',
        target,
        path: channel,
        value: json(value),
        time: PM.time,
        mode: 'auto',
        preserveHandEdits: false,
        markIntent: 'human'
      })
    };
  }

  if (path.startsWith('layer.')) {
    const key = path.slice('layer.'.length);
    const sourceKey = ({ visible: 'on', locked: 'lock', duration: 'dur', motionBlur: 'mblur' } as Record<string, string>)[key] || key;
    return {
      get: () => {
        const current = layer();
        return current && current[sourceKey] !== undefined ? current[sourceKey] : fallback;
      },
      command: (value) => ({ type: 'set_layer', target, patch: { [key]: json(value) } }) as EditCommand
    };
  }

  return null;
}

/** Create a scene parameter once, using the same name/shape as the legacy panel. */
export function ensureParam(PM: LegacyPM, control: GeneratedControl): SceneParameter {
  const name = String(controlValue(control, 'param') || control.label);
  const params = PM.proj.params as Record<string, SceneParameter>;
  if (!params[name]) {
    params[name] = {
      name,
      label: control.label,
      control: control.type === 'slider' ? 'num' : control.type,
      value: controlValue(control, 'def'),
      min: controlValue(control, 'min') as number | undefined,
      max: controlValue(control, 'max') as number | undefined,
      options: controlValue(control, 'options') as unknown[] | undefined
    };
  }
  return params[name];
}

/** Resolve by stable name after every project swap (undo/redo replaces PM.proj). */
export function currentParam(PM: LegacyPM, parameter: SceneParameter): SceneParameter {
  return PM.proj.params?.[parameter.name] || parameter;
}

export function sceneParameterCommand(parameter: SceneParameter, value: unknown): EditCommand {
  return { type: 'set_scene_parameter', name: parameter.name, value: json(value) };
}

/** Manifest command arrays are data and may not smuggle trust-bearing fields. */
export function stripManifestCommands(commands: unknown): EditCommand[] {
  if (!Array.isArray(commands)) return [];
  return commands.map((command) => {
    const clean = { ...(command as Record<string, unknown>) };
    delete clean.overrideLock;
    delete clean.preserveHandEdits;
    delete clean.markIntent;
    return clean as unknown as EditCommand;
  });
}

export function generatedButtonDisabled(PM: LegacyPM, control: GeneratedButtonControl, busy = false): boolean {
  if (busy) return true;
  const action = control.action;
  if (action?.type === 'transform') {
    const selector = action.transform.selector || { scope: 'selection', types: [] };
    return !(PM.Capabilities?.resolveTargets?.(selector)?.length);
  }
  if (action?.type === 'script') return !PM.Script?.canRun?.(action);
  if (action?.type === 'easing') return !(PM.Capabilities?.selectedKeyframes?.().length);
  return false;
}

export async function resolveButtonAction(
  PM: LegacyPM,
  control: GeneratedButtonControl,
  toolState: ToolState
): Promise<ButtonResolution> {
  const authored = control as unknown as Record<string, any>;
  const action = control.action;
  if (action?.type === 'transform') {
    const result: GeneratedResult = action.mode === 'apply'
      ? PM.Capabilities.apply(action.transform, toolState, { label: control.label, origin: 'generated-tool' })
      : PM.Capabilities.preview(action.transform, toolState);
    return {
      kind: 'preview', result, applied: action.mode === 'apply' && !!result.ok,
      toast: result.ok ? (action.mode === 'apply' ? `${control.label} applied` : 'Preview ready') : result.message
    };
  }
  if (action?.type === 'easing') {
    const result: GeneratedResult = action.mode === 'apply'
      ? PM.Capabilities.applyEasing(action, toolState, { label: control.label, origin: 'generated-tool' })
      : PM.Capabilities.previewEasing(action, toolState);
    return {
      kind: 'preview', result, applied: action.mode === 'apply' && !!result.ok,
      toast: result.ok ? (action.mode === 'apply' ? `${control.label} applied` : 'Preview ready') : result.message
    };
  }
  if (action?.type === 'script') {
    let result: GeneratedResult;
    try {
      result = action.mode === 'apply'
        ? await PM.Script.apply(action.code, toolState, { label: action.label || control.label, origin: 'generated-script' })
        : await PM.Script.preview(action.code, toolState, { label: action.label || control.label });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      result = { ok: false, message, changes: [] };
    }
    return {
      kind: 'preview', result, applied: action.mode === 'apply' && !!result.ok,
      toast: result.ok ? (action.mode === 'apply' ? `${control.label} applied` : 'Preview ready') : result.message
    };
  }
  if (action?.type === 'history') {
    const changed = PM.hist.undo();
    return {
      kind: 'preview', applied: false,
      result: {
        ok: changed,
        message: changed ? 'Restored the previous editable source.' : 'There is no edit to undo.',
        changes: []
      }
    };
  }
  if (action?.type === 'reset') return { kind: 'reset' };
  if (Array.isArray(authored.commands)) {
    PM.Edit.apply(stripManifestCommands(authored.commands), { label: control.label, origin: 'generated-ui' });
    return { kind: 'commands' };
  }
  if (control.cmd) {
    PM.cmd(control.cmd);
    return { kind: 'command' };
  }
  if (authored.prompt) {
    PM.toast('Ask the Powermove agent to change this section');
    return { kind: 'prompt' };
  }
  return { kind: 'none' };
}

export function generatedReadout(PM: LegacyPM, source: GeneratedReadoutSource): string {
  const selected = PM.selLayers?.() || [];
  if (source === 'selection.count') return String(selected.length);
  if (source === 'keyframes.count') return String(PM.Capabilities?.selectedKeyframes?.().length || 0);
  if (source === 'keyframes.summary') return PM.Capabilities?.keyframeSummary?.() || 'No keyframes selected';
  return PM.Capabilities?.selectionSummary?.() || `${selected.length} selected`;
}

export function formatPreviewValue(PM: LegacyPM, path: unknown, value: unknown): string {
  if (typeof value !== 'number') return String(value);
  if (path === 'layer.from' || path === 'layer.duration') {
    return `${Math.round(value * Math.max(1, PM.proj.fps || 30))}fr`;
  }
  return String(typeof PM.round === 'function' ? PM.round(value, 3) : Number(value.toFixed(3)));
}

/** DOM-free view model for the generated preview card. */
export function formatPreview(PM: LegacyPM, result: GeneratedResult | null, applied = false): PreviewModel | null {
  if (!result) return null;
  const ok = !!result.ok;
  const changes = (result.changes || result.preview?.changes || []);
  const items: PreviewItem[] = changes.slice(0, 8).map((change) => change.description
    ? { description: change.description }
    : {
        name: change.name,
        difference: `${formatPreviewValue(PM, change.path, change.before)} → ${formatPreviewValue(PM, change.path, change.after)}`
      });
  if (changes.length > 8) items.push({ description: `${changes.length - 8} more changes`, more: true });
  return {
    ok,
    title: ok ? (applied ? 'Applied to source' : 'Preview') : 'Nothing changed',
    message: result.message || 'The tool could not produce a valid source change.',
    items
  };
}
