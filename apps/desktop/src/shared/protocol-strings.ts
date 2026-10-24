/*
 * Strings the renderer puts into engine protocol messages.
 *
 * pm-protocol bounds strings in UTF-8 bytes (Rust `str::len`), and a message
 * that breaks a bound fails the whole connection (`codec::decode` validates
 * every envelope; RFC 09). JavaScript's `slice` counts UTF-16 code units, so a
 * 512-unit cut of non-ASCII text can be 1,536 bytes, and a cut through a
 * surrogate pair leaves a lone surrogate, which no UTF-8 encoder keeps.
 * These helpers make every bounded string fit before it is sent.
 */
import { sha256Hex } from './sha256';

/** pm_protocol snapshot::MAX_COMPAT_KEY: a compatibility raster key, in UTF-8 bytes. */
export const MAX_COMPAT_KEY_BYTES = 512;

const isHigh = (unit: number): boolean => unit >= 0xd800 && unit <= 0xdbff;
const isLow = (unit: number): boolean => unit >= 0xdc00 && unit <= 0xdfff;

/**
 * Walks `text` by code point: `size` is its UTF-8 bytes as TextEncoder
 * writes it (a lone surrogate is U+FFFD, 3 bytes), `units` its UTF-16 length.
 */
function forEachPoint(text: string, visit: (size: number, units: number, lone: boolean) => boolean | void): void {
  for (let i = 0; i < text.length; ) {
    const unit = text.charCodeAt(i);
    let size: number, units = 1, lone = false;
    if (unit < 0x80) size = 1;
    else if (unit < 0x800) size = 2;
    else if (isHigh(unit) && i + 1 < text.length && isLow(text.charCodeAt(i + 1))) { size = 4; units = 2; }
    else { size = 3; lone = isHigh(unit) || isLow(unit); }
    if (visit(size, units, lone) === false) return;
    i += units;
  }
}

/** UTF-8 bytes of `text` as TextEncoder writes it. */
export function utf8Length(text: string): number {
  let bytes = 0;
  forEachPoint(text, (size) => { bytes += size; });
  return bytes;
}

/** Whether `text` has no lone surrogates (it round-trips through UTF-8). */
export function wellFormed(text: string): boolean {
  let ok = true;
  forEachPoint(text, (_size, _units, lone) => { if (lone) ok = false; return ok; });
  return ok;
}

/**
 * The longest prefix of `text` made of whole code points whose UTF-8 form is
 * at most `maxBytes`, lone surrogates replaced by U+FFFD (as TextEncoder
 * would). For messages meant for people: a cut is fine, a broken message is
 * not.
 */
export function limitUtf8(text: string, maxBytes: number): string {
  let bytes = 0, out = '', start = 0, at = 0;
  forEachPoint(text, (size, units, lone) => {
    if (bytes + size > maxBytes) return false;
    bytes += size;
    if (lone) {
      out += text.slice(start, at) + '�';
      start = at + units;
    }
    at += units;
  });
  return out + text.slice(start, at);
}

/**
 * A producer's cache key as the engine stores it. A key that is not
 * well-formed or longer than the protocol allows is replaced by a digest of
 * its UTF-16 code units: never a cut, because two keys sharing a long
 * prefix (a wrapper's text at two frames) must stay two keys, or the
 * engine would `reuse` one frame's pixels for the other.
 */
export function compatKey(key: string): string {
  return utf8Length(key) <= MAX_COMPAT_KEY_BYTES && wellFormed(key) ? key : digestKey(key);
}

function digestKey(key: string): string {
  const units = new Uint8Array(key.length * 2);
  for (let i = 0; i < key.length; i++) {
    const unit = key.charCodeAt(i);
    units[i * 2] = unit & 0xff;
    units[i * 2 + 1] = unit >> 8;
  }
  return `powermove:key-sha256:${sha256Hex(units)}`;
}
