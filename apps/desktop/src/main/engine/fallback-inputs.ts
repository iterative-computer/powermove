/*
 * Shader fallback textures for snapshots (plan section 11, decision 19;
 * the Phase 3c render transport, docs/engine/rfcs/16-render-transport.md).
 *
 * An effect or a transition only the window's fallback runner draws
 * samples textures the engine rendered (its input, its backdrop, a
 * transition's two sides). For the viewer the engine posts them on the
 * window's session port after the request. A snapshot cannot use that
 * port: the renderer thread may be the one blocked in `sendSync` on the
 * snapshot, and a blocked thread reads no port. So for a snapshot the
 * interactive engine sends the bands to main, on main's host port, ahead
 * of its `needsRenderer` answer (the same port, so they arrive first), and
 * main hands them back to the window with the answer
 * (`EngineSnapshotReply.fallbackTextures`), which the runner draws on the
 * calling thread.
 *
 * Kept per request key until the answer takes them, bounded by entries and
 * bytes (the oldest go first: a request whose textures were dropped fails
 * as `internal` in the window, and the next snapshot asks again).
 */

/** A `shaderFallback` `input` event: one band of one texture. */
export interface FallbackInputBand {
  key: string;
  texture: number;
  width: number;
  height: number;
  row: number;
  rows: number;
  rgba16f: Uint8Array;
  rgba32f?: Uint8Array;
}

/** Most requests kept at once (pm_protocol shader_fallback::MAX_REQUESTS per answer). */
export const MAX_KEYS = 16;
/** Most binary16-equivalent bytes kept at once: two 8K textures with room to spare.
 *  RGBA32F uses up to twice the physical memory for the same pixel capacity. */
export const MAX_BYTES = 1024 * 1024 * 1024;
/** Most textures one request samples (pm_render's effect units). */
export const MAX_TEXTURES = 16;
/** Largest texture edge (pm_protocol's request limit). */
export const MAX_EDGE = 16_384;

interface Entry {
  width: number;
  height: number;
  textures: Map<number, { bytes: Uint8Array; rows: Uint8Array; filled: number; stride: number }>;
  bytes: number;
  budgetBytes: number;
}

export class FallbackInputs {
  private readonly entries = new Map<string, Entry>();
  private bytes = 0;
  private budgetBytes = 0;

  constructor(private readonly limits: { keys?: number; bytes?: number } = {}) {}

  /** Bytes held (tests and stats). */
  get held(): number {
    return this.bytes;
  }

  /** Keeps one band; false (and nothing kept) when it does not describe a
   *  band of a texture of its request's size. */
  receive(value: unknown): boolean {
    const band = value as Partial<FallbackInputBand>;
    const { key, texture, width, height, row, rows, rgba16f, rgba32f } = band;
    const int = (n: unknown, max: number): n is number => Number.isInteger(n) && (n as number) >= 0 && (n as number) <= max;
    if (typeof key !== 'string' || key.length === 0 || key.length > 256) return false;
    if (!int(texture, MAX_TEXTURES - 1) || !int(width, MAX_EDGE) || !int(height, MAX_EDGE) || width < 1 || height < 1) return false;
    if (!int(row, height - 1) || !int(rows, height - row) || rows < 1) return false;
    const stride = rgba32f === undefined ? 8 : 16;
    const data = rgba32f ?? rgba16f;
    if (!(rgba16f instanceof Uint8Array) || !(data instanceof Uint8Array)
      || (stride === 16 && rgba16f.byteLength !== 0)
      || data.byteLength !== width * rows * stride) return false;
    let entry = this.entries.get(key);
    if (entry && (entry.width !== width || entry.height !== height || [...entry.textures.values()].some(t => t.stride !== stride))) {
      // Another size under the same key is a new request: start again.
      this.drop(key);
      entry = undefined;
    }
    if (!entry) {
      entry = { width, height, textures: new Map(), bytes: 0, budgetBytes: 0 };
      this.entries.set(key, entry);
    }
    let target = entry.textures.get(texture);
    if (!target) {
      const size = width * height * stride;
      target = { bytes: new Uint8Array(size), rows: new Uint8Array(height), filled: 0, stride };
      entry.textures.set(texture, target);
      entry.bytes += size;
      this.bytes += size;
      entry.budgetBytes += size / (stride === 16 ? 2 : 1);
      this.budgetBytes += size / (stride === 16 ? 2 : 1);
    }
    target.bytes.set(data, row * width * stride);
    for (let r = row; r < row + rows; r++) {
      if (!target.rows[r]) {
        target.rows[r] = 1;
        target.filled++;
      }
    }
    this.trim(key);
    return true;
  }

  /**
   * The `count` textures of request `key` at `width × height` (binary16
   * RGBA bytes, GL row order), each with every row: removed from the
   * store. Null when any is missing or incomplete (also removed).
   */
  take(key: string, count: number, width: number, height: number): Uint8Array[] | null {
    const entry = this.entries.get(key);
    if (!entry) return null;
    this.drop(key);
    if (entry.width !== width || entry.height !== height) return null;
    const out: Uint8Array[] = [];
    for (let t = 0; t < count; t++) {
      const texture = entry.textures.get(t);
      if (!texture || texture.filled !== height) return null;
      out.push(texture.bytes);
    }
    return out;
  }

  clear(): void {
    this.entries.clear();
    this.bytes = 0;
    this.budgetBytes = 0;
  }

  private drop(key: string): void {
    const entry = this.entries.get(key);
    if (!entry) return;
    this.entries.delete(key);
    this.bytes -= entry.bytes;
    this.budgetBytes -= entry.budgetBytes;
  }

  /** The oldest requests go while over a limit (never `keep`, the one a
   *  band just arrived for, unless it alone is over). */
  private trim(keep: string): void {
    const keys = this.limits.keys ?? MAX_KEYS;
    const bytes = this.limits.bytes ?? MAX_BYTES;
    for (const key of [...this.entries.keys()]) {
      if (this.entries.size <= keys && this.budgetBytes <= bytes) return;
      if (key !== keep) this.drop(key);
    }
    if (this.budgetBytes > bytes) this.drop(keep);
  }
}
