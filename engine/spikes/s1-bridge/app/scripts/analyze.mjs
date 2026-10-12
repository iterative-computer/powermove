// Analyzes one S1 run (raw JSON written by main.cjs plus its Chromium trace) and
// prints a JSON summary. Usage: node analyze.mjs <run.json> [--out summary.json]
//
// Presentation times come from Chromium's own frame reporting: each compositor frame
// of the viewer's renderer emits a PipelineReporter trace event that begins at the
// BeginFrame time (the rAF timestamp) and ends at the frame's presentation time
// (viz presentation feedback). A drawn engine frame is matched to the
// PipelineReporter whose begin equals the rAF timestamp it was drawn in.
//
// Clocks: viewer times are performance.now() ms; any 's1-' user-timing mark
// gives the trace timestamp of performance.now() = 0. Engine and main times are
// CLOCK_UPTIME_RAW us; the viewer's ping-based offset maps them to performance.now().
// The analysis reports the disagreement between the two mappings.

import fs from 'node:fs';

const file = process.argv[2];
const outIdx = process.argv.indexOf('--out');
const raw = JSON.parse(fs.readFileSync(file, 'utf8'));

const pct = (xs, p) => {
  if (!xs.length) return null;
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor((s.length - 1) * p + 1e-9))];
};
const dist = (xs, digits = 2) => {
  if (!xs.length) return { n: 0 };
  const r = v => (v == null ? null : +v.toFixed(digits));
  return { n: xs.length, p50: r(pct(xs, 0.5)), p95: r(pct(xs, 0.95)), p99: r(pct(xs, 0.99)), max: r(pct(xs, 1)), min: r(pct(xs, 0)) };
};

function loadTrace(p) {
  const d = JSON.parse(fs.readFileSync(p, 'utf8'));
  return Array.isArray(d) ? d : d.traceEvents;
}

/** Trace context for the viewer renderer: clock mapping and frame presentations. */
function traceContext(events, clockOffsetUs) {
  const origin = events.find(e => typeof e.name === 'string' && e.name.startsWith('s1-') && e.cat === 'blink.user_timing' && e.args?.data?.startTime != null);
  if (!origin) throw new Error('no s1- user-timing mark in the trace');
  const pid = origin.pid;
  const traceOffsetUs = origin.ts - origin.args.data.startTime * 1000;
  const reporters = events.filter(e => e.name === 'PipelineReporter' && e.pid === pid).sort((a, b) => a.ts - b.ts);
  const open = new Map();
  const frames = [];
  for (const e of reporters) {
    const k = e.id2?.local ?? e.id;
    if (e.ph === 'b') {
      if (!open.has(k)) open.set(k, []);
      open.get(k).push(e);
    } else if (e.ph === 'e') {
      const b = open.get(k)?.shift();
      if (b) frames.push({ begin: b.ts, end: e.ts, state: b.args?.frame_reporter?.state ?? b.args?.chrome_frame_reporter?.state, seq: b.args?.frame_reporter?.frame_sequence });
    }
  }
  frames.sort((a, b) => a.begin - b.begin);
  const begins = frames.map(f => f.begin);
  // performance.now() ms -> trace us
  const toTrace = ms => traceOffsetUs + ms * 1000;
  // CLOCK_UPTIME_RAW us -> trace us (via the viewer's ping offset)
  const uptimeToTrace = us => us - clockOffsetUs + traceOffsetUs;
  const frameAt = rafMs => {
    const t = toTrace(rafMs);
    let lo = 0;
    let hi = begins.length - 1;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (begins[mid] < t) lo = mid + 1;
      else hi = mid;
    }
    let best = null;
    for (const i of [lo - 1, lo, lo + 1]) {
      const f = frames[i];
      if (f && Math.abs(f.begin - t) <= 400 && (!best || Math.abs(f.begin - t) < Math.abs(best.begin - t))) best = f;
    }
    return best;
  };
  return { pid, traceOffsetUs, frames, toTrace, uptimeToTrace, frameAt, clockDisagreementUs: clockOffsetUs - traceOffsetUs };
}

