/*
 * SubRip (.srt) and WebVTT (.vtt) readers and writers.
 *
 * Readers are forgiving the way players are: a byte-order mark, CRLF or CR
 * line endings, missing or wrong cue numbers, optional hours, comma or dot
 * millisecond separators, VTT headers, NOTE / STYLE / REGION blocks, cue
 * settings and inline markup are all accepted. Inline VTT timestamps
 * (karaoke-style `<00:00:01.200>`) become word timings. The resulting cues go
 * through normalizeCues, so overlapping and out-of-order cues are resolved.
 *
 * Times are seconds from the start of the file.
 */
import { MIN_CUE_DURATION, normalizeCues, type CaptionCue, type CaptionWord } from './model';

export type CaptionFormat = 'srt' | 'vtt';

export interface ParsedCaptions {
  format: CaptionFormat;
  cues: CaptionCue[];
  /** Blocks that could not be read, for a quiet import notice. */
  skipped: number;
}

const TIME = String.raw`(?:(\d{1,3}):)?(\d{1,2}):(\d{1,2})(?:[.,:](\d{1,3}))?`;
const TIMING = new RegExp(String.raw`^\s*${TIME}\s*-+>\s*${TIME}(.*)$`);
const INLINE_TIME = /<((?:\d{1,3}:)?\d{1,2}:\d{1,2}[.,]\d{1,3})>/g;

function seconds(hours: string | undefined, minutes: string, secs: string, fraction: string | undefined): number {
  const ms = fraction ? Number(fraction.padEnd(3, '0').slice(0, 3)) : 0;
  return Number(hours ?? 0) * 3600 + Number(minutes) * 60 + Number(secs) + ms / 1000;
}

export function parseTimestamp(value: string): number | null {
  const match = new RegExp(`^\\s*${TIME}\\s*$`).exec(value);
  return match ? seconds(match[1], match[2]!, match[3]!, match[4]) : null;
}

const ENTITIES: Record<string, string> = {
  amp: '&', lt: '<', gt: '>', nbsp: ' ', quot: '"', apos: "'", lrm: '‎', rlm: '‏'
};

function decodeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, name: string) => {
    if (name[0] === '#') {
      const code = name[1] === 'x' || name[1] === 'X' ? parseInt(name.slice(2), 16) : parseInt(name.slice(1), 10);
      return Number.isFinite(code) && code > 0 && code < 0x110000 ? String.fromCodePoint(code) : '';
    }
    return ENTITIES[name.toLowerCase()] ?? whole;
  });
}

/** Strip HTML-ish markup (<i>, <font>, <v Speaker>, <c.yellow>, <ruby>…),
    ASS override blocks ({\an8}) and decode entities. Ruby annotations are
    dropped so only the base text remains. */
export function stripMarkup(text: string): string {
  return decodeEntities(text
    .replace(/<rt>[\s\S]*?<\/rt>/gi, '')
    .replace(/<\/?[a-z][^>]*>/gi, '')
    .replace(/<\/?[a-z]*(?:\.[^>\s]*)?>/gi, '')
    .replace(/\{\\[^}]*\}/g, ''));
}

function lines(text: string): string[] {
  return text.replace(/^﻿/, '').replace(/\r\n?/g, '\n').split('\n');
}

/** Blocks separated by blank lines (whitespace-only lines count as blank). */
function blocks(text: string): string[][] {
  const out: string[][] = [];
  let current: string[] = [];
  for (const line of lines(text)) {
    if (!line.trim()) { if (current.length) out.push(current); current = []; continue; }
    current.push(line);
  }
  if (current.length) out.push(current);
  return out;
}

/** Word timings from inline VTT timestamps: each timestamp starts the text
    that follows it. */
function inlineWords(raw: string, start: number, end: number): CaptionWord[] | undefined {
  if (!INLINE_TIME.test(raw)) return undefined;
  INLINE_TIME.lastIndex = 0;
  const parts: Array<{ at: number; text: string }> = [];
  let last = 0, at = start, match: RegExpExecArray | null;
  while ((match = INLINE_TIME.exec(raw))) {
    parts.push({ at, text: raw.slice(last, match.index) });
    at = parseTimestamp(match[1]!) ?? at;
    last = match.index + match[0].length;
  }
  parts.push({ at, text: raw.slice(last) });
  const words: CaptionWord[] = [];
  for (const part of parts) {
    for (const token of stripMarkup(part.text).split(/\s+/).filter(Boolean)) words.push({ text: token, start: part.at, end: part.at });
  }
  for (let index = 0; index < words.length; index++) {
    const next = words.slice(index + 1).find(word => word.start > words[index]!.start);
    words[index]!.end = Math.max(words[index]!.start, Math.min(end, next ? next.start : end));
  }
  return words.length ? words : undefined;
}

function readCue(block: string[], vtt: boolean): Partial<CaptionCue> | null {
  const timingIndex = block.findIndex(line => TIMING.test(line));
  // SRT: number, timing, text. VTT: optional identifier, timing, text.
  if (timingIndex < 0 || timingIndex > 1) return null;
  const match = TIMING.exec(block[timingIndex]!)!;
  const start = seconds(match[1], match[2]!, match[3]!, match[4]);
  const end = seconds(match[5], match[6]!, match[7]!, match[8]);
  const body = block.slice(timingIndex + 1);
  const raw = body.join('\n');
  const text = stripMarkup(raw.replace(INLINE_TIME, '')).split('\n').map(line => line.trim()).filter(Boolean).join('\n');
  if (!text) return null;
  const cue: Partial<CaptionCue> = { start, end, text };
  if (vtt) {
    const words = inlineWords(raw, start, end);
    if (words) cue.words = words;
  }
  return cue;
}

