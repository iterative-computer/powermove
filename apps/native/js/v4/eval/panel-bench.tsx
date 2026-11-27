import { createRoot, type ComponentChild } from "powermove-v4";
import { TimelineRows, type LayerRow } from "../examples/panels/timeline";
import { InspectorGroup } from "../examples/panels/inspector";
import { EffectsBrowser } from "../examples/panels/effects";
const quantile = (samples: number[], q: number) =>
  [...samples].sort((a, b) => a - b)[Math.ceil(samples.length * q) - 1];
async function measure(
  name: string,
  view: (revision: number) => ComponentChild,
) {
  let ops = 0;
  const root = createRoot(async (commit) => {
    ops += commit.ops.length;
    return { accepted: true, revision: commit.baseRevision + 1 };
  });
  try {
    await root.render(view(0));
    const nativeNodes = root.inspect().nodes.length - 1;
    const samples: number[] = [];
    for (let revision = 1; revision <= 130; revision++) {
      const start = performance.now();
      await root.render(view(revision));
      if (revision > 30) samples.push(performance.now() - start);
    }
    return {
      name,
      nativeNodes,
      samples: samples.length,
      p50Ms: quantile(samples, 0.5),
      p95Ms: quantile(samples, 0.95),
      ops,
    };
  } finally {
    await root.dispose();
  }
}
const rows: LayerRow[] = Array.from({ length: 10000 }, (_, i) => ({
  id: `layer-${i}`,
  label: `Layer ${i}`,
  parent: null,
  group: false,
  collapsed: false,
  locked: false,
}));
const effects = Array.from({ length: 1000 }, (_, i) => ({
  id: `effect-${i}`,
  label: `Effect ${i}`,
  group: "Synthetic",
}));
const noop = () => {};
console.log(
  JSON.stringify(
    {
      runtime: Bun.version,
      warmups: 30,
      scope:
        "JS only; synthetic immediate async ack; no Rust, kernel, IPC, layout, paint or presentation; selected components/data only",
      results: [
        await measure("timeline-10000-window32", (revision) => (
          <TimelineRows
            rows={revision % 2 ? [...rows].reverse() : rows}
            selected={[rows[revision % rows.length].id]}
            anchor={null}
            start={0}
            size={32}
            onSelect={noop}
            onCollapse={noop}
          />
        )),
        await measure("inspector-50-properties", (revision) => (
          <InspectorGroup
            title="Transform"
            time={revision / 24}
            properties={Array.from({ length: 50 }, (_, i) => ({
              id: `property-${i}`,
              label: `Property ${i}`,
              values: [revision / 100, revision / 100],
              binding: `synthetic:${i}`,
              editable: true,
              animated: true,
              keyAtPlayhead: false,
              generic: false,
            }))}
            onKey={noop}
            onSelectChannel={noop}
          />
        )),
        await measure("effects-1000-items", (revision) => (
          <EffectsBrowser
            items={revision % 2 ? [...effects].reverse() : effects}
            category="all"
            recent={[]}
            selectedLayer="synthetic:layer"
            onPickCategory={noop}
            onApply={noop}
            onMissingSelection={noop}
          />
        )),
      ],
    },
    null,
    2,
  ),
);