const PRESENTED = new Set(['STATE_PRESENTED_ALL', 'STATE_PRESENTED_PARTIAL']);

function analyzePlayback(raw, events) {
  const v = raw.run.viewer;
  const clock = v.clockAfter ?? v.clock;
  const ctx = traceContext(events, clock.offsetUs);
  const periodUs = v.periodUs;
  const engineLog = raw.engineEnd.log;
  const durUs = engineLog.find(f => !f.gesture)?.dur ?? 16667;
  const startMs = v.marks.find(m => m[0] === 's1-playback-start')?.[1];
  const endMs = v.marks.find(m => m[0] === 's1-playback-end')?.[1];
  const winStartUs = startMs * 1000 + clock.offsetUs;
  const winEndUs = endMs * 1000 + clock.offsetUs;

  // Every content frame whose display interval lies inside the recording window.
  const byK = new Map(engineLog.filter(f => !f.gesture).map(f => [f.k, f]));
  const ks = [...byK.keys()].sort((a, b) => a - b);
  const kFirst = ks.find(k => byK.get(k).pts >= winStartUs + 100_000);
  const kLast = [...ks].reverse().find(k => byK.get(k).pts + durUs <= winEndUs - 100_000);
  const drawsBySeq = new Map(v.draws.map(d => [d[1], d]));
  const rafs = v.rafTimes;
  const perFrame = [];
  const counts = { expected: 0, engineSkipped: 0, notDrawn: 0, drawnLate: 0, notPresented: 0, offSlot: 0 };
  for (let k = kFirst; k <= kLast; k++) {
    counts.expected += 1;
    const f = byK.get(k);
    if (!f) {
      counts.engineSkipped += 1;
      perFrame.push({ k, status: 'engine-skipped' });
      continue;
    }
    const d = drawsBySeq.get(f.seq);
    if (!d) {
      counts.notDrawn += 1;
      perFrame.push({ k, seq: f.seq, status: 'not-drawn' });
      continue;
    }
    const rafMs = d[0];
    // The first vsync at or after the frame's pts: where it should have been drawn.
    const ptsMs = (f.pts - clock.offsetUs) / 1000;
    const dueRaf = rafs.find(t => t >= ptsMs - 0.05);
    const late = dueRaf != null && rafMs > dueRaf + 0.5;
    if (late) counts.drawnLate += 1;
    const pf = ctx.frameAt(rafMs);
    if (!pf || !PRESENTED.has(pf.state)) {
      counts.notPresented += 1;
      perFrame.push({ k, seq: f.seq, status: 'not-presented', state: pf?.state ?? 'no-reporter' });
      continue;
    }
    perFrame.push({ k, seq: f.seq, status: late ? 'late' : 'ok', presentUs: pf.end, ptsTraceUs: ctx.uptimeToTrace(f.pts), latencyUs: pf.end - ctx.toTrace(rafMs), gpuDoneToPresentUs: pf.end - ctx.uptimeToTrace(f.gpu_done), rafMinusPtsMs: rafMs - ptsMs, frameSeq: pf.seq });
  }
  const presented = perFrame.filter(p => p.presentUs != null);
  const offsets = presented.map(p => p.presentUs - p.ptsTraceUs);
  const med = pct(offsets, 0.5);
  const jitterMs = presented.map(p => (p.presentUs - p.ptsTraceUs - med) / 1000);
  for (const p of presented) if (Math.abs(p.presentUs - p.ptsTraceUs - med) > periodUs / 2) counts.offSlot += 1;
  const intervalsMs = [];
  // Cadence: each presentation interval between consecutive content frames minus the
  // content frame duration. A frame shown one vsync early or late makes two of these
  // ±1 vsync; a stretch of frames that all present one vsync earlier (a shorter
  // compositor pipeline) makes one at each end and none inside.
  const intervalErrMs = [];
  for (let i = 1; i < perFrame.length; i++) {
    const a = perFrame[i - 1];
    const b = perFrame[i];
    if (a.presentUs != null && b.presentUs != null) {
      intervalsMs.push((b.presentUs - a.presentUs) / 1000);
      intervalErrMs.push((b.presentUs - a.presentUs - durUs) / 1000);
    }
  }
  const irregularIntervals = intervalErrMs.filter(e => Math.abs(e) > periodUs / 2000).length;
  // Pipeline depth: rAF -> presentation in whole display periods, per presented frame.
  const depth = {};
  for (const p of presented) {
    const d = Math.round(p.latencyUs / periodUs);
    depth[d] = (depth[d] ?? 0) + 1;
  }
  // Attribute every irregular frame (off its vsync slot, late, or missing) to main-load
  // events whose main-thread span started up to 300 ms before the frame's pts and
  // ended no earlier than 50 ms before it.
  const loadEvents = raw.run.load?.events ?? [];
  const attribution = {};
  const irregular = [];
  for (const p of perFrame) {
    const f = byK.get(p.k);
    const off = p.presentUs != null && Math.abs(p.presentUs - p.ptsTraceUs - med) > periodUs / 2;
    if (p.status === 'ok' && !off) continue;
    const pts = f ? f.pts : null;
    const near = pts == null ? [] : loadEvents.filter(([, u, d]) => u <= pts && u >= pts - 300_000 && u + d * 1000 >= pts - 50_000).map(e => e[0]);
    const key = near.length ? [...new Set(near)].sort().join('+') : 'none';
    attribution[key] = (attribution[key] ?? 0) + 1;
    if (irregular.length < 40)
      irregular.push({
        k: p.k,
        status: off && p.status === 'ok' ? 'off-slot' : p.status,
        offsetMs: p.presentUs != null ? +((p.presentUs - p.ptsTraceUs - med) / 1000).toFixed(2) : null,
        // When it was drawn relative to its pts, and its rAF -> presentation latency:
        // tells a wrong pick (drawn a vsync early) from a shorter compositor pipeline.
        rafMinusPtsMs: p.rafMinusPtsMs != null ? +p.rafMinusPtsMs.toFixed(2) : null,
        rafToPresentMs: p.latencyUs != null ? +(p.latencyUs / 1000).toFixed(2) : null,
        near: key
      });
  }
  const drops = counts.engineSkipped + counts.notDrawn + counts.notPresented + perFrame.filter(p => p.status === 'late').length;
  // Where frames were when they arrived: lead time from viewer arrival to pts.
  const arrivalLeadMs = v.arrivals.filter(a => !(a[7] & 1)).map(a => (a[1] - clock.offsetUs) / 1000 - a[6]);
  const mainDelayMs = v.arrivals.map(a => (a[4] - a[3]) / 1000);
  const bridgeToViewerMs = v.arrivals.map(a => a[6] - (a[3] - clock.offsetUs) / 1000);
  const deliverMs = v.arrivals.map(a => a[6] - a[5]);
  const rafIntervals = [];
  for (let i = 1; i < rafs.length; i++) rafIntervals.push(rafs[i] - rafs[i - 1]);
  return {
    window: { seconds: +((winEndUs - winStartUs) / 1e6).toFixed(2), contentFrameUs: durUs, displayPeriodUs: +periodUs.toFixed(1) },
    clock: { pingOffsetVsTraceUs: +ctx.clockDisagreementUs.toFixed(1), pingRttUs: clock.rttUs },
    frames: { ...counts, drops, presented: presented.length },
    jitterAbsMs: dist(jitterMs.map(Math.abs), 3),
    jitterMs: dist(jitterMs, 3),
    presentIntervalMs: dist(intervalsMs, 3),
    intervalJitterAbsMs: dist(intervalErrMs.map(Math.abs), 3),
    irregularIntervals,
    pipelineDepthVsyncs: depth,
    rafIntervalMs: dist(rafIntervals, 3),
    rafToPresentMs: dist(presented.map(p => p.latencyUs / 1000), 2),
    gpuDoneToPresentMs: dist(presented.map(p => p.gpuDoneToPresentUs / 1000), 2),
    arrivalLeadMs: dist(arrivalLeadMs, 2),
    // Native receive -> JavaScript on the forwarding thread: main's JS for the per-frame
    // transport, the utility process's JS for the persistent transport.
    forwarderJsDelayMs: dist(mainDelayMs, 2),
    bridgeRecvToViewerMs: dist(bridgeToViewerMs, 2),
    isolatedToMainWorldMs: dist(deliverMs, 3),
    integrity: v.integrity,
    release: releaseSummary(v),
    redraws: v.redraws ?? 0,
    viewerDropReasons: v.viewerDrops.reduce((a, d) => ((a[d[2]] = (a[d[2]] ?? 0) + 1), a), {}),
    irregularByLoadEvent: attribution,
    irregular
  };
}

