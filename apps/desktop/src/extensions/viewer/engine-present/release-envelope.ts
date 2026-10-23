/*
 * The one engine message the presentation worker sends by itself: a
 * session's frame release (`releaseFrame`), as the engine's wire carries it
 * (contract 9: a MessagePack envelope `{ wire, session, revision, message:
 * { kind: 'event', body } }`). The worker hands frames back on the release
 * port the page gave it (present-worker.ts), without the page's main thread,
 * and cannot import the app's protocol module (the viewer's folder stays
 * self-contained), so this writes exactly those bytes. A test checks them
 * against the protocol's own encoder.
 */
import type { FrameRelease } from './protocol';

/** The wire version (main/engine/protocol.ts `WIRE_VERSION`). */
const WIRE_VERSION = 1;

class Bytes {
  readonly out: number[] = [];
  str(s: string): void {
    const b = new TextEncoder().encode(s);
    if (b.length > 31) throw new Error('release envelope keys are short strings');
    this.out.push(0xa0 | b.length, ...b);
  }
  uint(n: number): void {
    if (!Number.isSafeInteger(n) || n < 0) throw new Error(`not an unsigned integer: ${n}`);
    if (n <= 0x7f) this.out.push(n);
    else if (n <= 0xff) this.out.push(0xcc, n);
    else if (n <= 0xffff) this.out.push(0xcd, n >> 8, n & 0xff);
    else if (n <= 0xffffffff) this.out.push(0xce, (n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff);
    else {
      const big = BigInt(n);
      this.out.push(0xcf);
      for (let i = 7; i >= 0; i--) this.out.push(Number((big >> BigInt(i * 8)) & 0xffn));
    }
  }
  map(n: number): void {
    this.out.push(0x80 | n);
  }
}

/** The bytes of a `releaseFrame` event of engine session `session`. */
export function releaseEnvelope(session: number, release: FrameRelease): Uint8Array {
  const w = new Bytes();
  w.map(4);
  w.str('wire'); w.uint(WIRE_VERSION);
  w.str('session'); w.uint(session);
  w.str('revision'); w.out.push(0xc0);
  w.str('message'); w.map(2);
  w.str('kind'); w.str('event');
  w.str('body'); w.map(3);
  w.str('type'); w.str('releaseFrame');
  w.str('poolIndex'); w.uint(release.poolIndex);
  w.str('seq'); w.uint(release.seq);
  return Uint8Array.from(w.out);
}
