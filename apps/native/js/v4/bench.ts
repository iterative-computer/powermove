import { h, createRoot } from "./src";
const quantile = (samples: number[], q: number) =>
  [...samples].sort((a, b) => a - b)[Math.ceil(samples.length * q) - 1];
async function bench(widgets: number) {
  const root = createRoot(async (c) => ({
    accepted: true,
    revision: c.baseRevision + 1,
  }));
  const samples: number[] = [];
  let operations = 0;
  const tree = (revision: number) =>
    h(
      "div",
      null,
      Array.from({ length: widgets }, (_, index) => {
        const key = revision % 2 ? widgets - index - 1 : index;
        return h(
          "button",
          { key, "data-value": revision + key, onClick: () => {} },
          String(revision + key),
        );
      }),
      h("text", null),
    );
  await root.render(tree(0));
  // 1 root div + 1 empty text + 2 nodes/widget; 499 widgets = 1,000 native nodes.
  const nodes = root.host.nodes.size - 1;
  for (let i = 1; i <= 130; i++) {
    const start = performance.now();
    await root.render(tree(i));
    if (i > 30) samples.push(performance.now() - start);
  }
  const build = await Bun.build({
    entrypoints: ["./src/index.ts"],
    target: "browser",
    minify: true,
  });
  if (!build.success) throw Error(build.logs.join("\n"));
  return {
    widgets,
    nativeNodes: nodes,
    samples: samples.length,
    p50Ms: quantile(samples, 0.5),
    p95Ms: quantile(samples, 0.95),
    provisionalBudgetMs: 4,
    provisionalPass: quantile(samples, 0.95) <= 4,
    runtimeBundleBytes: build.outputs[0].size,
    scope:
      "JS reconciliation, op generation and inspector snapshot; synthetic immediate ack; excludes Rust, IPC, layout, paint, presentation",
  };
}
console.log(
  JSON.stringify(
    { runtime: Bun.version, results: [await bench(499), await bench(1000)] },
    null,
    2,
  ),
);
