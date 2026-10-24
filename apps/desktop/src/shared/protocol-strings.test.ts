import { describe, expect, it } from 'vitest';
import { compatKey, limitUtf8, MAX_COMPAT_KEY_BYTES, utf8Length, wellFormed } from './protocol-strings';

const bytes = (text: string) => new TextEncoder().encode(text).length;

describe('protocol strings', () => {
  it('counts UTF-8 bytes as TextEncoder writes them', () => {
    for (const text of ['', 'abc', 'mid · dot', '“quoted” — dash', 'emoji 🎬 and ✨', 'lone \ud800 high', 'lone \udc00 low', '\ud83c'])
      expect(utf8Length(text)).toBe(bytes(text));
  });

  it('keeps short well-formed keys as they are', () => {
    expect(compatKey('wrapped-text:L0text01:12')).toBe('wrapped-text:L0text01:12');
    const ascii = 'k'.repeat(MAX_COMPAT_KEY_BYTES);
    expect(compatKey(ascii)).toBe(ascii);
  });

  it('replaces a key over the protocol bound in UTF-8 bytes, even when it is short in UTF-16 units', () => {
    // 300 units, 900 bytes: the old 512-unit cut kept all of it.
    const key = `wrapped-text:${'·'.repeat(150)}${'—'.repeat(150)}`;
    expect(key.length).toBeLessThan(512);
    const replaced = compatKey(key);
    expect(bytes(replaced)).toBeLessThanOrEqual(MAX_COMPAT_KEY_BYTES);
    expect(replaced).toMatch(/^powermove:key-sha256:[0-9a-f]{64}$/);
    expect(compatKey(key)).toBe(replaced);
  });

  it('never merges two long keys that share their first 512 units', () => {
    const prefix = 'wrapped-text:'.padEnd(600, 'x');
    expect(compatKey(`${prefix}:frame-1`)).not.toBe(compatKey(`${prefix}:frame-2`));
  });

  it('replaces a key with a lone surrogate, which UTF-8 cannot carry', () => {
    expect(wellFormed('a\ud800b')).toBe(false);
    expect(compatKey('a\ud800b')).toMatch(/^powermove:key-sha256:/);
    expect(compatKey('a\ud800b')).not.toBe(compatKey('a\ud801b'));
  });

  it('cuts messages at a code point within the byte bound', () => {
    const message = `Missing font “${'Ü'.repeat(700)}” 🎬`;
    const cut = limitUtf8(message, 1024);
    expect(bytes(cut)).toBeLessThanOrEqual(1024);
    expect(message.startsWith(cut)).toBe(true);
    expect(limitUtf8('🎬🎬', 5)).toBe('🎬');
    expect(limitUtf8('ok \ud800 there', 64)).toBe('ok � there');
    expect(limitUtf8('short', 1024)).toBe('short');
  });
});
