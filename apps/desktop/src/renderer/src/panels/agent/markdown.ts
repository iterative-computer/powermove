/* Streaming-friendly markdown → Block[] for agent prose.

   Model output is never rendered as HTML: the parser yields a tree of blocks
   holding inline runs, and the Svelte side builds real elements from them.
   It covers what agents write — headings (ATX and underlined), paragraphs
   with hard breaks, nested and task lists, quotes that hold any block,
   fenced code, tables, rules, and images, video and sound — and tolerates
   mid-stream input: an unclosed fence or ** renders as literal text until its
   closer arrives, then upgrades in place, so a reply can re-parse on every
   chunk without flicker. */

import { agentFileUrl, agentMediaKind, localMediaPath, type AgentMediaKind } from '../../../../shared/agent-media';

/** An image, video or sound the reply shows. */
export interface Media {
  kind: AgentMediaKind;
  /** The URL the panel loads. */
  src: string;
  /** The path or URL the agent wrote. */
  path: string;
  alt: string;
  /** A web address: loads only when the user asks, since the URL could carry data out. */
  remote?: boolean;
}

export interface Run {
  text: string;
  b?: boolean;
  i?: boolean;
  c?: boolean;
  s?: boolean;
  href?: string;
  /** A media marker: an empty run whose file renders below its block. */
  media?: Media;
}

type Prose = { runs: Run[]; media?: Media[] };

export interface ListItem {
  blocks: Block[];
  /** Task-list items (`- [ ]` / `- [x]`) carry their state. */
  checked?: boolean;
}

export type Block =
  | ({ kind: 'p' } & Prose)
  | ({ kind: 'h'; level: number } & Prose)
  | { kind: 'list'; ordered: boolean; start: number; items: ListItem[] }
  | { kind: 'quote'; blocks: Block[] }
  | { kind: 'hr' }
  | { kind: 'table'; headers: Run[][]; rows: Run[][][]; align: Array<'left' | 'center' | 'right'> }
  | { kind: 'code'; text: string; lang?: string };

type InlineFlags = { b?: boolean; i?: boolean; s?: boolean; c?: boolean };

// Only what main's openExternal will actually open; anything else stays text.
const SAFE_HREF = /^https?:\/\//i;
const MEDIA_EXT = 'png|jpe?g|gif|webp|avif|svg|bmp|mp4|m4v|mov|webm|mp3|m4a|wav|aac|ogg|flac';
// A bare absolute or home-relative path to a media file, as agents write them.
const BARE_MEDIA = new RegExp(`^(?:~\\/|\\/)[^\\s<>()[\\]\`'"]*\\.(?:${MEDIA_EXT})(?![\\w/])`, 'i');
// Inline HTML agents reach for; the tag is dropped and its meaning kept.
const HTML_STYLE: Readonly<Record<string, InlineFlags>> = {
  b: { b: true }, strong: { b: true }, i: { i: true }, em: { i: true }, s: { s: true }, del: { s: true }, strike: { s: true },
  code: { c: true }, kbd: { c: true }, u: {}, mark: {}, sub: {}, sup: {}, small: {}, span: {}
};

