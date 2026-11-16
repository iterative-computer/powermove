import { expect, test } from "bun:test";
import { runInNewContext } from "node:vm";
import { RemoteDOM } from "../../../js/remote-dom/src/shim";
const source = await Bun.file(
  new URL("../../compat-runner/src/store-bootstrap.js", import.meta.url),
).text();

test("sandbox contribution roots carry their manifest style mode on the DOM wire", () => {
  for (const mode of ["full", "tokens-only", "none"]) {
    const batches: any[] = [];
    const context: any = {
      setTimeout,
      clearTimeout,
      performance,
      runtime: {
        engine(operation: string, value: any) {
          if (operation === "store.dom") batches.push(value.batch);
          return {};
        },
      },
      StoreCompat: {
        RemoteDOM,
        runtimeModules: {},
        sandboxReporter: () => () => {},
        createSandboxAPI: () => ({}),
        sandboxControl: () => ({
          panel: () => ({
            build(root: any) {
              root.textContent = "Sandbox panel";
            },
          }),
        }),
      },
    };
    runInNewContext(source, context);
    context.__storeCheck.create({ manifest: { nativeStyle: mode } });
    context.__storeCheck.openPanel("synthetic-panel", "synthetic-panel");
    const ops = batches.flatMap((b) => b.ops);
    const styles = ops.filter((op) => op.name === "data-pm-style");
    if (mode === "full") expect(styles).toHaveLength(0);
    else expect(styles.some((op) => op.value === mode)).toBe(true);
    context.__storeCheck.dom.dispose();
  }
});
