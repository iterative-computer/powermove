/**
 * Captions layer content. Cue and word times are seconds in layer time
 * (relative to the layer's start), so moving or trimming the layer carries its
 * captions with it, like any other clip.
 */

export interface CaptionWord {
  text: string;
  start: number;
  end: number;
}

export interface CaptionCue {
  id: string;
  start: number;
  end: number;
  /** Display text. A newline is an authored line break. */
  text: string;
  /** Word timings from transcription; absent for imported or typed cues. */
  words?: CaptionWord[];
}

export const CAPTION_PLACEMENTS = ['bottom', 'middle', 'top'] as const;
export type CaptionPlacement = (typeof CAPTION_PLACEMENTS)[number];

export const CAPTION_TEXT_CASES = ['original', 'upper', 'lower'] as const;
export type CaptionTextCase = (typeof CAPTION_TEXT_CASES)[number];

export const CAPTION_WORD_ANIMATIONS = ['none', 'pop', 'fade'] as const;
export type CaptionWordAnimation = (typeof CAPTION_WORD_ANIMATIONS)[number];

export const CAPTION_ALIGNS = ['left', 'center', 'right'] as const;
export type CaptionAlign = (typeof CAPTION_ALIGNS)[number];

export interface CaptionStyle {
  /** The preset the style started from; 'custom' once it no longer matches one. */
  preset: string;
  font: string;
  weight: number;
  italic: boolean;
  /** Pixels at the composition's resolution. */
  size: number;
  tracking: number;
  /** Line height as a multiple of size. */
  leading: number;
  fill: string;
  stroke: number;
  strokeColor: string;
  box: boolean;
  boxColor: string;
  /** 0–100. */
  boxOpacity: number;
  boxPadding: number;
  boxRadius: number;
  placement: CaptionPlacement;
  align: CaptionAlign;
  /** Pixels from the placement anchor; positive moves right / down. */
  offsetX: number;
  offsetY: number;
  /** Title-safe margin, percent of the composition height. */
  safeMargin: number;
  /** Percent of the composition width. */
  maxWidth: number;
  maxLines: number;
  textCase: CaptionTextCase;
  highlight: boolean;
  highlightColor: string;
  /** Percent; 100 keeps the spoken word at its size. */
  highlightScale: number;
  wordAnimation: CaptionWordAnimation;
}

export interface CaptionsContent {
  [key: string]: unknown;
  cues: CaptionCue[];
  style: CaptionStyle;
  /** BCP-47 tag, informational (sidecar export, agent context). */
  language?: string;
}
