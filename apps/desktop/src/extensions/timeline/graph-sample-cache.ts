/** Curves describe fixed composition times. Playback changes the playhead,
 * not these samples; edits, viewport changes and graph modes start a new cache. */
export function createGraphSampleCache(
  evaluate: (axis: any, time: number, speed: boolean) => number,
  limit = 8192,
) {
  let project: any, generation = '', count = 0;
  let tracks = new WeakMap<object, { layer: any; key: string; from: number; values: Map<number, number> }>();
  return {
    begin(nextProject: any, nextGeneration: string) {
      if (project === nextProject && generation === nextGeneration) return;
      project = nextProject; generation = nextGeneration; count = 0; tracks = new WeakMap();
    },
    sample(axis: any, time: number, speed: boolean): number {
      let track = tracks.get(axis.prop);
      if (!track || track.layer !== axis.L || track.key !== axis.key || track.from !== axis.L.from) {
        count -= track?.values.size || 0;
        track = { layer: axis.L, key: axis.key, from: axis.L.from, values: new Map() };
        tracks.set(axis.prop, track);
      }
      const cached = track.values.get(time);
      if (cached !== undefined) return cached;
      const value = evaluate(axis, time, speed);
      if (count < limit) { track.values.set(time, value); count++; }
      return value;
    },
  };
}
