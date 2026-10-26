import { expect, it, vi } from 'vitest';
import { EFFECTS } from '../../../../extensions/effects-basic/effects';
import type { EffectDefinition } from '../../kernel/api';
import { toLegacyFx } from '../../kernel/glsl';
import { Registry } from '../../kernel/registry';
import { exportEffects } from './export-effects';

const definition = (id = 'blur') => ({ ...EFFECTS.find(effect => effect.id === id)! });
function fixture() {
  const registry = new Registry<EffectDefinition>();
  for (const effect of EFFECTS) registry.register('effects-basic', { ...effect });
  const PM: any = { Kernel: { effects: registry } };
  return { PM, registry };
}

it('matches reviewed built-in optimization shapes and keeps newer operators on the general path', () => {
  const h = fixture(), ids = EFFECTS.slice(0,28).map(effect => effect.id);
  for (const effect of EFFECTS.slice(28)) {
    expect(exportEffects.supports(effect.id)).toBe(false);
    expect(exportEffects.allow(h.PM, [effect.id])).toBe(false);
  }
  expect(ids).toHaveLength(28);
  for (const id of ids) {
    expect(exportEffects.supports(id), id).toBe(true);
    expect(exportEffects.allow(h.PM, [id]), id).toBe(true);
    const stamp = exportEffects.capture(h.PM, [id])!;
    expect(stamp, id).not.toBeNull(); expect(exportEffects.valid(h.PM, stamp), id).toBe(true);
  }
  expect(exportEffects.allow(h.PM, ids)).toBe(true);
  const reviewed = ids;
  expect(exportEffects.allow(h.PM, reviewed)).toBe(true);
  const stamp = exportEffects.capture(h.PM, reviewed)!;
  expect(stamp).not.toBeNull(); expect(exportEffects.valid(h.PM, stamp)).toBe(true);
});

it('rejects a custom active override and recovers when it is disposed', () => {
  const h = fixture(), stamp = exportEffects.capture(h.PM, ['blur'])!;
  const override = h.registry.register('custom-effect', definition());
  expect(exportEffects.allow(h.PM, ['blur'])).toBe(false);
  expect(exportEffects.capture(h.PM, ['blur'])).toBeNull();
  expect(exportEffects.valid(h.PM, stamp)).toBe(false);
  override.dispose();
  expect(exportEffects.allow(h.PM, ['blur'])).toBe(true);
  expect(exportEffects.valid(h.PM, stamp)).toBe(true); // The original active entry is restored.
});

it('detects an override while registry notifications are deferred by an open batch', () => {
  const h = fixture(), changes = vi.fn(), stamp = exportEffects.capture(h.PM, ['blur'])!;
  h.registry.onChange(changes);
  const batch = h.registry.batchChanges();
  h.registry.register('custom-effect', definition());
  expect(changes).not.toHaveBeenCalled();
  expect(exportEffects.allow(h.PM, ['blur'])).toBe(false);
  expect(exportEffects.valid(h.PM, stamp)).toBe(false);
  batch.dispose();
  expect(changes).toHaveBeenCalledOnce();
});

it('detects same-definition owner changes even when batch close suppresses notifications', () => {
  const h = fixture(), changes = vi.fn(), item = h.registry.get('blur')!;
  const stamp = exportEffects.capture(h.PM, ['blur'])!;
  h.registry.onChange(changes);
  const batch = h.registry.batchChanges();
  h.registry.register('different-owner', item);
  batch.dispose();
  expect(changes).not.toHaveBeenCalled();
  expect(exportEffects.allow(h.PM, ['blur'])).toBe(false);
  expect(exportEffects.valid(h.PM, stamp)).toBe(false);
});

it('detects re-registration of the same built-in object using its new entry and sequence', () => {
  const h = fixture(), changes = vi.fn(), item = h.registry.get('blur')!;
  const stamp = exportEffects.capture(h.PM, ['blur'])!;
  h.registry.onChange(changes);
  const batch = h.registry.batchChanges();
  h.registry.register('effects-basic', item);
  batch.dispose();
  expect(changes).not.toHaveBeenCalled();
  expect(exportEffects.allow(h.PM, ['blur'])).toBe(true);
  expect(exportEffects.valid(h.PM, stamp)).toBe(false);
  expect(exportEffects.valid(h.PM, exportEffects.capture(h.PM, ['blur'])!)).toBe(true);
});

