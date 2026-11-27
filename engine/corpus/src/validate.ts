/**
 * Document validation. `sanitizeProject` (apps/desktop/src/renderer/src/core/
 * validate/project.ts) normalizes rather than rejects, so a document is valid
 * when it is a fixed point of that sanitizer: every value the generator wrote
 * survives unchanged. Additions (defaults the sanitizer fills in) are allowed;
 * drops and rewrites are errors. 3D layer content must also be a fixed point of
 * the scene schema the app parses it with.
 *
 * One deliberate exception: `sanitizeProject` keeps only the 16 built-in effect
 * types, while the app's loader (legacy/app.ts) keeps extension effects and marks
 * unregistered ones `missing`. Declared extension and missing effects
 * (catalog.ts) are taken out before the sanitizer comparison and checked on
 * their own; the capture records how the running app loaded them.
 */
import { effects, EXTENSION_EFFECTS, isForeignEffect, sanitizeProject, scene3d, transitions } from './catalog';
import type { Doc } from './doc';

export interface Issue { path: string; message: string }

const isRecord = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const isChannel = (value: unknown): value is { v: unknown; kf: unknown[]; expr: unknown } => isRecord(value) && 'v' in value && Array.isArray(value.kf);

/**
 * Keys saved before the temporal-ease rewrite carry `eo`/`ei` unit handles and
 * a `hold` flag. `adoptTemporalEase` converts them to native AE ease and drops
 * the legacy fields; that documented migration is the one rewrite allowed.
 */
const LEGACY_KEY_FIELDS = new Set(['eo', 'ei', 'hold']);
const KEYFRAME_PATH = /\.kf\[\d+\]$/;

/** Every leaf of `expected` must appear unchanged in `actual`. */
export function compareSubset(expected: unknown, actual: unknown, path: string, issues: Issue[]): void {
  if (Array.isArray(expected)) {
    if (!Array.isArray(actual)) { issues.push({ path, message: `array became ${JSON.stringify(actual)?.slice(0, 80)}` }); return; }
    if (actual.length !== expected.length) issues.push({ path, message: `array length ${expected.length} became ${actual.length}` });
    expected.forEach((item, index) => compareSubset(item, actual[index], `${path}[${index}]`, issues));
    return;
  }
  if (isRecord(expected)) {
    if (!isRecord(actual)) { issues.push({ path, message: `object became ${JSON.stringify(actual)?.slice(0, 80)}` }); return; }
    for (const [key, value] of Object.entries(expected)) {
      if (!(key in actual)) {
        if (LEGACY_KEY_FIELDS.has(key) && KEYFRAME_PATH.test(path) && typeof actual.inInterp === 'string' && typeof actual.outInterp === 'string') continue;
        issues.push({ path: `${path}.${key}`, message: 'dropped by the sanitizer' });
        continue;
      }
      compareSubset(value, actual[key], `${path}.${key}`, issues);
    }
    return;
  }
  // A static scalar the sanitizer promotes to a channel keeps its value.
  if (isChannel(actual) && actual.kf.length === 0 && actual.expr === null && Object.is(actual.v, expected)) return;
  if (!Object.is(expected, actual)) issues.push({ path, message: `${JSON.stringify(expected)} became ${JSON.stringify(actual)}` });
}

function deepEqual(a: unknown, b: unknown): boolean {
  return JSON.stringify(sortKeys(a)) === JSON.stringify(sortKeys(b));
}

export function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (isRecord(value)) return Object.fromEntries(Object.keys(value).sort().map(key => [key, sortKeys(value[key])]));
  return value;
}

function compositions(doc: Doc): Array<[string, Doc]> {
  return [['root', doc], ...Object.entries<Doc>(doc.comps ?? {}).map(([id, comp]) => [`comps.${id}`, comp] as [string, Doc])];
}

