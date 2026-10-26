/*
 * The two texts an opened project is written as, from one serialization.
 *
 * Opening a large project writes it twice: its local slot (the envelope
 * `{"v":…,"proj":…}`, exactly `JSON.stringify` of it, which the opened file's
 * hash and the slot's digest read) and, with Use New Engine on, the engine's
 * copy (the canonical project form, shared/canonical-json.ts, which moves the
 * open composition into `comps[compId]`). The canonical form is a shallow
 * rearrangement of the project's own members, so both texts are assembled
 * from the same text of each member, serialized once: the project's members,
 * its compositions, and their layers are serialized one native call each,
 * yielding between calls. The canonical text is made first, in order, and
 * can be streamed while the rest is still being serialized.
 *
 * Exactly `JSON.stringify`'s text: its member order (Object.keys), members
 * without a JSON form left out and array elements without one as `null`.
 * A `toJSON` at a value serialized alone here (where its own key would change
 * what it returns) is not assembled: the result is null and the caller
 * serializes as before.
 */
import { CANONICAL_COMP_FIELDS } from '../../../../shared/canonical-json';
import { nextTask, type Pause } from './json-slices';

export interface ProjectTexts {
  /** `JSON.stringify(canonicalProject(project))` in pieces, in order, each as soon as it is made. */
  canonical: AsyncIterable<string>;
  /** `JSON.stringify({ v: version, proj: project })`, once every member is serialized. */
  slot: Promise<string>;
}

const SLICE_MS = 4;
const COMP_FIELDS: ReadonlySet<string> = new Set(CANONICAL_COMP_FIELDS);
const hasOwn = (value: object, key: string) => Object.prototype.hasOwnProperty.call(value, key);
const isRecord = (value: unknown): value is Record<string, any> => !!value && typeof value === 'object' && !Array.isArray(value);
const hooked = (value: unknown) => value !== null && typeof value === 'object' && typeof (value as any).toJSON === 'function';
const boxed = (value: object) => value instanceof Number || value instanceof String || value instanceof Boolean;
/** How many levels of a member are assembled here rather than serialized whole. */
const depthOf = (key: string) => (key === 'layers' ? 1 : 0);

/** Whether `text(value, depth)` would serialize a value with a toJSON alone. */
function refuses(value: any, depth: number): boolean {
  if (hooked(value)) return true;
  if (depth === 0 || value === null || typeof value !== 'object' || boxed(value)) return false;
  if (Array.isArray(value)) { for (let i = 0; i < value.length; i++) if (refuses(value[i], depth - 1)) return true; return false; }
  for (const key of Object.keys(value)) if (refuses(value[key], depth - 1)) return true;
  return false;
}

