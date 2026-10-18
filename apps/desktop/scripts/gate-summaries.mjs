#!/usr/bin/env node
/* One line per frame-bridge harness summary, for the gate reports
   (docs/engine/phase2-gate.md): the 1-minute load average before and after,
   then the row's numbers (drag and pan pointer-to-pixel, playback drops,
   cadence, A/V sync by presentation, edits during playback, agent
   snapshots).

     node scripts/gate-summaries.mjs <summary.json>...  [--json] */
import { readFileSync } from 'node:fs';
import path from 'node:path';

const files = process.argv.slice(2).filter((a) => !a.startsWith('--'));
const asJson = process.argv.includes('--json');
const r1 = (v) => (typeof v === 'number' ? Math.round(v * 10) / 10 : v);
const pct = (d) => (d ? `${r1(d.p50)} / ${r1(d.p95)} / ${r1(d.p99)}` : '-');
const rows = [];
for (const file of files) {
  const s = JSON.parse(readFileSync(file, 'utf8'));
  const load = s.machineLoad ? `${r1(s.machineLoad.before?.[0])} → ${r1(s.machineLoad.after?.[0])}` : '-';
  const row = { run: s.run ?? path.basename(file, '.json'), load, invalid: s.invalid };
  if (s.playback) {
    const p = s.playback;
    row.frames = `${p.frames?.drops ?? '-'} of ${p.frames?.expected ?? '-'} dropped`;
    row.irregular = p.irregularIntervals;
    row.jitter = p.jitterAbsMs ? `p99 ${r1(p.jitterAbsMs.p99)}` : '-';
    if (p.avSync) row.avSync = `within ±10 ms ${p.avSync.within10Ms} of ${p.avSync.n}; |offset| p50/p95/p99 ${pct(p.avSync.absOffsetMs)}`;
    if (p.vizLags?.irregular) row.irregularBy = p.vizLags.irregular;
    if (p.edits) row.edits = p.edits;
  }
  if (s.latency) {
    row.latency = Object.fromEntries(Object.entries(s.latency).map(([mode, l]) => [mode, {
      p2p: pct(l.pointerToPixelMs),
      n: `${l.pointerToPixelMs?.n} of ${l.events}`,
      unpresented: l.unpresented,
      withDrops: pct(l.pointerToPixelWithDropsMs),
      hops: l.hopsMs ? Object.fromEntries(Object.entries(l.hopsMs).map(([k, d]) => [k, pct(d)])) : undefined
    }]));
  }
  if (s.agentSnapshots) {
    const a = s.agentSnapshots;
    row.agent = `${a.batches} batches, ${a.frames} frames, batch p50 ${a.batchMsP50} p95 ${a.batchMsP95} max ${a.batchMsMax} ms, failures ${a.failureCount}`;
  }
  rows.push(row);
}
if (asJson) process.stdout.write(`${JSON.stringify(rows, null, 2)}\n`);
else for (const row of rows) process.stdout.write(`${JSON.stringify(row)}\n`);
