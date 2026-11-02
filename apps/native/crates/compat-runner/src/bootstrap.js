// Runs in the actual Compat V8 owner. Missing app capabilities are errors, never test doubles.
globalThis.__extcheck = {
  create(initial) {
    const call = (operation, input = {}) => runtime.engine(operation, input);
    const unavailable = (operation, input) => call(operation, input);
    this.errors = [];
    const host = {
      call: (request) => unavailable("kernel.call", request),
      query: (method, args) => unavailable("kernel.query", { method, args }),
      scheduleFlush: (fn) => queueMicrotask(fn),
      reportError: (report) => {
        const error = report?.error ?? report;
        this.errors.push({ name: error?.name, message: error?.message });
      },
      pinGeneration: (handle) => unavailable("kernel.pin", { handle }),
      unpinGeneration: (handle) => unavailable("kernel.unpin", { handle }),
    };
    const recorder = Object.fromEntries(
      [
        "create",
        "reset",
        "command",
        "read",
        "retainVersion",
        "releaseVersion",
        "release",
      ].map((method) => [
        method,
        (...args) => call("recorder." + method, { args }),
      ]),
    );
    this.dom = PowermoveCompat.RemoteDOM
      ? new PowermoveCompat.RemoteDOM({
          session: { panel: "extcheck", generation: 1 },
          realmGlobal: globalThis,
          send: (batch) => call("dom.batch", { batch }),
          requestAnimationFrame: (fn) =>
            setTimeout(() => this.realm.task(() => fn(performance.now())), 16),
          cancelAnimationFrame: (id) => clearTimeout(id),
          reportError: (error) => host.reportError(error),
        })
      : undefined;
    const capabilities = {
      panelsBackend: Object.fromEntries(
        ["open", "close", "isOpen", "isHidden", "refresh", "list"].map(
          (name) => [
            name,
            (...args) => {
              if (!this.panels)
                throw Error("Remote DOM panels are not attached");
              return this.panels[name](...args);
            },
          ],
        ),
      ),
      PM: {
        invalidate: () => {
          this.dom?.flush();
          call("presentation.invalidate");
        },
        store: {
          get: (key, fallback) => storage[key] ?? fallback,
          set: (key, value) => {
            storage[key] = value;
          },
        },
      },
    };
    this.realm = new PowermoveCompat.CompatRealm({
      id: "extcheck",
      generation: 1,
      initial,
      host,
      recorder,
      dom: this.dom,
      runtimeModules: PowermoveCompat.runtimeModules,
      capabilities,
    });
    if (this.dom)
      this.panels = new PowermoveCompat.CompatPanels(
        this.realm.kernel.panels,
        this.dom.document,
      );
    this.realm.installGlobals(globalThis);
    return true;
  },
  async activate(manifest, module) {
    if (this.extension) this.dispose();
    const handle = this.realm.extension(manifest.id, manifest.apiVersion ?? 1, {
      manifest,
    });
    this.extension = handle;
    this.errors.length = 0;
    try {
      const activate = module.default;
      if (typeof activate !== "function")
        throw Error("Missing activation export");
      handle.setActivating(true);
      if (module.__powermoveAcquireStyles)
        handle.api.onDispose(
          module.__powermoveAcquireStyles((css) => {
            if (!this.dom) throw Error("Remote DOM is not attached");
            const style = this.dom.document.createElement("style");
            style.textContent = css;
            this.dom.document.head.append(style);
            handle.api.onDispose(() => style.remove());
          }),
        );
      const cleanup = await this.realm.task(() => activate(handle.api));
      if (typeof cleanup === "function") handle.api.onDispose(cleanup);
      else if (typeof cleanup?.dispose === "function")
        handle.api.onDispose(() => cleanup.dispose());
      this.realm.flush();
      if (this.errors.length) throw Error("Activation reported runtime errors");
      return { ok: true };
    } catch (error) {
      handle.dispose();
      this.extension = null;
      return {
        ok: false,
        error: { name: error?.name, message: error?.message },
        reports: this.errors.slice(),
      };
    } finally {
      handle.setActivating(false);
    }
  },
  dispose() {
    this.extension?.dispose();
    this.extension = null;
    return true;
  },
};
