import { describe, expect, it } from 'vitest';

import { toRichWords } from './rich-words';

const plain = (text: string): string => toRichWords(text).map((word) => word.w).join('');

describe('toRichWords', () => {
  it('splits plain prose into words and preserved whitespace', () => {
    expect(toRichWords('one two').map((word) => word.w)).toEqual(['one', ' ', 'two']);
    expect(toRichWords('one two').every((word) => !word.b && !word.c)).toBe(true);
  });

  it('flags **bold** spans without leaking the markers', () => {
    const words = toRichWords('I will **edit the file** now');
    expect(plain('I will **edit the file** now')).toBe('I will edit the file now');
    expect(words.filter((word) => word.b).map((word) => word.w)).toEqual(['edit', ' ', 'the', ' ', 'file']);
  });

  it('flags `code` spans without leaking the backticks', () => {
    const words = toRichWords('run `npm test` first');
    expect(plain('run `npm test` first')).toBe('run npm test first');
    expect(words.filter((word) => word.c).map((word) => word.w)).toEqual(['npm', ' ', 'test']);
  });

  it('treats a markdown heading as a bold line and drops the hashes', () => {
    const words = toRichWords('## Plan');
    expect(words.map((word) => word.w)).toEqual(['Plan']);
    expect(words[0]?.b).toBe(true);
  });

  it('turns list markers into bullets and keeps newlines as their own word', () => {
    const words = toRichWords('- first\n- second');
    expect(words.map((word) => word.w).join('')).toBe('• first\n• second');
    expect(words.some((word) => word.w === '\n')).toBe(true);
  });

  it('is safe on empty and nullish input', () => {
    expect(toRichWords('')).toEqual([]);
    expect(toRichWords(undefined as any)).toEqual([]);
  });
});