function fileName(value: string): string {
  return value.split(/[?#]/)[0]!.split('/').filter(Boolean).at(-1) || value;
}

function mediaRun(target: string, alt = '', image = false): Run | null {
  const local = localMediaPath(target);
  const kind = local ? agentMediaKind(local) : null;
  if (local && kind) return { text: '', media: { kind, src: agentFileUrl(local), path: local, alt: alt || fileName(local) } };
  // A web address renders only when written as an image: ![alt](https://…).
  if (image && /^https:\/\//i.test(target)) {
    const remoteKind = agentMediaKind(target) ?? 'image';
    return { text: '', media: { kind: remoteKind, src: target, path: target, alt: alt || fileName(target), remote: true } };
  }
  return null;
}

/** Pulls media markers out of inline runs, one per file, in reading order. */
export function prose(runs: Run[]): Prose {
  const media: Media[] = [];
  for (const run of runs) if (run.media && !media.some(item => item.path === run.media!.path)) media.push(run.media);
  const text = runs.filter(run => !run.media);
  return media.length ? { runs: text, media } : { runs: text };
}

/** Inline spans: `code`, ***bold italic***, **bold** / __bold__, *italic* / _italic_, ~~struck~~,
    [text](url), ![alt](src), <url>, bare URLs and media paths, <br> and simple inline HTML. */
export function inlineRuns(text: string, flags: InlineFlags = {}): Run[] {
  const runs: Run[] = [];
  let plain = '';
  let i = 0;
  const flush = (): void => {
    if (plain) { runs.push({ text: plain, ...flags }); plain = ''; }
  };
  const nested = (inner: string, extra: InlineFlags): void => { flush(); runs.push(...inlineRuns(inner, { ...flags, ...extra })); };
  while (i < text.length) {
    const rest = text.slice(i);
    const prev = i > 0 ? text[i - 1]! : ' ';
    let m: RegExpExecArray | null;
    if ((m = /^\\([!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~])/.exec(rest))) {
      plain += m[1]; i += m[0].length; continue;
    }
    if ((m = /^<br\s*\/?>\n?/i.exec(rest))) {
      plain += '\n'; i += m[0].length; continue;
    }
    if ((m = /^(`+)([\s\S]*?[^`])\1(?!`)/.exec(rest))) {
      flush(); runs.push({ text: m[2]!.replace(/\n/g, ' '), c: true });
      const media = mediaRun(m[2]!.trim());
      if (media) runs.push(media);
      i += m[0].length; continue;
    }
    if (!flags.s && (m = /^~~(\S(?:[\s\S]*?\S)?)~~/.exec(rest))) { nested(m[1]!, { s: true }); i += m[0].length; continue; }
    if (!flags.b && !flags.i && (m = /^\*\*\*([^\s*](?:[\s\S]*?[^\s*])?)\*\*\*/.exec(rest))) { nested(m[1]!, { b: true, i: true }); i += m[0].length; continue; }
    if (!flags.b && (m = /^\*\*([^\s*](?:[\s\S]*?[^\s*])?)\*\*/.exec(rest))) { nested(m[1]!, { b: true }); i += m[0].length; continue; }
    if (!flags.b && /[\s([{'"]/.test(prev) && (m = /^__([^\s_](?:[\s\S]*?[^\s_])?)__(?![a-zA-Z0-9])/.exec(rest))) { nested(m[1]!, { b: true }); i += m[0].length; continue; }
    if (!flags.i && (m = /^\*([^\s*](?:[^*]*[^\s*])?)\*/.exec(rest))) { nested(m[1]!, { i: true }); i += m[0].length; continue; }
    // Word-boundary guard so snake_case identifiers survive.
    if (!flags.i && /[\s([{'"]/.test(prev) && (m = /^_([^\s_](?:[^_]*[^\s_])?)_(?![a-zA-Z0-9])/.exec(rest))) { nested(m[1]!, { i: true }); i += m[0].length; continue; }
    if ((m = /^<(b|strong|i|em|s|del|strike|code|kbd|u|mark|sub|sup|small|span)(?:\s[^<>]*)?>([\s\S]*?)<\/\1>/i.exec(rest))) {
      const style = HTML_STYLE[m[1]!.toLowerCase()]!;
      if (style.c) { flush(); runs.push({ text: m[2]!, c: true }); } else nested(m[2]!, style);
      i += m[0].length; continue;
    }
    if ((m = /^<img\s[^<>]*?src=["']([^"']+)["'][^<>]*>/i.exec(rest))) {
      const alt = /alt=["']([^"']*)["']/i.exec(m[0])?.[1] ?? '';
      const media = mediaRun(m[1]!, alt, true);
      flush(); runs.push(media ?? { text: alt || fileName(m[1]!), ...flags });
      i += m[0].length; continue;
    }
    // URLs may carry one level of parentheses (Wikimedia file names do), and
    // <angle brackets> allow spaces in a local path. An optional "title" is dropped.
    if ((m = /^(!?)\[((?:[^\][]|\[[^\][]*\])*)\]\((?:<([^<>\n]+)>|((?:[^()\s<>]|\([^()\s]*\))+))(?:\s+"[^"]*")?\)/.exec(rest))) {
      flush();
      const href = (m[3] ?? m[4])!;
      const image = Boolean(m[1]);
      const media = mediaRun(href, image ? m[2] : '', image);
      if (media && image) { runs.push(media); i += m[0].length; continue; }
      // A linked image ([![alt](src)](url)) keeps both the picture and the link.
      const inner = inlineRuns(m[2] || (image ? fileName(href) : href), flags);
      const label = inner.filter(run => !run.media);
      const safe = SAFE_HREF.test(href);
      runs.push(...(label.length ? label : [{ text: inner.find(run => run.media)?.media!.alt ?? fileName(href), ...flags }]).map(run => safe && !run.c ? { ...run, href } : run));
      runs.push(...inner.filter(run => run.media));
      if (media) runs.push(media);
      i += m[0].length; continue;
    }
    if ((m = /^<(https?:\/\/[^\s<>]+)>/i.exec(rest))) {
      flush(); runs.push({ text: m[1]!, href: m[1]!, ...flags });
      i += m[0].length; continue;
    }
    // A bare URL ends before trailing punctuation, so "see https://x.y." links x.y.
    if (/[^\w/]|^$/.test(i ? prev : '') && (m = /^https?:\/\/(?:[^\s<>()]|\([^()\s]*\))*(?:[^\s<>().,;:!?'"*_~]|\([^()\s]*\))/.exec(rest))) {
      flush(); runs.push({ text: m[0], href: m[0], ...flags }); i += m[0].length; continue;
    }
    if (/[\s([{'"]/.test(prev) && (m = BARE_MEDIA.exec(rest))) {
      const media = mediaRun(m[0]);
      if (media) { plain += m[0]; flush(); runs.push(media); i += m[0].length; continue; }
    }
    plain += text[i]; i += 1;
  }
  flush();
  return runs;
}

/* ---------------------------------------------------------------- blocks */

const FENCE = /^( {0,3})(`{3,}|~{3,})\s*([^\s`]*)[^`]*$/;
const HEADING = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$/;
const RULE = /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/;
const QUOTE = /^ {0,3}>/;
const ITEM = /^( {0,3})([-*+]|\d{1,9}[.)])(?:([ \t]+)(.*))?$/;

const expandTabs = (line: string): string => line.replace(/^\t+/, tabs => '    '.repeat(tabs.length));
const indentOf = (line: string): number => /^ */.exec(line)![0].length;
const blank = (line: string | undefined): boolean => line !== undefined && !line.trim();
const cells = (line: string): string[] => line.trim().replace(/^\|/, '').replace(/(?<!\\)\|$/, '').split(/(?<!\\)\|/).map(cell => cell.trim());
const isSeparator = (line: string | undefined): boolean => !!line && line.includes('-') && cells(line).every(cell => /^:?-{3,}:?$/.test(cell));

/** Whether a line opens a block that ends a paragraph. */
function interrupts(line: string): boolean {
  if (FENCE.test(line) || HEADING.test(line) || RULE.test(line) || QUOTE.test(line)) return true;
  const item = ITEM.exec(line);
  return !!item && !!item[4]?.trim();
}

/** Lines join with spaces; two trailing spaces or a trailing backslash is a hard break. */
function paragraph(lines: string[]): Block {
  const text = lines.map((line, index) => {
    const trimmed = line.trimStart();
    if (index === lines.length - 1) return trimmed.trimEnd();
    if (/ {2,}$/.test(trimmed)) return `${trimmed.trimEnd()}\n`;
    if (/(?:^|[^\\])\\$/.test(trimmed)) return `${trimmed.slice(0, -1)}\n`;
    return `${trimmed.trimEnd()} `;
  }).join('');
  return { kind: 'p', ...prose(inlineRuns(text)) };
}

function parseBlocks(source: string[]): Block[] {
  const lines = source.map(expandTabs);
  const blocks: Block[] = [];
  let para: string[] = [];
  const flush = (): void => { if (para.length) { blocks.push(paragraph(para)); para = []; } };

  for (let index = 0; index < lines.length; index++) {
    const line = lines[index]!;
    let m: RegExpExecArray | null;
    if (blank(line)) { flush(); continue; }

    if ((m = FENCE.exec(line))) {
      flush();
      const [, indent, marker, lang] = m;
      const body: string[] = [];
      const close = new RegExp(`^ {0,3}${marker![0] === '`' ? '`' : '~'}{${marker!.length},}\\s*$`);
      while (++index < lines.length && !close.test(lines[index]!)) {
        body.push(lines[index]!.replace(new RegExp(`^ {0,${indent!.length}}`), ''));
      }
      // A still-open fence at stream end renders as code so the block does
      // not flicker between text and code while tokens arrive.
      blocks.push({ kind: 'code', text: body.join('\n'), ...(lang ? { lang } : {}) });
      continue;
    }
    if ((m = HEADING.exec(line))) {
      flush(); blocks.push({ kind: 'h', level: m[1]!.length, ...prose(inlineRuns((m[2] ?? '').trim())) }); continue;
    }
    // Underlined headings need three dashes so a list starting with "-" mid-stream stays a paragraph.
    if (para.length && (m = /^ {0,3}(=+|-{3,})[ \t]*$/.exec(line))) {
      const text = para.map(entry => entry.trim()).join(' ');
      para = [];
      blocks.push({ kind: 'h', level: m[1]![0] === '=' ? 1 : 2, ...prose(inlineRuns(text)) });
      continue;
    }
    if (RULE.test(line)) { flush(); blocks.push({ kind: 'hr' }); continue; }
    if (line.includes('|') && isSeparator(lines[index + 1]) && cells(line).length === cells(lines[index + 1]!).length) {
      flush();
      const headers = cells(line).map(cell => inlineRuns(cell));
      const align = cells(lines[index + 1]!).map(cell => cell.startsWith(':') && cell.endsWith(':') ? 'center' as const : cell.endsWith(':') ? 'right' as const : 'left' as const);
      const rows: Run[][][] = [];
      index++;
      while (lines[index + 1]?.trim() && lines[index + 1]!.includes('|')) {
        const row = cells(lines[++index]!);
        rows.push(headers.map((_, ci) => inlineRuns(row[ci] || '')));
      }
      blocks.push({ kind: 'table', headers, align, rows }); continue;
    }
    if (QUOTE.test(line)) {
      flush();
      const inner: string[] = [];
      for (; index < lines.length; index++) {
        const next = lines[index]!;
        if (QUOTE.test(next)) inner.push(next.replace(/^ {0,3}> ?/, ''));
        // Lazy continuation: a plain line right after quoted text stays in the quote.
        else if (!blank(next) && inner.length && !blank(inner.at(-1)) && !interrupts(next)) inner.push(next);
        else break;
      }
      index--;
      blocks.push({ kind: 'quote', blocks: parseBlocks(inner) });
      continue;
    }
    if ((m = ITEM.exec(line)) && (!para.length || m[4]?.trim())) {
      flush();
      index = parseList(lines, index, blocks) - 1;
      continue;
    }
    para.push(line);
  }
  flush();
  return blocks;
}

/** Parses the list starting at `start`, appends it, and returns the first line after it. */
function parseList(lines: string[], start: number, blocks: Block[]): number {
  const first = ITEM.exec(lines[start]!)!;
  const ordered = /\d/.test(first[2]!);
  const delimiter = ordered ? first[2]!.slice(-1) : first[2]!;
  const items: ListItem[] = [];
  let index = start;
  while (index < lines.length) {
    const m = ITEM.exec(lines[index]!);
    if (!m || /\d/.test(m[2]!) !== ordered || (ordered ? m[2]!.slice(-1) : m[2]) !== delimiter) break;
    const gap = m[3] ?? ' ';
    // Content past four spaces of gap is indented code in CommonMark; agents mean one space.
    const width = m[1]!.length + m[2]!.length + (gap.length > 4 ? 1 : gap.length);
    const body = [gap.length > 4 ? `${gap.slice(1)}${m[4] ?? ''}` : (m[4] ?? '')];
    index++;
    let inFence = FENCE.test(body[0]!);
    for (; index < lines.length; index++) {
      const next = lines[index]!;
      if (blank(next)) {
        // A blank line ends the item unless indented content follows it.
        let ahead = index + 1;
        while (blank(lines[ahead])) ahead++;
        if (inFence || (ahead < lines.length && indentOf(lines[ahead]!) >= width)) { body.push(''); continue; }
        break;
      }
      if (indentOf(next) >= width) { body.push(next.slice(width)); if (FENCE.test(next.slice(width))) inFence = !inFence; continue; }
      if (inFence) { body.push(next.trimStart()); if (FENCE.test(next.trimStart())) inFence = false; continue; }
      // Lazy continuation of the item's last paragraph.
      if (!blank(body.at(-1)) && !interrupts(next) && !ITEM.test(next) && !FENCE.test(body.at(-1)!)) { body.push(next.trimStart()); continue; }
      break;
    }
    let checked: boolean | undefined;
    const task = /^\[([ xX])\](?:[ \t]+|$)/.exec(body[0]!);
    if (task) { checked = task[1] !== ' '; body[0] = body[0]!.slice(task[0].length); }
    items.push({ blocks: parseBlocks(body), ...(checked === undefined ? {} : { checked }) });
    // A blank line between items keeps the list going.
    let ahead = index;
    while (blank(lines[ahead])) ahead++;
    if (ahead < lines.length && ITEM.test(lines[ahead]!) && indentOf(lines[ahead]!) < width) index = ahead;
  }
  blocks.push({ kind: 'list', ordered, start: ordered ? Number(/\d+/.exec(first[2]!)![0]) : 1, items });
  return index;
}

export function blocksFromMarkdown(text: string): Block[] {
  return parseBlocks(String(text ?? '').replace(/\r\n?/g, '\n').split('\n'));
}

/** One reveal unit: a word or a whitespace gap, carrying its run's styling. */
export interface Word extends Run {
  /** Stable key within the block: run index and word index. */
  key: string;
}

/* Runs split per word so the reveal can stagger and `white-space: pre-wrap`
   stays honest; a link stays whole so its underline does not break per word. */
export function wordsFromRuns(runs: Run[]): Word[] {
  const words: Word[] = [];
  runs.forEach((run, ri) => {
    if (run.media) return;
    if (run.href || run.c) { words.push({ ...run, key: `${ri}` }); return; }
    run.text.split(/(\s+)/).forEach((part, wi) => {
      if (part) words.push({ ...run, text: part, key: `${ri}-${wi}` });
    });
  });
  return words;
}

const runText = (runs: Run[]): string => runs.map(run => run.text).join('');

/** The plain text of a block tree, for tests and tooltips. */
export function plainText(blocks: Block[]): string {
  return blocks.map((block): string => {
    switch (block.kind) {
      case 'code': return block.text;
      case 'hr': return '';
      case 'table': return [block.headers, ...block.rows].map(row => row.map(runText).join(' | ')).join('\n');
      case 'quote': return plainText(block.blocks);
      case 'list': return block.items.map(item => plainText(item.blocks)).join('\n');
      default: return runText(block.runs);
    }
  }).join('\n');
}
