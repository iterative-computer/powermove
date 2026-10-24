/*
 * The engine's audio health (issue A5) for the status bar and the
 * performance panel, from `RenderStats.audio` (pm-node's `AudioStatus`).
 *
 * The engine sends it whenever it changes, paused or playing: a missing
 * source a voice dropped to silence after the bounded wait (A3), media
 * that did not resolve, mix errors, device underruns and overloads, and an
 * output device that did not open. Counters run since the session's audio
 * started. Today's app plays fully decoded WebAudio buffers and has none of
 * these, so with the new engine off nothing shows.
 *
 * Starved renders alone stay in the performance panel: a held segment that
 * recovered within the bounded wait plays bit-exact and is not heard.
 */
import type { AudioDiagnostics } from '../../../shared/engine-protocol';

export type AudioHealth = {
  error: string;
  problems: string[];
  starvedRenders: number;
  underruns: number;
  deviceOverloads: number;
};

/** The health from a stats message's diagnostics (null: no engine audio). */
export function audioHealth(diagnostics: AudioDiagnostics | null | undefined): AudioHealth | null {
  if (!diagnostics) return null;
  return {
    error: diagnostics.error ?? '',
    problems: [...(diagnostics.problems ?? [])],
    starvedRenders: Number(diagnostics.starvedRenders) || 0,
    underruns: Number(diagnostics.underruns) || 0,
    deviceOverloads: Number(diagnostics.deviceOverloads) || 0
  };
}

export function sameAudioHealth(a: AudioHealth | null, b: AudioHealth | null): boolean {
  if (!a || !b) return a === b;
  return a.error === b.error
    && a.starvedRenders === b.starvedRenders
    && a.underruns === b.underruns
    && a.deviceOverloads === b.deviceOverloads
    && a.problems.length === b.problems.length
    && a.problems.every((problem, i) => problem === b.problems[i]);
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** What the device lost, as a sentence ('' when nothing was). */
function dropoutSentence(health: AudioHealth): string {
  const parts: string[] = [];
  if (health.underruns) parts.push(plural(health.underruns, 'buffer underrun', 'buffer underruns'));
  if (health.deviceOverloads) parts.push(plural(health.deviceOverloads, 'device overload', 'device overloads'));
  return parts.length ? `${parts.join(' and ')} since audio started.` : '';
}

/**
 * The status bar field: text and its tooltip, or null when playback is
 * healthy. An unopened device comes first, then problems, then dropouts.
 */
export function audioStatusField(health: AudioHealth | null): { text: string; title: string } | null {
  if (!health) return null;
  const dropouts = health.underruns + health.deviceOverloads;
  const details = [...health.problems, dropoutSentence(health)].filter(Boolean);
  if (health.error) return { text: 'No audio output', title: [health.error, ...details].join('\n') };
  if (health.problems.length) {
    return { text: `Audio · ${plural(health.problems.length, 'problem', 'problems')}`, title: details.join('\n') };
  }
  if (dropouts) return { text: `Audio · ${plural(dropouts, 'dropout', 'dropouts')}`, title: details.join('\n') };
  return null;
}

/** A performance panel row: label, value, and the tooltip explaining it. */
export type AudioPerfRow = { label: string; value: string | number; title: string };

/** The performance panel's audio rows (none without engine audio). */
export function audioPerfRows(health: AudioHealth | null): AudioPerfRow[] {
  if (!health) return [];
  const rows: AudioPerfRow[] = [];
  if (health.error) rows.push({ label: 'Audio output', value: 'Unavailable', title: health.error });
  rows.push(
    { label: 'Underruns', value: health.underruns, title: 'Audio buffers the device played before the mix filled them' },
    { label: 'Overloads', value: health.deviceOverloads, title: 'Audio device cycles that missed their real-time deadline' },
    { label: 'Audio waits', value: health.starvedRenders, title: 'Audio segments that waited for a source to decode' }
  );
  for (const problem of health.problems) rows.push({ label: 'Audio issue', value: problem, title: problem });
  return rows;
}
