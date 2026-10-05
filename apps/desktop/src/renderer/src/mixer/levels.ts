/*
 * Mixer arithmetic: gain ↔ decibels, the fader taper, the meter scale and
 * meter ballistics. Pure on purpose — the panel, the audio engine and the
 * tests all agree on one set of numbers.
 */

/** The model's linear gain ceiling (audio `gain`, `audioGain`, master). */
export const MAX_GAIN = 4;
/** Top of the fader travel. 20·log10(4) ≈ 12.04 dB; the fader stops at +12. */
export const FADER_MAX_DB = 12;
/** Below this the fader's last stretch runs out to silence. */
export const FADER_FLOOR_DB = -100;
/** Numerical floor for levels; anything quieter is drawn as silence. */
export const SILENCE_DB = -120;

export function clampGain(gain: number): number {
  if (!Number.isFinite(gain)) return gain === Infinity ? MAX_GAIN : 0;
  return Math.min(MAX_GAIN, Math.max(0, gain));
}

/** Linear gain → dB. Silence is -Infinity. */
export function gainToDb(gain: number): number {
  return gain > 0 ? 20 * Math.log10(gain) : -Infinity;
}

/** dB → linear gain, clamped to the model's range. -Infinity is silence. */
export function dbToGain(db: number): number {
  if (!(db > -Infinity) || db <= FADER_FLOOR_DB) return 0;
  return clampGain(Math.pow(10, db / 20));
}

/**
 * Fader taper. Positions run 0 (bottom, silence) → 1 (top, +12 dB). Unity
 * sits a little above two thirds, and each mark is close to evenly spaced, so
 * the useful range near 0 dB gets most of the travel — the console law, not a
 * straight dB line. Marks double as the rail's tick positions.
 */
export const FADER_MARKS: ReadonlyArray<readonly [db: number, position: number]> = [
  [12, 1],
  [6, 0.86],
  [0, 0.72],
  [-6, 0.58],
  [-12, 0.45],
  [-20, 0.33],
  [-30, 0.22],
  [-40, 0.13],
  [-60, 0.04]
];
const LOWEST_MARK = FADER_MARKS[FADER_MARKS.length - 1]!;

export function faderPosition(db: number): number {
  if (!(db > FADER_FLOOR_DB)) return 0;
  if (db >= FADER_MARKS[0]![0]) return 1;
  if (db < LOWEST_MARK[0]) {
    // -60 → -100 dB share the bottom sliver of travel.
    const t = (db - FADER_FLOOR_DB) / (LOWEST_MARK[0] - FADER_FLOOR_DB);
    return t * LOWEST_MARK[1];
  }
  for (let index = 1; index < FADER_MARKS.length; index++) {
    const [lowDb, lowPos] = FADER_MARKS[index]!;
    const [highDb, highPos] = FADER_MARKS[index - 1]!;
    if (db >= lowDb) return lowPos + (db - lowDb) / (highDb - lowDb) * (highPos - lowPos);
  }
  return 0;
}

export function faderDb(position: number): number {
  if (!(position > 0)) return -Infinity;
  if (position >= 1) return FADER_MARKS[0]![0];
  if (position < LOWEST_MARK[1]) {
    return FADER_FLOOR_DB + position / LOWEST_MARK[1] * (LOWEST_MARK[0] - FADER_FLOOR_DB);
  }
  for (let index = 1; index < FADER_MARKS.length; index++) {
    const [lowDb, lowPos] = FADER_MARKS[index]!;
    const [highDb, highPos] = FADER_MARKS[index - 1]!;
    if (position >= lowPos) return lowDb + (position - lowPos) / (highPos - lowPos) * (highDb - lowDb);
  }
  return -Infinity;
}

/**
 * Meter deflection: the fader's own taper, so each strip has one scale. 0 dBFS
 * sits level with the fader's unity mark, and anything louder climbs into the
 * travel above it (a hot strip feeding the master, or an over on the master).
 */
export function meterPosition(db: number): number {
  return faderPosition(db);
}

/** Labelled scale marks, dB, top to bottom: the fader's marks. */
export const METER_MARKS: readonly number[] = FADER_MARKS.map(([db]) => db);

/** A scale label the way consoles print them: "+6", "0", "6" (below unity). */
export function scaleLabel(db: number): string {
  return db > 0 ? `+${db}` : String(Math.abs(db));
}

/** Meter colour zones, dBFS: below WARN is nominal, CLIP_ZONE up is hot. */
export const METER_WARN_DB = -12;
export const METER_HOT_DB = -3;

