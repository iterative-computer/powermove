import { mountExportPanel } from "./index";
import { fixture } from "./fixtures";
(globalThis as unknown as { exportProbe: Promise<unknown> }).exportProbe =
  (async () => {
    const f = fixture();
    const mounted = await mountExportPanel({
      host: f.host,
      subscription: "export.fixture",
      anchor: 0,
      transport: async (c) => ({
        accepted: true,
        revision: c.baseRevision + 1,
      }),
    });
    await mounted.show();
    const result = {
      nodes: mounted.root.inspect().nodes.length,
      process: typeof (globalThis as { process?: unknown }).process,
      document: typeof (globalThis as { document?: unknown }).document,
      window: typeof (globalThis as { window?: unknown }).window,
      buffer: typeof (globalThis as { Buffer?: unknown }).Buffer,
    };
    await mounted.dispose();
    return result;
  })();
