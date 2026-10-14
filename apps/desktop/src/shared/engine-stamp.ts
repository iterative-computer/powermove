/*
 * The `requiresEngine: "native"` stamp (plan sections 6 and 21).
 *
 * While the legacy engine can still be selected, a project that uses a
 * feature only the new engine has is stamped at its root. The legacy engine
 * refuses a stamped project with REQUIRES_NATIVE_MESSAGE instead of opening
 * it and quietly dropping what it cannot show. The stamp follows the
 * project: it is set while any native-only feature is in use and removed
 * once none is, so a project that stops linking media opens in either
 * engine again.
 *
 * Native-only features, from the section 6 table, as their fields exist
 * today: linked media (asset `storage: "linked"` or `link`) and
 * interpretation overrides (asset `interpret`). Lanes that add the other
 * native-only fields (precomp `keepFrameRate`, clip `pitch`, project
 * `color`) add a check to NATIVE_ONLY_CHECKS.
 */
import { REQUIRES_NATIVE_MESSAGE } from './media-store';

export { REQUIRES_NATIVE_MESSAGE };
export const REQUIRES_ENGINE_NATIVE = 'native';

const isRecord = (value: unknown): value is Record<string, any> => !!value && typeof value === 'object' && !Array.isArray(value);

export type NativeOnlyFeature = 'linkedMedia' | 'interpretation';

type Check = { feature: NativeOnlyFeature; test(project: Record<string, any>): boolean };

const assets = (project: Record<string, any>): any[] => (isRecord(project['assets']) ? Object.values(project['assets']) : []);

export const NATIVE_ONLY_CHECKS: readonly Check[] = [
  { feature: 'linkedMedia', test: (project) => assets(project).some((asset) => isRecord(asset) && (asset['storage'] === 'linked' || asset['link'] != null)) },
  { feature: 'interpretation', test: (project) => assets(project).some((asset) => isRecord(asset) && asset['interpret'] != null) }
];

/** The native-only features a project uses. */
export function nativeOnlyFeatures(project: unknown): NativeOnlyFeature[] {
  if (!isRecord(project)) return [];
  return NATIVE_ONLY_CHECKS.filter((check) => check.test(project)).map((check) => check.feature);
}

/** Sets or removes the stamp to match the project. True when it changed. */
export function applyEngineStamp(project: unknown): boolean {
  if (!isRecord(project)) return false;
  const needed = nativeOnlyFeatures(project).length > 0;
  const stamped = project['requiresEngine'] === REQUIRES_ENGINE_NATIVE;
  if (needed && !stamped) { project['requiresEngine'] = REQUIRES_ENGINE_NATIVE; return true; }
  // Only this build's own stamp is removed; an unknown value is left alone.
  if (!needed && stamped) { delete project['requiresEngine']; return true; }
  return false;
}

/**
 * True when a project file document (`{ proj, ... }`) or a bare project is
 * stamped for the new engine. Any value other than absent counts: an
 * unknown engine requirement must not open in the legacy engine.
 */
export function requiresNativeEngine(document: unknown): boolean {
  if (!isRecord(document)) return false;
  const stamp = (holder: unknown) => isRecord(holder) && holder['requiresEngine'] != null;
  return stamp(document) || stamp(document['proj']);
}

/** True when this build, running the legacy engine, must refuse the document. */
export function refusesProject(document: unknown, newEngine: boolean): boolean {
  return !newEngine && requiresNativeEngine(document);
}
