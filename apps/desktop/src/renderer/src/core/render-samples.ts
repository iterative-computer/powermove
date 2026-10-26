/** Exact render/preparation timestamps. Negative shutters preserve sample order. */
export function motionBlurTimes(time: number, fps: number, options: { mbSamples?: number; shutter?: number } = {}): number[] {
  const count = Math.max(1, Math.min(64, Math.round(options.mbSamples || 10)));
  const shutter = (options.shutter || .5) / Math.max(1, fps);
  return Array.from({length:count},(_,index)=>time+((index+.5)/count-.5)*shutter);
}
