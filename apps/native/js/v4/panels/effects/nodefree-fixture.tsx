import { createRoot } from "../../src";
import { EffectsPanel } from "./panel";
import { EffectsModel, workerSearch } from "./model";

// Browser target, evaluated without process/Buffer/window/document in the test VM.
(globalThis as unknown as { effectsProbe: Promise<unknown> }).effectsProbe =
  (async () => {
    const model = new EffectsModel({
      search: workerSearch,
      apply: async () => {},
      reveal: async () => {},
      persist: async () => {},
      report: (error) => {
        throw error;
      },
    });
    model.setViewport(0, 8);
    model.replaceRegistry({
      items: [
        {
          id: "blur",
          label: "Gaussian Blur",
          group: "Blur & Sharpen",
          kind: "effect",
          installed: true,
        },
      ],
      groups: [],
    });
    const root = createRoot(async (commit) => ({
      accepted: true,
      revision: commit.baseRevision + 1,
    }));
    await root.render(
      <EffectsPanel
        model={model}
        bindings={{ search: "search", scope: "scope" }}
      />,
    );
    await model.setQuery("blur");
    await root.flush();
    const result = {
      title: model.inspect().title,
      rows: model.rows().length,
      nodes: root.inspect().nodes.length,
      process: typeof (globalThis as { process?: unknown }).process,
      document: typeof (globalThis as { document?: unknown }).document,
    };
    model.dispose();
    await root.dispose();
    return result;
  })();
