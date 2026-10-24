/** Observe successful presentations rather than CPU submission time alone.
 * GPU saturation can miss most display frames while submission takes 2 ms. */
export function createPreviewQualityMonitor() {
  let previous: number | null = null, previouslyPlaying = false;
  const idleIntervals: number[] = [];
  const sortedIntervals: number[] = [];
  let displayInterval = 1000 / 60;
  let start = Infinity, frames = 0, failed = 0, uncached = 0, slowWindows = 0;
  let target = 0;
  const clearWindow = () => { frames = 0; failed = 0; uncached = 0; };
  const reset = (now: number) => {
    start = now + 750;
    clearWindow(); slowWindows = 0;
  };
  return {
    reset,
    ready(now: number) { return now >= start; },
    budget(fps: number) { return 1000 / Math.min(fps, 1000 / displayInterval); },
    tick(now: number, playing: boolean, fps: number): boolean {
      const gap = previous === null ? 0 : now - previous;
      if (!playing && !previouslyPlaying && gap >= 4 && gap <= 100) {
        if (idleIntervals.length === 30) {
          const expired = sortedIntervals.indexOf(idleIntervals.shift()!);
          for (let index = expired; index < sortedIntervals.length - 1; index++) {
            sortedIntervals[index] = sortedIntervals[index + 1]!;
          }
          sortedIntervals.length--;
        }
        idleIntervals.push(gap);
        // Keep the small window ordered as samples arrive, without allocating
        // and sorting a copy on every paused display frame.
        let index = sortedIntervals.length;
        while (index > 0 && sortedIntervals[index - 1]! > gap) {
          sortedIntervals[index] = sortedIntervals[index - 1]!;
          index--;
        }
        sortedIntervals[index] = gap;
        // Use an idle baseline: learning from saturated playback would mistake
        // its dropped frames for a slower monitor and prevent adaptation.
        if (idleIntervals.length >= 5) {
          displayInterval = sortedIntervals[Math.floor(sortedIntervals.length / 4)]!;
        }
      }
      // A genuine suspension is a fresh sample; several-hundred-ms GPU frames
      // must still qualify as overload rather than restart warmup forever.
      const changed = playing !== previouslyPlaying || fps !== target || gap > 1000 || gap < 0;
      previous = now; previouslyPlaying = playing; target = fps;
      if (changed) reset(now);
      if (!playing || now < start + 1000) return false;
      const expected = Math.min(fps, 1000 / displayInterval);
      const rate = frames * 1000 / (now - start);
      // Decoder waits, startup compilation and cached replays are not evidence
      // that lowering the render resolution will improve throughput.
      const overloaded = frames >= 3 && failed <= frames / 9
        && uncached >= frames / 2 && rate < expected * .8;
      slowWindows = overloaded ? slowWindows + 1 : 0;
      start = now; clearWindow();
      return slowWindows >= 2;
    },
    record(now: number, presented: boolean, cached = false) {
      if (now < start) return;
      if (!presented) { failed++; return; }
      frames++;
      if (!cached) uncached++;
    },
  };
}