export function projectTexts(version: string, project: any, pause: Pause = nextTask): ProjectTexts | null {
  if (!isRecord(project) || hooked(project) || hooked(project.comps)) return null;
  const compsRecord = isRecord(project.comps) ? project.comps : null;
  const id = project.compId;
  const canonicalForm = typeof id === 'string' && !!id && (!hasOwn(project, 'comps') || !!compsRecord);
  for (const key of Object.keys(project)) {
    if (key === 'comps' && compsRecord) { for (const comp of Object.keys(compsRecord)) if (refuses(compsRecord[comp], 2)) return null; }
    else if (refuses(project[key], depthOf(key))) return null;
  }
  if (canonicalForm) for (const key of ['compName', ...CANONICAL_COMP_FIELDS]) if (hasOwn(project, key) && refuses(project[key], depthOf(key))) return null;

  // The canonical pieces made so far, and whoever waits for more.
  const pieces: string[] = [];
  let finished = false, failure: unknown = null;
  const waiting: Array<() => void> = [];
  const wake = () => { for (const resolve of waiting.splice(0)) resolve(); };
  const push = (...made: string[]) => { for (const piece of made) pieces.push(piece); wake(); };

  let deadline = performance.now() + SLICE_MS;
  /** A value's text in pieces (undefined: no JSON form); arrays and records
   *  `depth` levels down are assembled from their members' texts. */
  const text = async (value: any, depth: number): Promise<string[] | undefined> => {
    if (depth === 0 || value === null || typeof value !== 'object' || boxed(value)) {
      const json = JSON.stringify(value);
      if (performance.now() >= deadline) { await pause(); deadline = performance.now() + SLICE_MS; }
      return json === undefined ? undefined : [json];
    }
    const out: string[] = [];
    if (Array.isArray(value)) {
      out.push('[');
      for (let index = 0; index < value.length; index++) {
        if (index) out.push(',');
        const item = await text(value[index], depth - 1);
        if (item) out.push(...item); else out.push('null');
      }
      out.push(']');
      return out;
    }
    out.push('{');
    let first = true;
    for (const key of Object.keys(value)) {
      const member = await text(value[key], depth - 1);
      if (!member) continue;
      out.push(`${first ? '' : ','}${JSON.stringify(key)}:`, ...member);
      first = false;
    }
    out.push('}');
    return out;
  };
  /** A record's text from its members' texts, in order. */
  const record = (entries: Iterable<[string, string[] | undefined]>): string[] => {
    const out = ['{'];
    let first = true;
    for (const [key, member] of entries) {
      if (!member) continue;
      out.push(`${first ? '' : ','}${JSON.stringify(key)}:`, ...member);
      first = false;
    }
    out.push('}');
    return out;
  };

  const members = new Map<string, string[] | undefined>();
  const comps = new Map<string, string[] | undefined>();
  const member = async (key: string) => {
    if (!members.has(key)) members.set(key, key === 'comps' && compsRecord ? undefined : await text(project[key], depthOf(key)));
    return members.get(key);
  };
  const slot = (async () => {
    try {
      if (canonicalForm) {
        // canonicalProject (shared/canonical-json.ts), member for member: the
        // project's own members, then the other compositions, then the open one.
        let first = true;
        const entry = (key: string, made: string[] | undefined) => {
          if (!made) return;
          push(`${first ? '{' : ','}${JSON.stringify(key)}:`, ...made);
          first = false;
        };
        for (const key of Object.keys(project)) {
          if (COMP_FIELDS.has(key) || key === 'compName' || key === 'comps') continue;
          entry(key, await member(key));
        }
        push(`${first ? '{' : ','}"comps":{`);
        let firstComp = true;
        for (const comp of compsRecord ? Object.keys(compsRecord) : []) {
          const made = await text(compsRecord![comp], 2);
          comps.set(comp, made);
          if (comp === id || !made) continue;
          push(`${firstComp ? '' : ','}${JSON.stringify(comp)}:`, ...made);
          firstComp = false;
        }
        push(`${firstComp ? '' : ','}${JSON.stringify(id)}:{"id":${JSON.stringify(id)}`);
        for (const key of ['compName', ...CANONICAL_COMP_FIELDS]) {
          if (!hasOwn(project, key)) continue;
          // Own but not enumerable: in the composition, never in the slot.
          const made = Object.prototype.propertyIsEnumerable.call(project, key) ? await member(key) : await text(project[key], depthOf(key));
          if (made) push(`,${JSON.stringify(key === 'compName' ? 'name' : key)}:`, ...made);
        }
        push('}}}');
      } else {
        for (const comp of compsRecord ? Object.keys(compsRecord) : []) comps.set(comp, await text(compsRecord![comp], 2));
      }
      for (const key of Object.keys(project)) await member(key);
      const realized = record(Object.keys(project).map((key): [string, string[] | undefined] =>
        [key, key === 'comps' && compsRecord ? record(comps) : members.get(key)]));
      if (!canonicalForm) push(...realized);
      return `{"v":${JSON.stringify(version)},"proj":${realized.join('')}}`;
    } catch (error) {
      failure = error;
      throw error;
    } finally {
      finished = true;
      wake();
    }
  })();
  slot.catch(() => {});
  const canonical: AsyncIterable<string> = {
    async *[Symbol.asyncIterator]() {
      for (let next = 0; ;) {
        while (next < pieces.length) yield pieces[next++]!;
        if (finished) {
          if (failure !== null) throw failure;
          if (next >= pieces.length) return;
          continue;
        }
        await new Promise<void>(resolve => waiting.push(resolve));
      }
    },
  };
  return { canonical, slot };
}
