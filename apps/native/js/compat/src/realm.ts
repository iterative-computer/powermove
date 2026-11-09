import * as powermove from "./frozen/kernel/editor-helpers";
import { installIcons } from "./frozen/host/icons";
import { HostChannel, HostError } from "./channel";
import {
  DocumentMirror,
  copy,
  diff,
  isProjectView,
  patch,
  rebase,
  reconcile,
} from "./mirror";
import type { ServicesRegistry } from "./services";
import { createKernel } from "./frozen/host/registries";
import {
  extensionAPI,
  type ExtensionHandle,
  type ExtensionOptions,
} from "./extensions";
import { EditGesture } from "./gesture";
import { CompatKeymap } from "./keymap";
import { installModelFactories } from "./legacy/model";
import {
  RecorderCanvas,
  NativeImage,
  type RecorderHost,
  type NativeSurface,
} from "./canvas";
import type { RemoteDOMBridge } from "./remote-dom";
import type {
  Data,
  Disposable,
  EditAPI,
  EditCommand,
  EditMeta,
  EditResult,
  HistoryAPI,
  KernelHost,
  Patch,
  Publication,
  PublicationDelta,
  Reply,
} from "./types";

export interface RealmOptions {
  id: string;
  generation: number;
  initial: Publication;
  host: KernelHost;
  recorder?: RecorderHost;
  dom?: RemoteDOMBridge;
  /** Existing pure-JS PM helpers can install here; native capabilities are explicit. */
  installLegacy?: (PM: Data) => void;
  /** Complete namespace backends supplied by the embedding runtime, not stubbed APIs. */
  capabilities?: Data;
  /** Shared Svelte/compiler modules are supplied by the embedding runtime. */
  runtimeModules?: Data;
}
/** What a base raster was asked for, and whether the override changed it. */
type BaseState = {
  id: string;
  own: boolean;
  scale: number;
  time: number;
  crop: unknown;
  touched: boolean;
};
/** Exports whose frozen projects the realm keeps at once. */
export class CompatRealm {
  readonly PM: Data;
  readonly keymap: CompatKeymap;
  readonly kernel = createKernel();
  readonly services: ServicesRegistry = this.kernel.services;
  readonly history: HistoryAPI;
  readonly edit: EditAPI;
  readonly workspace: {
    current(): Data;
    mutate(fn: (workspace: Data) => unknown, options?: Data): Data;
  };
  readonly channel: HostChannel;
  readonly mirror: DocumentMirror;
  readonly globals: Data;
  private publication: PublicationDelta;
  private workspaceValue: Data;
  private selection: Data;
  private busListeners = new Map<string, Set<(a: any, b: any) => void>>();
  private external = new Map<
    string,
    { undo: () => void; redo: () => void; cleanup?: () => void }
  >();
  private handles = 0;
  private extensions = new Map<string, { api: Data; dispose(): void }>();
  private disposed = false;
  private abandonedMutationStart = false;
  private deferredPublications: PublicationDelta[] = [];
  /** Writes sent and not yet answered; the answer settles where they went. */
  private inFlight?: Patch[];
  private unacknowledgedSelection?: Data;
  private unacknowledgedWorkspace?: Data;
  private domConnection?: { globals: Data; dispose(): void };
  /** Weak: a canvas an extension drops releases its native surface when collected. */
  private canvases = new Set<WeakRef<RecorderCanvas>>();
  private nativeCanvases = new Map<string, WeakRef<RecorderCanvas>>();
  private collected = new FinalizationRegistry<{
    surface: string;
    ref: WeakRef<RecorderCanvas>;
  }>(({ surface, ref }) => {
    this.canvases.delete(ref);
    if (this.nativeCanvases.get(surface) === ref)
      this.nativeCanvases.delete(surface);
    if (this.disposed) return;
    try {
      this.options.recorder?.release(surface);
    } catch {}
  });
  private track(canvas: RecorderCanvas): WeakRef<RecorderCanvas> {
    const ref = new WeakRef(canvas);
    this.canvases.add(ref);
    this.collected.register(canvas, { surface: canvas.surface, ref });
    return ref;
  }