/** Per-frame release timing: submit -> onSubmittedWorkDone, and the per-use callback. */
function releaseSummary(v) {
  const t = v.releaseTiming ?? [];
  const done = t.filter(r => r[2] > 0);
  const cb = t.filter(r => r[3] > 0);
  const both = t.filter(r => r[2] > 0 && r[3] > 0);
  return {
    mode: v.releaseMode ?? 'gpu-done',
    frames: t.length,
    submitToGpuDoneMs: dist(done.map(r => r[2] - r[1]), 3),
    submitToCallbackMs: dist(cb.map(r => r[3] - r[1]), 3),
    callbackMinusGpuDoneMs: dist(both.map(r => r[3] - r[2]), 3),
    callbackBeforeGpuDone: both.filter(r => r[3] < r[2]).length,
    callbackMissing: t.length - cb.length
  };
}

function analyzeLatency(raw, events) {
  const out = {};
  for (const [mode, m] of Object.entries(raw.run.modes)) {
    const v = m.viewer;
    const ctx = traceContext(events, v.clock.offsetUs);
    // seq -> revision of the dispatch that carried it (coalesced events included)
    const revOf = new Map();
    for (const [rev, seqs] of v.inputs) for (const s of seqs) if (!revOf.has(s)) revOf.set(s, rev);
    const draws = v.draws.map(d => ({ rafMs: d[0], rev: d[2] })).sort((a, b) => a.rafMs - b.rafMs);
    const lat = [];
    const toInput = [];
    let missing = 0;
    let unpresented = 0;
    for (const [seq, tsUs] of m.generated) {
      const rev = revOf.get(seq);
      if (rev == null) {
        missing += 1;
        continue;
      }
      const d = draws.find(x => x.rev >= rev);
      if (!d) {
        missing += 1;
        continue;
      }
      const pf = ctx.frameAt(d.rafMs);
      if (!pf || !PRESENTED.has(pf.state)) {
        unpresented += 1;
        continue;
      }
      lat.push((pf.end - ctx.uptimeToTrace(tsUs)) / 1000);
    }
    const dispatchMs = v.inputs.map(([, , evTs, handled]) => handled - evTs);
    const genToEventMs = [];
    for (const [rev, seqs, evTs] of v.inputs) {
      const g = m.generated.find(x => x[0] === seqs[seqs.length - 1]);
      if (g) genToEventMs.push(evTs - (g[1] - v.clock.offsetUs) / 1000);
    }
    // Stage breakdown for engine frames, per drawn frame: input handled in the
    // renderer -> engine render start -> GPU done -> main's bridge thread -> main JS
    // -> preload receiver -> main world -> drawn at rAF -> presented.
    const stages = { handlerToRenderStart: [], render: [], gpuDoneToNativeRecv: [], nativeRecvToForwarderJs: [], forwarderJsToReceiver: [], receiverToMainWorld: [], mainWorldToRaf: [], rafToPresent: [] };
    const handledByRev = new Map(v.inputs.map(i => [i[0], i[3]]));
    const logBySeq = new Map((raw.engineEnd?.log ?? []).map(f => [f.seq, f]));
    const arrBySeq = new Map(v.arrivals.map(a => [a[0], a]));
    const off = v.clock.offsetUs;
    const ms = us => (us - off) / 1000;
    for (const d of v.draws) {
      const pf = ctx.frameAt(d[0]);
      if (pf && PRESENTED.has(pf.state)) stages.rafToPresent.push((pf.end - ctx.toTrace(d[0])) / 1000);
      if (d[1] < 0) continue;
      const f = logBySeq.get(d[1]);
      const a = arrBySeq.get(d[1]);
      const h = handledByRev.get(d[2]);
      if (!f || !a) continue;
      if (h != null) stages.handlerToRenderStart.push(ms(f.start) - h);
      stages.render.push((f.gpu_done - f.start) / 1000);
      stages.gpuDoneToNativeRecv.push((a[3] - a[2]) / 1000);
      stages.nativeRecvToForwarderJs.push((a[4] - a[3]) / 1000);
      stages.forwarderJsToReceiver.push(a[5] - ms(a[4]));
      stages.receiverToMainWorld.push(a[6] - a[5]);
      stages.mainWorldToRaf.push(d[0] - a[6]);
    }
    out[mode] = {
      stagesMs: Object.fromEntries(Object.entries(stages).map(([k, xs]) => [k, dist(xs, 2)])),
      events: m.generated.length,
      dispatches: v.inputs.length,
      draws: v.draws.length,
      unmatched: missing,
      unpresented,
      pointerToPixelMs: dist(lat, 2),
      generatedToEventTimestampMs: dist(genToEventMs, 2),
      eventToHandlerMs: dist(dispatchMs, 2),
      integrity: v.integrity,
      clock: { pingOffsetVsTraceUs: +ctx.clockDisagreementUs.toFixed(1) }
    };
  }
  return out;
}

