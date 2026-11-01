// Snapshot-safe definitions: host functions are attached only after restore.
(() => {
  'use strict';
  const timers = new Map(), requests = new Map();
  let nextTimer = 0, retired = false;
  const eventTargets = new Set(), listenerTables = new WeakMap();
  const native = (op, arg) => JSON.parse(__host(op, JSON.stringify(arg)));
  globalThis.performance = Object.freeze({now: () => __now(), get timeOrigin() { return __timeOrigin; }});
  globalThis.queueMicrotask = fn => {
    if (typeof fn !== 'function') throw new TypeError('Callback must be callable');
    Promise.resolve().then(fn).catch(e => console.error(e));
  };
  function timer(fn, delay, interval, args) {
    if (retired) throw new Error('Generation retired');
    if (typeof fn !== 'function') throw new TypeError('String timers are unsupported');
    if (timers.size >= __limits.max_host_ops) throw new RangeError('Timer quota');
    const id = ++nextTimer;
    delay = Math.max(0, Math.min(2147483647, Number(delay) || 0));
    timers.set(id, {fn, due: __now() + delay, interval, delay: Math.max(1, delay), args});
    return id;
  }
  globalThis.setTimeout = (fn, delay = 0, ...args) => timer(fn, delay, false, args);
  globalThis.setInterval = (fn, delay = 0, ...args) => timer(fn, delay, true, args);
  globalThis.clearTimeout = globalThis.clearInterval = id => { timers.delete(Number(id)); };
  globalThis.console = Object.fromEntries(['log','info','warn','error','debug'].map(level => [level,
    (...args) => native('console', {level, message: args.map(String).join(' ')})]));
  class DOMException extends Error {
    constructor(message = '', name = 'Error') { super(message); this.name = name; }
  }
  globalThis.DOMException = DOMException;
  class EventTarget {
    #listeners = new Map();
    constructor() { listenerTables.set(this,this.#listeners); if (eventTargets.size >= 256) for (const ref of eventTargets) if (!ref.deref()) eventTargets.delete(ref); eventTargets.add(new WeakRef(this)); }
    addEventListener(type, fn, options = {}) {
      if (retired) throw new Error('Generation retired');
      if (!fn) return;
      const list = this.#listeners.get(type) || [];
      if (!list.some(x => x.fn === fn)) list.push({fn, once: !!options.once});
      this.#listeners.set(type, list);
    }
    removeEventListener(type, fn) { this.#listeners.set(type, (this.#listeners.get(type) || []).filter(x => x.fn !== fn)); }
    dispatchEvent(event) {
      for (const item of [...(this.#listeners.get(event.type) || [])]) {
        if (item.once) this.removeEventListener(event.type, item.fn);
        try { typeof item.fn === 'function' ? item.fn.call(this, event) : item.fn.handleEvent(event); }
        catch (e) { console.error(e); }
      }
      return !event.defaultPrevented;
    }
  }
  const removeEventListener = EventTarget.prototype.removeEventListener;
  globalThis.EventTarget = EventTarget;
  globalThis.Event = class Event {
    constructor(type) { this.type = String(type); this.defaultPrevented = false; }
  };
  class AbortSignal extends EventTarget {
    aborted = false; reason = undefined; onabort = null;
    throwIfAborted() { if (this.aborted) throw this.reason; }
    static abort(reason) { const c = new AbortController(); c.abort(reason); return c.signal; }
    static timeout(ms) { const c = new AbortController(); setTimeout(() => c.abort(new DOMException('Timed out', 'TimeoutError')), ms); return c.signal; }
  }
  class AbortController {
    signal = new AbortSignal();
    abort(reason = new DOMException('Aborted', 'AbortError')) {
      if (retired) return;
      const s = this.signal;
      if (s.aborted) return;
      s.aborted = true; s.reason = reason;
      const event = new Event('abort'); s.dispatchEvent(event);
      if (typeof s.onabort === 'function') s.onabort(event);
    }
  }
  globalThis.AbortController = AbortController; globalThis.AbortSignal = AbortSignal;
  class TextEncoder {
    encoding = 'utf-8';
    encode(input = '') {
      const bytes = [];
      for (const ch of String(input).toWellFormed()) {
        const c = ch.codePointAt(0);
        if (c < 128) bytes.push(c);
        else if (c < 2048) bytes.push(192 | c >> 6, 128 | c & 63);
        else if (c < 65536) bytes.push(224 | c >> 12, 128 | c >> 6 & 63, 128 | c & 63);
        else bytes.push(240 | c >> 18, 128 | c >> 12 & 63, 128 | c >> 6 & 63, 128 | c & 63);
      }
      return new Uint8Array(bytes);
    }
    encodeInto(input, dest) {
      if (!(dest instanceof Uint8Array)) throw new TypeError('Expected Uint8Array');
      let read = 0, written = 0;
      for (const c of String(input)) {
        const bytes = this.encode(c);
        if (written + bytes.length > dest.length) break;
        dest.set(bytes, written); written += bytes.length; read += c.length;
      }
      return {read, written};
    }
  }
  class TextDecoder {
    #pending = []; #bomSeen = false;
    constructor(label = 'utf-8', options = {}) {
      if (!['utf-8','utf8','unicode-1-1-utf-8'].includes(String(label).trim().toLowerCase())) throw new RangeError('Only UTF-8 is supported');
      this.encoding = 'utf-8'; this.fatal = !!options.fatal; this.ignoreBOM = !!options.ignoreBOM;
    }
    decode(input = new Uint8Array(), options = {}) {
      const view = input instanceof ArrayBuffer ? new Uint8Array(input) : new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      const b = [...this.#pending, ...view]; this.#pending = [];
      let out = '';
      const emit = c => { if (!this.#bomSeen) { this.#bomSeen = true; if (c === 0xFEFF && !this.ignoreBOM) return; } out += String.fromCodePoint(c); };
      const bad = () => { if (this.fatal) { this.#pending = []; this.#bomSeen = false; throw new TypeError('Invalid UTF-8'); } emit(0xFFFD); };
      let i = 0;
      while (i < b.length) {
        const start = i, first = b[i++];
        if (first <= 127) { emit(first); continue; }
        let need, cp, lo = 128, hi = 191;
        if (first >= 194 && first <= 223) { need = 1; cp = first & 31; }
        else if (first >= 224 && first <= 239) { need = 2; cp = first & 15; if (first === 224) lo = 160; if (first === 237) hi = 159; }
        else if (first >= 240 && first <= 244) { need = 3; cp = first & 7; if (first === 240) lo = 144; if (first === 244) hi = 143; }
        else { bad(); continue; }
        let valid = true;
        for (let j = 0; j < need; j++) {
          if (i === b.length) {
            if (options.stream) { this.#pending = b.slice(start); return out; }
            bad(); valid = false; break;
          }
          const x = b[i];
          if (x < lo || x > hi) { bad(); valid = false; break; }
          i++; cp = cp << 6 | x & 63; lo = 128; hi = 191;
        }
        if (valid) emit(cp);
      }
      if (!options.stream) this.#bomSeen = false;
      return out;
    }
  }
  globalThis.TextEncoder = TextEncoder; globalThis.TextDecoder = TextDecoder;
  class URLSearchParams {
    #items; #changed;
    constructor(init = '', changed = () => {}) {
      this.#items = typeof init === 'string' ? native('params.parse', init.replace(/^\?/, '')) :
        (init && typeof init[Symbol.iterator] === 'function' ? Array.from(init, p => {
          const pair = Array.from(p, String); if (pair.length !== 2) throw new TypeError('Expected pair'); return pair;
        }) : Object.entries(init).map(([k,v]) => [k,String(v)]));
      this.#changed = changed;
    }
    get size() { return this.#items.length; }
    append(k,v) { this.#items.push([String(k),String(v)]); this.#changed(this.toString()); }
    delete(k, v) { k=String(k); this.#items=this.#items.filter(p => p[0] !== k || (v !== undefined && p[1] !== String(v))); this.#changed(this.toString()); }
    get(k) { return this.#items.find(p => p[0] === String(k))?.[1] ?? null; }
    getAll(k) { return this.#items.filter(p => p[0] === String(k)).map(p => p[1]); }
    has(k,v) { return this.#items.some(p => p[0] === String(k) && (v === undefined || p[1] === String(v))); }
    set(k,v) { k=String(k); v=String(v); const first=this.#items.findIndex(p=>p[0]===k); this.#items=this.#items.filter(p=>p[0]!==k); this.#items.splice(first<0?this.#items.length:first,0,[k,v]); this.#changed(this.toString()); }
    sort() { this.#items.sort((a,b)=>a[0]<b[0]?-1:a[0]>b[0]?1:0); this.#changed(this.toString()); }
    *entries() { for(let i=0;i<this.#items.length;i++) yield [...this.#items[i]]; } *keys() { for(const p of this) yield p[0]; } *values() { for(const p of this) yield p[1]; }
    [Symbol.iterator]() { return this.entries(); }
    forEach(fn, thisArg) { for(const [k,v] of this) fn.call(thisArg,v,k,this); }
    toString() { return native('params.serialize', this.#items); }
    _reset(s) { this.#items = native('params.parse',s.replace(/^\?/, '')); }
  }
  class URL {
    #data; #params;
    constructor(input, base) { this.#data = native('url', {input:String(input), base:base === undefined ? null:String(base)}); this.#params = new URLSearchParams(this.#data.search, s => this.search=s); }
    get searchParams() { return this.#params; }
    toString() { return this.href; } toJSON() { return this.href; }
    static canParse(input,base) { try { new URL(input,base); return true; } catch { return false; } }
    static parse(input,base) { try { return new URL(input,base); } catch { return null; } }
    _get(k) { return this.#data[k]; }
    _set(k,v) { this.#data=native('url',{input:this.href, field:k,value:String(v)}); this.#params._reset(this.#data.search); }
  }
  for (const key of ['href','protocol','username','password','host','hostname','port','pathname','search','hash','origin']) Object.defineProperty(URL.prototype,key,{get(){return this._get(key)},...(key==='origin'?{}:{set(value){this._set(key,value)}})});
  globalThis.URL = URL; globalThis.URLSearchParams = URLSearchParams;
  globalThis.structuredClone = (value, options = {}) => __clone(value, [...(options?.transfer || [])]);
  class Headers {
    #items = [];
    constructor(init = []) { if(init instanceof Headers) init=[...init]; for(const [k,v] of (Array.isArray(init)?init:Object.entries(init))) this.append(k,v); }
    append(k,v) { k=String(k).toLowerCase(); v=String(v).trim(); if(!/^[!#$%&'*+.^_`|~0-9a-z-]+$/.test(k) || /[\r\n\0]/.test(v)) throw new TypeError('Invalid header'); const p=this.#items.find(x=>x[0]===k); if(p) p[1]+=', '+v; else this.#items.push([k,v]); }
    get(k) { return this.#items.find(p=>p[0]===String(k).toLowerCase())?.[1] ?? null; }
    has(k) { return this.get(k)!==null; }
    set(k,v) { this.delete(k); this.append(k,v); }
    delete(k) { this.#items=this.#items.filter(p=>p[0]!==String(k).toLowerCase()); }
    *entries() { yield* this.#items.map(p=>[...p]).sort((a,b)=>a[0]<b[0]?-1:1); }
    [Symbol.iterator]() { return this.entries(); }
    forEach(fn,thisArg) { for(const [k,v] of this) fn.call(thisArg,v,k,this); }
  }
  class Response {
    #bytes;
    constructor(body = null, init = {}) { this.#bytes=body===null?new Uint8Array():typeof body==='string'?new TextEncoder().encode(body):new Uint8Array(body); this.status=init.status??200; this.url=init.url??''; this.headers=new Headers(init.headers); this.bodyUsed=false; }
    get ok() { return this.status>=200 && this.status<300; }
    async arrayBuffer() { if(this.bodyUsed) throw new TypeError('Body already consumed'); this.bodyUsed=true; return this.#bytes.slice().buffer; }
    async text() { return new TextDecoder().decode(await this.arrayBuffer()); }
    async json() { return JSON.parse(await this.text()); }
    clone() { if(this.bodyUsed) throw new TypeError('Body already consumed'); return new Response(this.#bytes,{status:this.status,url:this.url,headers:this.headers}); }
  }
  globalThis.Headers=Headers; globalThis.Response=Response;
  globalThis.fetch = (input, init = {}) => {
    if (retired) return Promise.reject(new Error('Generation retired'));
    const signal=init.signal;
    if(signal?.aborted) return Promise.reject(signal.reason);
    const url=new URL(String(input));
    if(!['http:','https:'].includes(url.protocol)) return Promise.reject(new TypeError('Unsupported fetch protocol'));
    let body=init.body ?? [];
    body=typeof body==='string'?new TextEncoder().encode(body):body instanceof ArrayBuffer?new Uint8Array(body):body;
    return new Promise((resolve,reject) => {
      const id=native('fetch',{url:url.href,method:String(init.method??'GET').toUpperCase(),headers:[...new Headers(init.headers)],body:Array.from(body)});
      const abort=()=>{ native('abort',id); requests.delete(id); reject(signal.reason); };
      requests.set(id,{resolve,reject,signal,abort}); signal?.addEventListener('abort',abort,{once:true});
    });
  };
  let ops=[], opBytes=2;
  globalThis.storage = {};
  globalThis.hot = {data:{}};
  globalThis.runtime = Object.freeze({
    engine(operation, input = null) { return native('engine', {operation:String(operation), input}); },
    stage(op) { const text=JSON.stringify(op), copy=JSON.parse(text); const bytes=new TextEncoder().encode(text).byteLength+(ops.length?1:0); if(ops.length>=__limits.max_ops || opBytes+bytes>__limits.max_op_bytes) throw new RangeError('Operation quota'); ops.push(copy); opBytes+=bytes; }
  });
  globalThis.__pm = Object.freeze({
    take() { const out={ops,storage,hot:hot.data}; ops=[]; opBytes=2; return JSON.stringify(out); },
    nextDelay(now) { return timers.size ? Math.max(0,Math.min(...Array.from(timers.values(),t=>t.due-now))) : null; },
    restore(value, hotValue) { storage=value; hot.data=hotValue; ops=[]; opBytes=2; },
    resetOps() { ops=[]; opBytes=2; },
    retire() {
      retired=true; timers.clear();
      for (const req of requests.values()) if(req.signal) { try { removeEventListener.call(req.signal,'abort',req.abort); } catch {} }
      requests.clear();
      for (const ref of eventTargets) { const target=ref.deref(); if(target) listenerTables.get(target)?.clear(); }
      eventTargets.clear(); ops=[]; opBytes=2;
    },
    pump(now, completed) {
      for (const [id,result] of completed) {
        const req=requests.get(id); if(!req) continue;
        requests.delete(id); req.signal?.removeEventListener('abort',req.abort);
        if(result.error) req.reject(new TypeError(result.error));
        else req.resolve(new Response(result.body,{status:result.status,url:result.url,headers:result.headers}));
      }
      for(const [id,t] of [...timers]) if(t.due<=now && timers.has(id)) {
        if(t.interval) t.due=now+t.delay; else timers.delete(id);
        try { t.fn(...t.args); } catch(e) { console.error(e); }
      }
    }
  });
})();
