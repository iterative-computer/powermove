/*
 * The write-tracking Proxy for development and test builds (docs/rust-engine.md,
 * section 6, "Making every write visible", item 1).
 *
 * `PM.proj` hands out proxies; every write through them records its path
 * (relative to `PM.proj`) with the barrier. A write while no known writer is
 * active (a history transaction, `PM.doc.write`, a document replacement) is a
 * violation: `throw` mode throws, `report` mode records it and lets it
 * through. The underlying document always holds raw objects: proxies are
 * created on read and unwrapped on write, so serialization, cloning, and
 * identity checks inside the document are unchanged.
 *
 * Writes to an object obtained without going through `PM.proj` (a layer
 * created by a factory, then inserted and edited through the original
 * reference) are invisible to the Proxy. Writers announce them (`touch()`,
 * history), and the shadow-replay auditor is the net for the rest.
 *
 * Identity: an object that enters the document through a proxy is still held
 * raw by the code that inserted it, which often compares it with what it reads
 * back (`layer.masks.find(m => m.path === path)` right after pushing a new
 * mask). Until the current task ends, reads of such a fresh object return it
 * raw, so those comparisons hold; later tasks read it through the Proxy.
 */
import type { PathPart } from '../../../shared/patch';

export type ViolationMode = 'throw' | 'report';

export interface Violation {
  path: string;
  /** The first application frame of the stack, when available. */
  site: string;
  count: number;
}

const PROXIED = Symbol('pm-doc-proxied');

interface Link { parent: object | null; key: PathPart }

export interface Tracker {
  /** The proxy for a document root. */
  wrap(root: any): any;
  /** The raw object behind a proxy (or the value itself). */
  unwrap<T>(value: T): T;
  /** Deep-unwraps proxies inside a new value about to enter the document. */
  unwrapDeep<T>(value: T): T;
  /** Ends the current task's fresh objects (reads return proxies again). */
  settle(): void;
  violations(): Violation[];
  clearViolations(): void;
}

const isTrackable = (v: unknown): v is object => {
  if (!v || typeof v !== 'object') return false;
  const proto = Object.getPrototypeOf(v);
  return proto === Object.prototype || proto === Array.prototype || proto === null;
};

const firstAppFrame = (stack: string | undefined): string => {
  const lines = (stack || '').split('\n').slice(1);
  // writerStack already dropped the Proxy's frames; native frames have no location.
  const own = /\((native|<anonymous>)\)$/;
  const first = lines.find(line => !own.test(line)) ?? '';
  const harness = /eval at evaluate|__playwright|UtilityScript|<anonymous>:\d+:\d+\)?$/;
  // Code a test harness evaluates in the page (Playwright's page.evaluate),
  // not one of the app's writers.
  if (harness.test(first)) return `page.evaluate ${first.trim().replace(/^at\s+/, '').slice(0, 160)}`;
  const frame = lines.find(line => !own.test(line) && !/<anonymous>/.test(line)) || lines[0] || '';
  const where = frame.trim().replace(/^at\s+/, '').replace(/\?[^:)]*/, '').slice(0, 200);
  // An app function a test called directly (`PM.groupLayers(...)` from
  // page.evaluate) outside the transaction the app's own callers open.
  return lines.some(line => harness.test(line)) ? `page.evaluate via ${where}` : where;
};

/**
 * Where a query that met a stale mirror was asked from (test modes): the
 * first frames outside the native evaluator, and whether a test evaluated
 * the code in the page. The writer that skipped the barrier ran before it.
 */