  constructor(private options: RealmOptions) {
    this.keymap = new CompatKeymap(
      (error) => options.host.reportError(error),
      this.kernel,
    );
    // The first document becomes the live tree; the kept options drop it.
    const { document, ...initial } = options.initial;
    this.options = { ...options, initial: initial as Publication };
    this.publication = { ...copy(initial), document: undefined };
    this.mirror = new DocumentMirror(document, { owned: true });
    this.workspaceValue = copy(initial.workspace);
    this.selection = copy(initial.selection);
    this.channel = new HostChannel(
      options.host,
      options.id,
      options.generation,
      (p, reply) => this.accept(p, reply),
      (reply) => this.continueExternal(reply),
    );
    const call = (method: string, args: Data = {}) => this.call(method, args);
    this.history = {
      begin: (label) => call("history.begin", { label }),
      commit: (label) => !!call("history.commit", { label }),
      cancel: () => {
        call("history.cancel");
      },
      do: <T>(label: string, fn: () => T): T => {
        this.history.begin(label);
        let succeeded = false;
        try {
          const result = this.callback(fn);
          succeeded = true;
          return result;
        } catch (error) {
          try {
            this.flush();
          } catch (flushError) {
            options.host.reportError(flushError);
          }
          if (!this.channel.pending) {
            try {
              this.channel.call("history.rollback");
            } catch (rollbackError) {
              options.host.reportError(rollbackError);
            }
          }
          throw error;
        } finally {
          if (succeeded && this.publication.facts.historyBusy)
            this.history.commit(label);
        }
      },
      external: (label, undo, redo, settings = {}) => {
        if (typeof undo !== "function" || typeof redo !== "function")
          return null;
        const handle = `${options.generation}:${++this.handles}`;
        const entry = {
          undo: undo as () => void,
          redo: redo as () => void,
          cleanup:
            typeof settings.cleanup === "function"
              ? (settings.cleanup as () => void)
              : undefined,
        };
        this.external.set(handle, entry);
        options.host.pinGeneration(handle);
        try {
          const id = call("history.external", {
            label,
            handle,
            bytes: Math.max(256, Number(settings.bytes) || 0),
            cleanup: !!entry.cleanup,
          });
          if (id == null) this.retireExternal(handle, false);
          return id;
        } catch (error) {
          if (!this.channel.pending) this.retireExternal(handle, false);
          throw error;
        }
      },
      selection: (before, after) =>
        call("history.selection", { before: copy(before), after: copy(after) }),
      undo: () => !!call("history.undo"),
      redo: () => !!call("history.redo"),
    };
    this.edit = {
      apply: (commands, meta = {}) =>
        call("edit.apply", { commands: copy(commands), meta: copy(meta) }),
      dispatch: (command) => call("edit.dispatch", { commands: copy(command) }),
      begin: (label, meta = {}) => {
        call("edit.begin", { label, meta: copy(meta) });
      },
      commit: (label) => call("edit.commit", { label }),
      cancel: () => !!call("edit.cancel"),
      mutate: (label, action, meta = {}): EditResult => {
        // Refuse before invoking action; submit-time admission is checked again by the kernel.
        const admitted = call("edit.admitMutation");
        if (!admitted.ok) return admitted;
        if (typeof action !== "function")
          return { ok: false, message: "A structural edit action is required" };
        // A callback lease keeps reentrant writes in the same native history entry.
        let lease: EditResult;
        try {
          lease = call("edit.mutateBegin", { label, meta: copy(meta) });
        } catch (error) {
          this.abandonedMutationStart =
            this.channel.pendingMethod === "edit.mutateBegin";
          throw error;
        }
        if (!lease.ok) return lease;
        let result: unknown;
        try {
          result = (action as () => unknown)();
          const ops = this.mirror.take();
          this.inFlight = ops;
          const reply: EditResult = this.channel.call("edit.mutateFinish", {
            lease: lease.data!.lease,
            label,
            meta: copy(meta),
            ops,
          });
          if (reply.ok) {
            if (reply.data) reply.data.result = result;
            else reply.data = { result };
          }
          return reply;
        } catch (error) {
          // Discard only unacknowledged JS writes; the kernel selectively aborts acknowledged owner writes.
          if (!this.channel.pending) {
            this.mirror.discard(this.inFlight);
            this.inFlight = undefined;
            try {
              this.channel.call("edit.mutateAbort", {
                lease: lease.data!.lease,
              });
            } catch (abortError) {
              options.host.reportError(abortError);
            }
          }
          return {
            ok: false,
            message: String(error instanceof Error ? error.message : error),
          };
        }
      },
    };
    this.workspace = {
      current: () => copy(this.workspaceValue, true),
      mutate: (fn, settings = {}) => {
        call("workspace.prepare", { options: copy(settings) });
        let result: Data;
        try {
          this.callback(() => fn(this.workspaceValue));
        } catch (error) {
          try {
            call("workspace.mutate", {
              next: copy(this.workspaceValue, true),
              threw: true,
              options: copy(settings),
            });
          } catch (hostError) {
            options.host.reportError(hostError);
          }
          throw error;
        }
        result = call("workspace.mutate", {
          next: copy(this.workspaceValue, true),
          threw: false,
          options: copy(settings),
        });
        return result;
      },
    };
    const realm = this;
    this.PM = {
      get proj() {
        return realm.mirror.document;
      },
      set proj(value: Data) {
        realm.live();
        realm.mirror.replace(value);
        realm.schedule();
      },
      get sel() {
        return realm.selection;
      },
      set sel(value: Data) {
        reconcile(realm.selection, copy(value));
      },
      hist: Object.assign({}, this.history, {
        busy: () => !!call("history.busy"),
        canUndo: () => !!call("history.canUndo"),
        canRedo: () => !!call("history.canRedo"),
        label: () => call("history.label"),
        clear: () => {
          call("history.clear");
        },
        undoIfTop: (id: string) => !!call("history.undoIfTop", { id }),
        rollback: () => !!call("history.rollback"),
        export: () => call("history.export"),
        import: (saved: Data) => call("history.import", { saved: copy(saved) }),
      }),
      Edit: this.edit,
      WS: {
        get current() {
          return realm.workspaceValue;
        },
        set current(value: Data) {
          realm.call("workspace.replace", { next: copy(value, true) });
        },
        mutate: this.workspace.mutate,
      },
      Kernel: this.kernel,
      bus: {
        on: (name: string, fn: (a: any, b: any) => void) => this.on(name, fn),
        off: (name: string, fn: (a: any, b: any) => void) =>
          this.busListeners.get(name)?.delete(fn),
        emit: (name: string, a?: any, b?: any) => this.busEmit(name, a, b),
      },
      touch: () => {
        call("document.touch");
      },
      replaceProject: (document: Data) => {
        realm.mirror.replace(document);
        realm.schedule();
      },
      curComp: () => this.PM.proj,
      L: (id: string) =>
        this.PM.proj.layers?.find((layer: Data) => layer.id === id) ?? null,
      byName: (name: string) =>
        this.PM.proj.layers?.find((layer: Data) => layer.name === name) ?? null,
      P: (v: any, extra: Data = {}) => ({ v, kf: [], expr: null, ...extra }),

    };
    let uid = 0;
    this.PM.uid = (prefix = "l") =>
      `${options.id}:${options.generation}:${prefix}:${++uid}`;
    this.PM.clamp = (value: number, min: number, max: number) =>
      Math.max(min, Math.min(max, value));
    this.PM.Ease = options.capabilities?.ease ?? {
      handles: (ease: string) => options.host.query("ease.handles", { ease }),
    };
    this.PM.version = "1.0.0";
    installModelFactories(this.PM);
    this.PM.layerDefinition = (id: string) => this.kernel.layerTypes.get(id);
    // As main: a new canvas, or `target` resized and filled, returned.
    this.PM.renderFrameTo = (
      time: number,
      width: number,
      height: number,
      options: Data = {},
      target?: any,
    ) => {
      const frame = this.renderQuery("render.renderFrameTo", [
        time,
        width,
        height,
        { mblur: options?.mblur, mbSamples: options?.mbSamples },
      ]);
      if (!target) return frame;
      if (target.width !== frame.width) target.width = frame.width;
      if (target.height !== frame.height) target.height = frame.height;
      target.getContext("2d").drawImage(frame, 0, 0);
      return target;
    };
    this.PM.Export = {
      snapshot: (...args: any[]) => this.renderQuery("render.snapshot", args),
      snapshotAsync: async (...args: any[]) =>
        this.renderQuery("render.snapshot", args),
    };
    Object.assign(this.PM, options.capabilities?.PM ?? {});
    this.bridgeEvents();
    options.installLegacy?.(this.PM);
    // Extensions also replace `PM.raster` itself (and put it back).
    let raster: unknown = this.serviceRaster;
    Object.defineProperty(this.PM, "raster", {
      configurable: true,
      enumerable: true,
      get: () => raster,
      set: (next: unknown) => {
        raster = next;
        this.rasterChanged();
      },
    });
    // A `raster` registration changes what the engine asks this realm for.
    const register = this.services.register.bind(this.services);
    this.services.register = ((name: string, implementation: unknown) => {
      const registration = register(name, implementation);
      if (name !== "raster") return registration;
      this.rasterChanged();
      return {
        dispose: () => {
          registration.dispose();
          this.rasterChanged();
        },
      };
    }) as typeof this.services.register;
    this.services.register("raster", this.baseRaster);
    this.domConnection = options.dom?.connect({
      realm: options.id,
      generation: options.generation,
      createCanvas: (w, h) => this.createCanvas(w, h),
      task: (run) => this.task(run),
    });
    const window = this.domConnection?.globals.window;
    if (window && typeof this.PM.icon !== "function")
      installIcons(this.PM, window);
    if (window)
      this.kernel.installKeyListener(
        (id: string, args: any[]) => this.keymap.run(id, args),
        window,
      );
    this.globals = {
      ...(this.domConnection?.globals ?? {}),
      PM: this.PM,
      __powermove_runtime: { ...options.runtimeModules, powermove },
    };
  }
  private live(): void {
    if (this.disposed)
      throw new HostError("Disposed", "Compatibility realm is disposed");
  }
  private callback<T>(fn: () => T): T {
    return fn();
  }
  private accept(publication: PublicationDelta, reply: Reply): void {
    if (publication.revision < this.publication.revision)
      throw new HostError(
        "StalePublication",
        "Host publication moved backwards",
      );
    // Reads carry facts only; nothing here may walk the document for them.
    const sent = this.inFlight;
    const moved =
      !!publication.document ||
      !!publication.ops?.length ||
      !!reply.error ||
      !!sent;
    // Writes made after the flush that sent `sent`. The channel is synchronous,
    // so this is the rare path, and the only one that walks the whole tree.
    const slow = moved && this.mirror.writing;
    const documentBase = slow
      ? sent
        ? patch(this.mirror.acknowledged, sent)
        : copy(this.mirror.acknowledged)
      : undefined;
    const later =
      slow && (sent || !reply.error) ? diff(documentBase, this.mirror.raw) : [];
    const selectionBase =
      this.unacknowledgedSelection ?? this.publication.selection;
    const selected =
      this.unacknowledgedSelection || !reply.error
        ? diff(selectionBase, this.selection)
        : [];
    const workspaceBase =
      this.unacknowledgedWorkspace ?? this.publication.workspace;
    const workspaceChanges =
      this.unacknowledgedWorkspace || !reply.error
        ? diff(workspaceBase, copy(this.workspaceValue, true))
        : [];
    this.inFlight = undefined;
    this.unacknowledgedSelection = undefined;
    this.unacknowledgedWorkspace = undefined;
    this.adopt(publication);
    if (slow) {
      const document = this.mirror.acknowledged;
      this.mirror.accept(document);
      if (later.length)
        this.mirror.restore(
          patch(document, rebase(later, documentBase, document)),
        );
    } else if (moved)
      this.mirror.settle(
        publication.document
          ? [{ path: [] }]
          : [...((publication.ops ?? []) as Patch[]), ...(sent ?? [])],
      );
    reconcile(
      this.selection,
      selected.length
        ? patch(this.publication.selection, selected)
        : this.publication.selection,
    );
    reconcile(
      this.workspaceValue,
      workspaceChanges.length
        ? patch(this.publication.workspace, workspaceChanges)
        : this.publication.workspace,
    );
    for (const event of publication.events ?? [])
      this.emit(event.name, event.value);
    for (const handle of reply.retired ?? []) {
      try {
        this.retireExternal(handle, true);
      } catch (error) {
        this.options.host.reportError(error);
      }
    }
    if (publication.notice) this.emit("kernel:notice", publication.notice);
  }
  /** A publication carries a whole document, ops against the last one, or
   * neither; selection and workspace are likewise only sent when they moved. */
  private adopt(publication: PublicationDelta): void {
    if (publication.document) this.mirror.setAcknowledged(publication.document);
    else if (publication.ops?.length) this.mirror.applyOps(publication.ops);
    const { document: _, ops: __, ...rest } = publication;
    this.publication = {
      ...this.publication,
      ...copy(rest),
      selection: publication.selection ?? this.publication.selection,
      workspace: publication.workspace ?? this.publication.workspace,
    };
  }
  /** Host mailbox publication while local raw writes are still pending. */
  receive(publication: PublicationDelta): void {
    this.live();
    // A mailbox may overtake an acknowledgement lost in transport. Resolve that
    // immutable request before treating its acknowledged writes as new local intent.
    if (this.channel.pending) {
      this.deferredPublications.push(copy(publication));
      return;
    }
    if (publication.revision < this.publication.revision)
      throw new HostError(
        "StalePublication",
        "Host publication moved backwards",
      );
    const selected = diff(this.publication.selection, this.selection);
    const workspaceChanges = diff(
      this.publication.workspace,
      copy(this.workspaceValue, true),
    );
    let failed = false,
      failure: unknown;
    const moved = !!publication.document || !!publication.ops?.length;
    // Unflushed local writes rebase onto the native change: the rare path.
    const previous =
      moved && this.mirror.writing ? copy(this.mirror.acknowledged) : undefined;
    this.adopt(publication);
    if (previous)
      try {
        this.mirror.receive(this.mirror.acknowledged, previous);
      } catch (error) {
        failed = true;
        failure = error;
      }
    else if (moved)
      this.mirror.settle(
        publication.document ? [{ path: [] }] : (publication.ops as Patch[]),
      );
    reconcile(
      this.selection,
      selected.length
        ? patch(this.publication.selection, selected)
        : this.publication.selection,
    );
    reconcile(
      this.workspaceValue,
      workspaceChanges.length
        ? patch(this.publication.workspace, workspaceChanges)
        : this.publication.workspace,
    );
    for (const event of publication.events ?? [])
      this.emit(event.name, event.value);
    if (failed) throw failure;
  }

