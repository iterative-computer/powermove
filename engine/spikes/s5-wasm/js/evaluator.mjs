// The JS side of the S5 fast path.
//
// - Numeric ids per property: `intern(layerId, channel)` once, then `ev(id, t)`
//   crosses the boundary with two numbers and returns one.
// - `evP(L, prop, T, key)` keeps the kernel's live-object signature through an
//   identity registry (WeakMap from the live property object to its id).
// - Batch results land in a shared Float64Array view of wasm memory; ids and
//   patches are written through Uint32Array and Uint8Array views.
// - Views are re-acquired whenever linear memory grows (the old ArrayBuffer is
//   detached) or a buffer moves inside it.
// - Writes queue and flush before the next query (section 6, "flush before
//   query").

const encoder = new TextEncoder();

export class WasmMirror {
  /**
   * @param {object} bindings the wasm-bindgen module namespace (`pkg/s5_wasm.js`)
   * @param {WebAssembly.Memory} memory the instance's memory
   */
  constructor(bindings, memory) {
    this.ev_ = bindings.Evaluator;
    this.e = new bindings.Evaluator();
    this.memory = memory;
    this.buffer = null;
    this.u8 = null; this.inputPtr = 0; this.inputCap = 0;
    this.ids = null; this.idsPtr = 0; this.idsCap = 0;
    this.out = null; this.outPtr = 0; this.outCap = 0;
    this.registry = new WeakMap();
    this.pending = [];
    this.viewRefreshes = 0;
  }

  /** Re-acquire every view if memory grew since they were made. */
  views() {
    if (this.buffer !== this.memory.buffer) {
      this.buffer = this.memory.buffer;
      this.viewRefreshes++;
      if (this.inputCap) this.u8 = new Uint8Array(this.buffer, this.inputPtr, this.inputCap);
      if (this.idsCap) this.ids = new Uint32Array(this.buffer, this.idsPtr, this.idsCap);
      if (this.outCap) this.out = new Float64Array(this.buffer, this.outPtr, this.outCap);
    }
  }

  reserveInput(len) {
    if (len > this.inputCap || this.buffer !== this.memory.buffer) {
      if (len > this.inputCap) {
        this.inputPtr = this.e.input_ptr(len);
        this.inputCap = this.e.input_capacity();
      }
      this.buffer = null; // force every view to be rebuilt
      this.views();
      this.u8 = new Uint8Array(this.buffer, this.inputPtr, this.inputCap);
    }
    return this.u8;
  }

  reserveIds(len) {
    if (len > this.idsCap) {
      this.idsPtr = this.e.ids_ptr(len);
      this.idsCap = len;
      this.buffer = null;
    }
    this.views();
    if (!this.ids || this.ids.length !== this.idsCap || this.ids.byteOffset !== this.idsPtr) this.ids = new Uint32Array(this.buffer, this.idsPtr, this.idsCap);
    return this.ids;
  }

  reserveOut(len) {
    if (len > this.outCap) {
      this.outPtr = this.e.out_ptr(len);
      this.outCap = len;
      this.buffer = null;
    }
    this.views();
    if (!this.out || this.out.length !== this.outCap || this.out.byteOffset !== this.outPtr) this.out = new Float64Array(this.buffer, this.outPtr, this.outCap);
    return this.out;
  }

  /** Load a whole project (object or JSON string). */
  load(project) {
    const json = typeof project === 'string' ? project : JSON.stringify(project);
    const len = this.writeString(0, json);
    this.e.load(len);
    this.registry = new WeakMap();
  }

  /** Free the input buffer after a load; patches re-reserve what they need. */
  releaseInput() {
    this.e.input_release();
    this.u8 = null; this.inputPtr = 0; this.inputCap = 0;
  }

  /** Free the wasm-side evaluator. The mirror is unusable afterwards. */
  free() {
    this.e.free();
    this.e = null;
  }

  /** Write a UTF-8 string at `offset` in the input buffer; returns its byte length. */
  writeString(offset, str) {
    let view = this.reserveInput(offset + str.length + 16);
    let { read, written } = encoder.encodeInto(str, view.subarray(offset));
    if (read < str.length) {
      view = this.reserveInput(offset + str.length * 3 + 16);
      ({ read, written } = encoder.encodeInto(str, view.subarray(offset)));
    }
    return written;
  }

  writeU32(offset, value) {
    const view = this.reserveInput(offset + 4);
    view[offset] = value & 255; view[offset + 1] = (value >>> 8) & 255;
    view[offset + 2] = (value >>> 16) & 255; view[offset + 3] = value >>> 24;
  }

