/** Shipped examples use exactly the public, declarative extension API. */
import type {
  AudioEffectDefinition,
  AudioEffectsAPI,
} from "../api-pack/audio-effects";

const eqParams = [
  {
    type: "number",
    k: "lowFrequency",
    label: "Low frequency",
    min: 20,
    max: 1000,
    unit: "Hz",
    def: 100,
  },
  {
    type: "number",
    k: "lowQ",
    label: "Low Q",
    min: 0.1,
    max: 20,
    unit: "ratio",
    def: 0.707,
  },
  {
    type: "number",
    k: "lowGain",
    label: "Low gain",
    min: -24,
    max: 24,
    unit: "dB",
    def: 0,
  },
  {
    type: "number",
    k: "midFrequency",
    label: "Mid frequency",
    min: 20,
    max: 20000,
    unit: "Hz",
    def: 1000,
  },
  {
    type: "number",
    k: "midQ",
    label: "Mid Q",
    min: 0.1,
    max: 20,
    unit: "ratio",
    def: 1,
  },
  {
    type: "number",
    k: "midGain",
    label: "Mid gain",
    min: -24,
    max: 24,
    unit: "dB",
    def: 0,
  },
  {
    type: "number",
    k: "highFrequency",
    label: "High frequency",
    min: 1000,
    max: 20000,
    unit: "Hz",
    def: 8000,
  },
  {
    type: "number",
    k: "highQ",
    label: "High Q",
    min: 0.1,
    max: 20,
    unit: "ratio",
    def: 0.707,
  },
  {
    type: "number",
    k: "highGain",
    label: "High gain",
    min: -24,
    max: 24,
    unit: "dB",
    def: 0,
  },
] as const;

export const parametricEq: AudioEffectDefinition<typeof eqParams> = {
  id: "powermove-audio-effects.parametric-eq",
  label: "Parametric EQ",
  group: "Audio",
  params: eqParams,
  graph: [
    {
      type: "biquad",
      mode: { value: "lowShelf" },
      frequency: { param: "lowFrequency" },
      q: { param: "lowQ" },
      gainDb: { param: "lowGain" },
    },
    {
      type: "biquad",
      mode: { value: "peaking" },
      frequency: { param: "midFrequency" },
      q: { param: "midQ" },
      gainDb: { param: "midGain" },
    },
    {
      type: "biquad",
      mode: { value: "highShelf" },
      frequency: { param: "highFrequency" },
      q: { param: "highQ" },
      gainDb: { param: "highGain" },
    },
  ],
};

const compressorParams = [
  {
    type: "number",
    k: "threshold",
    label: "Threshold",
    min: -96,
    max: 0,
    unit: "dB",
    def: -18,
  },
  {
    type: "number",
    k: "ratio",
    label: "Ratio",
    min: 1,
    max: 100,
    unit: "ratio",
    def: 4,
  },
  {
    type: "number",
    k: "attack",
    label: "Attack",
    min: 0.0001,
    max: 1,
    unit: "s",
    def: 0.01,
  },
  {
    type: "number",
    k: "release",
    label: "Release",
    min: 0.001,
    max: 10,
    unit: "s",
    def: 0.1,
  },
  {
    type: "number",
    k: "knee",
    label: "Knee",
    min: 0,
    max: 24,
    unit: "dB",
    def: 6,
  },
  {
    type: "number",
    k: "makeup",
    label: "Make-up",
    min: 0,
    max: 24,
    unit: "dB",
    def: 0,
  },
] as const;

export const compressor: AudioEffectDefinition<typeof compressorParams> = {
  id: "powermove-audio-effects.compressor",
  label: "Compressor",
  group: "Audio",
  params: compressorParams,
  graph: [
    {
      type: "compressor",
      threshold: { param: "threshold" },
      ratio: { param: "ratio" },
      attack: { param: "attack" },
      release: { param: "release" },
      knee: { param: "knee" },
      makeup: { param: "makeup" },
    },
  ],
};

export function activateAudioEffects(api: {
  readonly audioEffects: AudioEffectsAPI;
}): void {
  api.audioEffects.register(parametricEq);
  api.audioEffects.register(compressor);
}
