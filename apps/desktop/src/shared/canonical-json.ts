/*
 * Canonical JSON, the canonical project form, and the canonical hash, shared
 * byte for byte with the native engine (engine/crates/pm-model: json/write.rs,
 * canonical.rs, hash.rs). The production consistency check compares
 * `canonicalProjectHash(PM.proj)` with the engine mirror's hash for the same
 * revision (docs/rust-engine.md, section 6).
 *
 * Canonical serialization (normative): `JSON.stringify(value)` with no
 * indentation, except that every object's keys are written in ascending order
 * of their UTF-16 code units (the default `Array.prototype.sort()` order, so
 * "10" sorts before "2"). Everything else is JSON.stringify's: numbers as
 * Number::toString (shortest round-trip digits, `-0` as `0`, `1e+21`, `1e-7`),
 * NaN and ±Infinity as `null`, strings as QuoteJSONString (lone surrogates as
 * lowercase `\udxxx`, other control characters as `\b \t \n \f \r` or
 * lowercase `\u00xx`, everything else literal), `undefined` members omitted,
 * `undefined` array elements and holes as `null`, `toJSON` honored. The hash
 * is the SHA-256 of the UTF-8 bytes as 64 lowercase hex digits.
 *
 * Canonical project form: the open composition is stored like every other
 * composition. A project keeps the open composition's fields at its top level
 * (`w`, `h`, `fps`, `layers`, ... plus `compId` and `compName`) and the others
 * in `comps` (legacy/core/compositions.ts). The canonical form moves the open
 * composition into `comps[compId]` as `{ id: compId, name: compName, ...its
 * composition fields }` and keeps `compId` at the top level as the pointer.
 * Patches canonicalized by `PM.Comps.canonicalize` address exactly this form
 * (`['comps', compId, 'layers', ...]`).
 */
import { Sha256 } from './sha256';

/** Must equal COMP_FIELDS in legacy/core/compositions.ts and pm-model's list (tests check both). */
export const CANONICAL_COMP_FIELDS = ['w', 'h', 'fps', 'fpsRational', 'dur', 'bg', 'backgroundFill', 'layers', 'markers', 'work', 'shutter', 'audioGain', 'render3d'] as const;
const COMP_FIELD_SET: ReadonlySet<string> = new Set(CANONICAL_COMP_FIELDS);

const hasOwn = (value: object, key: string) => Object.prototype.hasOwnProperty.call(value, key);
const isRecord = (value: unknown): value is Record<string, any> => !!value && typeof value === 'object' && !Array.isArray(value);
/** Null-prototype records accept any key, `__proto__` included, as an own member. */
const record = (): Record<string, any> => Object.create(null);

function serialize(key: string, value: any, stack: Set<object>): string | undefined {
  if (value && typeof value === 'object' && typeof value.toJSON === 'function') value = value.toJSON(key);
  if (value instanceof Number || value instanceof String || value instanceof Boolean) value = value.valueOf();
  if (value === null) return 'null';
  switch (typeof value) {
    case 'boolean': return value ? 'true' : 'false';
    case 'number': return Number.isFinite(value) ? JSON.stringify(value) : 'null';
    case 'string': return JSON.stringify(value);
    case 'bigint': throw new TypeError('Do not know how to serialize a BigInt');
    case 'object': break;
    default: return undefined; // undefined, functions, symbols
  }
  if (stack.has(value)) throw new TypeError('Converting circular structure to JSON');
  stack.add(value);
  let out: string;
  if (Array.isArray(value)) {
    const items: string[] = [];
    for (let index = 0; index < value.length; index++) items.push(serialize(String(index), value[index], stack) ?? 'null');
    out = '[' + items.join(',') + ']';
  } else {
    const members: string[] = [];
    for (const name of Object.keys(value).sort()) {
      const text = serialize(name, value[name], stack);
      if (text !== undefined) members.push(JSON.stringify(name) + ':' + text);
    }
    out = '{' + members.join(',') + '}';
  }
  stack.delete(value);
  return out;
}

/** The canonical serialization; `undefined` for values JSON.stringify skips. */
export function canonicalStringify(value: unknown): string | undefined {
  return serialize('', value, new Set());
}

/** A value as serialization sees it (`toJSON`, boxed primitives), and
 *  whether it is written at all (`undefined`, functions, symbols are not). */
function prepared(key: string, value: any): { value: any; written: boolean } {
  if (value && typeof value === 'object' && typeof value.toJSON === 'function') value = value.toJSON(key);
  if (value instanceof Number || value instanceof String || value instanceof Boolean) value = value.valueOf();
  const type = typeof value;
  return { value, written: value === null || type === 'boolean' || type === 'number' || type === 'string' || type === 'bigint' || type === 'object' };
}

