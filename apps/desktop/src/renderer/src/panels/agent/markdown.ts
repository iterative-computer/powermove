/* Streaming-friendly markdown → Block[] for agent prose.

   Model output is never rendered as HTML: the parser yields blocks of inline
   runs, and the Svelte side builds real elements from them. It tolerates
   mid-stream input — an unclosed fence or ** renders as literal text until
   its closer arrives, then upgrades in place — so a paragraph can re-parse on
   every chunk without flicker. Ported from stead-ui, kept to what agent
   replies actually use. */

export interface Run {
  text: string;
  b?: boolean;
  i?: boolean;
  c?: boolean;
  href?: string;
}

export type Block =
  | { kind: 'p'; runs: Run[] }
  | { kind: 'h'; level: number; runs: Run[] }
  | { kind: 'li'; ordinal?: number; runs: Run[] }
  | { kind: 'quote'; runs: Run[] }
  | { kind: 'code'; text: string; lang?: string };

type InlineFlags = { b?: boolean; i?: boolean };

const SAFE_HREF = /^(https?:\/\/|mailto:)/i;

/** Inline spans: `code`, **bold**, *italic* / _italic_, [text](url), bare URLs. */
export function inlineRuns(text: string, flags: InlineFlags = {}): Run[] {
  const runs: Run[] = [];
  let plain = '';
  let i = 0;
  const flush = (): void => {
    if (plain) { runs.push({ text: plain, ...flags }); plain = ''; }
  };
  while (i < text.length) {
    const rest = text.slice(i);
    let m: RegExpExecArray | null;
    if ((m = /^`([^`]+)`/.exec(rest))) {
      flush(); runs.push({ text: m[1]!, c: true }); i += m[0].length; continue;
    }
    if (!flags.b && (m = /^\*\*([^\s*](?:.*?[^\s*])?)\*\*/.exec(rest))) {
      flush(); runs.push(...inlineRuns(m[1]!, { ...flags, b: true })); i += m[0].length; continue;
    }
    if (!flags.i && (m = /^\*([^\s*](?:[^*]*[^\s*])?)\*/.exec(rest))) {
      flush(); runs.push(...inlineRuns(m[1]!, { ...flags, i: true })); i += m[0].length; continue;
    }
    if (!flags.i && (m = /^_([^\s_](?:[^_]*[^\s_])?)_(?![a-zA-Z0-9])/.exec(rest))) {
      // Word-boundary guard so snake_case identifiers survive.
      const prev = i > 0 ? text[i - 1]! : ' ';
      if (/[\s([{'"]/.test(prev)) {
        flush(); runs.push(...inlineRuns(m[1]!, { ...flags, i: true })); i += m[0].length; continue;
      }
    }
    // URLs may carry one level of parentheses (Wikimedia file names do).
    if ((m = /^\[([^\]]+)\]\(((?:[^()\s]|\([^()\s]*\))+)\)/.exec(rest))) {
      flush();
      const href = m[2]!;
      runs.push(SAFE_HREF.test(href) ? { text: m[1]!, href, ...flags } : { text: m[1]!, ...flags });
      i += m[0].length; continue;
    }
    // A bare URL ends before trailing punctuation, so "see https://x.y." links x.y.
    if ((m = /^https?:\/\/(?:[^\s<>()]|\([^()\s]*\))*(?:[^\s<>().,;:!?'"]|\([^()\s]*\))/.exec(rest))) {
      flush(); runs.push({ text: m[0], href: m[0], ...flags }); i += m[0].length; continue;
    }
    plain += text[i]; i += 1;
  }
  flush();
  return runs;
}

export function blocksFromMarkdown(text: string): Block[] {
  const blocks: Block[] = [];
  const lines = String(text ?? '').replace(/\r\n?/g, '\n').split('\n');
  let paragraph: string[] = [];
  let quote: string[] = [];
  let code: string[] | null = null;
  let lang = '';

  const flushParagraph = (): void => {
    if (paragraph.length) { blocks.push({ kind: 'p', runs: inlineRuns(paragraph.join(' ')) }); paragraph = []; }
  };
  const flushQuote = (): void => {
    if (quote.length) { blocks.push({ kind: 'quote', runs: inlineRuns(quote.join(' ')) }); quote = []; }
  };
  const flushAll = (): void => { flushParagraph(); flushQuote(); };

  for (const line of lines) {
    if (code !== null) {
      if (/^\s*(```|~~~)\s*$/.test(line)) {
        blocks.push({ kind: 'code', text: code.join('\n'), ...(lang ? { lang } : {}) });
        code = null; lang = '';
      } else {
        code.push(line);
      }
      continue;
    }
    let m: RegExpExecArray | null;
    if ((m = /^\s*(```|~~~)\s*([\w+-]*)\s*$/.exec(line))) {
      flushAll(); code = []; lang = m[2] ?? ''; continue;
    }
    if (!line.trim()) { flushAll(); continue; }
    if ((m = /^(#{1,6})\s+(.*)$/.exec(line))) {
      flushAll(); blocks.push({ kind: 'h', level: m[1]!.length, runs: inlineRuns(m[2]!) }); continue;
    }
    if (/^\s*([-*_])\s*(?:\1\s*){2,}$/.test(line)) { flushAll(); continue; }
    if ((m = /^\s*[-*+]\s+(.*)$/.exec(line))) {
      flushAll(); blocks.push({ kind: 'li', runs: inlineRuns(m[1]!) }); continue;
    }
    if ((m = /^\s*(\d+)[.)]\s+(.*)$/.exec(line))) {
      flushAll(); blocks.push({ kind: 'li', ordinal: Number(m[1]), runs: inlineRuns(m[2]!) }); continue;
    }
    if ((m = /^\s*>\s?(.*)$/.exec(line))) { flushParagraph(); quote.push(m[1]!); continue; }
    flushQuote();
    paragraph.push(line);
  }
  // A still-open fence at stream end renders as code so the block does not
  // flicker between text and code while tokens arrive.
  if (code !== null) blocks.push({ kind: 'code', text: code.join('\n'), ...(lang ? { lang } : {}) });
  flushAll();
  return blocks;
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
    if (run.href) { words.push({ ...run, key: `${ri}` }); return; }
    run.text.split(/(\s+)/).forEach((part, wi) => {
      if (part) words.push({ ...run, text: part, key: `${ri}-${wi}` });
    });
  });
  return words;
}

/** The plain text of a block, for tests and tooltips. */
export function plainText(blocks: Block[]): string {
  return blocks.map((block) => block.kind === 'code' ? block.text : block.runs.map((run) => run.text).join('')).join('\n');
}
