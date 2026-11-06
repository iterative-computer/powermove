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
          requestAnimationFrame: globalThis.__extcheckClock
            ? requestAnimationFrame
            : (fn) =>
                setTimeout(
                  () => this.realm.task(() => fn(performance.now())),
                  16,
                ),
          cancelAnimationFrame: globalThis.__extcheckClock
            ? cancelAnimationFrame
            : (id) => clearTimeout(id),
          reportError: (error) => host.reportError(error),
        })
      : undefined;
    const capabilities = {
      ui: { controls: PowermoveCompat.controls },
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
          this.controls?.sync();
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
    // Transient playback belongs to the native worker, separately from the
    // kernel's document/history writer. Publications are read through getters.
    const transport = (method, extra = {}) =>
      call("transport.command", { method, now: performance.now(), ...extra });
    const updateTransport = (reply, force = false) => {
      if (reply.timeChanged || force)
        this.realm.PM.bus.emit("time", reply.time);
      if (reply.playingChanged) this.realm.PM.bus.emit("transport");
      if (reply.timeChanged || reply.playingChanged || force)
        this.realm.PM.invalidate();
    };
    let playback = 0;
    Object.defineProperties(this.realm.PM, {
      time: {
        enumerable: true,
        get: () => transport("state").time,
        set: (time) => {
          if (Number.isFinite(time))
            updateTransport(transport("seek", { time, raw: true }));
        },
      },
      playing: { enumerable: true, get: () => transport("state").playing },
      loop: {
        enumerable: true,
        get: () => transport("state").loop,
        set: (value) => {
          transport("loop", { value: !!value });
        },
      },
    });
    this.realm.PM.setTime = (time, options = {}) => {
      if (Number.isFinite(time))
        updateTransport(
          transport("seek", { time, raw: !!options.raw }),
          !!options.force,
        );
    };
    this.realm.PM.step = (frames) =>
      this.realm.PM.setTime(
        this.realm.PM.time + frames / this.realm.PM.proj.fps,
      );
    this.realm.PM.play = () => {
      if (this.realm.PM.playing) return;
      updateTransport(transport("play"));
      const generation = ++playback;
      const frame = () => {
        if (generation !== playback || !this.realm.PM.playing) return;
        this.realm.task(() => updateTransport(transport("tick")));
        if (this.realm.PM.playing) schedule(frame);
      };
      const schedule = (fn) =>
        typeof requestAnimationFrame === "function"
          ? requestAnimationFrame(fn)
          : setTimeout(fn, 16);
      schedule(frame);
    };
    this.realm.PM.pause = () => {
      ++playback;
      updateTransport(transport("pause"));
    };
    this.realm.PM.toggle = () =>
      this.realm.PM.playing ? this.realm.PM.pause() : this.realm.PM.play();
    this.realm.installGlobals(globalThis);
    __installNativeCanvas(this);
    if (globalThis.__extcheckClock)
      globalThis.__extcheckClock.task = (fn) => this.realm.task(fn);
    if (this.dom && PowermoveCompat.controlStores)
      this.controls = PowermoveCompat.connectControlState(
        this.realm,
        PowermoveCompat.controlStores,
        this.dom.document,
        PowermoveCompat.controlStyles,
      );
    return true;
  },
  active() {
    return !!this.extension;
  },
  async reload() {
    await null;
    return this.activate(this.manifest, this.loadModule());
  },
  async disable() {
    await null;
    return this.dispose();
  },
  driverAPI() {
    return (this.driver ??= this.realm.extension(
      "synthetic-extcheck-driver",
      3,
    ).api);
  },
  async activate(manifest, module) {
    if (this.extension) this.dispose();
    this.manifest = manifest;
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