it('detects removal and registry replacement without change-event subscriptions', () => {
  const h = fixture(), stamp = exportEffects.capture(h.PM, ['blur'])!;
  h.registry.disposeOwner('effects-basic');
  expect(exportEffects.allow(h.PM, ['blur'])).toBe(false);
  expect(exportEffects.valid(h.PM, stamp)).toBe(false);
  h.registry.register('effects-basic', definition());
  const next = exportEffects.capture(h.PM, ['blur'])!;
  h.PM.Kernel.effects = fixture().registry;
  expect(exportEffects.allow(h.PM, ['blur'])).toBe(true);
  expect(exportEffects.valid(h.PM, next)).toBe(false);
});

it('ignores changes to unrelated or buried entries because they cannot affect the captured picture', () => {
  const h = fixture();
  const buried = h.registry.register('custom-effect', definition());
  h.registry.register('effects-basic', definition());
  const stamp = exportEffects.capture(h.PM, ['blur'])!;
  buried.dispose();
  h.registry.register('custom-effect', { ...definition(), id: 'unrelated' });
  expect(exportEffects.valid(h.PM, stamp)).toBe(true);
});

it.each(['owner', 'passes', 'original', 'backdrop', 'shader-kind', 'shader-source', 'parameter-key', 'parameter-type'])(
  'rejects a provider with an unsupported %s shape', change => {
    const h = fixture(), item = definition();
    let owner = 'effects-basic';
    if (change === 'owner') owner = 'third-party';
    if (change === 'passes') item.passes = 1;
    if (change === 'original') item.keepOrig = true;
    if (change === 'backdrop') item.backdrop = true;
    if (change === 'shader-kind') item.rawShader = false;
    if (change === 'shader-source') item.frag = '';
    if (change === 'parameter-key') item.params = [{ ...item.params[0]!, k: 'other' }];
    if (change === 'parameter-type') item.params = [{ k: 'amount', label: 'Amount', type: 'color', def: '#123456' }];
    const previous = exportEffects.capture(h.PM, ['blur'])!;
    h.registry.register(owner, item);
    expect(exportEffects.allow(h.PM, ['blur'])).toBe(false);
    expect(exportEffects.capture(h.PM, ['blur'])).toBeNull();
    expect(exportEffects.valid(h.PM, previous)).toBe(false);
  },
);

it('keeps unknown effect IDs, missing registries and incomplete provider APIs on their existing path', () => {
  const h = fixture();
  for (const id of ['custom-effect', 'toString', '__proto__', 'constructor']) {
    expect(exportEffects.supports(id)).toBe(false);
    h.registry.register('effects-basic', { ...definition(), id });
    expect(exportEffects.allow(h.PM, [id])).toBe(false);
    expect(exportEffects.capture(h.PM, [id])).toBeNull();
  }
  for (const PM of [{}, { Kernel: {} }, { Kernel: { effects: {} } }, { Kernel: { effects: { topEntry: 1 } } }]) {
    expect(exportEffects.allow(PM, ['blur'])).toBe(false);
    expect(exportEffects.capture(PM, ['blur'])).toBeNull();
  }
});

it.each(['duotone', 'gradient'])('rejects incorrect color metadata for %s', id => {
  const h = fixture(), item = definition(id), stamp = exportEffects.capture(h.PM, [id])!;
  item.params = item.params.map((param) => param.type === 'color' ? { ...param, type: 'number' } as any : param);
  h.registry.register('effects-basic', item);
  expect(exportEffects.allow(h.PM, [id])).toBe(false);
  expect(exportEffects.capture(h.PM, [id])).toBeNull();
  expect(exportEffects.valid(h.PM, stamp)).toBe(false);
});

it.each(['radial', 'shade'])('rejects incorrect historical gradient toggle metadata for %s', key => {
  const h = fixture(), item = structuredClone(definition('gradient').legacy![0]!);
  h.registry.register('effects-basic', item);
  const stamp = exportEffects.capture(h.PM, ['gradient'])!;
  item.params = item.params.map(param => param.k === key ? { ...param, type: 'number' } as any : param);
  h.registry.register('effects-basic', item);
  expect(exportEffects.allow(h.PM, ['gradient'])).toBe(false);
  expect(exportEffects.capture(h.PM, ['gradient'])).toBeNull();
  expect(exportEffects.valid(h.PM, stamp)).toBe(false);
});

