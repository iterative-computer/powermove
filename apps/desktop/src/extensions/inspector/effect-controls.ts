import { getContext, setContext } from 'svelte';

const EFFECT_CONTROLS = Symbol('powermove.effect-controls');

/** Rows inside Effect Controls draw After Effects' stopwatch, plain number
    fields and checkboxes instead of the Properties panel's diamonds,
    slider wells and On/Off switches. */
export function provideEffectControls(): void {
  setContext(EFFECT_CONTROLS, true);
}

export function inEffectControls(): boolean {
  return getContext<boolean | undefined>(EFFECT_CONTROLS) === true;
}
