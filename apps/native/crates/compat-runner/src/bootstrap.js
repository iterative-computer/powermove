// Runs in the actual Compat V8 owner. Missing app capabilities are errors, never test doubles.
// The editor window's size, as Electron's `window` had it (v1–3 extensions
// size docks from it); the host keeps it current. Defaults until it does.
for (const [key, value] of [
  ["innerWidth", 1440],
  ["innerHeight", 900],
  ["outerWidth", 1440],
  ["outerHeight", 900],
  ["devicePixelRatio", 2],
])
  globalThis[key] = value;
globalThis.__compat = {
  /** The editor window's viewport changed (points and backing scale). */
  viewport(width, height, scale) {
    const changed =
      globalThis.innerWidth !== width || globalThis.innerHeight !== height;
    for (const target of [globalThis, globalThis.window].filter(Boolean)) {
      target.innerWidth = target.outerWidth = width;
      target.innerHeight = target.outerHeight = height;
      target.devicePixelRatio = scale;
    }
    if (changed && this.realm)
      this.realm.task(() => globalThis.window?.dispatchEvent?.(new Event("resize")));
  },
  create(initial) {
    const call = (operation, input = {}) => runtime.engine(operation, input);
    initial ??= call("kernel.publication");
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
          measureSync: (request) => call("dom.measureSync", { request }),
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
          pageAttribute: (node, name, value) =>
            this.pageAttribute ? this.pageAttribute(node, name, value) : value,
        })
      : undefined;
    // AppKit answers on the main thread, so the realm asks and carries on.
    const asks = new Map();
    let modals = 0;
    let asked = 0;
    let openMenu = 0;
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
      // AppKit shows one menu; the last one's closures go when the next opens.
      this.answer(openMenu, null);
      const answer = ask({ kind: "menu", items: wire, ...request });
      openMenu = asked;
      const choice = await answer;
      if (choice !== null && choice !== undefined) await runs[choice]?.();
    };
    this.gatherMenu = (location, context, key) => {
      // An empty location gathers each `[location, context]` pair, in order.
      const lists = location
        ? [this.realm.kernel.gatherMenu(location, context)]
        : context.map(([at, value]) => this.realm.kernel.gatherMenu(at, value));
      showMenu(Promise.all(lists).then((all) => all.flat()), { key }).catch(
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
    // Media an extension imports goes through the app's Import Media: probed,
    // retained in the project's media store, in the Project panel. A picked
    // file stays on disk behind a token; only bytes the realm made are written.
    const imported = (id, file, reply) =>
      this.realm.PM.proj.assets?.[id] ?? {
        id,
        name: file.name,
        kind: reply.kind,
        size: file.size,
      };
    this.assets = {
      get: (id) => this.realm.PM.proj.assets?.[id],
      import: async (file, options = {}) => {
        if (!(file instanceof File)) throw Error("assets.import requires a File");
        const token =
          runtime.fileToken(file) ??
          runtime.bytes("assets.stage", file, { name: file.name }).token;
        const reply = await ask({
          kind: "import",
          file: token,
          name: file.name,
          layerDefinition: options?.layerDefinition ?? null,
        });
        if (!reply?.id) throw Error(reply?.error ?? `Could not import ${file.name || "asset"}`);
        return imported(reply.id, file, reply);
      },
      pick: async (options = {}) => {
        const files = await ask({
          kind: "pick",
          accept: String(options?.accept ?? ""),
          multiple: options?.multiple === true,
        });
        return (files ?? []).map((file) => runtime.hostFile(file));
      },
      importUrl: async (url) => {
        const target = new URL(String(url));
        if (target.protocol !== "https:" || target.username || target.password || String(url).length > 2048)
          throw new TypeError(
            "assets.importUrl accepts an https URL of at most 2 KB without credentials",
          );
        // The app downloads it as main does: public HTTPS, at most five
        // redirects, 512 MiB, its type from its bytes; then Import Media.
        const reply = await ask({ kind: "download", url: target.href });
        if (!reply?.id) throw Error(reply?.error ?? "The download could not be imported");
        return reply.id;
      },
    };
    const reveal = (id, open) =>
      call("ui.request", { kind: "panel", id, open });
    // Raster overrides: the viewer and exports ask for layers they need, a
    // batch at a time. An override's canvas goes back as pixels, a pass-through
    // as native, a null result (main draws nothing) as hidden, and a failure
    // as an error: the frame fails rather than draw the layer without it.
    const layerIn = (project, composition, id) => {
      const comps = project?.comps;
      const nested = Array.isArray(comps)
        ? comps.find((c) => c?.id === composition)
        : comps?.[composition];
      for (const list of [nested?.layers, project?.layers])
        for (const layer of list ?? []) if (layer?.id === id) return layer;
      return null;
    };
    const failure = (error) => String(error?.message ?? error ?? "The override failed");
    /** `ask`: `{revision}` for the viewer; `{revision, exact}` for a one-frame
     * look (an agent's still), which only this exact revision answers; nothing
     * for an export, whose own realm holds its frozen project (`export.rs`). */
    this.answerRasters = (batch, needs, ask = {}) => {
      const realm = this.realm;
      const answers = needs.map((need) => {
        // This realm has not seen the viewer's revision yet: ask again.
        if (ask.revision !== undefined && ask.revision > realm.revision) return { retry: true };
        if (ask.exact && ask.revision < realm.revision) return { moved: true };
        try {
          const layer = layerIn(realm.PM.proj, need.composition, need.layer);
          if (!layer) return { native: true };
          realm.rasterContext = { composition: need.composition, layer };
          const result = realm.PM.raster(
            layer,
            need.scale,
            need.seconds,
            undefined,
            need.crop ?? undefined,
          );
          if (!result) return { hidden: true };
          if (realm.isBaseRaster(result, need)) return { native: true };
          const surface = this.nativeSurfaceOf?.(result.cv);
          if (!surface) return { error: "The override returned a canvas the engine cannot read" };
          return {
            surface,
            w: result.w,
            h: result.h,
            anchorX: result.anchorX,
            anchorY: result.anchorY,
            uv: Array.isArray(result.uv) ? result.uv : undefined,
          };
        } catch (error) {
          this.reportFailure(error);
          return { error: failure(error) };
        } finally {
          realm.rasterContext = null;
        }
      });
      try {
        call("compat.answers", { batch, answers });
      } catch (error) {
        // The asker waits for this batch; say why it never came.
        this.reportFailure(error);
        call("compat.answers", { batch, answers: answers.map(() => ({ error: failure(error) })) });
      }
    };
    // PM.Fonts as on main: this Mac's families, and `ensure`, which gives the
    // page the engine's face for a family, so a hidden element an extension
    // measures lays out as the engine draws its text. The project's own text
    // families are ensured at boot and as it changes (`projectFonts`).
    const NATIVE = ["SF Pro Text", "SF Pro Display", "SF Mono"];
    const WEB_FALLBACKS = [
      "Helvetica Neue", "Helvetica", "Arial", "Arial Black", "Arial Narrow",
      "Avenir", "Avenir Next", "Futura", "Gill Sans", "Optima",
      "Georgia", "Times New Roman", "Baskerville", "Didot", "American Typewriter",
      "Courier New", "Menlo", "Monaco", "Andale Mono", "Impact",
    ];
    const clean = (value) => String(value || "").replace(/[\u0000-\u001f\u007f]/g, "").trim();
    const unique = (values) => [...new Set(values.map(clean).filter(Boolean))];
    // Each face settles once the page has loaded it (or it could not be had);
    // `ensure` waits for its own faces only.
    const settled = new Set();
    const pending = new Map();
    const faceKey = (face) =>
      `${face.family}|${Math.trunc(Math.min(1000, Math.max(1, face.weight)))}|${face.italic ? 1 : 0}`;
    const requestFaces = (faces) => {
      const fresh = [];
      const waits = faces.map((face) => {
        const key = faceKey(face);
        if (settled.has(key)) return Promise.resolve();
        if (!pending.has(key)) {
          let resolve;
          const promise = new Promise((done) => (resolve = done));
          pending.set(key, { promise, resolve });
          fresh.push(face);
        }
        return pending.get(key).promise;
      });
      if (fresh.length) call("ui.request", { kind: "fonts", faces: fresh });
      // A window that never answers (its page gone) does not hold `ensure` forever.
      return Promise.race([
        Promise.all(waits),
        new Promise((resolve) => setTimeout(resolve, 15000)),
      ]);
    };
    const fonts = {
      bundled: [],
      system: [],
      families: unique([...NATIVE, ...WEB_FALLBACKS]),
      setSystemFamilies: (values) => {
        fonts.system = unique(Array.isArray(values) ? values : []).sort((a, b) => a.localeCompare(b));
        fonts.families = unique([...NATIVE, ...fonts.system, ...WEB_FALLBACKS]);
        this.realm?.PM.bus.emit("fonts", fonts.families);
      },
      options: (current) => unique([clean(current), ...fonts.families]),
      ensure: async (family, weight = 400) => {
        const name = clean(family);
        if (!name) return;
        await requestFaces([{ family: name, weight: Number(weight) || 400, italic: false }]);
        this.realm.PM.rasterClear?.();
        this.realm.PM.invalidate();
      },
    };
    const value = (v) => (v && typeof v === "object" && "v" in v ? v.v : v);
    /** The faces the project's text layers use, as the engine resolves them. */
    this.projectFonts = () => {
      const faces = [];
      const scan = (layers) => {
        for (const layer of layers ?? []) {
          const family = layer?.type === "text" ? clean(value(layer.d?.font)) : "";
          if (!family) continue;
          const weight = Number(value(layer.d?.weight)) || 500;
          faces.push({ family, weight, italic: !!value(layer.d?.italic) });
        }
      };
      const project = this.realm.PM.proj;
      scan(project?.layers);
      for (const comp of Object.values(project?.comps ?? {})) scan(comp?.layers);
      return requestFaces(faces);
    };
    /** The page settled these faces: layouts measured before now re-measure. */
    this.fontsLoaded = (keys = []) => {
      this.realm.task(() => {
        for (const key of keys) {
          settled.add(key);
          pending.get(key)?.resolve();
          pending.delete(key);
        }
        this.realm.PM.bus.emit("fonts", fonts.families);
        this.realm.PM.rasterClear?.();
        this.realm.PM.invalidate();
      });
    };
    const capabilities = {
      rasterProducing: (on, epoch) => call("compat.producing", { on: !!on, epoch }),
      // A capture this realm waits for asks it for overrides: answered inline.
      answerRasters: (batch, needs, ask) => this.answerRasters(batch, needs, ask),
      ui: { controls: PowermoveCompat.controls },
      extensions: {
        list: () => this.loader.records(),
        setEnabled: async (id, enabled) => {
          call("extension.enabled", { id, enabled: !!enabled });
        },
        reload: (id) => this.loader.reload(id),
      },
      assets: this.assets,
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
        Fonts: fonts,
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
        // ui.modal: the body is DOM the extension keeps using. The page shows
        // it in a native sheet (title and actions are the sheet's own); a sheet
        // button clicks its hidden twin here, so actions run in this realm.
        modal: (options = {}) => {
          const document = this.dom?.document;
          if (!document) throw Error("ui.modal needs the window's page");
          const id = ++modals;
          const el = document.createElement("div");
          el.setAttribute("data-pm-modal", String(id));
          const body = document.createElement("div");
          body.className = "pm-modal-body";
          if (typeof options.body === "string") body.textContent = options.body;
          else if (options.body) body.append(options.body);
          const actions = Array.isArray(options.actions) ? options.actions : [];
          const twins = document.createElement("div");
          twins.hidden = true;
          let pending = false;
          let open = true;
          const handle = {
            el,
            body,
            // As main: closing (an action, a dismissal or close()) removes the
            // sheet, then calls onClose once.
            close: () => {
              if (!open) return;
              open = false;
              el.remove();
              try {
                options.onClose?.();
              } catch (error) {
                this.reportFailure(error);
              }
            },
          };
          actions.forEach((action, index) => {
            const button = document.createElement("button");
            button.setAttribute("data-pm-modal-action", String(index));
            button.addEventListener("click", async () => {
              if (pending) return;
              try {
                const result = action?.run?.();
                if (result && typeof result.then === "function") {
                  pending = true;
                  const value = await result;
                  if (value !== false) handle.close();
                } else if (result !== false) handle.close();
              } catch (error) {
                this.reportFailure(error);
              } finally {
                pending = false;
              }
            });
            twins.append(button);
          });
          // Escape, the close button or a click outside: as main's dismissible modal.
          const dismiss = document.createElement("button");
          dismiss.setAttribute("data-pm-modal-dismiss", "");
          dismiss.addEventListener("click", () => {
            if (options.dismissible !== false && !pending) handle.close();
          });
          twins.append(dismiss);
          el.setAttribute(
            "data-pm-modal-frame",
            JSON.stringify({
              title: String(options.title ?? ""),
              width: Math.max(240, Math.min(1200, Number(options.width) || 420)),
              actions: actions.map((a) => ({ label: String(a?.label ?? ""), pri: !!a?.pri })),
              dismissible: options.dismissible !== false,
            }),
          );
          el.append(body, twins);
          document.body.append(el);
          return handle;
        },
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
          add: (file, options) => this.assets.import(file, options),
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
    // Built-in services v1 registered for extensions: `tool` (the shell's tool)
    // and `viewer` (its navigation; the native viewer has no DOM stage).
    const shell = { tool: "select", toolShape: "rect", zoom: 1, pan: [0, 0], fit: true };
    this.shellState = (state) => {
      const tool = state.tool !== undefined && state.tool !== shell.tool;
      Object.assign(shell, state);
      if (tool) this.realm.PM.bus.emit("tool", shell.tool);
    };
    // As main's toolbar service: the change is immediate here, and the shell
    // follows (its command carries a shape tool's shape).
    this.realm.services.register("tool", {
      get tool() {
        return shell.tool;
      },
      set tool(value) {
        shell.tool = String(value);
      },
      get toolShape() {
        return shell.toolShape;
      },
      set toolShape(value) {
        shell.toolShape = String(value);
      },
      setTool: (tool, detail) => {
        const name = String(tool ?? "");
        const changed = name !== shell.tool;
        shell.tool = name;
        if (name === "shape" && detail) shell.toolShape = String(detail);
        if (changed) this.realm.PM.bus.emit("tool", name);
        if (!/^[a-z]+$/.test(name)) return;
        call("ui.request", {
          kind: "command",
          name: "tool" + name[0].toUpperCase() + name.slice(1),
          shape: name === "shape" ? shell.toolShape : undefined,
        });
      },
    });
    this.realm.services.register("viewer", {
      get pan() {
        return [...shell.pan];
      },
      get zoom() {
        return shell.zoom;
      },
      get fit() {
        return shell.fit;
      },
      shown: 1,
      showControls: true,
      stage: null,
      ov: null,
      layout: () => undefined,
      attach: () => undefined,
      worldBounds: (layer, time) => this.realm.PM.GL?.bounds?.(layer, time) ?? null,
      snapshotSnapCandidates: () => ({ x: [], y: [] }),
      isNavigating: () => false,
      deferNavigationRender: () => false,
      // As main: a finite zoom leaves fit (centred when it was fitting),
      // clamped to 5%–800%; the viewer takes it and reports where it landed.
      setZoom: (zoom) => {
        const value = Number(zoom);
        if (!Number.isFinite(value)) return false;
        if (shell.fit) shell.pan = [0, 0];
        shell.fit = false;
        shell.zoom = Math.min(8, Math.max(0.05, value));
        call("ui.request", { kind: "viewerZoom", zoom: shell.zoom, pan: [...shell.pan] });
        return shell.zoom;
      },
    });
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
    // Activating a theme changes no registry, but the shell follows it.
    kernel.events.on("theme:changed", changed, "powermove-host");
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
   * promise is never returned to it: long extension work must not hold the owner.
   * Store code runs in its helper; nothing confined ever enters this plan. */
  boot(records) {
    this.realm.task(() => {
      this.realm.PM.Fonts.setSystemFamilies(runtime.engine("fonts.families", {}));
      void this.projectFonts();
    });
    this.loader
      .boot(this.trusted(records))
      .catch((error) => this.reportFailure(error))
      .finally(() => {
        this.publishContributions();
        runtime.engine("extensions.ready", {});
      });
  },
  trusted: (records) => records.filter((record) => record.trust !== "store"),
  update(records, ids, reason) {
    this.loader
      .update(this.trusted(records), ids, reason)
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
    this.realm.task(() => {
      this.realm.receive(runtime.engine("kernel.publication", {}));
      void this.projectFonts();
    });
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
      nativeTheme: this.nativeTheme(),
      definitions: this.definitions(),
    };
  },
  /** The active theme as native token patches (translator in this realm). */
  nativeTheme() {
    const kernel = this.realm.kernel;
    const active = kernel.theme.activeId;
    const entry = kernel.themes.entries().find(({ id }) => id === active);
    if (!entry) return { id: active, dark: {}, light: {}, blockers: [] };
    try {
      return PowermoveCompat.nativeTheme(entry.item, kernel.theme.scheme);
    } catch (error) {
      return { id: active, error: String(error?.message ?? error) };
    }
  },
  definitions() {
    return PowermoveCompat.definitionSnapshots(this.realm.kernel, (id, error) =>
      this.errors.push({ id, name: error?.name, message: error?.message }),
    );
  },
  publishContributions() {
    runtime.engine("contributions.publish", { value: this.contributions() });
  },
};
