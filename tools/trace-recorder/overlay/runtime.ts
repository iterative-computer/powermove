import { Recorder, type Row } from './core';

class TraceController {
  buildFingerprint = '__PM_TRACE_BUILD_FINGERPRINT__';
  recorder: Recorder | null = null;
  apis: Record<string, any> = {};
  gesture: any;
  private PM: any;
  private history: () => unknown = () => null;
  private generated = new Map<string, string>();
  attach(PM: any, history: () => unknown) { this.PM = PM; this.history = history; }
  expose<T>(name: string, api: T): T { this.apis[name] = api; return api; }
  uid<T>(value: T): T {
    if (typeof value === 'string') {
      if (!this.generated.has(value)) this.generated.set(value, 'id');
      this.recorder?.token(value);
    }
    return value;
  }
  start(metadata: Row) {
    if (this.recorder) throw new Error('Trace already active');
    this.recorder = new Recorder(() => ({ project: this.PM.proj, selection: this.PM.sel, history: this.history() }), metadata);
    // Only ids reachable in this synthetic fixture are admitted as aliases.
    const visit = (x: any, seen = new Set()) => {
      if (!x || typeof x !== 'object' || seen.has(x)) return;
      seen.add(x);
      for (const [key, d] of Object.entries(Object.getOwnPropertyDescriptors(x))) {
        if (!('value' in d)) continue;
        if (typeof d.value === 'string' && (key === 'id' || this.generated.has(d.value))) this.recorder!.token(d.value);
        else visit(d.value, seen);
      }
    };
    visit(this.PM.proj);
    this.recorder.checkpoint('initial');
  }
  call<T>(name: string, args: unknown[], fn: () => T): T { return this.recorder ? this.recorder.call(name, args, fn) : fn(); }
  point(type: string, data?: unknown) { this.recorder?.point(type, data); }
  checkpoint(label: string) { this.recorder?.checkpoint(label); }
  finish() { const result = this.recorder!.finish(); this.recorder = null; return result; }
}
export const trace = new TraceController();
(globalThis as any).__PMTrace = trace;
if (typeof window !== 'undefined') {
  for (const type of ['keydown', 'keyup']) {
    window.addEventListener(type, event => trace.point('dom.capture', event), true);
  }
}
