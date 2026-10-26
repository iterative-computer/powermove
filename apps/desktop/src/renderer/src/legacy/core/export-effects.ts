import { validEffectParameterDefault } from '../../kernel/effect-defaults';
import type { EffectDefinition } from '../../kernel/api';
import { effectProgramKey, effectVersion, resolveEffectDefinition } from '../../kernel/effect-versions';
import type { Registry, RegistryEntry } from '../../kernel/registry';
import type { PMRegistry } from '../registry';

export type EffectRef = string | { type: string; version?: number };
type EffectRegistry = Pick<Registry<EffectDefinition>, 'topEntry'>;
type Selection = { ref: EffectRef; id: string; version: number };
type Active = {
  id: string; version: number; entry: RegistryEntry<EffectDefinition>; root: EffectDefinition;
  item: EffectDefinition; key: string;
};
type Snapshot = { registry: EffectRegistry; selections: Selection[]; entries: Array<Active & {
  seq: number; params: EffectDefinition['params']; defaults: Array<number | string | boolean>; contexts: string[];
  frag: string; passes: number | undefined; original: boolean; backdrop: boolean; raw: boolean;
}> };
type Param = string | readonly [key: string, type: 'color' | 'toggle'];
// Metadata only: importing the built-in shader bundle here would load its GLSL
// at exporter startup. Tests compare these shapes with the actual definitions.
type Shape = { passes: number; keepOrig?: true; params: readonly Param[]; sampling?: EffectDefinition['sampling']; grid?: EffectDefinition['grid'] };
const shapes: Record<string, Shape> = {
  tritone: { passes: 1, params: [['highlights','color'],['midtones','color'],['shadows','color'],'blendWithOriginal'] },
  colorBalanceHLS: { passes: 1, params: ['hue','lightness','saturation'] },
  brightnessContrast: { passes: 1, params: ['brightness','contrast',['useLegacy','toggle']] },
  blackWhite: { passes: 1, params: ['reds','yellows','greens','cyans','blues','magentas',['tint','toggle'],['tintColor','color']] },
  unsharpMask: { passes: 2, keepOrig: true, params: ['amount', 'radius', 'threshold'], sampling: { radius: 'radius', support: 3, maxRadius: 32 } },
  findEdges: { passes: 1, params: [['invert', 'toggle'], 'blendWithOriginal'] },
  threshold: { passes: 1, params: ['level'] },
  shiftChannels: { passes: 1, params: ['takeAlphaFrom', 'takeRedFrom', 'takeGreenFrom', 'takeBlueFrom'] },
  exposure: { passes: 1, params: ['exposure', 'offset', 'gamma', 'redExposure', 'redOffset', 'redGamma',
    'greenExposure', 'greenOffset', 'greenGamma', 'blueExposure', 'blueOffset', 'blueGamma', ['bypassLinearLight', 'toggle']] },
  levelsIndividual: { passes: 1, params: ['inBlack', 'inWhite', 'gamma', 'outBlack', 'outWhite',
    'redInBlack', 'redInWhite', 'redGamma', 'redOutBlack', 'redOutWhite',
    'greenInBlack', 'greenInWhite', 'greenGamma', 'greenOutBlack', 'greenOutWhite',
    'blueInBlack', 'blueInWhite', 'blueGamma', 'blueOutBlack', 'blueOutWhite',
    'alphaInBlack', 'alphaInWhite', 'alphaGamma', 'alphaOutBlack', 'alphaOutWhite',
    ['clipBlack', 'toggle'], ['clipWhite', 'toggle']] },
  blur: { passes: 2, params: ['amount'] },
  motionblurDir: { passes: 1, params: ['amount', 'angle'] },
  sharpen: { passes: 1, params: ['amount'] },
  glow: { passes: 3, keepOrig: true, params: ['threshold', 'radius', 'intensity'] },
  color: { passes: 1, params: ['exposure', 'contrast', 'saturation', 'hue', 'temperature'] },
  levels: { passes: 1, params: ['inBlack', 'inWhite', 'gamma'] },
  duotone: { passes: 1, params: [['shadow', 'color'], ['highlight', 'color'], 'amount'] },
  grain: { passes: 1, params: ['amount', 'size'] },
  vignette: { passes: 1, params: ['amount', 'feather'] },
  chroma: { passes: 1, params: ['amount'] },
  pixelate: { passes: 1, params: ['size'] },
  posterize: { passes: 1, params: ['levels'] },
  displace: { passes: 1, params: ['amount', 'scale', 'speed'] },
  shadow: { passes: 3, keepOrig: true, params: ['distance', 'angle', 'softness', 'opacity', ['color', 'color']] },
  invert: { passes: 1, params: ['amount'] },
  tint: { passes: 1, params: [['mapBlackTo', 'color'], ['mapWhiteTo', 'color'], 'amountToTint'] },
  channelMixer: { passes: 1, params: ['redRed', 'redGreen', 'redBlue', 'redConst',
    'greenRed', 'greenGreen', 'greenBlue', 'greenConst', 'blueRed', 'blueGreen', 'blueBlue',
    'blueConst', ['monochrome', 'toggle']] },
  gradient: { passes: 1, params: [['startColor', 'color'], ['endColor', 'color'], 'angle', 'centerX', 'centerY',
    'spread', 'midpoint', ['radial', 'toggle'], ['shade', 'toggle'], 'dither', 'amount'] },
};
const revisions: Record<string, Record<number, (typeof shapes)[string]>> = {
  shadow: { 2: { passes: 3, keepOrig: true, params: [['color','color'],'opacity','angle','distance','softness',['shadowOnly','toggle']], sampling: { radius:'softness', support:1.5, maxRadius:32, space:'layer', firstPass:1 } } },
  gradient: { 2: { passes: 1, params: ['startX','startY',['startColor','color'],'endX','endY',['endColor','color'],'rampShape','rampScatter','blendWithOriginal'] } },
  blur: { 2: { passes: 2, params: ['amount', 'dimensions', ['repeatEdgePixels', 'toggle']], sampling: { radius: 'amount', support: 1.5, maxRadius: 32 } } },
  pixelate: { 2: { passes: 2, params: ['horizontalBlocks', 'verticalBlocks', ['sharpColors', 'toggle']], grid: { columns: 'horizontalBlocks', rows: 'verticalBlocks', pointSample: 'sharpColors' } } },
  invert: { 2: { passes: 1, params: ['channel', 'blendWithOriginal'] } },
  levels: { 2: shapes.levelsIndividual! },
};
const supports = (id: unknown): id is string => typeof id === 'string' && Object.hasOwn(shapes, id);
const requested = (ref: EffectRef) => typeof ref === 'string' ? { id: ref, version: undefined }
  : { id: ref?.type, version: effectVersion(ref?.version) };

