// Host-installed transport for the unchanged frozen asynchronous Store facade.
// No PM global or trusted extension API is installed in this helper.
globalThis.__storeCheck = {
  create(init) {
    if (this.api) throw Error("Store helper already initialized");
    let sequence = 0,
      closed = false;
    const handles = new Map();
    const call = async (method, ...args) => {
      if (closed) throw Error("Store RPC is closed");
      const reply = runtime.engine("store.rpc", { method, args });
      if (reply.callback) {
        // Pinned runKernelCommand invokes run directly. `when` belongs to
        // eligibility polling; it never gates a programmatic commands.run.
        const { run, args } = reply.callback;
        try {
          return await rpc.invokeHandle(run, ...args);
        } catch (error) {
          if (!closed)
            await call("runtime-error", {
              name: error.name,
              message: error.message,
            });
          return undefined; // The pinned host's guarded command fallback.
        }
      }
      return reply.value;
    };
    const rpc = {
      call,
      notify: (method, ...args) => {
        void call(method, ...args).catch((error) =>
          this.errors.push({ name: error.name, message: error.message }),
        );
      },
      handle(fn) {
        if (handles.size >= 2000) throw Error("Store callback handle limit");
        const id = ++sequence;
        handles.set(id, fn);
        return id;
      },
      invokeHandle: async (id, ...args) => {
        const fn = handles.get(id);
        if (!fn) throw Error("Unknown Store callback");
        return fn(...args);
      },
      release: (id) => {
        handles.delete(id);
      },
      close() {
        closed = true;
        handles.clear();
      },
    };
    this.errors = [];
    this.rpc = rpc;
    this.views = new Map();
    this.dom = new StoreCompat.RemoteDOM({
      session: { panel: "store", generation: 1 },
      realmGlobal: globalThis,
      send: (batch) => runtime.engine("store.dom", { batch }),
      requestAnimationFrame: (fn) => setTimeout(() => fn(performance.now()), 16),
      cancelAnimationFrame: (id) => clearTimeout(id),
      reportError: (error) =>
        this.errors.push({ name: error?.name, message: error?.message }),
    });
    this.dom.installGlobals(globalThis);
    if (typeof globalThis.crypto === "undefined")
      globalThis.crypto = Object.freeze({
        randomUUID: () => runtime.engine("store.uuid"),
      });
    globalThis.__powermove_runtime = StoreCompat.runtimeModules;
    StoreCompat.report = StoreCompat.sandboxReporter(rpc);
    this.api = StoreCompat.createSandboxAPI(rpc, init);
    this.control = StoreCompat.sandboxControl(this.api);
    return true;
  },
  async activate(module) {
    if (this.started)
      throw Error("Store helper requires replacement for reload");
    this.started = true;
    try {
      if (typeof module.default !== "function")
        throw Error("Missing activation export");
      const cleanup = await module.default(this.api);
      if (typeof cleanup?.dispose === "function")
        this.api.onDispose(() => cleanup.dispose());
      await this.control.ready();
      if (this.errors.length) throw Error("Store activation reported errors");
      return { ok: true };
    } catch (error) {
      this.control.dispose();
      return {
        ok: false,
        error: { name: error.name, message: error.message },
        reports: this.errors.slice(),
      };
    }
  },
  /** A panel's own document lives here; only its description reaches WebKit. */
  openPanel(id, short) {
    if (this.views.has(id)) return;
    const definition = this.control.panel(id) ?? this.control.panel(short);
    if (!definition) return;
    const root = this.dom.document.createElement("section");
    root.setAttribute("data-pm-contribution", id);
    this.dom.document.body.append(root);
    let cleanup;
    if (definition.component) {
      const svelte = StoreCompat.runtimeModules.svelte;
      const instance = svelte.mount(definition.component, {
        target: root,
        props: { api: this.api },
      });
      cleanup = () => svelte.unmount(instance);
    } else {
      const built = definition.build(root, { spec: { id, panel: id } });
      cleanup =
        typeof built === "function" ? built : built?.dispose?.bind(built);
    }
    this.views.set(id, { root, cleanup });
    this.dom.flush();
  },
  placePanel(id, rect) {
    const [x, y, w, h] = rect;
    this.views
      .get(id)
      ?.root.setAttribute(
        "style",
        `position:fixed;left:${x}px;top:${y}px;width:${w}px;height:${h}px;overflow:auto;box-sizing:border-box`,
      );
    this.dom.flush();
  },
  closePanel(id) {
    const view = this.views.get(id);
    if (!view) return;
    this.views.delete(id);
    try {
      view.cleanup?.();
    } finally {
      view.root.remove();
      this.dom.flush();
    }
  },
  /** The native compiler's graph, linked inside this confined realm. */
  async activateGraph(text) {
    const graph = JSON.parse(text);
    const url = (id) =>
      id.startsWith("runtime:") ? `pmruntime://${id.slice(8)}` : `pmext://${id}`;
    const sources = {},
      edges = {};
    for (const [id, source] of Object.entries(graph.sources))
      sources[url(id)] = source;
    const runtimeSource = (name) => {
      if (name === "svelte/internal/disclose-version") return "export {}";
      const keys = Object.keys(globalThis.__powermove_runtime[name] ?? {});
      return [
        `const m = globalThis.__powermove_runtime[${JSON.stringify(name)}];`,
        "export default m;",
        ...keys
          .filter((key) => key !== "default" && /^[A-Za-z_$][\w$]*$/.test(key))
          .map((key) => `export const ${key} = m.${key};`),
      ].join("\n");
    };
    for (const [id, table] of Object.entries(graph.edges)) {
      edges[url(id)] = {};
      for (const [specifier, target] of Object.entries(table)) {
        edges[url(id)][specifier] = url(target);
        if (target.startsWith("runtime:"))
          sources[url(target)] ??= runtimeSource(target.slice(8));
      }
    }
    const [done, namespace] = __linkModules(
      JSON.stringify({ sources, edges, entry: url("powermove:bundle") }),
    );
    await done;
    namespace.__powermoveAcquireStyles?.((css) => {
      const style = this.dom.document.createElement("style");
      style.textContent = css;
      this.dom.document.head.append(style);
    });
    return this.activate(namespace);
  },
  dispose() {
    if (this.disposed) return true;
    this.disposed = true;
    this.control.dispose();
    this.rpc.close();
    return true;
  },
};