export function detectCaptionFormat(text: string, name = ''): CaptionFormat {
  if (/\.vtt$/i.test(name)) return 'vtt';
  if (/\.srt$/i.test(name)) return 'srt';
  return /^﻿?WEBVTT/.test(text.trimStart()) ? 'vtt' : 'srt';
}

export function parseSrt(text: string): ParsedCaptions {
  const cues: Partial<CaptionCue>[] = [];
  let skipped = 0;
  for (const block of blocks(text)) {
    const cue = readCue(block, false);
    if (cue) cues.push(cue); else skipped++;
  }
  return { format: 'srt', cues: normalizeCues(cues), skipped };
}

export function parseVtt(text: string): ParsedCaptions {
  const cues: Partial<CaptionCue>[] = [];
  let skipped = 0;
  const all = blocks(text);
  for (let index = 0; index < all.length; index++) {
    const block = all[index]!;
    const head = block[0]!.trim();
    // The header block may carry metadata lines (Kind:, Language:).
    if (index === 0 && /^﻿?WEBVTT\b/.test(head)) {
      if (block.some(line => TIMING.test(line))) {
        const cue = readCue(block.slice(1), true);
        if (cue) cues.push(cue);
      }
      continue;
    }
    if (/^(NOTE|STYLE|REGION)(\s|$)/.test(head)) continue;
    const cue = readCue(block, true);
    if (cue) cues.push(cue); else skipped++;
  }
  return { format: 'vtt', cues: normalizeCues(cues), skipped };
}

export function parseCaptions(text: string, format: CaptionFormat | 'auto' = 'auto', name = ''): ParsedCaptions {
  const resolved = format === 'auto' ? detectCaptionFormat(text, name) : format;
  const parsed = resolved === 'vtt' ? parseVtt(text) : parseSrt(text);
  // A file with the wrong extension still reads.
  if (!parsed.cues.length && format === 'auto') {
    const other = resolved === 'vtt' ? parseSrt(text) : parseVtt(text);
    if (other.cues.length) return other;
  }
  return parsed;
}

/* ── writers ─────────────────────────────────────────────── */

function stamp(value: number, separator: ',' | '.'): string {
  const total = Math.max(0, Math.round(value * 1000));
  const ms = total % 1000, s = Math.floor(total / 1000) % 60, m = Math.floor(total / 60000) % 60, h = Math.floor(total / 3600000);
  const two = (n: number) => String(n).padStart(2, '0');
  return `${two(h)}:${two(m)}:${two(s)}${separator}${String(ms).padStart(3, '0')}`;
}

export interface WriteOptions {
  /** Seconds added to every cue, e.g. the layer start for composition time. */
  offset?: number;
  /** Cues ending at or before this time (after offset) are left out. */
  from?: number;
  /** Cues starting at or after this time (after offset) are left out; the last
      one is cut to it. */
  to?: number;
}

function timed(cues: readonly CaptionCue[], options: WriteOptions) {
  const offset = options.offset ?? 0, from = options.from ?? -Infinity, to = options.to ?? Infinity;
  const base = Number.isFinite(from) && from > 0 ? from : 0;
  return cues
    .map(cue => ({ text: cue.text, start: Math.max(cue.start + offset, from), end: Math.min(cue.end + offset, to) }))
    .filter(cue => cue.end - cue.start >= MIN_CUE_DURATION / 2)
    .map(cue => ({ ...cue, start: cue.start - base, end: cue.end - base }))
    .map(cue => {
      // Millisecond rounding must never produce an empty or reversed cue.
      const start = Math.round(cue.start * 1000) / 1000;
      return { ...cue, start, end: Math.max(start + 0.001, Math.round(cue.end * 1000) / 1000) };
    });
}

export function formatSrt(cues: readonly CaptionCue[], options: WriteOptions = {}): string {
  return timed(cues, options).map((cue, index) =>
    `${index + 1}\n${stamp(cue.start, ',')} --> ${stamp(cue.end, ',')}\n${cue.text.split('\n').filter(line => line.trim()).join('\n')}\n`
  ).join('\n');
}

const vttText = (text: string) => text
  .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  .split('\n').filter(line => line.trim()).join('\n');

export function formatVtt(cues: readonly CaptionCue[], options: WriteOptions & { language?: string } = {}): string {
  const header = options.language ? `WEBVTT\nLanguage: ${options.language}\n` : 'WEBVTT\n';
  return [header, ...timed(cues, options).map(cue => `${stamp(cue.start, '.')} --> ${stamp(cue.end, '.')}\n${vttText(cue.text)}\n`)].join('\n');
}

export function formatCaptions(format: CaptionFormat, cues: readonly CaptionCue[], options: WriteOptions & { language?: string } = {}): string {
  return format === 'vtt' ? formatVtt(cues, options) : formatSrt(cues, options);
}
