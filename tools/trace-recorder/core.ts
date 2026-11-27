/** Browser/Node neutral trace engine. No clock, IO, proxies or callback replacement. */
export type Row = Record<string, unknown>;
export class Recorder {
  rows: Row[] = [];
  private objects = new WeakMap<object, string>();
  private symbols = new Map<symbol, string>();
  private ids = 0;
  private calls = 0;
  private parents: number[] = [];
  private failed = false;
  private tokens = new Map<string, string>();
  constructor(readonly state: () => unknown, private readonly metadata: Row) {
    this.rows.push({ type: 'header', format: 'powermove.behavior', version: 1, ...metadata });
  }
  token(value: string, category = 'id') {
    if (!this.tokens.has(value)) this.tokens.set(value, `${category}:${this.tokens.size + 1}`);
  }
  // A serialized snapshot is copied data, not a live object identity.
  private jsonValue(value: any): any {
    if(typeof value==='string') return this.tokens.has(value) ? {$token:this.tokens.get(value)} : value;
    if(Array.isArray(value)) return value.map(x=>this.jsonValue(x));
    if(value && typeof value==='object') return Object.fromEntries(Object.entries(value).map(([key,x])=>[
      this.tokens.has(key) ? `@token:${this.tokens.get(key)}` : key.startsWith('@') ? '@'+key : key,
      key==='at' && typeof x==='number' && x>1e12 ? {$volatile:'wall-clock-ms'} : this.jsonValue(x),
    ]));
    return value;
  }
  value(value: any, seen = new Set<object>(), key = ''): any {
    if (value === undefined) return { $type: 'undefined' };
    if (typeof value === 'number' && (!Number.isFinite(value) || Object.is(value, -0))) return { $type: 'number', value: String(Object.is(value, -0) ? '-0' : value) };
    if (typeof value === 'bigint') return { $type: 'bigint', value: String(value) };
    if (typeof value === 'symbol') {
      if (!this.symbols.has(value)) this.symbols.set(value, `s${this.symbols.size + 1}`);
      return { $type: 'symbol', id: this.symbols.get(value), description: value.description };
    }
    if (typeof value === 'string') {
      if (this.tokens.has(value)) return { $token: this.tokens.get(value) };
      // history.begin returns a JSON string; retain its type and structural content.
      if (value.startsWith('{') && value.includes('"layers"')) {
        try { return { $type: 'json-string', value: this.jsonValue(JSON.parse(value)) }; } catch { /* ordinary string */ }
      }
      return value;
    }
    if (value === null || (typeof value !== 'object' && typeof value !== 'function')) return value;
    let id = this.objects.get(value);
    if (!id) { id = `o${++this.ids}`; this.objects.set(value, id); }
    if (seen.has(value)) return { $ref: id };
    seen.add(value);
    if (typeof value === 'function') return { $id: id, $type: 'function', name: value.name, arity: value.length };
    if (value instanceof Error) return { $id: id, $type: 'error', name: value.name, message: value.message };
    if (typeof KeyboardEvent !== 'undefined' && value instanceof KeyboardEvent) return {
      $id: id, $type: 'key', event: value.type, key: value.key, code: value.code,
      meta: value.metaKey, ctrl: value.ctrlKey, alt: value.altKey, shift: value.shiftKey,
      repeat: value.repeat, defaultPrevented: value.defaultPrevented, phase: value.eventPhase,
      target: this.value(value.target, seen), trusted: value.isTrusted,
    };
    if (typeof Element !== 'undefined' && value instanceof Element) return {
      $id: id, $type: 'element', tag: value.tagName.toLowerCase(), id: value.id,
      role: value.getAttribute('role'), editable: value.getAttribute('contenteditable'),
    };
    if (value instanceof Promise) return { $id: id, $type: 'promise' }; // Do not attach then/rejection handlers.
    if (Array.isArray(value)) return { $id: id, $type: 'array', values: value.map(x => this.value(x, seen, key)) };
    if (value instanceof Map) return { $id: id, $type: 'map', entries: [...value].map(([k, v]) => [this.value(k, seen), this.value(v, seen)]) };
    if (value instanceof Set) return { $id: id, $type: 'set', values: [...value].map(v => this.value(v, seen)) };
    const props: Record<string, unknown> = {};
    // Read data descriptors, never getters. Key order is retained as observable JS order.
    for (const name of Object.keys(value)) {
      const d = Object.getOwnPropertyDescriptor(value, name)!;
      const encodedName = this.tokens.has(name) ? `@token:${this.tokens.get(name)}` : name.startsWith('@') ? '@'+name : name;
      props[encodedName] = 'value' in d
        ? name === 'at' && typeof d.value === 'number' && d.value > 1e12
          ? { $volatile: 'wall-clock-ms' } : this.value(d.value, seen, name)
        : { $type: 'accessor', get: this.value(d.get, seen), set: this.value(d.set, seen) };
    }
    return { $id: id, $type: 'object', props };
  }
  private safe(fn: () => void) {
    try { fn(); } catch { this.failed = true; }
  }
  point(type: string, data: unknown = {}) {
    if (this.metadata.eventScope==='synchronous-calls' && type.startsWith('event.') && !this.parents.length) return;
    this.safe(() => this.rows.push({ seq: this.rows.length, type, parent: this.parents.at(-1) ?? null, data: this.value(data) }));
  }
  checkpoint(label: string) { this.safe(() => this.point('state', { label, state: this.state() })); }
  call<T>(name: string, args: unknown[], fn: () => T): T {
    const call = ++this.calls;
    this.safe(() => this.rows.push({ seq: this.rows.length, type: 'call', call, parent: this.parents.at(-1) ?? null, name, args: this.value(args), state: this.value(this.state()) }));
    this.parents.push(call);
    try {
      const result = fn();
      this.safe(() => this.rows.push({ seq: this.rows.length, type: 'return', call, value: this.value(result), state: this.value(this.state()) }));
      return result;
    } catch (error) {
      this.safe(() => this.rows.push({ seq: this.rows.length, type: 'throw', call, error: this.value(error), state: this.value(this.state()) }));
      throw error;
    } finally { this.parents.pop(); }
  }
  finish() {
    this.checkpoint('final');
    this.rows.push({ seq: this.rows.length, type: 'end', valid: !this.failed, calls: this.calls });
    return this.rows.map(row => JSON.stringify(row)).join('\n') + '\n';
  }
}