export function querySite(stack: string | undefined = new Error().stack, frames = 4): string {
  const lines = (stack || '').split('\n').slice(1).map(line => line.trim());
  const own = /NativeApi\.|DualAnswers\.|onStale|querySite|\((native|<anonymous>)\)$/;
  const harness = /eval at evaluate|__playwright|UtilityScript/;
  const app = lines.filter(line => !own.test(line)).slice(0, frames)
    .map(line => line.replace(/^at\s+/, '').replace(/\?[^:)]*/, '').replace(/\(app:\/\/powermove\/host\//, '(').slice(0, 120));
  return `${lines.some(line => harness.test(line)) ? 'page.evaluate: ' : ''}${app.join(' < ')}`;
}

export function createTracker(options: {
  /** Called with the path (relative to the root) of every write. */
  onWrite(path: PathPart[]): void;
  /** Whether a known writer is active. */
  inWriter(): boolean;
  /** Called once per new violation (path and site). */
  onViolation?(violation: Violation): void;
  mode: ViolationMode;
}): Tracker {
  const proxies = new WeakMap<object, any>();
  const targets = new WeakMap<object, object>();
  const links = new WeakMap<object, Link>();
  const roots = new WeakSet<object>();
  const violations = new Map<string, Violation>();

  const pathOf = (target: object): PathPart[] | null => {
    const out: PathPart[] = [];
    let cur: object | null = target;
    for (let guard = 0; cur && guard < 4096; guard++) {
      if (roots.has(cur)) return out.reverse();
      const link = links.get(cur);
      if (!link) return null;
      out.push(link.key);
      cur = link.parent;
    }
    return null;
  };

  const unwrap = <T>(value: T): T => (value && typeof value === 'object' ? (targets.get(value as any) as any) ?? value : value);

  /** Objects that entered the document raw during the current task. */
  let fresh = new WeakSet<object>();
  let settling = false;
  const settle = () => { fresh = new WeakSet(); settling = false; };
  const markFresh = (v: object) => {
    fresh.add(v);
    if (!settling) { settling = true; setTimeout(settle, 0); }
  };

  const unwrapDeep = <T>(value: T, admit = false): T => {
    const seen = new Set<object>();
    const visit = (v: any): any => {
      if (!v || typeof v !== 'object') return v;
      const raw = targets.get(v);
      if (raw) return raw; // raw trees never hold proxies
      if (seen.has(v) || !isTrackable(v)) return v;
      seen.add(v);
      if (admit) markFresh(v);
      if (Array.isArray(v)) {
        for (let i = 0; i < v.length; i++) { const u = visit(v[i]); if (u !== v[i]) v[i] = u; }
      } else {
        for (const key of Object.keys(v)) {
          const desc = Object.getOwnPropertyDescriptor(v, key);
          if (!desc || !('value' in desc)) continue;
          const u = visit(desc.value);
          if (u !== desc.value) (v as Record<string, unknown>)[key] = u;
        }
      }
      return v;
    };
    return visit(value);
  };

  /** The stack from the writing code: frames from `trap` up (the Proxy's own) are dropped. */
  const writerStack = (trap: Function): string | undefined => {
    const holder: { stack?: string } = {};
    const capture = (Error as unknown as { captureStackTrace?: (o: object, f: Function) => void }).captureStackTrace;
    if (typeof capture !== 'function') return new Error().stack;
    capture(holder, trap);
    return `Error\n${String(holder.stack ?? '').split('\n').slice(1).join('\n')}`;
  };

  const record = (target: object, key: PathPart | null, trap: Function) => {
    const base = pathOf(target);
    // Writes to objects that left the document are not document writes.
    if (!base) return;
    // An array's elements and length are the array's own structure: record
    // the array, so the barrier diffs it with id-keyed operations.
    const path = Array.isArray(target) || key === null ? base : [...base, key];
    if (!options.inWriter()) {
      const where = firstAppFrame(writerStack(trap));
      const text = path.map(String).join('.');
      const id = `${text} @ ${where}`;
      const known = violations.get(id);
      if (known) known.count += 1;
      else {
        const violation = { path: text, site: where, count: 1 };
        violations.set(id, violation);
        try { options.onViolation?.(violation); } catch { /* reporting never breaks a write */ }
      }
      if (options.mode === 'throw') throw new Error(`[engine-doc] document write outside a known writer: ${text} (${where})`);
    }
    options.onWrite(path);
  };

  const keyOf = (target: object, prop: string | symbol): PathPart | null => {
    if (typeof prop === 'symbol') return null;
    if (Array.isArray(target)) {
      const n = Number(prop);
      return Number.isInteger(n) && n >= 0 && String(n) === prop ? n : prop;
    }
    return prop;
  };

  const SEARCH = new Set(['indexOf', 'lastIndexOf', 'includes']);

  const handler: ProxyHandler<any> = {
    get(target, prop, receiver) {
      if (prop === PROXIED) return true;
      if (Array.isArray(target) && typeof prop === 'string' && SEARCH.has(prop)) {
        const method = (Array.prototype as any)[prop];
        return (search: unknown, from?: number) => method.call(target, unwrap(search), from);
      }
      let value = Reflect.get(target, prop, receiver);
      if (typeof prop === 'symbol' || !value || typeof value !== 'object') return value;
      // A proxy stored through a raw reference: the document holds the raw object.
      const stored = targets.get(value);
      if (stored) {
        const desc = Object.getOwnPropertyDescriptor(target, prop);
        if (desc && 'value' in desc && desc.writable) target[prop] = stored;
        value = stored;
      }
      if (!isTrackable(value)) return value;
      if (fresh.has(value)) return value;
      const key = keyOf(target, prop);
      if (key === null) return value;
      // Accessor-backed members are views, not document objects.
      const desc = Object.getOwnPropertyDescriptor(target, prop);
      if (!desc || !('value' in desc)) return value;
      const link = links.get(value);
      if (!link || link.parent !== target || link.key !== key) links.set(value, { parent: target, key });
      return proxyOf(value);
    },
    set(target, prop, value, receiver) {
      const raw = unwrapDeep(value, true);
      const key = keyOf(target, prop);
      if (key !== null) record(target, key, handler.set!);
      return Reflect.set(target, prop, raw, receiver === proxies.get(target) ? target : receiver);
    },
    deleteProperty(target, prop) {
      const key = keyOf(target, prop);
      if (key !== null && Object.prototype.hasOwnProperty.call(target, prop)) record(target, key, handler.deleteProperty!);
      return Reflect.deleteProperty(target, prop);
    },
    defineProperty(target, prop, descriptor) {
      const key = keyOf(target, prop);
      // Non-enumerable accessors (temporalKeys' legacy views) are not JSON.
      const visible = descriptor.enumerable !== false || Object.prototype.propertyIsEnumerable.call(target, prop);
      if (key !== null && visible) record(target, key, handler.defineProperty!);
      if ('value' in descriptor) descriptor = { ...descriptor, value: unwrapDeep(descriptor.value, true) };
      return Reflect.defineProperty(target, prop, descriptor);
    },
  };

  const proxyOf = (target: object): any => {
    let proxy = proxies.get(target);
    if (!proxy) {
      proxy = new Proxy(target, handler);
      proxies.set(target, proxy);
      targets.set(proxy, target);
    }
    return proxy;
  };

  return {
    wrap(root: any) {
      if (!isTrackable(root)) return root;
      const raw = unwrap(root);
      roots.add(raw);
      links.delete(raw);
      return proxyOf(raw);
    },
    unwrap,
    unwrapDeep: <T>(value: T) => unwrapDeep(value),
    settle,
    violations: () => [...violations.values()].sort((a, b) => b.count - a.count),
    clearViolations: () => violations.clear(),
  };
}

/** Whether a value is a tracking proxy. */
export const isTrackingProxy = (value: unknown): boolean => !!value && typeof value === 'object' && (value as any)[PROXIED] === true;

/**
 * Proxies cannot be structured-cloned or posted. In tracking mode the page's
 * `structuredClone` and `postMessage` unwrap them first (only proxies need
 * it: the raw document never holds a proxy).
 */
export function patchCloning(tracker: Tracker, scope: any = globalThis): () => void {
  const restores: Array<() => void> = [];
  const original = scope.structuredClone;
  if (typeof original === 'function') {
    scope.structuredClone = (value: unknown, options?: StructuredSerializeOptions) => original.call(scope, tracker.unwrapDeep(shallowCopyIfProxy(tracker, value)), options);
    restores.push(() => { scope.structuredClone = original; });
  }
  for (const ctor of ['MessagePort', 'Worker', 'BroadcastChannel', 'Window', 'DedicatedWorkerGlobalScope']) {
    const proto = scope[ctor]?.prototype;
    const post = proto && Object.getOwnPropertyDescriptor(proto, 'postMessage')?.value;
    if (typeof post !== 'function') continue;
    proto.postMessage = function (this: unknown, message: unknown, ...rest: unknown[]) {
      return post.call(this, tracker.unwrapDeep(shallowCopyIfProxy(tracker, message)), ...rest);
    };
    restores.push(() => { proto.postMessage = post; });
  }
  return () => restores.forEach(fn => fn());
}

/** A message that is itself a proxy unwraps to its target; containers built by the caller are fixed in place. */
function shallowCopyIfProxy(tracker: Tracker, value: unknown): unknown {
  return tracker.unwrap(value);
}