const events = raw.trace ? loadTrace(raw.trace) : null;
const { out: _out, ...config } = raw.config;
const summary = {
  run: file.split('/').pop().replace(/\.json$/, ''),
  scenario: raw.config.scenario,
  config,
  versions: raw.versions,
  display: raw.display,
  machineLoad: raw.machineLoad ?? null,
  errors: raw.errors,
  bridge: raw.bridge,
  engine: raw.engine,
  engineStats: raw.engineEnd?.stats,
  imports: raw.imports,
  sendsPerImport: raw.sendsPerImport,
  receiver: raw.receiver,
  load: raw.run?.load ?? null,
  frameForwarding: (() => {
    const f = raw.frames;
    const st = f.reduce((a, r) => ((a[r[6]] = (a[r[6]] ?? 0) + 1), a), {});
    return { transport: raw.config.transport ?? 'per-frame', frames: f.length, status: { ok: st[1] ?? 0, pending: st[0] ?? 0, noTarget: st[2] ?? 0, failed: st[3] ?? 0, engineToViewerDirect: st[4] ?? 0 }, transferMs: dist(f.filter(r => r[6] === 1).map(r => (r[5] - r[4]) / 1000), 2) };
  })()
};
if (raw.config.scenario === 'playback') summary.playback = analyzePlayback(raw, events);
if (raw.config.scenario === 'latency') summary.latency = analyzeLatency(raw, events);
if (raw.config.scenario === 'quality') summary.quality = raw.run;

const text = JSON.stringify(summary, null, 2);
if (outIdx > 0) fs.writeFileSync(process.argv[outIdx + 1], text + '\n');
else process.stdout.write(text + '\n');
