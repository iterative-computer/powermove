import { readFile } from 'node:fs/promises';

/** Same Graphics.Pipeline receive -> DrawAndSwap presentation feedback as S1.
 * Apply it to both canvas sinks; submission/worker prediction times are never
 * compared with one another as though they were display feedback. */
export async function compositorTiming(tracePath: string, submits: number[], originMark = 'pm-mask-origin') {
  const data = JSON.parse(await readFile(tracePath, 'utf8'));
  const events: any[] = Array.isArray(data) ? data : data.traceEvents;
  const origin = events.find(e => e.name === originMark && e.cat === 'blink.user_timing' && e.args?.data?.startTime != null);
  if (!origin) throw new Error('Missing renderer time-origin mark');
  const offset = origin.ts - origin.args.data.startTime * 1000;
  const first = (xs: any[], value: number, key = (x: any) => x): number => {
    let lo = 0, hi = xs.length;
    while (lo < hi) { const mid = (lo + hi) >>> 1; if (key(xs[mid]) < value) lo = mid + 1; else hi = mid; }
    return lo;
  };
  const receives = new Map<string, number[]>();
  for (const e of events) {
    const p = e.name === 'Graphics.Pipeline' && e.ph === 'X' ? e.args?.chrome_graphics_pipeline : null;
    if (p?.step !== 'STEP_RECEIVE_COMPOSITOR_FRAME' || !p.frame_sink_id) continue;
    const id = `${p.frame_sink_id.frame_sink_client_id}:${p.frame_sink_id.frame_sink_id}`;
    if (!receives.has(id)) receives.set(id, []);
    receives.get(id)!.push(e.ts);
  }
  for (const xs of receives.values()) xs.sort((a,b) => a-b);
  const swaps: { begin: number; end: number | null }[] = [], open = new Map();
  for (const e of events.filter(e => e.name === 'Graphics.Pipeline.DrawAndSwap').sort((a,b) => a.ts-b.ts)) {
    const id = `${e.pid}:${e.id2?.local ?? e.id}`;
    if (e.ph === 'b') { const swap = { begin: e.ts, end: null }; swaps.push(swap); open.set(id, swap); }
    else if (e.ph === 'e' && open.has(id)) { open.get(id).end = e.ts; open.delete(id); }
  }
  // The canvas's own sink receives each submission soonest: another sink
  // that commits every refresh (the page's, beside a worker canvas) matches
  // as many submissions, only later.
  let sink: string | null = null, best = Infinity;
  for (const [id,xs] of receives) {
    const lags = submits.flatMap(ms => { const t = offset + ms*1000 - 2000; const i = first(xs,t); return i < xs.length && xs[i]-t <= 12000 ? [xs[i]-t] : []; });
    if (lags.length < submits.length*.9) continue;
    const lag = lags.sort((a,b) => a-b)[lags.length >> 1]!;
    if (lag < best) { best = lag; sink = id; }
  }
  if (!sink) throw new Error(`No compositor sink matches ${submits.length} submissions (${receives.size} sinks)`);
  const xs = receives.get(sink)!;
  const presented: number[] = [];
  const missing = { received: 0, drawn: 0, superseded: 0 };
  const unmatched: {submitMs:number;reason:string}[] = [];
  for (const ms of submits) {
    const t = offset+ms*1000-2000, i = first(xs,t);
    if (i >= xs.length || xs[i]-t > 50000) { missing.received++; unmatched.push({submitMs:ms,reason:'not-received'}); continue; }
    const swap = swaps[first(swaps,xs[i],s=>s.begin)];
    if (!swap?.end) { missing.drawn++; unmatched.push({submitMs:ms,reason:'not-drawn'}); continue; }
    if (i+1 < xs.length && xs[i+1] <= swap.begin) { missing.superseded++; unmatched.push({submitMs:ms,reason:'superseded'}); continue; }
    presented.push(swap.end/1000);
  }
  const unique = [...new Set(presented)].sort((a,b)=>a-b);
  const intervals = unique.slice(1).map((t,i)=>t-unique[i]).sort((a,b)=>a-b);
  const q = (p:number) => intervals[Math.min(intervals.length-1,Math.floor(intervals.length*p))] ?? null;
  return { source:'Graphics.Pipeline.DrawAndSwap presentation feedback',sink,submitted:submits.length,presented:unique.length,missing,unmatched,duplicates:presented.length-unique.length,intervalMs:{p50:q(.5),p95:q(.95),p99:q(.99),max:q(1)} };
}
