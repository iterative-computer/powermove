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
    // AppKit answers on the main thread, so the realm asks and carries on.
    const asks = new Map();
    let asked = 0;
    const ask = (request) =>
      new Promise((resolve) => {
        const token = ++asked;
        asks.set(token, resolve);
        call("ui.request", { ...request, token });
      });
    this.answer = (token, value) => {
      const resolve = asks.get(token);
      asks.delete(token);
      this.realm.task(() => resolve?.(value));
    };
    const wireMenu = (items, runs) =>
      items.map((item) => {
        if (item === "-") return { separator: true };
        if (item?.header !== undefined)
          return { label: String(item.header), disabled: true };
        const wire = {
          label: String(item?.label ?? ""),
          disabled: !!item?.disabled,
          on: !!item?.on,
        };
        if (item?.submenu) wire.submenu = wireMenu(item.submenu, runs);
        else if (item?.run) wire.run = runs.push(item.run) - 1;
        return wire;
      });
    const showMenu = async (items, request) => {
      const runs = [];
      const wire = wireMenu((await items) ?? [], runs);
      if (!wire.length && request.key === undefined) return;
      const choice = await ask({ kind: "menu", items: wire, ...request });
      if (choice !== null && choice !== undefined) await runs[choice]?.();
    };
    this.gatherMenu = (location, context, key) => {
      showMenu(this.realm.kernel.gatherMenu(location, context), { key }).catch(
        (error) => this.reportFailure(error),
      );
    };
    // One write per realm task, however often an extension saves.
    const unsaved = new Map();
    const save = (key, value) => {
      if (!unsaved.size)
        queueMicrotask(() => {
          for (const [key, value] of unsaved)
            call("storage.set", { key, value });
          unsaved.clear();
        });
      unsaved.set(key, value);
    };
    const reveal = (id, open) =>
      call("ui.request", { kind: "panel", id, open });
    const capabilities = {
      ui: { controls: PowermoveCompat.controls },
      assets: {
        get: (id) => this.realm.PM.proj.assets?.[id],
      },
      panelsBackend: Object.fromEntries(
        ["open", "close", "isOpen", "isHidden", "refresh", "list"].map(
          (name) => [
            name,
            (...args) => {
              if (!this.panels)
                throw Error("Remote DOM panels are not attached");
              const result = this.panels[name](...args);
              if (name === "open" || name === "close")
                reveal(args[0], name === "open");
              return result;
            },
          ],
        ),
      ),
      PM: {
        toast: (text, options = {}) =>
          call("ui.toast", {
            text: String(text),
            options: {
              kind: options.kind,
              error: options.error,
              key: options.key,
              sticky: options.sticky,
            },
          }),
        confirm: (title, body = "") =>
          ask({ kind: "confirm", title: String(title), body: String(body) }),
        openExternal: (url) => ask({ kind: "openExternal", url: String(url) }),
        menu: (anchor, items) => {
          const rect = anchor?.getBoundingClientRect?.();
          const at = rect
            ? { x: rect.left, y: rect.bottom }
            : { x: anchor?.x ?? 0, y: anchor?.y ?? 0 };
          showMenu(items, at).catch((error) => this.reportFailure(error));
        },
        closeMenus: () => undefined,
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
        GL: {
          bounds: (layer, time) =>
            host.query("engine.bounds", {
              layer: layer?.id,
              time: time ?? this.realm.PM.time,
            }),
          pick: (x, y, time) =>
            host.query("engine.pick", {
              x,
              y,
              time: time ?? this.realm.PM.time,
            }),
          compileError: () => null,
          previewViewport: null,
          canvas: null,
          init: () => null,
          resize: () => undefined,
        },
        assets: {
          get: (id) => this.realm.PM.proj.assets?.[id],
          add: () => {
            throw Error("Import media from the Project panel");
          },
        },
        store: {
          get: (key, fallback) => storage[key] ?? fallback,
          set: (key, value) => {
            storage[key] = value;
            save(key, value);
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
        this.realm.kernel.inspectorSections,
        () => [...this.realm.PM.sel.layers],
      );
    if (this.panels)
      this.realm.PM.bus.on("sel", () => this.panels.selectionChanged());
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
      activating: (id) => call("extension.activating", { id }),
      document: this.dom?.document,
    });
    this.realm.PM.Kernel.loader = this.loader;
    const kernel = this.realm.kernel;
    let publishing = false;
    const changed = () => {
      if (publishing) return;
      publishing = true;
      queueMicrotask(() => {
        publishing = false;
        this.publishContributions();
      });
    };
    for (const registry of [
      kernel.commands,
      kernel.panels,
      kernel.inspectorSections,
      kernel.effects,
      kernel.transitions,
      kernel.layerTypes,
      kernel.themes,
      kernel.keybindings,
      kernel.status,
    ])
      registry.onChange(changed);
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
  hostAPI() {
    return (this.api ??= this.realm.extension("powermove-host", 3).api);
  },
  /** Every entry below is called by the native host as one synchronous task. A
   * promise is never returned to it: long extension work must not hold the owner. */
  boot(records) {
    this.loader
      .boot(records)
      .catch((error) => this.reportFailure(error))
      .finally(() => {
        this.publishContributions();
        runtime.engine("extensions.ready", {});
      });
  },
  update(records, ids, reason) {
    this.loader
      .update(records, ids, reason)
      .catch((error) => this.reportFailure(error))
      .finally(() => this.publishContributions());
  },
  reportFailure(error) {
    runtime.engine("ui.toast", {
      text: String(error?.message ?? error),
      options: {},
    });
  },
  sync() {
    this.realm.task(() =>
      this.realm.receive(runtime.engine("kernel.publication", {})),
    );
  },
  moved(time, timeChanged, playingChanged) {
    this.realm.task(() => {
      if (timeChanged) this.realm.PM.bus.emit("time", time);
      if (playingChanged) this.realm.PM.bus.emit("transport");
      this.realm.PM.invalidate();
    });
  },
  run(id, args) {
    this.realm.task(() => {
      Promise.resolve(this.hostAPI().commands.run(id, ...args)).catch((error) =>
        this.realm.reportError(error),
      );
    });
  },
  openPanel(id) {
    this.realm.task(() => this.panels.open(id));
    this.dom.flush();
  },
  placePanel(id, rect) {
    this.panels.place(id, rect);
    this.dom.flush();
  },
  closePanel(id) {
    this.realm.task(() => this.panels.close(id));
    this.dom.flush();
  },
  retire(handle) {
    this.realm.task(() => this.realm.retireExternal(handle, true));
  },
  runExternal(next) {
    this.realm.task(() => this.realm.runExternal(next));
  },
  /** Definitions as data. Callbacks stay here and are reached by id. */
  contributions() {
    const kernel = this.realm.kernel;
    const data = (value, seen = new Set()) => {
      if (value === null || typeof value !== "object")
        return typeof value === "function" || value === undefined
          ? undefined
          : value;
      if (seen.has(value)) return undefined;
      seen.add(value);
      const out = Array.isArray(value)
        ? value.map((item) => data(item, seen) ?? null)
        : Object.fromEntries(
            Object.entries(value)
              .map(([key, item]) => [key, data(item, seen)])
              .filter(([, item]) => item !== undefined),
          );
      seen.delete(value);
      return out;
    };
    const entries = (registry) =>
      registry.entries().map(({ id, ownerId, item }) => ({
        ...data(item),
        id,
        owner: ownerId,
      }));
    const label = (fn) => {
      try {
        return typeof fn === "function" ? String(fn() ?? "") : "";
      } catch {
        return "";
      }
    };
    return {
      commands: entries(kernel.commands),
      panels: [
        ...entries(kernel.panels),
        ...(kernel.inspectorSections.list().length
          ? [
              {
                id: PowermoveCompat.SECTIONS_PANEL,
                title: "Extension Properties",
                owner: "powermove-host",
              },
            ]
          : []),
      ],
      inspector: entries(kernel.inspectorSections),
      effects: entries(kernel.effects),
      transitions: entries(kernel.transitions),
      layers: entries(kernel.layerTypes),
      themes: entries(kernel.themes),
      keybindings: kernel.listBindings().map((binding) => data(binding)),
      status: kernel.status.entries().map(({ id, ownerId, item }) => ({
        id,
        owner: ownerId,
        text: label(item.text),
        side: item.side,
      })),
      theme: kernel.theme.activeId,
      definitions: this.definitions(),
    };
  },
  /** A definition the generators refuse never stops the others publishing. */
  definitions() {
    try {
      return PowermoveCompat.definitionSnapshots(this.realm.kernel);
    } catch (error) {
      this.errors.push({ name: error?.name, message: error?.message });
      return [];
    }
  },
  publishContributions() {
    runtime.engine("contributions.publish", { value: this.contributions() });
  },
};
