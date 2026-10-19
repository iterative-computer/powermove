/* Detached scopes have no write barrier: callers can mutate any original
 * reference. Validate exactly the input reads of the previous native answer,
 * including container shapes and identities. Never infer immutability from
 * an unchanged parent identity. */
import type { PathPart } from '../../../shared/patch';
import type { PmWasm } from './wasm';

type Input = { path: PathPart[]; value: any; keys: string[] | null; length?: number };
type Answer = { wasm: PmWasm; revision: number; context: number; mirror: any; inputs: Input[]; reply: any; id: string; key: string };
type Scope = { id: string; layers: any; structure: any[]; answer?: Answer };
let serial = 0;
const object = (v: any): boolean => v !== null && typeof v === 'object';
const scalar = (v: any): any => typeof v === 'number' && !Number.isFinite(v) ? null : v === undefined ? null : v;
const keys = (v: any): string[] | null => !object(v) ? null : Array.isArray(v)
  ? Array.from({ length: v.length }, (_, i) => String(i))
  : Object.keys(v).filter(k => v[k] !== undefined && typeof v[k] !== 'function' && typeof v[k] !== 'symbol');
const at = (root: any, path: PathPart[]): any => { for (const k of path) { if (!object(root)) return undefined; root = root[k]; } return root; };
const snapshot = (root: any, path: PathPart[]): Input => {
  const value = at(root, path);
  return { path, value, keys: Array.isArray(value) ? null : keys(value), ...(Array.isArray(value) ? {length:value.length} : {}) };
};
const shallow = (a: any, b: any): boolean => {
  if (!object(a) || !object(b)) return Object.is(scalar(a), scalar(b)) || scalar(a) === scalar(b);
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a)) return a.length === b.length;
  const ka = keys(a)!, kb = keys(b)!;
  return ka.length === kb.length && ka.every(k => Object.prototype.hasOwnProperty.call(b, k));
};
const unchanged = (root: any, input: Input): boolean => {
  const value = at(root, input.path);
  if (!Object.is(value, input.value) || (object(value) && typeof value.toJSON === 'function')) return false;
  if (input.length !== undefined) return value.length === input.length;
  if (!input.keys) return true;
  const ks = keys(value)!;
  return ks.length === input.keys.length && ks.every((k, i) => k === input.keys![i]);
};
const json = (v: any): any => { const text = JSON.stringify(v); return text === undefined ? undefined : JSON.parse(text); };
function write(root: any, path: PathPart[], value: any): any {
  if (!path.length) return value;
  const owner = at(root, path.slice(0, -1)), key = path[path.length - 1]!;
  if (object(owner)) { if (value === undefined) delete owner[key]; else owner[key] = value; }
  return root;
}

export class DetachedScopes {
  private scopes = new WeakMap<object, Scope>();
  private addresses = new Map<string, WeakRef<object>>();
  private collected = new FinalizationRegistry<string>(address => this.addresses.delete(address));

  address(comp: object): string {
    let scope = this.scopes.get(comp);
    if (!scope) {
      scope = { id: '@scope:' + ++serial, layers: null, structure: [] };
      this.scopes.set(comp, scope);
      this.addresses.set(scope.id, new WeakRef(comp));
      this.collected.register(comp, scope.id);
    }
    return scope.id;
  }

