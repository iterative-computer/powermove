import type { TranscriptSegment, TranscriptWord } from '../../shared/transcription';

/*
 * From recognizer tokens to words and caption-sized segments. Pure, so the
 * worker and the tests share it.
 *
 * Parakeet's SentencePiece tokens mark a new word with a leading space
 * (sherpa-onnx turns '▁' into ' '). A TDT token carries the frame it was
 * emitted on and how many frames it covers, so a word spans its first token's
 * start to its last token's start + duration, clipped at the next word.
 */

export interface RecognizedTokens {
  tokens: string[];
  /** Seconds from the start of the decoded chunk. */
  timestamps: number[];
  /** Seconds each token covers (TDT); absent for models that do not report it. */
  durations?: number[];
  /** Natural-log token probabilities. */
  logProbs?: number[];
}

/** Punctuation that never starts a word of its own. */
const ATTACHED = /^[\p{P}\p{S}]+$/u;
/** CJK has no spaces: each character is a word. */
const CJK = /[぀-ヿ㐀-䶿一-鿿가-힯豈-﫿]/u;
const FALLBACK_TOKEN_SECONDS = 0.08;

export function tokensToWords(result: RecognizedTokens, offset = 0): TranscriptWord[] {
  const words: Array<TranscriptWord & { logSum: number; count: number }> = [];
  const { tokens, timestamps, durations, logProbs } = result;
  for (let index = 0; index < tokens.length; index++) {
    const raw = tokens[index] ?? '';
    const start = (timestamps[index] ?? 0) + offset;
    const duration = durations?.[index] && durations[index]! > 0 ? durations[index]! : FALLBACK_TOKEN_SECONDS;
    const end = start + duration;
    const logProb = logProbs?.[index];
    const text = raw.replace(/▁/g, ' ');
    if (!text.trim()) continue;
    const current = words[words.length - 1];
    const startsWord = !current || /^\s/.test(text) || (CJK.test(text) && !ATTACHED.test(text.trim()));
    const trimmed = text.trim();
    if (!startsWord || (current && ATTACHED.test(trimmed) && !/^\s/.test(text))) {
      current!.text += trimmed;
      /* Punctuation is emitted in the pause after a word; it is not speech. */
      if (!ATTACHED.test(trimmed)) {
        current!.end = Math.max(current!.end, end);
        if (logProb !== undefined) { current!.logSum += logProb; current!.count++; }
      }
      continue;
    }
    words.push({ text: trimmed, start, end, logSum: logProb ?? 0, count: logProb === undefined ? 0 : 1 });
  }
  return words.map((word, index) => {
    const next = words[index + 1];
    const end = next ? Math.min(word.end, next.start) : word.end;
    const out: TranscriptWord = { text: word.text, start: round(word.start), end: round(Math.max(word.start, end)) };
    if (word.count) out.confidence = round(Math.exp(word.logSum / word.count));
    return out;
  });
}

const SENTENCE_END = /[.!?…。！？]["'”’)\]]*$/u;
const CLAUSE_END = /[,;:，、；：]["'”’)\]]*$/u;

export interface SegmentOptions {
  /** A silence at least this long ends a segment. */
  pause?: number;
  /** Segments longer than this split at their best clause break. */
  maxDuration?: number;
}

/** Groups words into sentence-like segments: ends at sentence punctuation or a long pause, never runs past maxDuration. */
export function wordsToSegments(words: TranscriptWord[], options: SegmentOptions = {}): TranscriptSegment[] {
  const pause = options.pause ?? 0.8;
  const maxDuration = options.maxDuration ?? 12;
  const groups: TranscriptWord[][] = [];
  let current: TranscriptWord[] = [];
  const flush = () => {
    if (current.length) groups.push(current);
    current = [];
  };
  for (const word of words) {
    const previous = current[current.length - 1];
    if (previous && word.start - previous.end >= pause) flush();
    current.push(word);
    if (SENTENCE_END.test(word.text)) flush();
  }
  flush();
  return groups.flatMap((group) => splitLong(group, maxDuration)).map(segment);
}

function splitLong(words: TranscriptWord[], maxDuration: number): TranscriptWord[][] {
  const first = words[0];
  const last = words[words.length - 1];
  if (!first || !last || last.end - first.start <= maxDuration || words.length < 2) return [words];
  /* Prefer a clause break nearest the middle, else the widest gap. */
  const middle = (first.start + last.end) / 2;
  let best = -1;
  let bestScore = -Infinity;
  for (let index = 0; index < words.length - 1; index++) {
    const word = words[index]!;
    const next = words[index + 1]!;
    const gap = next.start - word.end;
    const clause = CLAUSE_END.test(word.text) ? 1 : 0;
    const centred = 1 - Math.abs(word.end - middle) / (last.end - first.start);
    const score = clause * 2 + gap * 4 + centred;
    if (score > bestScore) { bestScore = score; best = index; }
  }
  return [...splitLong(words.slice(0, best + 1), maxDuration), ...splitLong(words.slice(best + 1), maxDuration)];
}

function segment(words: TranscriptWord[]): TranscriptSegment {
  return {
    text: joinWords(words),
    start: words[0]!.start,
    end: words[words.length - 1]!.end,
    words
  };
}

export function joinWords(words: TranscriptWord[]): string {
  let text = '';
  for (const word of words) {
    if (!text) text = word.text;
    else if (CJK.test(word.text[0] ?? '') && CJK.test(text[text.length - 1] ?? '')) text += word.text;
    else text += ` ${word.text}`;
  }
  return text;
}

function round(value: number): number {
  return Math.round(value * 1000) / 1000;
}