function sameSpatial(item: Pick<EffectDefinition, 'sampling' | 'grid'>, shape: Shape): boolean {
  return !!item.sampling === !!shape.sampling && !!item.grid === !!shape.grid
    && item.sampling?.radius === shape.sampling?.radius && item.sampling?.support === shape.sampling?.support
    && item.sampling?.maxRadius === shape.sampling?.maxRadius && item.sampling?.space === shape.sampling?.space
    && item.sampling?.firstPass === shape.sampling?.firstPass && item.grid?.columns === shape.grid?.columns
    && item.grid?.rows === shape.grid?.rows && item.grid?.pointSample === shape.grid?.pointSample;
}

function active(PM: PMRegistry, registry: EffectRegistry | undefined, ref: EffectRef): Active | null {
  const request = requested(ref), id = request.id;
  if (!supports(id) || typeof ref !== 'string' && request.version === undefined) return null;
  const entry = typeof registry?.topEntry === 'function' ? registry.topEntry(id) : undefined;
  const root = entry?.item;
  const version = request.version ?? effectVersion(root?.version);
  if (version === undefined) return null;
  const item = resolveEffectDefinition(root, version);
  const shape = version === 1 ? shapes[id] : revisions[id]?.[version];
  if (!shape || !entry || !root || entry.ownerId !== 'effects-basic' || root.id !== id || item?.id !== id
      || item.rawShader !== true || item.passes !== shape.passes || !!item.keepOrig !== !!shape.keepOrig
      || !sameSpatial(item, shape) || item.backdrop || typeof item.frag !== 'string' || !item.frag.length
      || !Array.isArray(item.params) || item.params.length !== shape.params.length
      || item.params.some((param, index) => {
        const expected = shape.params[index]!;
        return !param || !validEffectParameterDefault(param) || !['number', 'string', 'boolean'].includes(typeof param.def) || param.k !== (typeof expected === 'string' ? expected : expected[0])
          || (param.type || 'number') !== (typeof expected === 'string' ? 'number' : expected[1]);
      })) return null;
  // Validate the exact implementation the compositor would use, including its
  // cached parameter array. A current definition cannot stand in for revision 1.
  const legacy = typeof PM.effectDef === 'function' ? PM.effectDef({ type: id, version })
    : version === (root.version ?? 1) ? PM.FX?.[id] : undefined;
  if ((PM.FX || PM.effectDef) && (!legacy || legacy.params !== item.params || legacy.frag !== item.frag
      || !sameSpatial(legacy, shape) || legacy.passes !== item.passes || !!legacy.keepOrig !== !!item.keepOrig || !!legacy.backdrop !== !!item.backdrop)) return null;
  return { id, version, entry, root, item, key: effectProgramKey(id, version, root) };
}

