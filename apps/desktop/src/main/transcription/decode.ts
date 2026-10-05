import type { TranscriptWord } from '../../shared/transcription';
import type { TranscriptionTiming as CatalogTiming } from '../../shared/transcription';
import type { WordLag } from './catalog';
import { removeLag, spreadWords, tokensToWords } from './words';

/*
 * From one window's transcribe.cpp result to words with source times. Pure,
 * so it is tested without the native runtime.
 *
 *   word     token timestamps (Parakeet, Nemotron): tokens → words as before
 *   segment  phrase timestamps (Whisper): each phrase's words share its span
 *   none     text only (Canary, Cohere): the window's sound span is shared
 */

export interface EngineToken { text: string; t0Ms: number; t1Ms: number; p: number }
export interface EngineSegment { text: string; t0Ms: number; t1Ms: number }
export interface EngineResult {
  text: string;
  tokens: readonly EngineToken[];
  segments: readonly EngineSegment[];
}

export interface WindowTiming {
  /** Source seconds of the decoded audio's first sample (lead-in included). */
  offset: number;
  /** Source seconds where the window's sound starts and ends. */
  soundFrom: number;
  soundTo: number;
}

export function windowWords(result: EngineResult, timing: CatalogTiming, window: WindowTiming, lag?: WordLag): TranscriptWord[] {
  if (timing === 'word' && result.tokens.length) {
    const words = tokensToWords({
      tokens: result.tokens.map((token) => token.text),
      timestamps: result.tokens.map((token) => token.t0Ms / 1000),
      durations: result.tokens.map((token) => Math.max(0, token.t1Ms - token.t0Ms) / 1000),
      logProbs: result.tokens.map((token) => Math.log(Math.max(1e-6, Math.min(1, token.p))))
    }, window.offset);
    return lag ? removeLag(words, lag) : words;
  }
  const clamp = (value: number) => Math.min(window.soundTo, Math.max(window.soundFrom, value));
  if (timing !== 'none') {
    const timed = result.segments.filter((segment) => segment.text.trim() && segment.t1Ms > segment.t0Ms);
    if (timed.length) {
      return timed.flatMap((segment) => {
        const start = clamp(window.offset + segment.t0Ms / 1000);
        const end = clamp(window.offset + segment.t1Ms / 1000);
        return spreadWords(segment.text, start, Math.max(start, end));
      });
    }
  }
  return spreadWords(result.text, window.soundFrom, window.soundTo);
}
