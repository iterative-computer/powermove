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
        const {run, args} = reply.callback;
        try { return await rpc.invokeHandle(run, ...args); }
        catch (error) {
          if (!closed) await call("runtime-error", {name:error.name,message:error.message});
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
  dispose() {
    if (this.disposed) return true;
    this.disposed = true;
    this.control.dispose();
    this.rpc.close();
    return true;
  },
};
