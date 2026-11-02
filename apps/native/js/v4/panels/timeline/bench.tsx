/** Headless accepted v4 operation-tree timings. No production model fallback. */
import { createRoot } from "../../src";
import { TimelinePanel } from "./panel";
import { frame, layer } from "./test/fixtures";
const rows = Array.from({ length: 90 }, (_, i) =>
  layer(`layer-${30000 + i}`, 30000 + i),
);
let maxOps = 0;
let measuring = false;
let maxFrameOps = 0;
const root = createRoot(async (commit) => {
  maxOps = Math.max(maxOps, commit.ops.length);
  if (measuring) maxFrameOps = Math.max(maxFrameOps, commit.ops.length);
  return { accepted: true, revision: commit.baseRevision + 1 };
});
const base = frame({
  window: { first: 30000, total: 60000, rows },
  indexDigits: 5,
});
await root.render(<TimelinePanel frame={base} send={() => {}} />);
const nodes = root.inspect().nodes.length;
const samples: number[] = [];
// Stable callback keeps row memoisation identical to a connected session.
const send = () => {};
for (let i = 0; i < 1320; i++) {
  measuring = i >= 120;
  const start = performance.now();
  await root.render(
    <TimelinePanel
      frame={{
        ...base,
        playing: true,
        timecode: `00:00:04:${String(i % 30).padStart(2, "0")}`,
        spokenTime: `${i} frames`,
      }}
      send={send}
    />,
  );
  if (i >= 120) samples.push((performance.now() - start) * 1000);
}
samples.sort((a, b) => a - b);
console.log(
  JSON.stringify({
    logicalLayers: 60000,
    mountedRows: rows.length,
    treeNodes: nodes,
    initialMaxCommitOps: maxOps,
    maxFrameCommitOps: maxFrameOps,
    samples: samples.length,
    frameP95Us: samples[Math.floor(samples.length * 0.95)],
    frameP99Us: samples[Math.floor(samples.length * 0.99)],
    budgetUs: 1e6 / 120,
  }),
);
await root.dispose();
