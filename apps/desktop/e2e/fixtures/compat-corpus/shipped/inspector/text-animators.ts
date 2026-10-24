import {
  TEXT_ANIMATOR_PROPERTIES,
  animatorMode,
  countTextUnits,
  staggerLength,
  type TextAnimatorMode,
  type TextAnimatorOrder,
  type TextAnimatorProperty,
  type TextAnimatorUnit
} from '../../renderer/src/legacy/core/text-animation';

export { TEXT_ANIMATOR_PROPERTIES, animatorMode };
export type { TextAnimatorMode, TextAnimatorProperty };

/** Channels each selector mode owns, with their starting values. */
export const MODE_SETTINGS: Record<TextAnimatorMode, Record<string, number>> = {
  stagger: { delay: 0, duration: 0.6, stagger: 0.04 },
  range: { start: 0, end: 100, offset: 0, smoothness: 0 },
  wave: { speed: 1, spread: 8 }
};

export const EASINGS: Array<{ v: string; label: string }> = [
  { v: 'expoOut', label: 'Expo out' },
  { v: 'cubicOut', label: 'Ease out' },
  { v: 'cubicInOut', label: 'Ease in-out' },
  { v: 'cubicIn', label: 'Ease in' },
  { v: 'backOut', label: 'Back out' },
  { v: 'spring', label: 'Spring' },
  { v: 'power', label: 'Power' },
  { v: 'linear', label: 'Linear' }
];

export const ORDERS: Array<{ v: TextAnimatorOrder; label: string }> = [
  { v: 'forward', label: 'First to last' },
  { v: 'reverse', label: 'Last to first' },
  { v: 'center', label: 'Center out' },
  { v: 'edges', label: 'Edges in' },
  { v: 'random', label: 'Random' }
];

export const UNITS: Array<{ v: TextAnimatorUnit; label: string }> = [
  { v: 'characters', label: 'Characters' },
  { v: 'words', label: 'Words' },
  { v: 'lines', label: 'Lines' }
];

export const SHAPES = [
  { v: 'square', label: 'Square' },
  { v: 'rampUp', label: 'Ramp up' },
  { v: 'rampDown', label: 'Ramp down' },
  { v: 'triangle', label: 'Triangle' },
  { v: 'round', label: 'Round' }
];

export interface AnimatorPreset {
  id: string;
  label: string;
  group: 'in' | 'loop' | 'custom';
  build(size: number): {
    mode: TextAnimatorMode;
    unit: TextAnimatorUnit;
    easing?: string;
    order?: TextAnimatorOrder;
    settings?: Record<string, number>;
    props: Partial<Record<TextAnimatorProperty, number | string>>;
  };
}

const round = (value: number) => Math.round(value);

