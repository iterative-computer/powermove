import type {
  CompositionPatch,
  JsonValue,
  LayerPatch,
  LayerTarget
} from '../core/types/commands';
import type { EditBinding } from './gesture';

export type { EditBinding } from './gesture';

type LegacyPM = Record<string, any>;
type TimeSource = number | (() => number);

interface BindingOptions {
  label?: string;
  origin?: string;
}

interface ChannelBindingOptions extends BindingOptions {
  time?: TimeSource;
}

const commandBinding = (
  label: string,
  origin: string | undefined,
  command: Extract<EditBinding, { mode: 'command' }>['command']
): EditBinding => ({ mode: 'command', label, ...(origin ? { origin } : {}), command });

/** Inspector/generated-panel channel edit with the exact legacy intent flags. */
export function channelBinding(
  PM: LegacyPM,
  layerId: LayerTarget,
  channel: string,
  options: ChannelBindingOptions = {}
): EditBinding {
  const path = channel.replace(/^properties\./, '').replace(/^transform\./, '');
  const time = (): number => {
    const source = options.time;
    return typeof source === 'function' ? source() : source ?? PM.time;
  };
  let scale: { linked: boolean; ratio: number } | null = null;
  const prepare = () => {
    if (path !== 'scale.x' && path !== 'scale.y') return;
    const layer = PM.L?.(layerId);
    const primary = Number(layer && PM.ev(layer, path, time()));
    const other = Number(layer && PM.ev(layer, path === 'scale.x' ? 'scale.y' : 'scale.x', time()));
    scale = { linked: !!layer?.scaleLinked, ratio: Math.abs(primary) > 1e-8 ? other / primary : 1 };
  };
  const binding = commandBinding(options.label ?? path, options.origin, (value) => {
    const command = {
      type: 'set_property',
      target: layerId,
      path,
      value: value as JsonValue,
      time: time(),
      mode: 'auto',
      preserveHandEdits: false,
      markIntent: 'human'
    } as const;
    if (!scale?.linked) return command;
    return [command, { ...command, path: path === 'scale.x' ? 'scale.y' : 'scale.x', value: Number(value) * scale.ratio }];
  });
  return { ...binding, prepare } as EditBinding;
}

/** Mirrors sourceBinding's `layer.*` → set_layer path. */
export function layerFieldBinding(
  _PM: LegacyPM,
  layerId: LayerTarget,
  field: keyof LayerPatch | `layer.${string}`,
  options: BindingOptions = {}
): EditBinding {
  const key = String(field).replace(/^layer\./, '') as keyof LayerPatch;
  return commandBinding(options.label ?? key, options.origin, (value) => ({
    type: 'set_layer', target: layerId, patch: { [key]: value } as LayerPatch
  }));
}

/** Mirrors sourceBinding's `content.*` → set_content path. */
export function contentBinding(
  _PM: LegacyPM,
  layerId: LayerTarget,
  field: string,
  options: BindingOptions = {}
): EditBinding {
  const key = field.replace(/^content\./, '');
  return commandBinding(options.label ?? key, options.origin, (value) => ({
    type: 'set_content', target: layerId, patch: { [key]: value as JsonValue }
  }));
}

/** Mirrors sourceBinding's direct composition fields and work-area endpoints. */
export function compositionBinding(
  PM: LegacyPM,
  field: keyof CompositionPatch | 'workArea.start' | 'workArea.end' | `composition.${string}`,
  options: BindingOptions = {}
): EditBinding {
  const key = String(field).replace(/^composition\./, '') as keyof CompositionPatch | 'workArea.start' | 'workArea.end';
  return commandBinding(options.label ?? key, options.origin, (value) => {
    if (key === 'workArea.start' || key === 'workArea.end') {
      const index = key.endsWith('start') ? 0 : 1;
      const duration = Number(PM.proj?.dur ?? PM.proj?.duration ?? 0);
      const source = PM.proj?.work ?? [0, duration];
      const workArea: [number, number] = [Number(source[0]) || 0, Number(source[1]) || duration];
      const frame = 1 / Math.max(1, Number(PM.proj?.fps) || 30);
      const clamp = PM.clamp ?? ((n: number, min: number, max: number) => Math.max(min, Math.min(max, n)));
      workArea[index] = index === 0
        ? clamp(Number(value), 0, Math.max(0, workArea[1] - frame))
        : clamp(Number(value), Math.min(duration, workArea[0] + frame), duration);
      return { type: 'set_composition', patch: { workArea } };
    }
    return { type: 'set_composition', patch: { [key]: value } as CompositionPatch };
  });
}
