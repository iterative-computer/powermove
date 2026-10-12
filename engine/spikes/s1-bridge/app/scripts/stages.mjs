// Breaks one playback run's presented frames down by Chromium pipeline stage, grouped
// by depth (rAF to presentation in whole display refreshes). Reads the raw run and its
// trace from results/raw/.
//   node app/scripts/stages.mjs results/raw/<run>.json
import fs from 'node:fs';
const raw = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const ev = JSON.parse(fs.readFileSync(raw.trace, 'utf8'));
const events = (Array.isArray(ev) ? ev : ev.traceEvents).filter(e => e.ph === 'b' || e.ph === 'e' || e.cat === 'blink.user_timing');
const origin = events.find(e => typeof e.name === 'string' && e.name.startsWith('s1-') && e.cat === 'blink.user_timing' && e.args?.data?.startTime != null);
const pid = origin.pid;
const period = (raw.run?.viewer?.periodUs) ?? 8333;
events.sort((a, b) => a.ts - b.ts);
const cur = new Map();
const frames = [];
for (const e of events) {
  if (e.pid !== pid || !e.id2) continue;
  const k = e.id2.local;
  if (e.name === 'PipelineReporter') {
    if (e.ph === 'b') cur.set(k, { begin: e.ts, state: e.args?.frame_reporter?.state, stages: {}, open: {} });
    else { const f = cur.get(k); if (f) { f.end = e.ts; frames.push(f); cur.delete(k); } }
    continue;
  }
  const f = cur.get(k);
  if (!f) continue;
  if (e.ph === 'b') f.open[e.name] = e.ts;
  else if (f.open[e.name] != null) f.stages[e.name] = (f.stages[e.name] ?? 0) + (e.ts - f.open[e.name]) / 1000;
}
const stat = {};
const n = {};
for (const f of frames) {
  if (!String(f.state).includes('PRESENTED')) continue;
  const d = Math.round((f.end - f.begin) / period);
  n[d] = (n[d] ?? 0) + 1;
  for (const [s, v] of Object.entries(f.stages)) ((stat[s] ??= {})[d] ??= []).push(v);
}
const med = xs => { if (!xs) return '-'; const s = [...xs].sort((a, b) => a - b); return s[s.length >> 1].toFixed(2) + ' (p90 ' + s[Math.floor(s.length * 0.9)].toFixed(2) + ')'; };
console.log('presented frames by depth', n);
for (const [s, by] of Object.entries(stat)) console.log(s.padEnd(52), Object.keys(n).map(d => `d${d}: ${med(by[d])}`).join('  '));
