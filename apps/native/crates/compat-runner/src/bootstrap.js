// Runs in the actual Compat V8 owner. Missing app capabilities are errors, never test doubles.
globalThis.__compat = {
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
        if (report?.extensionId)
          this.loader?.reportRuntimeError(report.extensionId, error);
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
          session: { panel: "compat", generation: 1 },
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
        invalidate: (what = "all") => {
          if (what === "all" || what === "render")
            this.realm.PM.bus.emit("draw");
          if (what === "all") dirty.add("ui").add("timeline");
          else if (what !== "render") dirty.add(what);
          scheduleDirty();
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
    // Rendering demand is immediate; UI and timeline notices coalesce on one frame.
    const dirty = new Set();
    let dirtyFrame = 0;
    const scheduleDirty = () => {
      if (dirtyFrame) return;
      const flush = () => {
        dirtyFrame = 0;
        const notices = new Set(dirty);
        dirty.clear();
        for (const name of ["timeline", "ui", "status"])
          if (notices.has(name)) this.realm.PM.bus.emit("draw:" + name);
      };
      dirtyFrame =
        typeof requestAnimationFrame === "function"
          ? requestAnimationFrame(flush)
          : setTimeout(flush, 16);
    };
    this.realm = new PowermoveCompat.CompatRealm({
      id: "compat",
      generation: 1,
      initial,
      host,
      recorder,
      dom: this.dom,
      installLegacy: PowermoveCompat.installLegacyRuntime,
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
    this.loader = PowermoveCompat.createLoader({
      realm: this.realm,
      link: (graph) => __linkModules(graph),
      report: (id, health) => call("extension.health", { id, health }),
      toast: (text, options) => call("ui.toast", { text, options }),
      document: this.dom?.document,
    });
    this.realm.PM.Kernel.loader = this.loader;
    if (this.dom && PowermoveCompat.controlStores)
      this.controls = PowermoveCompat.connectControlState(
        this.realm,
        PowermoveCompat.controlStores,
        this.dom.document,
        PowermoveCompat.controlStyles,
      );
    return true;
  },
  /** One enabled extension, activated through the production loader. */
  async install(record) {
    const known = this.loader.records().filter((r) => r.id !== record.id);
    await this.loader.update([...known, record], [record.id], "install");
    const health = this.loader.records().find((r) => r.id === record.id).health;
    return health.state === "ok"
      ? { ok: true }
      : { ok: false, error: { message: health.error } };
  },
  driverAPI() {
    return (this.driver ??= this.realm.extension(
      "synthetic-extcheck-driver",
      3,
    ).api);
  },
};
