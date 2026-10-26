import type { EffectDefinition } from './api';

/** Unversioned saved instances always mean the original implementation. */
export function effectVersion(value: unknown): number | undefined {
  if (value == null) return 1;
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0 ? value : undefined;
}

/** Definitions keep historical revisions as data, shared by native and web export. */
export function resolveEffectDefinition(definition: EffectDefinition | undefined, requested?: number): EffectDefinition | undefined {
  if (!definition) return undefined;
  if (requested === undefined) return definition;
  const version = effectVersion(requested);
  if (version === undefined) return undefined;
  if (version === (definition.version ?? 1)) return definition;
  return Array.isArray(definition.legacy) ? definition.legacy.find(revision => revision.version === version) : undefined;
}

/** Keep the existing program key for current definitions, isolate old programs. */
export function effectProgramKey(type: string, requested: unknown, current: EffectDefinition | undefined): string {
  const version = effectVersion(requested);
  return version === (current?.version ?? 1) ? 'fx:' + type : `fx:${type}@${String(version)}`;
}