  private call(method: string, args: Data = {}): any {
    this.live();
    this.flush();
    if (method === "workspace.mutate" || method === "workspace.replace")
      this.unacknowledgedWorkspace = copy(args.next);
    return this.channel.call(method, args);
  }
  flush(): void {
    this.live();
    if (this.channel.pending) {
      const recovered = this.channel.retry();
      if (this.abandonedMutationStart) {
        this.abandonedMutationStart = false;
        if (recovered?.ok && recovered.data?.lease)
          this.channel.call("edit.mutateAbort", {
            lease: recovered.data.lease,
          });
      }
    }
    while (this.deferredPublications.length)
      this.receive(this.deferredPublications.shift()!);
    let ops;
    try {
      ops = this.mirror.take();
    } catch (error) {
      this.mirror.discard();
      throw error;
    }
    if (ops.length) {
      this.inFlight = ops;
      this.channel.call("document.directWrite", { ops });
    }
    if (diff(this.publication.selection, this.selection).length) {
      this.unacknowledgedSelection = copy(this.selection);
      this.channel.call("selection.set", { selection: copy(this.selection) });
    }
  }
  private schedule(): void {
    this.options.host.scheduleFlush(() => {
      if (this.disposed) return;
      try {
        this.flush();
      } catch (error) {
        this.options.host.reportError(error);
      }
    });
  }
  /** Every host→JS entry and microtask checkpoint must use this boundary. */
  task<T>(run: () => T): T {
    this.live();
    try {
      return run();
    } finally {
      this.schedule();
    }
  }
  private on(name: string, listener: (a: any, b: any) => void): () => void {
    const listeners = this.busListeners.get(name) ?? new Set();
    listeners.add(listener);
    this.busListeners.set(name, listeners);
    return () => {
      listeners.delete(listener);
    };
  }
  private busEmit(name: string, a: any, b: any): void {
    for (const listener of [...(this.busListeners.get(name) ?? [])]) {
      try {
        listener(a, b);
      } catch (error) {
        this.options.host.reportError(error);
      }
    }
  }
  /** The reference's one subscription per legacy topic (kernel/install.ts). */
  private bridgeEvents(): void {
    const events = this.kernel.events;
    const on = (name: string, fn: (...args: any[]) => void) =>
      this.on(name, fn);
    const kinds: Data = {
      layers: "structure",
      project: "project",
      assets: "assets",
      library: "library",
      history: "history",
    };
    for (const [name, kind] of Object.entries(kinds))
      on(name, () => events.emit("project:changed", { kind }));
    on("sel", () => events.emit("selection", this.selectionSnapshot()));
    on("time", (time: number) =>
      events.emit("time", Number(time ?? this.PM.time ?? 0)),
    );
    on("transport", () =>
      events.emit("transport", { playing: !!this.PM.playing }),
    );
    on("fonts", (families: string[]) => events.emit("fonts", families));
    on("layout", () => events.emit("layout", undefined));
    on("quality", () => events.emit("layout", undefined));
    on("draw", () => events.emit("invalidate", "render"));
    on("draw:timeline", () => events.emit("invalidate", "timeline"));
    on("draw:ui", () => events.emit("invalidate", "ui"));
    on("draw:status", () => events.emit("invalidate", "status"));
    on("preview:presented", (frame: Data) =>
      events.emit("frame:rendered", {
        time: Number(frame?.time ?? 0),
        viewport: frame?.viewport ?? null,
        version: frame?.version,
        quality: Number(frame?.quality ?? 1),
      }),
    );
    on("overlay", () => events.emit("overlay", undefined));
  }
  /** Native publications name either a legacy topic or a stable kernel event. */
  private emit(name: string, value: any): void {
    if (name.includes(":")) this.kernel.events.emit(name, value);
    this.busEmit(name, value, undefined);
  }
  /** A native undo reached this realm's closure outside any realm call. */
  runExternal(next: NonNullable<Reply["continuation"]>): void {
    this.continueExternal({ continuation: next });
  }
  private continueExternal(reply: Reply): any {
    const next = reply.continuation!,
      closure = this.external.get(next.handle);
    let error: unknown,
      failed = false;
    try {
      if (!closure)
        throw new HostError(
          "MissingGeneration",
          "External history generation is unavailable",
        );
      if (next.phase === "cleanup") this.retireExternal(next.handle, true);
      else this.callback(closure[next.phase]);
      this.flush();
    } catch (caught) {
      error = caught;
      failed = true;
    }
    const result = this.channel.call("history.externalDone", {
      token: next.token,
      failed,
    });
    if (failed) throw error;
    return result;
  }
  /** Mailbox cleanup is allowed after extension disposal, while history pins remain. */
  retireExternal(handle: string, cleanup = true): void {
    const closure = this.external.get(handle);
    if (!closure) return;
    this.external.delete(handle);
    try {
      if (cleanup) closure.cleanup?.();
    } finally {
      this.options.host.unpinGeneration(handle);
    }
  }
  createCanvas(width?: number, height?: number): RecorderCanvas {
    this.live();
    if (!this.options.recorder)
      throw new HostError(
        "Unavailable",
        "Native recorder-canvas host is not attached",
      );
    const canvas = new RecorderCanvas(this.options.recorder, width, height);
    this.track(canvas);
    return canvas;
  }
  /** An image the host decoded (`image.load`), drawable on this realm's canvases. */
  createImage(
    surface: string,
    width: number,
    height: number,
    vector: boolean,
  ): NativeImage {
    this.live();
    if (!this.options.recorder)
      throw new HostError(
        "Unavailable",
        "Native recorder-canvas host is not attached",
      );
    return new NativeImage(this.options.recorder, surface, width, height, vector);
  }
  /** The kernel revision this realm's project mirrors. */
  get revision(): number {
    return this.publication.revision;
  }
  private renderQuery(method: string, args: any[]): any {
    this.flush();
    const query = (name: string, values: any[]) =>
      this.options.host.query(name, {
        args: values,
        revision: this.publication.revision,
        realm: this.options.id,
        generation: this.options.generation,
      });
    let value = query(method, args);
    // The capture draws this project, whose overrides live here: while it
    // waits, it asks this realm for them, and they are answered inline.
    while (value?.kind === "raster-asks") {
      for (const asked of value.batches ?? [])
        this.options.capabilities?.answerRasters?.(asked.batch, asked.needs, asked.ask ?? {});
      value = query("render.continue", []);
    }
    if (value?.kind !== "recorder-surface") return value;
    return this.adoptSurface(value as NativeSurface);
  }
  /** A recorder surface the host made, as this realm's one canvas for it. */
  private adoptSurface(surface: NativeSurface): RecorderCanvas {
    let canvas = this.nativeCanvases.get(surface.surface)?.deref();
    if (!canvas) {
      if (!this.options.recorder)
        throw new HostError(
          "Unavailable",
          "Recorder surface returned without a recorder host",
        );
      canvas = new RecorderCanvas(
        this.options.recorder,
        surface.width,
        surface.height,
        surface.surface,
      );
      this.nativeCanvases.set(surface.surface, this.track(canvas));
    }
    return canvas;
  }
  /** Base rasters handed out, with what they were asked for. */
  private bases = new WeakMap<object, BaseState>();
  /**
   * What an override answer is for, while the realm answers it: the need's
   * composition (a nested layer's base paints in its own composition).
   */
  rasterContext: { composition?: string; layer?: object } | null = null;
  /**
   * Whether `value` is the base raster of exactly `need`'s layer, frame, scale
   * and crop, untouched: an override that returns it passes the layer through.
   * A replacement layer object, another frame, a canvas drawn into or a
   * geometry field changed all make it the override's own raster.
   */
  isBaseRaster(
    value: unknown,
    need: { layer: string; seconds: number; scale: number; crop?: Data | null },
  ): boolean {
    if (typeof value !== "object" || value === null) return false;
    const state = this.bases.get(value);
    return (
      !!state &&
      state.own &&
      !state.touched &&
      state.id === need.layer &&
      state.time === need.seconds &&
      state.scale === need.scale &&
      JSON.stringify(state.crop ?? null) === JSON.stringify(need.crop ?? null)
    );
  }
  /** The composition holding layer `id` (null: the open one). */
  private compositionOf(id: string): string | null {
    const project = this.PM.proj;
    if ((project?.layers ?? []).some((layer: Data) => layer?.id === id)) return null;
    const comps = project?.comps;
    for (const [key, comp] of Object.entries(comps ?? {}) as [string, Data][])
      if ((comp?.layers ?? []).some((layer: Data) => layer?.id === id))
        return String(comp.id ?? key);
    return null;
  }
  /**
   * `render.raster(L, scale, T, uploaded, crop)`'s base: the layer as the
   * engine paints it, with today's geometry (`cv`, `w`, `h`, `anchorX`,
   * `anchorY`, `selection`, `fontOffset`, `uv`), each field writable as on
   * main. Lazy: an override that passes a layer through never makes the
   * engine paint it here.
   */
  private baseRaster = (
    layer: any,
    scale = 1,
    time = this.PM.time,
    _uploaded?: unknown,
    crop?: { x: number; y: number; width: number; height: number },
  ): any => {
    if (!layer || typeof layer !== "object") return null;
    const context = this.rasterContext;
    // The project's own record: a view of it, the exact layer this answer
    // is for.
    const own =
      isProjectView(layer) || layer === context?.layer;
    const id = String(layer.id ?? "");
    const composition = context?.composition ?? this.compositionOf(id);
    const spec: Data = {
      id,
      ...(composition ? { composition } : {}),
      // The project's own layer paints from the document; any other object
      // (`{ ...L, d: { ...L.d, text } }`) paints as given.
      ...(own ? {} : { content: JSON.parse(JSON.stringify(layer)) }),
    };
    let loaded: Data | null | undefined;
    const load = () => {
      if (loaded !== undefined) return loaded;
      this.flush();
      const value = this.options.host.query("render.raster", {
        args: [spec, scale, time, crop ?? null],
        revision: this.publication.revision,
        realm: this.options.id,
        generation: this.options.generation,
      });
      if (!value || value.kind !== "recorder-surface") return (loaded = null);
      const cv = this.adoptSurface(value as NativeSurface);
      return (loaded = {
        cv,
        w: value.w,
        h: value.h,
        anchorX: value.anchorX,
        anchorY: value.anchorY,
        selection: value.selection ?? undefined,
        fontOffset: value.fontOffset ?? undefined,
        uv: value.uv ?? [0, 0, 1, 1],
      });
    };
    const state: BaseState = { id, own, scale, time, crop: crop ?? null, touched: false };
    const written: Data = {};
    // Today's cache key: the same layer, frame, scale and crop paint the same.
    const surface: Data = {
      key: JSON.stringify([
        id,
        this.publication.revision,
        time,
        scale,
        crop ?? null,
        spec.content ?? null,
      ]),
    };
    for (const key of ["cv", "w", "h", "anchorX", "anchorY", "selection", "fontOffset", "uv"])
      Object.defineProperty(surface, key, {
        enumerable: true,
        configurable: true,
        get: () => {
          // A canvas handed out may be drawn into: no longer the base.
          if (key === "cv") state.touched = true;
          return key in written ? written[key] : load()?.[key];
        },
        set: (value: unknown) => {
          state.touched = true;
          written[key] = value;
        },
      });
    this.bases.set(surface, state);
    return surface;
  };
  /** Changes whenever what draws `raster` changes (a wrapper registered,
   * removed or reloaded; `PM.raster` replaced): earlier answers are stale. */
  private rasterEpoch = 0;
  private rasterTop: unknown[] = [];
  /** Tells the host whether an extension wraps the `raster` service, and
   * which generation of wrappers answers. */
  private rasterChanged(): void {
    const top = [this.services.get("raster"), this.PM.raster];
    const on = top[0] !== this.baseRaster || top[1] !== this.serviceRaster;
    if (top[0] === this.rasterTop[0] && top[1] === this.rasterTop[1]) return;
    this.rasterTop = top;
    this.rasterEpoch += 1;
    if (!on && !this.overriding) return;
    this.overriding = on;
    this.options.capabilities?.rasterProducing?.(on, this.rasterEpoch);
  }
  private overriding = false;
  get rasterGeneration(): number {
    return this.rasterEpoch;
  }
  /** `PM.raster` as the realm defines it: the `raster` service's top. */
  private serviceRaster = (...args: any[]): any =>
    (this.services.get<Function>("raster") ?? this.baseRaster)(...args);
  /** Metadata only: safe for headless inspection without exposing vars/user data. */
  inspect() {
    return {
      realm: this.options.id,
      generation: this.options.generation,
      revision: this.publication.revision,
      lifecycle: this.disposed ? "retired" : "active",
      pendingMethod: this.channel.pendingMethod ?? null,
      deferredPublications: this.deferredPublications.length,
      extensions: [...this.extensions.keys()],
      externalEntries: this.external.size,
    };
  }
  selectionSnapshot(): Data {
    return copy(this.selection);
  }
  reportError(error: unknown): void {
    this.options.host.reportError(error);
  }
  extension(
    id: string,
    apiVersion: 1 | 2 | 3 = 1,
    settings: ExtensionOptions = {},
  ): ExtensionHandle {
    this.live();
    if (!id || this.extensions.has(id))
      throw new Error("Extension id is empty or already active");
    const handle = extensionAPI(
      this,
      id,
      apiVersion,
      settings,
      this.options.capabilities ?? {},
      this.options.runtimeModules ?? {},
    );
    const dispose = handle.dispose;
    handle.dispose = () => {
      dispose();
      this.extensions.delete(id);
    };
    this.extensions.set(id, handle);
    return handle;
  }
  installGlobals(target: Data): () => void {
    const previous = new Map<string, PropertyDescriptor | undefined>();
    for (const [key, value] of Object.entries(this.globals)) {
      previous.set(key, Object.getOwnPropertyDescriptor(target, key));
      Object.defineProperty(target, key, {
        configurable: true,
        writable: true,
        value,
      });
    }
    return () => {
      for (const [key, descriptor] of previous) {
        if (descriptor) Object.defineProperty(target, key, descriptor);
        else delete target[key];
      }
    };
  }
  dispose(): void {
    if (this.disposed) return;
    this.flush();
    this.channel.call("realm.failed"); // Kernel seals acknowledged edits; never rolls them away.
    for (const extension of [...this.extensions.values()]) extension.dispose();
    for (const handle of [...this.external.keys()]) {
      try {
        this.retireExternal(handle, true);
      } catch (error) {
        this.options.host.reportError(error);
      }
    }
    for (const ref of this.canvases) ref.deref()?.dispose();
    this.domConnection?.dispose();
    this.kernel.dispose();
    this.busListeners.clear();
    this.disposed = true;
  }
}
export { EditGesture };
