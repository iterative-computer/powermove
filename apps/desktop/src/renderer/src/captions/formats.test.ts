import { describe, expect, it } from 'vitest';
import { detectCaptionFormat, formatSrt, formatVtt, parseCaptions, parseSrt, parseTimestamp, parseVtt, stripMarkup } from './formats';

const ids = () => { let n = 0; return () => `id${++n}`; };

describe('SRT', () => {
  it('reads BOM, CRLF, wrong numbering and markup', () => {
    const text = '﻿1\r\n00:00:01,000 --> 00:00:02,500\r\n<i>Hello</i> <b>there</b>\r\n\r\n7\r\n00:00:03,000 --> 00:00:04,000\r\n{\\an8}Second &amp; last\r\nline two\r\n';
    const parsed = parseSrt(text);
    expect(parsed.cues.map(({ start, end, text }) => ({ start, end, text }))).toEqual([
      { start: 1, end: 2.5, text: 'Hello there' },
      { start: 3, end: 4, text: 'Second & last\nline two' }
    ]);
    expect(parsed.skipped).toBe(0);
  });

  it('accepts missing hours, dot separators, no numbers and stray whitespace lines', () => {
    const parsed = parseSrt('00:01.5 --> 00:02.25\nshort\n   \n00:00:03.000-->00:00:04.000\nnext');
    expect(parsed.cues.map(cue => [cue.start, cue.end, cue.text])).toEqual([[1.5, 2.25, 'short'], [3, 4, 'next']]);
  });

  it('sorts out-of-order cues and resolves overlaps', () => {
    const parsed = parseSrt([
      '2\n00:00:05,000 --> 00:00:07,000\nlater',
      '1\n00:00:01,000 --> 00:00:06,000\nearly and long',
      '3\n00:00:05,000 --> 00:00:06,000\nsame start'
    ].join('\n\n'));
    expect(parsed.cues.map(cue => [cue.start, cue.end, cue.text])).toEqual([
      [1, 5, 'early and long'],
      [5, 7, 'later\nsame start']
    ]);
  });

  it('skips unreadable blocks and empty cues', () => {
    const parsed = parseSrt('garbage\n\n1\n00:00:01,000 --> 00:00:02,000\n\n2\n00:00:02,000 --> 00:00:03,000\n<i></i>');
    expect(parsed.cues).toEqual([]);
    expect(parsed.skipped).toBeGreaterThan(0);
  });

  it('writes numbered cues with comma milliseconds and an offset', () => {
    const out = formatSrt([
      { id: 'a', start: 0, end: 1.2346, text: 'One\n\ntwo' },
      { id: 'b', start: 3661.5, end: 3662, text: 'Hour' }
    ], { offset: 1 });
    expect(out).toBe('1\n00:00:01,000 --> 00:00:02,235\nOne\ntwo\n\n2\n01:01:02,500 --> 01:01:03,000\nHour\n');
  });

  it('round-trips through the parser', () => {
    const cues = [
      { id: 'a', start: 0.5, end: 1.75, text: 'Hello, world' },
      { id: 'b', start: 2, end: 4.125, text: 'Two\nlines' }
    ];
    const back = parseSrt(formatSrt(cues)).cues.map(({ start, end, text }) => ({ start, end, text }));
    expect(back).toEqual(cues.map(({ start, end, text }) => ({ start, end, text })));
  });
});

describe('WebVTT', () => {
  it('reads headers, NOTE/STYLE/REGION blocks, identifiers and cue settings', () => {
    const text = [
      'WEBVTT - Some title\nKind: captions\nLanguage: en',
      'NOTE This is a comment\nspanning lines',
      'STYLE\n::cue { color: lime }',
      'REGION\nid:fred width:40%',
      'intro\n00:01.000 --> 00:02.000 align:start position:10%\n<v Roger>Hi <c.yellow>there</c></v>',
      '00:00:03.000 --> 00:00:05.000 line:0\nA &lt;tag&gt; &amp; more'
    ].join('\n\n');
    const parsed = parseVtt(text);
    expect(parsed.cues.map(cue => [cue.start, cue.end, cue.text])).toEqual([[1, 2, 'Hi there'], [3, 5, 'A <tag> & more']]);
  });

  it('turns inline timestamps into word timings', () => {
    const parsed = parseVtt('WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nOne <00:00:01.500>two <00:00:02.200>three');
    expect(parsed.cues[0]!.words).toEqual([
      { text: 'One', start: 1, end: 1.5 },
      { text: 'two', start: 1.5, end: 2.2 },
      { text: 'three', start: 2.2, end: 3 }
    ]);
    expect(parsed.cues[0]!.text).toBe('One two three');
  });

  it('writes a valid file with escaped text and an optional language', () => {
    const out = formatVtt([{ id: 'a', start: 1, end: 2, text: 'a < b & c' }], { language: 'en' });
    expect(out).toBe('WEBVTT\nLanguage: en\n\n00:00:01.000 --> 00:00:02.000\na &lt; b &amp; c\n');
    expect(parseVtt(out).cues[0]!.text).toBe('a < b & c');
  });

  it('clips to a range and rebases to its start', () => {
    const cues = [
      { id: 'a', start: 0, end: 2, text: 'before' },
      { id: 'b', start: 2, end: 6, text: 'across' },
      { id: 'c', start: 9, end: 10, text: 'after' }
    ];
    expect(parseVtt(formatVtt(cues, { from: 3, to: 8 })).cues.map(cue => [cue.start, cue.end, cue.text])).toEqual([[0, 3, 'across']]);
  });
});

describe('detection and helpers', () => {
  it('detects by name, then by header, and falls back across formats', () => {
    expect(detectCaptionFormat('', 'x.VTT')).toBe('vtt');
    expect(detectCaptionFormat('WEBVTT\n')).toBe('vtt');
    expect(detectCaptionFormat('1\n00:00:01,000 --> 00:00:02,000\nx')).toBe('srt');
    // VTT content in a .srt-named file still imports.
    expect(parseCaptions('WEBVTT\n\nNOTE x\n\n00:01.000 --> 00:02.000\nhi', 'auto', 'x.srt').cues).toHaveLength(1);
  });

  it('parses timestamps and strips markup', () => {
    expect(parseTimestamp('01:02:03,004')).toBeCloseTo(3723.004);
    expect(parseTimestamp('02:03.5')).toBeCloseTo(123.5);
    expect(parseTimestamp('nope')).toBeNull();
    expect(stripMarkup('<font color="#fff">x</font> <ruby>漢<rt>kan</rt></ruby> {\\b1}y')).toBe('x 漢 y');
  });

  it('keeps ids unique through normalisation', () => {
    const make = ids();
    const parsed = parseSrt('1\n00:00:01,000 --> 00:00:02,000\na\n\n2\n00:00:02,000 --> 00:00:03,000\nb');
    expect(new Set(parsed.cues.map(cue => cue.id)).size).toBe(2);
    expect(make()).toBe('id1');
  });
});
