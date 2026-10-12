// Prints the S1 gate numbers from every summary in results/ as Markdown tables.
//   node app/scripts/table.mjs
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const results = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../results');
const runs = fs
  .readdirSync(results)
  .filter(f => f.endsWith('.json'))
  .map(f => JSON.parse(fs.readFileSync(path.join(results, f), 'utf8')));

const f = (v, d = 2) => (v == null ? 'n/a' : (+v).toFixed(d));
const row = cells => `| ${cells.join(' | ')} |`;

const playback = runs.filter(r => r.scenario === 'playback');
console.log('### Playback\n');
console.log(row(['Run', 'Transport', 'Draw', 'Format', 'Load', 'Seconds', 'Expected', 'Drops', 'Off-slot', 'Jitter p50', 'Jitter p99', 'Jitter max', 'Interval jitter p99 / max', 'Irregular intervals', 'Pipeline depth (vsyncs: frames)', 'rAF to present p50 / p99', 'Forwarder JS p99 / max', 'Integrity']));
console.log(row(Array(18).fill('---')));
for (const r of playback) {
  const p = r.playback;
  const c = r.config;
  const load = !c.load ? 'none' : c.load.historyEntries ? 'heavy' : 'gate';
  console.log(
    row([
      r.run,
      c.transport ?? 'per-frame',
      c.drawPath + (c.redraw === 'every-vsync' ? '+redraw' : ''),
      c.format,
      load + (c.onscreen ? ', on screen' : ''),
      p.window.seconds,
      p.frames.expected,
      p.frames.drops,
      p.frames.offSlot,
      f(p.jitterAbsMs.p50, 3),
      f(p.jitterAbsMs.p99, 3),
      f(p.jitterAbsMs.max, 3),
      `${f(p.intervalJitterAbsMs?.p99, 3)} / ${f(p.intervalJitterAbsMs?.max, 3)}`,
      p.irregularIntervals ?? 'n/a',
      Object.entries(p.pipelineDepthVsyncs ?? {}).map(([k, v]) => `${k}: ${v}`).join(', '),
      `${f(p.rafToPresentMs.p50)} / ${f(p.rafToPresentMs.p99)}`,
      `${f(p.forwarderJsDelayMs.p99)} / ${f(p.forwarderJsDelayMs.max)}`,
      p.integrity.checked ? `${p.integrity.ok}/${p.integrity.checked}` : 'control'
    ])
  );
}

console.log('\n### Load applied\n');
console.log(row(['Run', 'Document MB', 'Long tasks', 'Autosaves (main ms p50 / max)', 'Agent requests / replies (MB)', 'Resizes', 'loadavg before']));
console.log(row(Array(7).fill('---')));
for (const r of runs.filter(x => x.load)) {
  const l = r.load;
  console.log(
    row([
      r.run,
      f(l.documentBytes / 1e6, 1),
      l.longTasks.n,
      `${l.autosaves.n} (${f(l.autosaves.mainThreadMs?.p50, 0)} / ${f(l.autosaves.mainThreadMs?.max, 0)})`,
      `${l.agent.requests} / ${l.agent.replies} (${f(l.agent.replyBytes / 1e6, 1)})`,
      l.resizes.n,
      (r.machineLoad?.before?.loadavg ?? []).join(', ')
    ])
  );
}

const latency = runs.filter(r => r.scenario === 'latency');
console.log('\n### Pointer-to-pixel (ms)\n');
console.log(row(['Run', 'Mode', 'Events', 'p50', 'p95', 'p99', 'max', 'Unpresented', 'Engine render p50', 'rAF to present p50']));
console.log(row(Array(10).fill('---')));
for (const r of latency) {
  for (const [mode, m] of Object.entries(r.latency)) {
    const p = m.pointerToPixelMs;
    console.log(row([r.run, mode, m.events, f(p.p50), f(p.p95), f(p.p99), f(p.max), m.unpresented + m.unmatched, f(m.stagesMs.render.p50), f(m.stagesMs.rafToPresent.p50)]));
  }
}

for (const r of runs.filter(x => x.scenario === 'quality')) {
  console.log(`\n### Quality: ${r.run}\n`);
  console.log('```json');
  const q = r.quality;
  console.log(
    JSON.stringify(
      {
        frameColorSpace: q.viewer.colorSpace,
        copy: { dithered: q.viewer.copy.dithered, undithered: q.viewer.copy.undithered, patches: q.viewer.copy.patches },
        external: { dithered: q.viewer.external.dithered, undithered: q.viewer.external.undithered, patches: q.viewer.external.patches },
        canvasExtended: q.viewer.canvasExtended,
        capture: q.capture,
        edr: { before: q.edrBefore, during: q.edrDuring, onscreen: q.edrOnscreen }
      },
      null,
      1
    )
  );
  console.log('```');
}
