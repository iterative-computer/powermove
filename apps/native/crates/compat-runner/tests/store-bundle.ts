import { createSandboxAPI, sandboxControl, sandboxReporter } from "../../../js/compat/src/frozen/host/store-api";
Object.assign(globalThis, {
  StoreCompat: { createSandboxAPI, sandboxControl, sandboxReporter, runtimeModules: {} },
});
