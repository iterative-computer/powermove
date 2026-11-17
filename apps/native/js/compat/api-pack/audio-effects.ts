/** Audio uses the existing effect metadata spellings (k, label, def, group).
 * Numbers are in dB, Hz, seconds, or a dimensionless ratio as documented.
 * Validation is performed by the native host before a registration is visible.
 * IDs follow the visual effect identifier rules; parameter keys are lower
 * camel case. A graph has 1..32 nodes and at most 64 parameters.
 *
 * Processor bounds (the entire range of a bound parameter must fit):
 * gain: -96..24 dB; biquad: 10..20000 Hz, Q 0.1..20, -24..24 dB;
 * compressor: threshold -96..0 dB, ratio 1..100, attack 0.0001..1 s,
 * release 0.001..10 s, knee 0..24 dB, make-up 0..24 dB;
 * limiter: ceiling -96..0 dB, release 0.001..10 s;
 * delay: time 0.000125..maxTime s, maxTime <= 2 s, feedback -0.95..0.95,
 * mix 0..1. Total delay storage is limited to 768000 stereo frames per
 * effect at 192 kHz (including each ring's extra frame).
 */
export type AudioParameter =
  | {
      type: "number";
      k: string;
      label: string;
      min: number;
      max: number;
      unit: string;
      def: number;
    }
  | { type: "toggle"; k: string; label: string; def: boolean }
  | {
      type: "select";
      k: string;
      label: string;
      options: readonly string[];
      def: string;
    };
export type AudioBinding<T, K extends string = string> =
  | { value: T }
  | { param: K };
export type AudioFilter =
  | "lowPass"
  | "highPass"
  | "peaking"
  | "lowShelf"
  | "highShelf"
  | "notch";
type Key<
  P extends readonly AudioParameter[],
  T extends AudioParameter["type"],
> = Extract<P[number], { type: T }>["k"];
type NumberBinding<P extends readonly AudioParameter[]> = AudioBinding<
  number,
  Key<P, "number">
>;
export type AudioProcessor<
  P extends readonly AudioParameter[] = readonly AudioParameter[],
> =
  | { type: "gain"; gainDb: NumberBinding<P> }
  | {
      type: "biquad";
      mode: AudioBinding<AudioFilter, Key<P, "select">>;
      frequency: NumberBinding<P>;
      q: NumberBinding<P>;
      gainDb: NumberBinding<P>;
    }
  | {
      type: "compressor";
      threshold: NumberBinding<P>;
      ratio: NumberBinding<P>;
      attack: NumberBinding<P>;
      release: NumberBinding<P>;
      knee: NumberBinding<P>;
      makeup: NumberBinding<P>;
    }
  | { type: "limiter"; ceiling: NumberBinding<P>; release: NumberBinding<P> }
  | {
      type: "delay";
      time: NumberBinding<P>;
      feedback: NumberBinding<P>;
      mix: NumberBinding<P>;
      maxTime: number;
    }
  | { type: "gate"; enabled: AudioBinding<boolean, Key<P, "toggle">> };
export interface AudioEffectDefinition<
  P extends readonly AudioParameter[] = readonly AudioParameter[],
> {
  id: string;
  label: string;
  group: string;
  params: P;
  graph: readonly AudioProcessor<NoInfer<P>>[];
}
export interface AudioEffectsAPI {
  register<const P extends readonly AudioParameter[]>(
    definition: AudioEffectDefinition<P>,
  ): { dispose(): void };
  list(): AudioEffectDefinition[];
  get(id: string): AudioEffectDefinition | undefined;
}

/** Saved on audio/embedded-audio layers as audioFx. Parameters are static. */
export interface AudioEffectInstance {
  id: string;
  type: string;
  name: string;
  on: boolean;
  p: Record<string, number | boolean | string>;
}