/** The canonical serialization of a prepared, written value, in pieces. */
function stream(value: any, stack: Set<object>, emit: (text: string) => void): void {
  if (value === null) return emit('null');
  switch (typeof value) {
    case 'boolean': return emit(value ? 'true' : 'false');
    case 'number': return emit(Number.isFinite(value) ? JSON.stringify(value) : 'null');
    case 'string': return emit(JSON.stringify(value));
    case 'bigint': throw new TypeError('Do not know how to serialize a BigInt');
  }
  if (stack.has(value)) throw new TypeError('Converting circular structure to JSON');
  stack.add(value);
  if (Array.isArray(value)) {
    emit('[');
    for (let index = 0; index < value.length; index++) {
      if (index) emit(',');
      const item = prepared(String(index), value[index]);
      if (item.written) stream(item.value, stack, emit);
      else emit('null');
    }
    emit(']');
  } else {
    emit('{');
    let first = true;
    for (const name of Object.keys(value).sort()) {
      const member = prepared(name, value[name]);
      if (!member.written) continue;
      emit((first ? '' : ',') + JSON.stringify(name) + ':');
      first = false;
      stream(member.value, stack, emit);
    }
    emit('}');
  }
  stack.delete(value);
}

/**
 * SHA-256 of the canonical serialization, as lowercase hex. The text is
 * hashed as it is produced, in pieces of whole JSON tokens (never splitting
 * a surrogate pair), so a large project (C1: 180 MB) never becomes one
 * string: the save-time consistency check used to build two of them.
 */
export function canonicalHash(value: unknown): string {
  const root = prepared('', value);
  if (!root.written) throw new TypeError('The value has no JSON representation');
  const hash = new Sha256();
  const encoder = new TextEncoder();
  let pending: string[] = [];
  let size = 0;
  const flush = () => {
    if (!pending.length) return;
    hash.update(encoder.encode(pending.join('')));
    pending = [];
    size = 0;
  };
  stream(root.value, new Set(), (text) => {
    pending.push(text);
    size += text.length;
    if (size >= 1 << 16) flush();
  });
  flush();
  return hash.digest();
}

/**
 * The canonical project form. Returns the input unchanged when it is not a
 * project with a non-empty string `compId`, or when its `comps` member exists
 * but is not a plain object. A stale `comps[compId]` entry is replaced, as
 * opening a composition does. Shallow: nested values are shared, not copied.
 */
export function canonicalProject(project: any): any {
  if (!isRecord(project)) return project;
  const id = project.compId;
  if (typeof id !== 'string' || !id) return project;
  if (hasOwn(project, 'comps') && !isRecord(project.comps)) return project;
  const out = record();
  const comp = record();
  comp.id = id;
  if (hasOwn(project, 'compName')) comp.name = project.compName;
  for (const key of CANONICAL_COMP_FIELDS) if (hasOwn(project, key)) comp[key] = project[key];
  for (const key of Object.keys(project)) {
    if (COMP_FIELD_SET.has(key) || key === 'compName' || key === 'comps') continue;
    out[key] = project[key];
  }
  const comps = record();
  if (isRecord(project.comps)) for (const key of Object.keys(project.comps)) if (key !== id) comps[key] = project.comps[key];
  comps[id] = comp;
  out.comps = comps;
  return out;
}

/**
 * The inverse of canonicalProject for well-formed projects (a non-empty
 * string `compId`, a `comps` record without the open composition):
 * `realizeProject(canonicalProject(p))` serializes exactly like `p`. Members
 * of `comps[compId]` other than `name` and the composition fields are dropped,
 * as `realizePatches` drops them.
 */
export function realizeProject(canonical: any): any {
  if (!isRecord(canonical)) return canonical;
  const id = canonical.compId;
  if (typeof id !== 'string' || !id || !isRecord(canonical.comps) || !hasOwn(canonical.comps, id) || !isRecord(canonical.comps[id])) return canonical;
  const out = record();
  for (const key of Object.keys(canonical)) if (key !== 'comps') out[key] = canonical[key];
  const comp = canonical.comps[id];
  if (hasOwn(comp, 'name')) out.compName = comp.name;
  for (const key of CANONICAL_COMP_FIELDS) if (hasOwn(comp, key)) out[key] = comp[key];
  const comps = record();
  for (const key of Object.keys(canonical.comps)) if (key !== id) comps[key] = canonical.comps[key];
  out.comps = comps;
  return out;
}

/** The hash the production consistency check compares: canonicalHash(canonicalProject(project)). */
export function canonicalProjectHash(project: any): string {
  return canonicalHash(canonicalProject(project));
}