  writeChunk(offset, str) {
    this.reserveInput(offset + 4);
    const n = this.writeString(offset + 4, str);
    this.writeU32(offset, n);
    return offset + 4 + n;
  }

  /** A chunk from UTF-8 bytes already encoded elsewhere (a plain copy). */
  writeBytesChunk(offset, bytes) {
    const view = this.reserveInput(offset + 4 + bytes.length);
    this.writeU32(offset, bytes.length);
    view.set(bytes, offset + 4);
    return offset + 4 + bytes.length;
  }

  /**
   * Encode patches into the input buffer. Each patch is either a leaf patch
   * `{path, exists, value}` (shared/patch.ts), optionally with its value
   * pre-serialized as a `json` string or as UTF-8 `bytes` (the history's
   * scoped commit already produces both), or an id-keyed structural patch
   * `{path: ['layers'], order: [ids], layers: [changed layer objects or JSON]}`.
   */
  encode(patches) {
    this.reserveInput(64);
    this.writeU32(0, patches.length);
    let at = 4;
    for (const patch of patches) {
      const kind = patch.order ? 2 : patch.exists ? 1 : 0;
      this.reserveInput(at + 1);
      this.u8[at++] = kind;
      at = this.writeChunk(at, JSON.stringify(patch.path));
      if (kind === 1) at = patch.bytes ? this.writeBytesChunk(at, patch.bytes) : this.writeChunk(at, patch.json ?? JSON.stringify(patch.value));
      else if (kind === 2) {
        at = this.writeChunk(at, JSON.stringify(patch.order));
        this.writeU32(at, patch.layers.length); at += 4;
        for (const layer of patch.layers) at = this.writeChunk(at, typeof layer === 'string' ? layer : JSON.stringify(layer));
      }
    }
    return at;
  }

  /** Apply patches now. */
  apply(patches) {
    const len = this.encode(patches);
    this.e.apply(len);
  }

  /** Apply already-encoded input bytes (benchmarks split encode from apply). */
  applyEncoded(len) {
    this.e.apply(len);
  }

  /** Queue patches; they are applied before the next query. */
  write(patches) {
    for (const patch of patches) this.pending.push(patch);
  }

  flush() {
    if (this.pending.length) {
      const patches = this.pending;
      this.pending = [];
      this.apply(patches);
    }
  }

  intern(layerId, channel) {
    return this.e.intern(layerId, channel);
  }

  /** One property by numeric id at global time `t`. */
  ev(id, t) {
    if (this.pending.length) this.flush();
    return this.e.ev(id, t);
  }

  /**
   * `PM.evP(L, prop, T, key)` with live objects: the identity registry maps
   * the live property object to its numeric id.
   */
  evP(L, prop, T, key) {
    if (this.pending.length) this.flush();
    let id = this.registry.get(prop);
    if (id === undefined) {
      id = this.e.intern(L.id, key);
      this.registry.set(prop, id);
    }
    return this.e.ev(id, T);
  }

  /** Evaluate ids at time t; returns a view valid until the next call. */
  evBatch(ids, t) {
    if (this.pending.length) this.flush();
    const n = ids.length;
    const view = this.reserveIds(n);
    view.set(ids);
    this.reserveOut(n);
    this.e.ev_batch(n, t);
    this.views();
    return this.out.subarray(0, n);
  }

  /** World matrices of every layer (6 numbers each), as a view. */
  worldAll(t) {
    if (this.pending.length) this.flush();
    const n = this.e.layer_count();
    this.reserveOut(n * 6);
    this.e.world_all(t);
    this.views();
    return this.out.subarray(0, n * 6);
  }

  stats() {
    const [patches, parsed, kept, added, removed, invalidated, bytesCompared] = this.e.stats();
    return { patches, parsed, kept, added, removed, invalidated, bytesCompared };
  }

  toJSON() { return this.e.to_json(); }
  layerCount() { return this.e.layer_count(); }
  handleCount() { return this.e.handle_count(); }
  rawBytes() { return this.e.raw_bytes(); }
  unsupported() { const s = this.e.unsupported(); return s ? s.split('\n') : []; }
  memoryBytes() { return this.memory.buffer.byteLength; }
}

/** Intern every transform channel of every layer, in document order. */
export function internAll(mirror, project) {
  const ids = [];
  const refs = [];
  for (const layer of project.layers) {
    for (const key of Object.keys(layer.p || {})) {
      ids.push(mirror.intern(layer.id, key));
      refs.push([layer, layer.p[key], key]);
    }
  }
  return { ids: Uint32Array.from(ids), refs };
}