export function validateDocument(doc: Doc): Issue[] {
  const issues: Issue[] = [];
  const serialized = JSON.stringify(doc);
  if (!deepEqual(JSON.parse(serialized), doc)) issues.push({ path: '', message: 'document does not survive a JSON round trip (NaN, Infinity, or undefined values)' });
  // -0 serializes as 0, so the in-memory document and the file would disagree.
  const negativeZero = (value: unknown, path: string): void => {
    if (Object.is(value, -0)) issues.push({ path, message: '-0 does not survive serialization' });
    else if (Array.isArray(value)) value.forEach((item, index) => negativeZero(item, `${path}[${index}]`));
    else if (isRecord(value)) for (const [key, item] of Object.entries(value)) negativeZero(item, `${path}.${key}`);
  };
  negativeZero(doc, 'doc');
  const comparable = JSON.parse(serialized);
  for (const [, comp] of compositions(comparable)) for (const layer of comp.layers as Doc[]) {
    if (Array.isArray(layer.fx)) layer.fx = layer.fx.filter((fx: Doc) => !isForeignEffect(fx.type));
  }
  let sanitized: Doc;
  try { sanitized = sanitizeProject(structuredClone(comparable)); }
  catch (error) { return [...issues, { path: '', message: `sanitizeProject threw: ${(error as Error).message}` }]; }
  compareSubset(comparable, sanitized, 'doc', issues);

  const effectIds = new Map(effects.map(effect => [effect.id, new Set(effect.params.map(param => param.k))]));
  const transitionIds = new Map(transitions.map(transition => [transition.id, new Set(transition.params.map(param => param.k))]));
  const keyIds = new Set<string>();
  const collectKeys = (value: unknown, path: string) => {
    if (Array.isArray(value)) value.forEach((item, index) => collectKeys(item, `${path}[${index}]`));
    else if (isRecord(value)) {
      if (isChannel(value)) for (const key of value.kf as Array<Record<string, unknown>>) {
        const id = String(key.i ?? '');
        if (!id) issues.push({ path, message: 'keyframe without an id' });
        else if (keyIds.has(id)) issues.push({ path, message: `duplicate keyframe id ${id} (the loader would reassign it)` });
        keyIds.add(id);
      }
      for (const [key, item] of Object.entries(value)) collectKeys(item, `${path}.${key}`);
    }
  };
  collectKeys(doc, 'doc');

  for (const [where, comp] of compositions(doc)) {
    const ids = new Set<string>();
    for (const [index, layer] of (comp.layers as Doc[]).entries()) {
      const at = `${where}.layers[${index}](${layer.name})`;
      if (ids.has(layer.id)) issues.push({ path: at, message: `duplicate layer id ${layer.id}` });
      ids.add(layer.id);
      for (const fx of layer.fx ?? []) {
        if (isForeignEffect(fx.type)) {
          const allowed = EXTENSION_EFFECTS[fx.type]?.params;
          if (typeof fx.id !== 'string' || !fx.id) issues.push({ path: at, message: `effect ${fx.type} has no id` });
          for (const [key, value] of Object.entries<any>(fx.p ?? {})) {
            if (allowed && !allowed.includes(key)) issues.push({ path: at, message: `extension effect ${fx.type} has no parameter ${key}` });
            if (!value || typeof value !== 'object' || !Array.isArray(value.kf) || !('v' in value)) issues.push({ path: at, message: `effect ${fx.type} parameter ${key} is not a channel` });
          }
          continue;
        }
        const params = effectIds.get(fx.type);
        if (!params) { issues.push({ path: at, message: `unknown effect ${fx.type}` }); continue; }
        for (const key of Object.keys(fx.p)) if (!params.has(key)) issues.push({ path: at, message: `effect ${fx.type} has no parameter ${key}` });
        for (const key of params) if (!(key in fx.p)) issues.push({ path: at, message: `effect ${fx.type} is missing parameter ${key}` });
      }
      for (const field of ['transitionIn', 'transitionOut']) {
        const transition = layer[field];
        if (!transition) continue;
        const params = transitionIds.get(transition.type);
        if (!params) { issues.push({ path: at, message: `unknown transition ${transition.type}` }); continue; }
        for (const key of Object.keys(transition.p)) if (!params.has(key)) issues.push({ path: at, message: `transition ${transition.type} has no parameter ${key}` });
      }
      if (layer.type === 'precomp' && layer.d.comp && !doc.comps?.[layer.d.comp] && layer.d.comp !== doc.compId) issues.push({ path: at, message: `precomp references missing composition ${layer.d.comp}` });
      if (layer.parent && !comp.layers.some((other: Doc) => other.id === layer.parent)) issues.push({ path: at, message: `parent ${layer.parent} is not in the same composition` });
      if (layer.matteSource && !comp.layers.some((other: Doc) => other.id === layer.matteSource)) issues.push({ path: at, message: `matte source ${layer.matteSource} is not in the same composition` });
      if (layer.type === 'extension') {
        const role = (Object.entries(scene3d.definitions) as Array<[string, string]>).find(([, id]) => id === layer.d.definition)?.[0] as 'object' | 'light' | 'camera' | undefined;
        if (role) {
          try {
            const parsed = scene3d.parseLayer3DData(role, structuredClone(layer.d.data));
            if (!deepEqual(parsed, layer.d.data)) issues.push({ path: `${at}.d.data`, message: `3D ${role} content is not a fixed point of the scene schema` });
          } catch (error) { issues.push({ path: `${at}.d.data`, message: `3D ${role} content rejected: ${(error as Error).message}` }); }
          if (!layer.threeD) issues.push({ path: at, message: '3D layers must have threeD set' });
        }
        if (layer.d.definition === scene3d.sceneDefinition) {
          try { scene3d.parseScene(structuredClone(layer.d.data.scene)); }
          catch (error) { issues.push({ path: `${at}.d.data.scene`, message: `legacy scene rejected: ${(error as Error).message}` }); }
        }
      }
    }
  }
  return issues;
}
