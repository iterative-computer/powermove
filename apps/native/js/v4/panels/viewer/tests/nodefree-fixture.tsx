import { createRoot } from "../../../src";
import { ViewerPanel } from "../panel";
import type { ViewerSnapshot } from "../contract";
// Synthetic no-project state; the production bundle contains no test machinery.
const state: ViewerSnapshot = {
  model: null,
  host: {
    presentation: "Docked",
    frame: { resource: null, loading: false, failed: false },
    source: null,
    interaction: "idle",
  },
};
(globalThis as Record<string, unknown>).viewerProbe = (async () => {
  const root = createRoot(async (c) => ({
    accepted: true,
    revision: c.baseRevision + 1,
  }));
  await root.render(
    <ViewerPanel
      state={state}
      actions={{ drive() {}, open() {}, command() {} }}
    />,
  );
  const result = {
    revision: root.inspect().revision,
    process: typeof process,
    document: typeof document,
    window: typeof window,
    buffer: typeof Buffer,
  };
  await root.dispose();
  return result;
})();
