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
    // `PM.MediaStore.get`: the asset's file, held on disk like a picked one.
    this.mediaFile = (asset) => ask({ kind: "mediaFile", asset });
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
    // as an error: the viewer reports the layer as not drawn, with the error,
    // and an export fails rather than draw the layer without it.
    // A need's layer: the first with its id in its composition's layers, then
    // the project's. One batch indexes each composition once (a frame asks for
    // every override layer, and each scan read hundreds of project views); a
    // write that replaces or resizes the project's layers starts a new index.
    const layerIndex = () => {
      let project, comps, top, topLength;
      let byComposition = new Map();
      return (proj, composition, id) => {
        const layers = proj?.layers;
        if (proj !== project || proj?.comps !== comps || layers !== top || layers?.length !== topLength) {
          project = proj;
          comps = proj?.comps;
          top = layers;
          topLength = layers?.length;
          byComposition = new Map();
        }
        let ids = byComposition.get(composition);
        if (!ids) {
          const nested = Array.isArray(comps)
            ? comps.find((c) => c?.id === composition)
            : comps?.[composition];
          ids = new Map();
          for (const list of [nested?.layers, top])
            for (const layer of list ?? []) {
              const key = layer?.id;
              if (!ids.has(key)) ids.set(key, layer);
            }
          byComposition.set(composition, ids);
        }
        return ids.get(id) ?? null;
      };
    };
    const failure = (error) => String(error?.message ?? error ?? "The override failed");
    // The override's own throw: the same revision throws again.
    const thrown = (error) => {
      this.reportFailure(error);
      return { error: failure(error), threw: true };
    };
    // An override's raster as a canvas the host reads when the batch goes.
    const canvasAnswer = (result) => {
      const surface = this.nativeSurfaceOf?.(result.cv);
      if (!surface)
        return { error: "The override returned a canvas the engine cannot read", threw: true };
      return {
        surface,
        w: result.w,
        h: result.h,
        anchorX: result.anchorX,
        anchorY: result.anchorY,
        uv: Array.isArray(result.uv) ? result.uv : undefined,
      };
    };
    /** `ask`: `{revision}` for the viewer; `{revision, exact}` for a one-frame
     * look (an agent's still), which only this exact revision answers; nothing
     * for an export, whose own realm holds its frozen project (`export.rs`). */
    this.answerRasters = (batch, needs, ask = {}) => {
      const realm = this.realm;
      const layerIn = layerIndex();
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
          // Decided once every need ran: a later override may draw into it.
          if (realm.isUncroppedBase?.(result, need)) return { pending: { value: result, need } };
          return canvasAnswer(result);
        } catch (error) {
          return thrown(error);
        } finally {
          realm.rasterContext = null;
        }
      });
      const pending = answers.flatMap((answer, i) => (answer.pending ? [[i, answer.pending]] : []));
      if (pending.length) {
        let windows = [];
        try {
          windows = realm.windowBases(pending.map(([, entry]) => entry));
        } catch (error) {
          this.reportFailure(error);
        }
        pending.forEach(([i, { value }], k) => {
          const baseWindow = windows[k];
          if (baseWindow) answers[i] = { baseWindow };
          else {
            try {
              answers[i] = canvasAnswer(value);
            } catch (error) {
              answers[i] = thrown(error);
            }
          }
        });
      }
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
      scene3d: PowermoveCompat.scene3dAPI(
        new Proxy({}, { get: (_, key) => this.realm.PM[key] }),
        (args, meta) => this.realm.scene3dEdit(args, meta),
        (time) => host.query("scene3d.describe", { time }),
        (selection) => this.realm.kernel.events.emit("scene3d:selection", selection),
      ),
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
          // As main: the topmost layer under a composition point, or null.
          pick: (x, y, time, options) => {
            const id = host.query("engine.pick", {
              x,
              y,
              time: time ?? this.realm.PM.time,
              includeLocked: !!options?.includeLocked,
            });
            return id ? (this.realm.PM.L?.(id) ?? null) : null;
          },
          compileError: () => null,
          // The composition window the viewer drew last (or a presenter asked
          // for), in composition pixels; null while all of it shows.
          get previewViewport() {
            return presenter.viewport ?? viewport();
          },
          // D7: a canvas bound with init presents the viewer's frames by
          // readback, at its own size: slower than main, same contract.
          get canvas() {
            return presenter.canvas;
          },
          init: (canvas) => {
            if (typeof canvas?.getContext !== "function") return false;
            presenter.canvas = canvas;
            this.present();
            return true;
          },
          resize: (width, height, previewViewport = null) => {
            presenter.viewport = previewViewport?.width > 0 ? previewViewport : null;
            const canvas = presenter.canvas;
            if (!canvas || (canvas.width === width && canvas.height === height)) return false;
            canvas.width = width;
            canvas.height = height;
            this.present();
            return true;
          },
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
    // v1's model members that extensions reach raw (`api.host.pm`), as main.
    this.realm.PM.addLayer = (layer, at) => {
      const PM = this.realm.PM;
      PM.proj.layers.splice(at == null ? 0 : at, 0, layer);
      PM.ProjectIndex?.invalidate();
      PM.bus.emit("layers");
      return layer;
    };
    // v1's media store, read side: an asset's bytes as a Blob (the file behind
    // it on disk, read when asked), or null when it has none. The native app
    // keeps media itself, so writing to it throws by name.
    const storeOnly = (member) => () => {
      throw new Error(`PM.MediaStore.${member} is not available in this version of Powermove`);
    };
    this.realm.PM.MediaStore = {
      get: async (asset) => {
        const id = typeof asset === "string" ? asset : asset?.id;
        if (!id) return null;
        const file = await this.mediaFile(String(id));
        return file?.token ? runtime.hostFile(file) : null;
      },
      put: storeOnly("put"),
      getForSave: storeOnly("getForSave"),
      remove: storeOnly("remove"),
    };
    // Built-in services v1 registered for extensions: `tool` (the shell's tool)
    // and `viewer` (its navigation and its stage, a page element over the
    // native viewer that the shell keeps on it).
    const shell = {
      tool: "select",
      toolShape: "rect",
      zoom: 1,
      pan: [0, 0],
      fit: true,
      // Window points: the viewer's area and the composition frame in it.
      stage: null,
      frame: null,
      // The native timeline's view and visible rows (`[layerId, top, bottom]`).
      timeline: null,
    };
    const compat = this;
    const stage = { el: null, inner: null, ov: null, ovUsed: false };
    const px = (n) => `${Math.round(n * 100) / 100}px`;
    const dpr = () => Math.min(globalThis.devicePixelRatio || 1, 2);
    // As main's viewer layout: `#stage` fills the viewer, `#stage-inner` is the
    // composition frame inside it, `#overlay` covers the stage at device pixels.
    const placeStage = () => {
      if (!stage.el) return;
      const at = shell.stage;
      if (!at) {
        stage.el.style.display = "none";
        return;
      }
      const [x, y, w, h] = at;
      stage.el.style.cssText = `position:fixed;left:${px(x)};top:${px(y)};width:${px(w)};height:${px(h)};overflow:hidden;pointer-events:none`;
      const f = shell.frame ?? [x, y, w, h];
      stage.inner.style.cssText = `position:absolute;left:${px(f[0] - x)};top:${px(f[1] - y)};width:${px(f[2])};height:${px(f[3])};overflow:hidden`;
      const [ow, oh] = [Math.round(w * dpr()), Math.round(h * dpr())];
      // Assigning a canvas dimension clears it, so only on a real change.
      if (stage.ov.width !== ow) stage.ov.width = ow;
      if (stage.ov.height !== oh) stage.ov.height = oh;
      stage.ov.style.cssText = `position:absolute;left:0;top:0;width:${px(w)};height:${px(h)};pointer-events:none`;
    };
    const ensureStage = () => {
      const document = this.dom?.document;
      if (!document) return null;
      if (!stage.el) {
        stage.el = document.createElement("div");
        stage.el.id = "stage";
        stage.inner = document.createElement("div");
        stage.inner.id = "stage-inner";
        stage.ov = document.createElement("canvas");
        stage.ov.id = "overlay";
        stage.el.append(stage.inner, stage.ov);
        document.body.append(stage.el);
        placeStage();
        // The shell shows the page over the viewer only once a stage exists.
        call("ui.request", { kind: "viewerStage" });
      }
      return stage.el;
    };
    // The composition rectangle the viewer shows, in composition pixels.
    const viewport = () => {
      const [s, f] = [shell.stage, shell.frame];
      const project = this.realm.PM.proj;
      if (!s || !f || !(f[2] > 0) || !project?.w) return null;
      const scale = f[2] / project.w;
      const x0 = Math.max(0, (s[0] - f[0]) / scale);
      const y0 = Math.max(0, (s[1] - f[1]) / scale);
      const x1 = Math.min(project.w, (s[0] + s[2] - f[0]) / scale);
      const y1 = Math.min(project.h, (s[1] + s[3] - f[1]) / scale);
      if (x0 <= 0 && y0 <= 0 && x1 >= project.w && y1 >= project.h) return null;
      return { x: x0, y: y0, width: Math.max(0, x1 - x0), height: Math.max(0, y1 - y0) };
    };
    // Main's overlay pass: the viewer clears its canvas, then listeners draw.
    // The presenter canvas fills from the engine, its crop drawn from a
    // frame of the whole composition at the crop's scale (at most 8192 px).
    // It shows `frame`, the viewer's last frame this realm drew: its time,
    // from the publication the realm holds (captures draw that one), which
    // is the frame's revision. Before any frame, or once the project moved
    // past it (init, resize), the playhead now.
    const presenter = { canvas: null, viewport: null, frame: null };
    this.presenting = () => !!(presenter.canvas?.width && presenter.canvas?.height);
    this.present = (frame = presenter.frame) => {
      const canvas = presenter.canvas;
      const PM = this.realm.PM;
      if (!this.presenting()) return;
      if (frame) presenter.frame = frame;
      const time = frame?.version === this.realm.revision ? frame.time : PM.time;
      const crop = presenter.viewport;
      if (!crop) {
        PM.renderFrameTo(time, canvas.width, canvas.height, {}, canvas);
        return;
      }
      const project = PM.proj;
      const scale = Math.min(
        canvas.width / crop.width,
        8192 / Math.max(project.w, project.h),
      );
      const whole = PM.renderFrameTo(
        time,
        Math.max(1, Math.round(project.w * scale)),
        Math.max(1, Math.round(project.h * scale)),
      );
      const context = canvas.getContext("2d");
      context.clearRect(0, 0, canvas.width, canvas.height);
      context.drawImage(
        whole,
        crop.x * scale,
        crop.y * scale,
        crop.width * scale,
        crop.height * scale,
        0,
        0,
        canvas.width,
        canvas.height,
      );
    };
    this.overlay = () => {
      if (stage.ovUsed) {
        const context = stage.ov.getContext("2d");
        context.setTransform(1, 0, 0, 1, 0, 0);
        context.clearRect(0, 0, stage.ov.width, stage.ov.height);
      }
      this.realm.PM.bus.emit("overlay");
    };
    this.shellState = (state) => {
      const tool = state.tool !== undefined && state.tool !== shell.tool;
      const placed = ["stage", "frame", "zoom", "pan", "fit", "showControls"].some(
        (key) => key in state && JSON.stringify(state[key]) !== JSON.stringify(shell[key]),
      );
      const resized =
        "stage" in state &&
        JSON.stringify(state.stage?.slice(2)) !== JSON.stringify(shell.stage?.slice(2));
      const timeline =
        "timeline" in state && JSON.stringify(state.timeline) !== JSON.stringify(shell.timeline);
      Object.assign(shell, state);
      if (placed) placeStage();
      this.realm.task(() => {
        if (tool) this.realm.PM.bus.emit("tool", shell.tool);
        // As main: scrolling, zooming or revealing redraws the timeline.
        if (timeline) this.realm.PM.bus.emit("draw:timeline");
        if (!placed) return;
        // As main: a panel size change is a layout; pan and zoom repaint the overlay.
        if (resized) this.realm.PM.bus.emit("layout");
        this.overlay();
      });
      this.dom?.flush();
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
    // Main's timeline service over the native timeline: its view reads back
    // at once and the timeline follows; rows come from what it last drew.
    const lane = () => (shell.timeline ??= {});
    const timeline = (request) => call("ui.request", { kind: "timeline", ...request });
    const view = (key, min, max) => ({
      get: () => lane()[key] ?? (key === "pps" ? 100 : 0),
      set: (value) => {
        const number = Number(value);
        if (!Number.isFinite(number)) return;
        lane()[key] = Math.min(max, Math.max(min, number));
        timeline({ [key]: lane()[key] });
      },
      enumerable: true,
    });
    // timeline.ts edges(): ends of the composition, layers and keyframes.
    const edges = () => {
      const PM = this.realm.PM;
      const round = (v) => Math.round(v * 1e4) / 1e4;
      const project = PM.proj;
      const all = new Set([0, project.dur]);
      for (const layer of project.layers) {
        all.add(round(layer.from));
        all.add(round(layer.from + layer.dur));
        for (const { prop } of PM.allProps(layer))
          for (const key of prop.kf ?? []) all.add(round(layer.from + key.t));
      }
      return [...all].sort((a, b) => a - b);
    };
    const timelineService = {
      get graph() {
        return !!lane().graph;
      },
      set graph(value) {
        lane().graph = !!value;
        timeline({ graph: !!value });
      },
      // The native timeline draws in Metal; v4's timeline.paint replaces cv.
      cv: null,
      get keySelectionActive() {
        return !!lane().keySelectionActive;
      },
      nextEdge: () => {
        const now = this.realm.PM.time;
        return edges().find((t) => t > now + 1e-4) ?? this.realm.PM.proj.dur;
      },
      prevEdge: () => {
        const now = this.realm.PM.time;
        return [...edges()].reverse().find((t) => t < now - 1e-4) ?? 0;
      },
      layerAtPoint: (clientX, clientY) => {
        const facts = lane();
        const [x, y, w, h] = facts.rect ?? [0, 0, 0, 0];
        if (clientX < x || clientX > x + w || clientY < y + (facts.ruler ?? 0) || clientY > y + h)
          return null;
        const row = (facts.rows ?? []).find(([, top, bottom]) => clientY >= top && clientY < bottom);
        return row ? (this.realm.PM.L?.(row[0]) ?? null) : null;
      },
      frameView: () => timeline({ frameView: true }),
      reveal: (layer, keys) => {
        const id = typeof layer === "string" ? layer : layer?.id;
        if (id) timeline({ reveal: { layer: id, keys: [...(keys ?? [])].map(String) } });
      },
    };
    Object.defineProperties(timelineService, {
      pps: view("pps", 4, 4000),
      scrollT: view("scrollT", -Infinity, Infinity),
      scrollY: view("scrollY", 0, Infinity),
    });
    this.realm.services.register("timeline", timelineService);
    // Main's inspector service. The native Properties panel and Effect
    // Controls draw the inspector; v1–3 sections live in their own panel.
    // The app's one effect clipboard, shared with Effect Controls in every
    // window; copy reads Effect Controls' selected effects, as main's did.
    const clipboard = (op) => runtime.engine("effects.clipboard", { op });
    const toast = (text) => runtime.engine("ui.toast", { text, options: {} });
    const effects = (count) => `${count} ${count === 1 ? "effect" : "effects"}`;
    const syncShaderUniforms = (layer) => {
      // As main (inspector/shader.ts:39), with the kernel's parser.
      const PM = this.realm.PM;
      const content = layer.d ?? (layer.d = {});
      const definitions = host.query("shader.parameters", { code: String(content.code ?? "") });
      PM.UIState?.setShaderMeta?.(layer, { udefs: definitions });
      const uniforms = content.uniforms ?? (content.uniforms = {});
      for (const definition of definitions)
        if (!uniforms[definition.name]) uniforms[definition.name] = PM.P(definition.def);
      for (const name of Object.keys(uniforms))
        if (!definitions.some((definition) => definition.name === name)) delete uniforms[name];
    };
    this.realm.services.register("inspector", {
      refresh: () => this.panels?.refresh(PowermoveCompat.SECTIONS_PANEL),
      body: null,
      syncs: [],
      // As main: select the layer, then its text field in Properties takes
      // the keyboard with the text selected.
      focusText: (layer) => {
        if (!layer?.id) return;
        this.realm.PM.selectLayers([layer.id]);
        call("ui.request", { kind: "focusText", layer: String(layer.id) });
      },
      copySelectedEffects: () => {
        const { copied } = clipboard("copy");
        if (!copied) return false;
        toast(`Copied ${effects(copied)}`);
        return true;
      },
      clearEffectClipboard: () => void clipboard("clear"),
      pasteCopiedEffects: () => {
        const result = clipboard("paste");
        if (result.empty) return false;
        if (result.reason === "select") toast("Select a layer to paste the effect");
        else if (result.reason === "unavailable") toast("The copied effect is not available");
        else {
          // As main's edit.apply: the realm reads the pasted effects at once.
          this.realm.receive(runtime.engine("kernel.publication", {}));
          toast(`Pasted ${effects(result.pasted)}`);
        }
        return true;
      },
      syncShaderUniforms,
    });
    this.realm.services.register("shaderHooks", { syncShaderUniforms });
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
      // Points per composition pixel on screen.
      get shown() {
        const project = compat.realm.PM.proj;
        return shell.frame && project?.w ? shell.frame[2] / project.w : shell.zoom;
      },
      // The viewer's layer controls (View ▸ Layer Controls).
      get showControls() {
        return shell.showControls !== false;
      },
      set showControls(value) {
        shell.showControls = !!value;
        call("ui.request", { kind: "viewerControls", shown: !!value });
      },
      // Main's source monitor; the native viewer has none, so nothing shows.
      preview: {
        show: () => false,
        toggle: () => undefined,
        clear: () => undefined,
        dispose: () => undefined,
        activeId: null,
        playing: false,
      },
      get stage() {
        return ensureStage();
      },
      get ov() {
        if (!ensureStage()) return null;
        stage.ovUsed = true;
        return stage.ov;
      },
      // The shell lays the viewer out; this re-places the stage and repaints.
      layout: () => {
        placeStage();
        this.overlay();
      },
      attach: () => undefined,
      worldBounds: (layer, time) => this.realm.PM.GL?.bounds?.(layer, time) ?? null,
      // As main: the topmost layer under a window point inside the frame.
      layerAtPoint: (clientX, clientY) => {
        const [s, f] = [shell.stage, shell.frame];
        const project = this.realm.PM.proj;
        if (!s || !f || !project?.w) return null;
        if (clientX < s[0] || clientY < s[1] || clientX > s[0] + s[2] || clientY > s[1] + s[3])
          return null;
        const scale = f[2] / project.w;
        return this.realm.PM.GL.pick((clientX - f[0]) / scale, (clientY - f[1]) / scale, this.realm.PM.time);
      },
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
      // v1's loader: each activation reads the values entered for the keys
      // its manifest declares (apiVersion 2+); a key without one is absent.
      vars: (record) => {
        const keys = (record.manifest?.vars ?? [])
          .map((decl) => decl?.key)
          .filter((key) => typeof key === "string");
        return keys.length ? call("extension.vars", { id: record.id, keys }) : {};
      },
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
    this.loading += 1;
    this.loader
      .boot(this.trusted(records))
      .catch((error) => this.reportFailure(error))
      .finally(() => {
        this.loading -= 1;
        this.publishContributions();
        runtime.engine("extensions.ready", {});
      });
  },
  /** Load passes running: until none is, a definition the project uses may
   * still be registered, so nothing may treat it as not installed. */
  loading: 0,
  trusted: (records) => records.filter((record) => record.trust !== "store"),
  update(records, ids, reason) {
    this.loading += 1;
    this.publishContributions();
    this.loader
      .update(this.trusted(records), ids, reason)
      .catch((error) => this.reportFailure(error))
      .finally(() => {
        this.loading -= 1;
        this.publishContributions();
      });
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
  /** The viewer presented a frame: main's `preview:presented`, then the
   * overlay pass that follows each compositor frame. */
  presented(frame) {
    this.realm.task(() => {
      // A bound presenter shows this frame's pixels before anyone hears of
      // it. Behind the frame, the realm takes the newer publication first;
      // a frame it still can't draw (the project moved past it) is dropped,
      // as playback drops frames.
      if (this.presenting()) {
        if (frame.version > this.realm.revision)
          this.realm.receive(runtime.engine("kernel.publication", {}));
        if (frame.version !== this.realm.revision) return this.overlay();
        this.present(frame);
      }
      this.realm.PM.bus.emit("preview:presented", {
        time: frame.time,
        viewport: this.realm.PM.GL.previewViewport,
        version: frame.version,
        quality: frame.quality,
      });
      this.overlay();
    });
    this.dom?.flush();
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
      settled: this.loading === 0,
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
