/*
 * One level gesture: begin → write… → commit is a single live edit
 * transaction (one Undo step); `once` is a single applied edit. Each write
 * re-tunes the playing voices so a dragged fader is heard as it moves.
 */
import type { PowermoveAPI } from '../kernel/api';
import { levelCommand, levelLabel, MIXER_ORIGIN, type LevelTarget } from './edits';

type ControlAPI = Pick<PowermoveAPI, 'edit' | 'transport' | 'ui'>;

export interface LevelControl {
  /** Start a gesture. False when another edit is already live. */
  begin(): boolean;
  write(gain: number): void;
  commit(): void;
  cancel(): void;
  once(gain: number): void;
  readonly active: boolean;
}

export interface AudioRetune {
  retune?: () => void;
}

export function levelControl(api: ControlAPI, audio: () => AudioRetune | undefined, target: () => LevelTarget): LevelControl {
  let active = false;
  let time = 0;
  let warned = false;
  const report = (result: unknown) => {
    const failure = result as { ok?: boolean; message?: string } | undefined;
    if (failure?.ok !== false || warned) return;
    warned = true;
    api.ui.toast(failure.message || 'Could not change the level');
  };
  const retune = () => audio()?.retune?.();
  return {
    get active() { return active; },
    begin() {
      if (active) return true;
      time = api.transport.time();
      warned = false;
      try { api.edit.begin(levelLabel(target()), { origin: MIXER_ORIGIN }); }
      catch (error) { console.warn('[mixer] level gesture could not start', error); return false; }
      active = true;
      return true;
    },
    write(gain) {
      if (!active) return;
      report(api.edit.dispatch(levelCommand(target(), gain, time)));
      retune();
    },
    commit() {
      if (!active) return;
      active = false;
      api.edit.commit(levelLabel(target()));
      retune();
    },
    cancel() {
      if (!active) return;
      active = false;
      api.edit.cancel();
      retune();
    },
    once(gain) {
      if (active) return;
      warned = false;
      report(api.edit.apply(levelCommand(target(), gain, api.transport.time()), { label: levelLabel(target()), origin: MIXER_ORIGIN }));
      retune();
    }
  };
}