it('keeps support checks pure and rejects non-string identifiers', () => {
  for (const id of [null, undefined, 1, {}, ['blur']]) expect(exportEffects.supports(id)).toBe(false);
});

it.each(['grain', 'gradient'])('keeps a custom owner of the supported %s ID off the fast path', id => {
  const h = fixture(), stamp = exportEffects.capture(h.PM, [id])!;
  const override = h.registry.register('custom-effect', definition(id));
  expect(exportEffects.supports(id)).toBe(true);
  expect(exportEffects.allow(h.PM, [id])).toBe(false);
  expect(exportEffects.capture(h.PM, [id])).toBeNull();
  expect(exportEffects.valid(h.PM, stamp)).toBe(false);
  override.dispose();
  expect(exportEffects.valid(h.PM, stamp)).toBe(true);
});

it('accepts explicit numeric parameter types as well as the built-in numeric defaults', () => {
  const h = fixture(), item = definition('displace');
  item.params = item.params.map(param => ({ ...param, type: 'number' } as any));
  h.registry.register('effects-basic', item);
  expect(exportEffects.allow(h.PM, ['displace'])).toBe(true);
});

it('does not touch the kernel for a scene without effects or subscribe to provider changes', () => {
  const PM = { get Kernel(): never { throw new Error('No effects need no registry access'); } };
  expect(exportEffects.allow(PM, [])).toBe(true);
  expect(exportEffects.capture(PM, [])).toBeNull();
  const h = fixture(), subscribe = vi.spyOn(h.registry, 'onChange');
  const stamp = exportEffects.capture(h.PM, ['blur'])!;
  exportEffects.allow(h.PM, ['blur']); exportEffects.valid(h.PM, stamp);
  expect(subscribe).not.toHaveBeenCalled();
});


it.each([
  { id: 'blur', key: 'amount', next: 89 },
  { id: 'duotone', key: 'shadow', next: '#f2c541' },
  { id: 'gradient', key: 'rampShape', next: 1 },
])('invalidates an in-place $id parameter $key default edit shared with the actual legacy renderer', ({ id, key, next }) => {
  const item = structuredClone(EFFECTS.find(effect => effect.id === id)!);
  const registry = new Registry<EffectDefinition>(); registry.register('effects-basic', item);
  const PM: any = { Kernel: { effects: registry } }, legacy = toLegacyFx(item, '');
  const stamp = exportEffects.capture(PM, [id])!;
  expect(legacy.params).toBe(item.params);
  const param = item.params.find(param => param.k === key)!;
  (param as any).def = next;
  // Missing saved channels bind these shared defaults in effectParamValue.
  expect(legacy.params.find(param => param.k === key)!.def).toBe(next);
  expect(exportEffects.allow(PM, [id])).toBe(true);
  expect(exportEffects.valid(PM, stamp)).toBe(false);
  expect(exportEffects.valid(PM, exportEffects.capture(PM, [id])!)).toBe(true);
});

it('invalidates a replaced parameter array even when a cached legacy renderer retains the original defaults', () => {
  const item = structuredClone(EFFECTS.find(effect => effect.id === 'blur')!);
  const registry = new Registry<EffectDefinition>(); registry.register('effects-basic', item);
  const PM: any = { Kernel: { effects: registry } }, legacy = toLegacyFx(item, '');
  const stamp = exportEffects.capture(PM, ['blur'])!;
  item.params = item.params.map(param => ({ ...param }));
  (legacy.params[0] as any).def = 89;
  expect(item.params[0]!.def).toBe(0); expect(legacy.params[0]!.def).toBe(89);
  expect(exportEffects.valid(PM, stamp)).toBe(false);
});

it.each([null, undefined, {}, [], () => 1].map(value => ({ value })))('keeps a nonprimitive parameter default on the serial path ($value)', ({ value }) => {
  const item = structuredClone(EFFECTS.find(effect => effect.id === 'blur')!);
  (item.params[0] as any).def = value;
  const registry = new Registry<EffectDefinition>(); registry.register('effects-basic', item);
  const PM: any = { Kernel: { effects: registry } };
  expect(exportEffects.allow(PM, ['blur'])).toBe(false);
  expect(exportEffects.capture(PM, ['blur'])).toBeNull();
});