/** Check both the selected implementation and each live source's revision.
 * Snapshot only unique implementations, so large stacks share metadata checks. */
export const exportEffects = {
  supports,
  allow(PM: PMRegistry, refs: readonly EffectRef[]): boolean {
    if (!refs.length) return true;
    const registry = PM.Kernel?.effects as EffectRegistry | undefined;
    return refs.every(ref => !!active(PM, registry, ref));
  },
  capture(PM: PMRegistry, refs: readonly EffectRef[]): Snapshot | null {
    if (!refs.length) return null;
    const registry = PM.Kernel?.effects as EffectRegistry | undefined;
    if (!registry) return null;
    const entries: Snapshot['entries'] = [], selections: Selection[] = [];
    const seen = new Map<string, Active>();
    for (const ref of refs) {
      const request = requested(ref);
      const identity = request.id + '@' + String(request.version ?? 'current');
      let selected = seen.get(identity);
      if (!selected) {
        const match = active(PM, registry, ref);
        if (!match) return null;
        selected = match; seen.set(identity, selected);
        entries.push({ ...selected, seq: selected.entry.seq, params: selected.item.params,
          defaults: selected.item.params.map(param => param.def), contexts: selected.item.params.map(param => JSON.stringify(param.defaultFrom ?? null)), frag: selected.item.frag,
          passes: selected.item.passes, original: !!selected.item.keepOrig,
          backdrop: !!selected.item.backdrop, raw: !!selected.item.rawShader });
      }
      selections.push({ ref, id: selected.id, version: selected.version });
    }
    return { registry, entries, selections };
  },
  valid(PM: PMRegistry, stamp: Snapshot): boolean {
    if (PM.Kernel?.effects !== stamp.registry) return false;
    if (!stamp.selections.every(saved => {
      const request = requested(saved.ref);
      return request.id === saved.id && (typeof saved.ref === 'string' || request.version === saved.version);
    })) return false;
    return stamp.entries.every(saved => {
      const selected = active(PM, stamp.registry, { type: saved.id, version: saved.version });
      // String callers mean the current catalog revision, so check that too.
      if (stamp.selections.some(selection => typeof selection.ref === 'string' && selection.id === saved.id)
          && (selected?.root.version ?? 1) !== saved.version) return false;
      return selected?.entry === saved.entry && selected.root === saved.root && selected.item === saved.item
        && selected.entry.seq === saved.seq && selected.key === saved.key
        && saved.item.frag === saved.frag && saved.item.passes === saved.passes
        && !!saved.item.keepOrig === saved.original && !!saved.item.backdrop === saved.backdrop && !!saved.item.rawShader === saved.raw
        && saved.item.params === saved.params && saved.params.length === saved.defaults.length
        && saved.params.every((param, index) => Object.is(param.def, saved.defaults[index]) && JSON.stringify(param.defaultFrom ?? null) === saved.contexts[index]);
    });
  },
};
