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
  // Blob, File and object URLs hold bytes in this realm. `__bytes` hands a
  // payload to the host without a JSON copy (image decode, file import).
  const blobData = new WeakMap(), hostFiles = new WeakMap(), objectURLs = new Map();
  // A file the host holds (one the user picked) reads its bytes only when asked.
  const blobBytes = {get(blob) { let b = blobData.get(blob); if (typeof b === 'function') blobData.set(blob, b = b()); return b; }, set: (blob, b) => blobData.set(blob, b)};
  let nextObjectURL = 0;
  const viewBytes = part => part instanceof ArrayBuffer ? new Uint8Array(part) : ArrayBuffer.isView(part) ? new Uint8Array(part.buffer, part.byteOffset, part.byteLength) : null;
  const mimeType = type => { type = String(type ?? ''); return /^[\x20-\x7e]*$/.test(type) ? type.toLowerCase() : ''; };
  class Blob {
    constructor(parts = [], options = {}) {
      if (parts === null || typeof parts !== 'object' || typeof parts[Symbol.iterator] !== 'function') throw new TypeError("Blob parts must be a sequence");
      const chunks = [];
      for (const part of parts) chunks.push(part instanceof Blob ? blobBytes.get(part) : viewBytes(part) ?? new TextEncoder().encode(String(part)));
      const bytes = new Uint8Array(chunks.reduce((n, c) => n + c.byteLength, 0));
      let at = 0; for (const c of chunks) { bytes.set(c, at); at += c.byteLength; }
      blobBytes.set(this, bytes); this._type = mimeType(options?.type);
    }
    get size() { return hostFiles.get(this)?.size ?? blobBytes.get(this).byteLength; }
    get type() { return this._type; }
    slice(start = 0, end = this.size, type = '') {
      const b = blobBytes.get(this), clamp = v => { v = Math.trunc(Number(v) || 0); return v < 0 ? Math.max(b.byteLength + v, 0) : Math.min(v, b.byteLength); };
      return new Blob([b.subarray(clamp(start), Math.max(clamp(start), clamp(end)))], {type});
    }
    async arrayBuffer() { return blobBytes.get(this).slice().buffer; }
    async bytes() { return blobBytes.get(this).slice(); }
    async text() { return new TextDecoder().decode(blobBytes.get(this)); }
    get [Symbol.toStringTag]() { return 'Blob'; }
  }
  class File extends Blob {
    constructor(parts, name, options = {}) { super(parts, options); this._name = String(name); this._lastModified = options?.lastModified === undefined ? Date.now() : Number(options.lastModified); }
    get name() { return this._name; }
    get lastModified() { return this._lastModified; }
    get [Symbol.toStringTag]() { return 'File'; }
  }
  const base64 = bytes => { const a = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'; let out = ''; for (let i = 0; i < bytes.length; i += 3) { const n = bytes[i] << 16 | (bytes[i+1] ?? 0) << 8 | (bytes[i+2] ?? 0); out += a[n>>18&63] + a[n>>12&63] + (i+1 < bytes.length ? a[n>>6&63] : '=') + (i+2 < bytes.length ? a[n&63] : '='); } return out; };
  const unbase64 = text => { text = String(text).replace(/[\t\n\f\r ]/g, ''); if (text.length % 4 === 0) text = text.replace(/==?$/, ''); if (text.length % 4 === 1 || /[^A-Za-z0-9+/]/.test(text)) throw new DOMException('The string to be decoded is not correctly encoded.', 'InvalidCharacterError'); const a = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'; const out = []; let n = 0, bits = 0; for (const c of text) { n = n << 6 | a.indexOf(c); bits += 6; if (bits >= 8) { bits -= 8; out.push(n >> bits & 255); } } return new Uint8Array(out); };
  globalThis.btoa = text => { text = String(text); if (/[^\0-\xff]/.test(text)) throw new DOMException('The string to be encoded contains characters outside of the Latin1 range.', 'InvalidCharacterError'); return base64(Uint8Array.from(text, c => c.charCodeAt(0))); };
  globalThis.atob = text => String.fromCharCode(...unbase64(text));
  class FileReader extends EventTarget {
    readyState = 0; result = null; error = null; onload = null; onerror = null; onloadend = null; onloadstart = null; onabort = null; onprogress = null;
    #read(blob, as) {
      if (!(blob instanceof Blob)) throw new TypeError('FileReader reads a Blob');
      if (this.readyState === 1) throw new DOMException('The object is already busy reading Blobs.', 'InvalidStateError');
      this.readyState = 1; this.result = null;
      const bytes = blobBytes.get(blob);
      queueMicrotask(() => {
        if (this.readyState !== 1) return;
        this.result = as === 'buffer' ? bytes.slice().buffer : as === 'url' ? 'data:' + (blob.type || 'application/octet-stream') + ';base64,' + base64(bytes) : as === 'binary' ? String.fromCharCode(...bytes) : new TextDecoder().decode(bytes);
        this.readyState = 2;
        for (const type of ['load', 'loadend']) { const e = new Event(type); this['on' + type]?.call(this, e); this.dispatchEvent(e); }
      });
    }
    readAsArrayBuffer(blob) { this.#read(blob, 'buffer'); }
    readAsText(blob) { this.#read(blob, 'text'); }
    readAsDataURL(blob) { this.#read(blob, 'url'); }
    readAsBinaryString(blob) { this.#read(blob, 'binary'); }
    abort() { if (this.readyState !== 1) return; this.readyState = 2; this.result = null; for (const type of ['abort', 'loadend']) { const e = new Event(type); this['on' + type]?.call(this, e); this.dispatchEvent(e); } }
  }
  Object.assign(FileReader, {EMPTY: 0, LOADING: 1, DONE: 2});
  globalThis.Blob = Blob; globalThis.File = File; globalThis.FileReader = FileReader;
  URL.createObjectURL = blob => { if (!(blob instanceof Blob)) throw new TypeError('createObjectURL takes a Blob'); const url = 'blob:powermove/' + (++nextObjectURL).toString(36) + '-' + Math.floor(Math.random() * 2 ** 48).toString(36); objectURLs.set(url, blob); return url; };
  URL.revokeObjectURL = url => { objectURLs.delete(String(url)); };
  // Realm-local: the bytes behind a blob: or data: URL, or null for anything else.
  const localBytes = input => {
    input = String(input);
    if (input.startsWith('blob:')) { const blob = objectURLs.get(input.split('#')[0]); return blob ? {bytes: blobBytes.get(blob), type: blob.type} : undefined; }
    const data = /^data:([^,]*?)(;base64)?,(.*)$/is.exec(input);
    if (!data) return null;
    const bytes = data[2] ? unbase64(decodeURIComponent(data[3])) : new TextEncoder().encode(decodeURIComponent(data[3]));
    return {bytes, type: mimeType(data[1].split(';')[0] || 'text/plain')};
  };
  class Response {
    #bytes;
    constructor(body = null, init = {}) { this.#bytes=body===null?new Uint8Array():typeof body==='string'?new TextEncoder().encode(body):body instanceof Blob?blobBytes.get(body).slice():new Uint8Array(viewBytes(body) ?? body); this.status=init.status??200; this.url=init.url??''; this.headers=new Headers(init.headers); this.bodyUsed=false; if(body instanceof Blob && body.type && !this.headers.has('content-type')) this.headers.set('content-type', body.type); }
    get ok() { return this.status>=200 && this.status<300; }
    async arrayBuffer() { if(this.bodyUsed) throw new TypeError('Body already consumed'); this.bodyUsed=true; return this.#bytes.slice().buffer; }
    async bytes() { return new Uint8Array(await this.arrayBuffer()); }
    async blob() { return new Blob([await this.arrayBuffer()], {type: (this.headers.get('content-type') ?? '').split(';')[0].trim()}); }
    async text() { return new TextDecoder().decode(await this.arrayBuffer()); }
    async json() { return JSON.parse(await this.text()); }
    clone() { if(this.bodyUsed) throw new TypeError('Body already consumed'); return new Response(this.#bytes,{status:this.status,url:this.url,headers:this.headers}); }
  }
  globalThis.Headers=Headers; globalThis.Response=Response;
  globalThis.fetch = (input, init = {}) => {
    if (retired) return Promise.reject(new Error('Generation retired'));
    const signal=init.signal;
    if(signal?.aborted) return Promise.reject(signal.reason);
    const local=localBytes(input?.url ?? input);
    if(local) return Promise.resolve(new Response(local.bytes,{status:200,url:String(input),headers:{'content-type':local.type}}));
    if(local===undefined) return Promise.reject(new TypeError('Failed to fetch'));
    const url=new URL(String(input));
    if(!['http:','https:'].includes(url.protocol)) return Promise.reject(new TypeError('Unsupported fetch protocol'));
    let body=init.body ?? [];
    body=typeof body==='string'?new TextEncoder().encode(body):body instanceof Blob?blobBytes.get(body):viewBytes(body)??body;
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
    /** One engine call with a byte payload: a Blob, ArrayBuffer or view. */
    bytes(operation, payload, input = null) { const bytes = payload instanceof Blob ? blobBytes.get(payload) : viewBytes(payload); if (!bytes) throw new TypeError('runtime.bytes takes a Blob or bytes'); return JSON.parse(__bytes(String(operation), bytes, JSON.stringify(input))); },
    /** The bytes and type behind a realm blob: or data: URL; null for other URLs, undefined for a revoked one. */
    local: localBytes,
    /** A File the host holds under `token`; its bytes are read on first use. */
    hostFile({token, name, type, size, lastModified}) { const file = new File([], name, {type, lastModified}); hostFiles.set(file, {token, size: Number(size)}); blobData.set(file, () => new Uint8Array(__read('file.read', JSON.stringify({token})))); return file; },
    /** The host's token for a File it holds, else undefined. */
    fileToken: file => hostFiles.get(file)?.token,
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