it('keeps cached legacy defaults serial when the definition params were replaced before capture', () => {
  const item = structuredClone(EFFECTS.find(effect => effect.id === 'blur')!);
  const registry = new Registry<EffectDefinition>(); registry.register('effects-basic', item);
  const legacy = toLegacyFx(item, '');
  const PM: any = { Kernel: { effects: registry }, FX: { blur: legacy } };
  item.params = item.params.map(param => ({ ...param }));
  expect(exportEffects.allow(PM, ['blur'])).toBe(false);
  expect(exportEffects.capture(PM, ['blur'])).toBeNull();
  (legacy.params[0] as any).def = 89;
  expect(item.params[0]!.def).toBe(0); expect(legacy.params[0]!.def).toBe(89);
  expect(exportEffects.allow(PM, ['blur'])).toBe(false);
});

it('approves each reviewed historical revision and invalidates live source revision changes', () => {
  const h = fixture();
  const old = { type: 'invert', version: 1 }, current = { type: 'invert', version: 2 };
  const stamp = exportEffects.capture(h.PM, [old, current])!;
  expect(stamp).not.toBeNull();
  expect(stamp.entries.map(entry => entry.key)).toEqual(['fx:invert@1', 'fx:invert']);
  expect(exportEffects.valid(h.PM, stamp)).toBe(true);
  old.version = 2;
  expect(exportEffects.valid(h.PM, stamp)).toBe(false);
  expect(exportEffects.capture(h.PM, [old, current])!.entries).toHaveLength(1);
  for (const version of [0, 1.5, 3, 99, NaN]) expect(exportEffects.allow(h.PM, [{ type: 'invert', version }])).toBe(false);
});

it.each(['frag', 'default', 'revision-object'] as const)('invalidates a historical %s change during queued readback', change => {
  const h = fixture(), root = structuredClone(h.registry.get('invert')!);
  h.registry.register('effects-basic', root);
  const ref = { type: 'invert', version: 1 }, stamp = exportEffects.capture(h.PM, [ref])!;
  expect(stamp).not.toBeNull();
  if (change === 'frag') root.legacy![0]!.frag += '\n// changed historical shader';
  if (change === 'default') (root.legacy![0]!.params[0] as any).def = 31;
  if (change === 'revision-object') root.legacy![0] = { ...root.legacy![0]! };
  expect(exportEffects.valid(h.PM, stamp)).toBe(false);
});


it.each(['blur','pixelate','unsharpMask'] as const)('rejects changed %s spatial contracts before or during queued readback', id => {
  const item = structuredClone(EFFECTS.find(effect => effect.id === id)!);
  const registry = new Registry<EffectDefinition>(); registry.register('effects-basic', item);
  const PM: any = { Kernel: { effects: registry } };
  const stamp = exportEffects.capture(PM, [id])!; expect(stamp).not.toBeNull();
  if (item.sampling) item.sampling.support += .1;
  else item.grid!.columns = 'verticalBlocks';
  expect(exportEffects.valid(PM, stamp)).toBe(false);
  expect(exportEffects.allow(PM, [id])).toBe(false);
});
it.each(['blur','pixelate'] as const)('approves both saved and current %s revisions independently', type => {
  const h = fixture();
  const refs = [{type,version:1},{type,version:2}];
  const stamp = exportEffects.capture(h.PM, refs)!;
  expect(stamp.entries.map(entry => entry.key)).toEqual([`fx:${type}@1`,`fx:${type}`]);
  expect(exportEffects.valid(h.PM, stamp)).toBe(true);
});

it('invalidates queued captures after a contextual source-size default changes',()=>{
  const h=fixture(),item=structuredClone(h.registry.get('gradient')!);
  h.registry.register('effects-basic',item);const stamp=exportEffects.capture(h.PM,['gradient'])!;
  item.params[0]!.defaultFrom!.factor=.25;
  expect(exportEffects.valid(h.PM,stamp)).toBe(false);
  expect(exportEffects.allow(h.PM,['gradient'])).toBe(true);
  item.params[0]!.defaultFrom!.factor=NaN;
  expect(exportEffects.allow(h.PM,['gradient'])).toBe(false);
});