  /** A new operation/time reuses the scope, but discovers and hydrates its
   * own reads. Only the last answer is memoized; Rust bounds resident scopes. */
  query(address: string, request: Record<string, unknown>, wasm: PmWasm, revision: number): any {
    const comp: any = this.addresses.get(address)?.deref();
    if (!comp) throw new Error('Detached scope was collected');
    const scope = this.scopes.get(comp)!;
    const serialized = () => wasm.query({...request, comp:'@detached', compJsonText:JSON.stringify(comp) ?? 'undefined'});
    if (typeof comp.toJSON === 'function') { scope.answer=undefined; return this.copy(serialized()); }
    const layers = Array.isArray(comp.layers) ? comp.layers : [];
    // Structural edits invalidate the entire scope even when this answer did
    // not traverse the changed layer (parent/index lookup plans depend on it).
    let structural = scope.layers !== comp.layers || scope.structure.length !== layers.length * 4;
    if (!structural) for (let i=0;i<layers.length;i++) {
      const layer=layers[i], offset=i*4;
      if (scope.structure[offset] !== layer || scope.structure[offset+1] !== layer?.id
        || scope.structure[offset+2] !== layer?.parent || scope.structure[offset+3] !== layer?.group) {
        structural=true; break;
      }
    }
    if (structural) {
      scope.layers = comp.layers;
      scope.structure = [];
      for (const layer of layers) scope.structure.push(layer,layer?.id,layer?.parent,layer?.group);
      scope.answer = undefined;
    }
    const key = JSON.stringify(request);
    let answer = scope.answer;
    if (answer && (answer.wasm !== wasm || answer.revision !== revision || answer.context !== wasm.contextRevision)) { scope.answer = undefined; answer = undefined; }
    const changed = answer?.inputs.filter(input => !unchanged(comp, input));
    if (changed?.some(input => { const value=at(comp,input.path); return object(value) && typeof value.toJSON === 'function'; })) {
      scope.answer=undefined;
      return this.copy(serialized());
    }
    if (answer && answer.key === key && !changed!.length) return this.copy(answer.reply);
    if (!answer) {
      const mirror=json(comp);
      if (!object(mirror) || Array.isArray(mirror)) return this.copy(wasm.query({...request,comp:'@detached',compJsonText:JSON.stringify(mirror) ?? 'undefined'}));
      answer = { id: 'scope-' + ++serial, wasm, revision, context: wasm.contextRevision, mirror, inputs: [], reply: null, key };
    }
    const payload: Record<string, unknown> = { ...request, comp: '@detached', scopeId: answer.id };
    if (!answer.reply) payload.compJson = answer.mirror;
    else if (changed!.length) payload.scopePatches = this.patches(comp, answer, changed!.map(i => i.path));
    let fresh = !answer.reply;
    const repaired = new Set<string>();
    for (;;) {
      const reply = wasm.query(payload);
      delete payload.compJson;
      delete payload.scopePatches;
      if (reply.throw?.message === 'detached-scope-missing') {
        answer.mirror = json(comp);
        fresh = true;
        payload.compJson = answer.mirror;
        continue;
      }
      if (!Array.isArray(reply.reads)) throw new Error('Detached scope reply omitted input reads');
      if (reply.reads.some((p: PathPart[]) => { const value=at(comp,p); return object(value) && typeof value.toJSON === 'function'; })) {
        scope.answer=undefined;
        return this.copy(fresh ? reply : serialized());
      }
      fresh = false;
      // A changed condition can reveal previously unread data. Hydrate those
      // inputs from the live scope and re-evaluate until every actual read is
      // consistent with it. Snapshot identities alone cannot establish this.
      const stale = reply.reads.filter((p: PathPart[]) => !shallow(at(comp, p), at(answer!.mirror, p)));
      if (stale.length) {
        // toJSON/boxed host objects can serialize to a different shape from
        // their live object. Keep their full native serialization semantics;
        // such a scope cannot reuse a plain JSON read-set snapshot.
        if (stale.some((p: PathPart[]) => repaired.has(JSON.stringify(p)))) {
          scope.answer = undefined;
          return this.copy(wasm.query({...request, comp:'@detached', compJson:json(comp)}));
        }
        stale.forEach((p: PathPart[]) => repaired.add(JSON.stringify(p)));
        payload.scopePatches = this.patches(comp, answer, stale);
        continue;
      }
      answer.inputs = reply.reads.map((p: PathPart[]) => snapshot(comp, p));
      answer.reply = reply;
      answer.key = key;
      if ('ok' in reply || reply.throw) scope.answer = answer;
      return this.copy(reply);
    }
  }

  private patches(comp: any, answer: Answer, paths: PathPart[][]): any[] {
    const selected: PathPart[][] = [];
    for (const path of paths.sort((a,b) => a.length-b.length)) {
      if (!selected.some(p => p.length <= path.length && p.every((k,i) => k === path[i]))) selected.push(path);
    }
    return selected.map(path => {
      // JSON serializes a hole/undefined/function array entry as null.
      // A delete patch would splice the Rust array, changing its length.
      const raw = json(at(comp, path));
      const value = raw === undefined && path.length && Array.isArray(at(comp, path.slice(0, -1))) ? null : raw;
      answer.mirror = write(answer.mirror, path, value);
      return { path, exists: value !== undefined, ...(value !== undefined ? { value } : {}) };
    });
  }

  private copy(reply: any): any {
    // Queries returning objects have fresh value semantics. Do not expose the
    // cached answer to a caller who can mutate it.
    return object(reply.ok) ? structuredClone(reply) : reply;
  }
}