/** Readout text for a gain in dB: "0.0", "+2.5", "−3.2", "−∞". */
export function formatDb(db: number): string {
  if (!(db > FADER_FLOOR_DB)) return '−∞';
  const rounded = Math.round(db * 10) / 10;
  if (Object.is(rounded, -0) || rounded === 0) return '0.0';
  const text = Math.abs(rounded).toFixed(1);
  return rounded > 0 ? `+${text}` : `−${text}`;
}

/** Parse what a person types into the dB readout. "-inf", "−∞" and "off" are silence. */
export function parseDb(text: string): number | null {
  const raw = text.trim().replace(/−/g, '-').replace(/\s*db$/i, '').trim();
  if (!raw) return null;
  if (/^-?(inf|infinity|∞)$/i.test(raw) || /^off$/i.test(raw)) return -Infinity;
  const value = Number(raw);
  return Number.isFinite(value) ? value : null;
}

/* ── ballistics ─────────────────────────────────────────── */

/** Peak bar and hold line fall at this rate once the signal drops. */
export const PEAK_FALL_DB_PER_S = 20;
/** The hold line waits this long before it falls. */
export const PEAK_HOLD_S = 1.5;
/** RMS integration: quick to rise, slower to settle, like a VU. */
export const RMS_ATTACK_S = 0.06;
export const RMS_RELEASE_S = 0.3;
/** A sample at or above this is a clip (≈ -0.01 dBFS). */
export const CLIP_LEVEL = 0.999;

export interface ChannelMeter {
  /** Displayed peak, dB. */
  peak: number;
  /** Displayed RMS as linear power (mean square), so the integrator is linear. */
  power: number;
  /** Peak hold line, dB. */
  hold: number;
  /** Seconds since the hold line was last pushed up. */
  holdAge: number;
  /** Latched until cleared. */
  clipped: boolean;
}

export function createChannelMeter(): ChannelMeter {
  return { peak: SILENCE_DB, power: 0, hold: SILENCE_DB, holdAge: 0, clipped: false };
}

export function resetChannelMeter(meter: ChannelMeter): void {
  meter.peak = SILENCE_DB; meter.power = 0; meter.hold = SILENCE_DB; meter.holdAge = 0; meter.clipped = false;
}

/**
 * Advance one meter channel by `dt` seconds given the raw sample peak and RMS
 * (both linear) measured over the latest window. Peaks attack instantly and
 * fall at a fixed dB rate; RMS integrates in the power domain.
 */
export function stepChannelMeter(meter: ChannelMeter, rawPeak: number, rawRms: number, dt: number): void {
  const elapsed = Math.max(0, Math.min(0.25, dt));
  const peakDb = Math.max(SILENCE_DB, gainToDb(Math.abs(rawPeak)));
  const fall = PEAK_FALL_DB_PER_S * elapsed;
  meter.peak = Math.max(peakDb, meter.peak - fall, SILENCE_DB);

  const power = rawRms * rawRms;
  const tau = power > meter.power ? RMS_ATTACK_S : RMS_RELEASE_S;
  meter.power += (power - meter.power) * (1 - Math.exp(-elapsed / tau));
  if (meter.power < 1e-12) meter.power = 0;

  if (peakDb >= meter.hold) {
    meter.hold = peakDb;
    meter.holdAge = 0;
  } else {
    meter.holdAge += elapsed;
    if (meter.holdAge > PEAK_HOLD_S) meter.hold = Math.max(SILENCE_DB, meter.hold - fall, meter.peak);
  }
  if (Math.abs(rawPeak) >= CLIP_LEVEL) meter.clipped = true;
}

/** RMS in dB for drawing. */
export function meterRmsDb(meter: ChannelMeter): number {
  return meter.power > 0 ? Math.max(SILENCE_DB, 10 * Math.log10(meter.power)) : SILENCE_DB;
}

/** True once nothing visible remains, so the draw loop may stop. */
export function meterSettled(meter: ChannelMeter): boolean {
  return meterPosition(meter.peak) <= 0 && meterPosition(meter.hold) <= 0 && meterPosition(meterRmsDb(meter)) <= 0;
}

/** Peak and RMS (linear) of one block of samples. */
export function measureBlock(samples: Float32Array): { peak: number; rms: number } {
  let peak = 0;
  let sum = 0;
  for (let index = 0; index < samples.length; index++) {
    const sample = samples[index]!;
    const magnitude = sample < 0 ? -sample : sample;
    if (magnitude > peak) peak = magnitude;
    sum += sample * sample;
  }
  return { peak, rms: samples.length ? Math.sqrt(sum / samples.length) : 0 };
}
