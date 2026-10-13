import { describe, expect, it } from 'vitest';
import { layoutCaption } from './layout';
import { cueWords, displayLength, joinWords, mergeCues, presetStyle, splitCue, textTokens, type CaptionCue } from './model';
import { segmentWords } from './segment';

/* Chinese and Japanese captions: no spaces between words, line breaks
   between characters, punctuation that never starts or ends a line. */

const ids = () => { let n = 0; return () => `j${++n}`; };
const cue = (text: string, extra: Partial<CaptionCue> = {}): CaptionCue => ({ id: 'c', start: 0, end: 4, text, ...extra });
const timed = (tokens: string[], step = 0.3) => tokens.map((text, index) => ({ text, start: index * step, end: index * step + step * 0.9 }));

describe('CJK text', () => {
  it('joins transcript words without spaces where Chinese or Japanese meet', () => {
    expect(joinWords(['我们', '今天', '去', '公园。'])).toBe('我们今天去公园。');
    expect(joinWords(['今日は', 'いい', '天気', 'ですね'])).toBe('今日はいい天気ですね');
    expect(joinWords(['用', 'iPhone', '拍', '的'])).toBe('用iPhone拍的');
    expect(joinWords(['Hello', ' there ', 'friend'])).toBe('Hello there friend');
    expect(joinWords(['안녕하세요', '여러분'])).toBe('안녕하세요 여러분');
    expect(displayLength('我们ab')).toBe(6);
  });

  it('breaks between characters; closing punctuation and small kana stay with the character before', () => {
    expect(textTokens('你好，世界。').map(token => token.text)).toEqual(['你', '好，', '世', '界。']);
    expect(textTokens('「はい」と言った').map(token => token.text)).toEqual(['「は', 'い」', 'と', '言っ', 'た']);
    expect(textTokens('用 iPhone拍').map(token => [token.text, token.gap])).toEqual([['用', false], ['iPhone', true], ['拍', false]]);
    expect(textTokens('two words').map(token => [token.text, token.gap])).toEqual([['two', false], ['words', true]]);
  });

  it('wraps a long Chinese cue across lines with no spaces between characters', () => {
    const measure = (text: string, size: number) => Array.from(text).length * size;
    const style = { ...presetStyle('classic'), size: 40, maxWidth: 50, maxLines: 2 };
    const layout = layoutCaption(cue('我们今天下午一起去公园散步然后吃晚饭'), style, measure, { compositionWidth: 1000 });
    expect(layout.lines).toHaveLength(2);
    expect(layout.size).toBe(40);
    for (const line of layout.lines) {
      line.words.forEach((word, index) => { if (index) expect(word.x).toBeCloseTo(line.words[index - 1]!.x + line.words[index - 1]!.width); });
    }
    expect(layout.lines.map(line => line.words.map(word => word.text).join('')).join('')).toBe('我们今天下午一起去公园散步然后吃晚饭');
  });

  it('spreads word timings over the characters they cover, for highlight', () => {
    const words = cueWords(cue('你好世界', { words: [{ text: '你好', start: 0, end: 1 }, { text: '世界', start: 2, end: 3 }] }));
    expect(words.map(word => [word.text, word.start, word.end])).toEqual([['你', 0, 0.5], ['好', 0.5, 1], ['世', 2, 2.5], ['界', 2.5, 3]]);
  });

  it('segments transcripts into unspaced cues sized by display width', () => {
    const tokens = Array.from({ length: 30 }, (_, index) => '我们今天去公园散步'[index % 9]!);
    const cues = segmentWords(timed(tokens, 0.15), { maxCharsPerLine: 16, maxLines: 1 }, ids());
    expect(cues.length).toBeGreaterThan(1);
    for (const item of cues) {
      expect(item.text).not.toMatch(/\s/);
      expect(displayLength(item.text)).toBeLessThanOrEqual(16);
    }
  });

  it('splits and merges Chinese cues without inventing spaces', () => {
    const source = cue('你好世界', { words: [{ text: '你好', start: 0, end: 1 }, { text: '世界', start: 2, end: 3 }], end: 3 });
    const split = splitCue([source], 'c', 1.5, ids());
    expect(split.cues.map(item => item.text)).toEqual(['你好', '世界']);
    expect(mergeCues(split.cues, split.cues.map(item => item.id)).cues[0]!.text).toBe('你好世界');
  });
});