export const ANIMATOR_PRESETS: AnimatorPreset[] = [
  { id: 'fade', label: 'Fade', group: 'in', build: () => ({ mode: 'stagger', unit: 'characters', easing: 'cubicOut', settings: { duration: 0.5, stagger: 0.03 }, props: { opacity: 0 } }) },
  { id: 'rise', label: 'Rise', group: 'in', build: (size) => ({ mode: 'stagger', unit: 'characters', easing: 'expoOut', settings: { duration: 0.8, stagger: 0.03 }, props: { opacity: 0, y: round(size * 0.6) } }) },
  { id: 'rise-words', label: 'Rise by word', group: 'in', build: (size) => ({ mode: 'stagger', unit: 'words', easing: 'expoOut', settings: { duration: 0.9, stagger: 0.08 }, props: { opacity: 0, y: round(size * 0.8) } }) },
  { id: 'pop', label: 'Pop', group: 'in', build: () => ({ mode: 'stagger', unit: 'characters', easing: 'backOut', settings: { duration: 0.5, stagger: 0.035 }, props: { opacity: 0, scale: 0 } }) },
  { id: 'drop', label: 'Spring drop', group: 'in', build: (size) => ({ mode: 'stagger', unit: 'characters', easing: 'spring', settings: { duration: 1, stagger: 0.04 }, props: { opacity: 0, y: -round(size * 0.9) } }) },
  { id: 'tumble', label: 'Tumble', group: 'in', build: (size) => ({ mode: 'stagger', unit: 'characters', easing: 'backOut', settings: { duration: 0.7, stagger: 0.04 }, props: { opacity: 0, y: round(size * 0.4), rotation: -45, scale: 60 } }) },
  { id: 'blur', label: 'Blur in', group: 'in', build: (size) => ({ mode: 'stagger', unit: 'characters', easing: 'cubicOut', settings: { duration: 0.8, stagger: 0.03 }, props: { opacity: 0, blur: round(size * 0.25) } }) },
  { id: 'tracking', label: 'Tracking in', group: 'in', build: (size) => ({ mode: 'stagger', unit: 'characters', easing: 'expoOut', settings: { duration: 1.2, stagger: 0 }, props: { opacity: 0, tracking: round(size * 0.4) } }) },
  { id: 'typewriter', label: 'Typewriter', group: 'in', build: () => ({ mode: 'stagger', unit: 'characters', easing: 'linear', settings: { duration: 0, stagger: 0.05 }, props: { opacity: 0 } }) },
  { id: 'wave', label: 'Wave', group: 'loop', build: (size) => ({ mode: 'wave', unit: 'characters', settings: { speed: 1, spread: 8 }, props: { y: -round(size * 0.15) } }) },
  { id: 'pulse', label: 'Pulse', group: 'loop', build: () => ({ mode: 'wave', unit: 'characters', settings: { speed: 0.8, spread: 6 }, props: { scale: 115 } }) },
  { id: 'custom-stagger', label: 'Stagger', group: 'custom', build: () => ({ mode: 'stagger', unit: 'characters', easing: 'cubicOut', props: { opacity: 0 } }) },
  { id: 'custom-range', label: 'Range selector', group: 'custom', build: () => ({ mode: 'range', unit: 'characters', props: { opacity: 0 } }) }
];

/** A delay that lands the last unit of an Out animation on the layer's end.
 *  `lines` are the layer's wrapped lines, so paragraphs count correctly. */
export function outDelay(values: Record<string, number>, lines: Array<{ text: string }>, unit: TextAnimatorUnit, order: TextAnimatorOrder | undefined, layerDuration: number): number {
  const length = staggerLength(values, Math.max(1, countTextUnits(lines, unit)), order);
  return Math.max(0, Math.round((layerDuration - length) * 100) / 100);
}

export function makeAnimator(P: (value: any) => any, id: string, name: string, preset: AnimatorPreset, size: number) {
  const plan = preset.build(size);
  const settings = { ...MODE_SETTINGS[plan.mode], ...plan.settings };
  const channels = { amount: 100, ...settings, ...plan.props };
  return {
    id,
    name,
    enabled: true,
    mode: plan.mode,
    unit: plan.unit,
    order: plan.order ?? 'forward',
    ...(plan.mode === 'stagger' ? { direction: 'in', easing: plan.easing ?? 'cubicOut' } : {}),
    ...(plan.mode === 'range' ? { shape: 'square' } : {}),
    p: Object.fromEntries(Object.entries(channels).map(([key, value]) => [key, P(value)]))
  };
}

/** Animated properties in display order. */
export function animatorProperties(animator: any): TextAnimatorProperty[] {
  return (Object.keys(TEXT_ANIMATOR_PROPERTIES) as TextAnimatorProperty[]).filter((key) => animator.p?.[key]);
}

/** Switch selector modes, replacing the old mode's settings with the new one's. */
export function setAnimatorMode(P: (value: any) => any, animator: any, mode: TextAnimatorMode): void {
  const current = animatorMode(animator);
  if (current === mode && animator.mode) return;
  if (!animator.unit) {
    const legacy = animator.p?.unit?.v;
    animator.unit = legacy === 'words' || legacy === 'lines' ? legacy : 'characters';
  }
  delete animator.p.unit;
  for (const key of Object.keys(MODE_SETTINGS[current])) if (!(key in MODE_SETTINGS[mode])) delete animator.p[key];
  for (const [key, value] of Object.entries(MODE_SETTINGS[mode])) animator.p[key] ??= P(value);
  animator.p.amount ??= P(100);
  animator.mode = mode;
  if (mode === 'stagger') { animator.direction ??= 'in'; animator.easing ??= 'cubicOut'; }
  if (mode === 'range') animator.shape ??= 'square';
}
