import { createRoot, h } from "../src";
import { useState } from "../src/hooks";
const globals = globalThis as unknown as { nativeProbe: Promise<unknown> };
globals.nativeProbe = (async () => {
  const root = createRoot(async (c) => ({
    accepted: true,
    revision: c.baseRevision + 1,
  }));
  let set!: (n: number) => void;
  function Counter() {
    const [n, s] = useState(0);
    set = s;
    return h("text", null, String(n));
  }
  await root.render(h(Counter, null));
  set(17);
  await root.flush();
  return {
    text: root.host.root.textContent,
    revision: root.inspect().revision,
    process: typeof (globalThis as { process?: unknown }).process,
    document: typeof (globalThis as { document?: unknown }).document,
  };
})();
