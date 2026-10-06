/*
 * Mixer edits as typed commands. Every change goes through the edit engine
 * with origin "mixer", so it is undoable, recorded, and coalesced: a fader
 * drag or a held arrow key is one live transaction and one Undo step.
 */
import type { EditCommand } from '../core/types/commands';
import type { MixerStrip } from './strips';

export const MIXER_ORIGIN = 'mixer';

/** Target of a level edit: a layer strip, or the composition's master. */
export type LevelTarget = Pick<MixerStrip, 'id' | 'gainKey' | 'animated' | 'name'> | 'master';

export function levelLabel(target: LevelTarget): string {
  if (target === 'master') return 'Master level';
  return target.gainKey === 'gain' ? 'Gain' : 'Audio gain';
}

/**
 * The command that sets a level. A keyframed level is written as a keyframe
 * at `time` — the playhead when the gesture began — exactly as the inspector
 * edits an animated property, and never smeared across a moving playhead.
 */
export function levelCommand(target: LevelTarget, gain: number, time: number): EditCommand {
  if (target === 'master') return { type: 'set_composition', patch: { audioGain: gain } };
  if (target.animated) {
    return {
      type: 'set_property', target: target.id, path: `c.${target.gainKey}`, value: gain,
      time, mode: 'auto', preserveHandEdits: false
    } as EditCommand;
  }
  return { type: 'set_content', target: target.id, patch: { [target.gainKey]: gain } } as EditCommand;
}

/** Mute is an audio layer's switch, or a soundtrack/precomp's audio mute. Toggling it never needs an unlock, like the timeline's switch. */
export function muteCommand(strip: Pick<MixerStrip, 'id' | 'kind' | 'muted'>): EditCommand {
  if (strip.kind === 'audio') {
    return { type: 'set_layer', target: strip.id, patch: { visible: strip.muted }, overrideLock: true } as unknown as EditCommand;
  }
  return { type: 'set_content', target: strip.id, patch: { audioMuted: !strip.muted }, overrideLock: true } as EditCommand;
}

/** Start animating a level, or add a key at the playhead to an animated one. */
export function addKeyCommand(strip: Pick<MixerStrip, 'id' | 'gainKey'>, gain: number, time: number): EditCommand {
  return {
    type: 'set_property', target: strip.id, path: `c.${strip.gainKey}`, value: gain,
    time, mode: 'keyframe', preserveHandEdits: false
  } as EditCommand;
}

/** Next monitor-solo set for a click; Option-click listens to this strip alone. */
export function nextSolo(current: ReadonlySet<string>, id: string, exclusive: boolean): Set<string> {
  if (exclusive) return current.size === 1 && current.has(id) ? new Set() : new Set([id]);
  const next = new Set(current);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return next;
}
