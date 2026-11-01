import * as powermove from "./frozen/kernel/editor-helpers";
import { HostChannel, HostError } from "./channel";
import { DocumentMirror, copy, diff, patch, rebase, reconcile } from "./mirror";
import { createServicesRegistry } from "./services";
import { EditGesture } from "./gesture";
import { CompatKeymap } from "./keymap";
import { installModelFactories } from "./legacy/model";
import {
  RecorderCanvas,
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
  Publication,
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
export class CompatRealm {
  readonly PM: Data;
  readonly keymap: CompatKeymap;
  readonly services = createServicesRegistry();
  readonly history: HistoryAPI;
  readonly edit: EditAPI;
  readonly workspace: {
    current(): Data;
    mutate(fn: (workspace: Data) => unknown, options?: Data): Data;
  };
  readonly channel: HostChannel;
  readonly mirror: DocumentMirror;
  readonly globals: Data;
  private publication: Publication;
  private workspaceValue: Data;
  private selection: Data;
  private listeners = new Map<string, Set<(value: any) => void>>();
  private external = new Map<
    string,
    { undo: () => void; redo: () => void; cleanup?: () => void }
  >();
  private handles = 0;
  private extensions = new Map<string, { api: Data; dispose(): void }>();
  private disposed = false;
  private unacknowledgedDocument?: Data;
  private unacknowledgedSelection?: Data;
  private unacknowledgedWorkspace?: Data;
  private domConnection?: { globals: Data; dispose(): void };
  private canvases = new Set<RecorderCanvas>();
  private nativeCanvases = new Map<string, RecorderCanvas>();

  constructor(private options: RealmOptions) {
    this.keymap = new CompatKeymap((error) => options.host.reportError(error));
    this.publication = copy(options.initial);
    this.mirror = new DocumentMirror(options.initial.document);
    this.workspaceValue = copy(options.initial.workspace);
    this.selection = copy(options.initial.selection);
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
        const lease = call("edit.mutateBegin", { label, meta: copy(meta) });
        if (!lease.ok) return lease;
        let result: unknown;
        try {
          result = (action as () => unknown)();
          const ops = this.mirror.pending();
          this.unacknowledgedDocument = this.mirror.snapshot();
          const reply: EditResult = this.channel.call("edit.mutateFinish", {
            lease: lease.data.lease,
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
          this.mirror.accept(this.publication.document);
          if (!this.channel.pending) {
            try {
              this.channel.call("edit.mutateAbort", {
                lease: lease.data.lease,
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
      current: () => copy(this.workspaceValue),
      mutate: (fn, settings = {}) => {
        call("workspace.prepare", { options: copy(settings) });
        let result: Data;
        try {
          this.callback(() => fn(this.workspaceValue));
        } catch (error) {
          try {
            call("workspace.mutate", {
              next: copy(this.workspaceValue),
              threw: true,
              options: copy(settings),
            });
          } catch (hostError) {
            options.host.reportError(hostError);
          }
          throw error;
        }
        result = call("workspace.mutate", {
          next: copy(this.workspaceValue),
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
          realm.call("workspace.replace", { next: copy(value) });
        },
        mutate: this.workspace.mutate,
      },
      Kernel: {
        services: this.services,
        commands: this.keymap.commands,
        keybindings: this.keymap.keybindings,
        bind: this.keymap.bind.bind(this.keymap),
        unbind: this.keymap.unbind.bind(this.keymap),
        bindingsFor: this.keymap.bindingsFor.bind(this.keymap),
      },
      bus: {
        on: (name: string, fn: (value: any) => void) => this.on(name, fn),
        emit: (name: string, value?: any) => this.emit(name, value),
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
      raster: (...args: any[]) =>
        (this.services.get<Function>("raster") ?? this.baseRaster)(...args),
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
    this.PM.renderFrameTo = (...args: any[]) =>
      this.renderQuery("render.renderFrameTo", args);
    this.PM.Export = {
      snapshot: (...args: any[]) => this.renderQuery("render.snapshot", args),
      snapshotAsync: async (...args: any[]) =>
        this.renderQuery("render.snapshot", args),
    };
    Object.assign(this.PM, options.capabilities?.PM ?? {});
    options.installLegacy?.(this.PM);
    this.services.register("raster", this.baseRaster);
    this.domConnection = options.dom?.connect({
      realm: options.id,
      generation: options.generation,
      createCanvas: (w, h) => this.createCanvas(w, h),
      task: (run) => this.task(run),
    });
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
  private accept(publication: Publication, reply: Reply): void {
    if (publication.revision < this.publication.revision)
      throw new HostError(
        "StalePublication",
        "Host publication moved backwards",
      );
    const documentBase =
      this.unacknowledgedDocument ?? this.publication.document;
    const later = !reply.error
      ? diff(documentBase, this.mirror.snapshot())
      : [];
    const selectionBase =
      this.unacknowledgedSelection ?? this.publication.selection;
    const selected = !reply.error ? diff(selectionBase, this.selection) : [];
    const workspaceBase =
      this.unacknowledgedWorkspace ?? this.publication.workspace;
    const workspaceChanges = !reply.error
      ? diff(workspaceBase, this.workspaceValue)
      : [];
    this.unacknowledgedDocument = undefined;
    this.unacknowledgedSelection = undefined;
    this.unacknowledgedWorkspace = undefined;
    this.publication = copy(publication);
    this.mirror.accept(publication.document);
    if (later.length)
      this.mirror.restore(
        patch(
          publication.document,
          rebase(later, documentBase, publication.document),
        ),
      );
    reconcile(
      this.selection,
      selected.length
        ? patch(publication.selection, selected)
        : publication.selection,
    );
    reconcile(
      this.workspaceValue,
      workspaceChanges.length
        ? patch(publication.workspace, workspaceChanges)
        : publication.workspace,
    );
    for (const event of publication.events ?? [])
      this.emit(event.name, event.value);
    for (const handle of reply.retired ?? []) this.retireExternal(handle, true);
    if (publication.notice) this.emit("kernel:notice", publication.notice);
  }
  /** Host mailbox publication while local raw writes are still pending. */
  receive(publication: Publication): void {
    this.live();
    if (publication.revision < this.publication.revision)
      throw new HostError(
        "StalePublication",
        "Host publication moved backwards",
      );
    const selected = diff(this.publication.selection, this.selection);
    const workspaceChanges = diff(
      this.publication.workspace,
      this.workspaceValue,
    );
    let failed = false,
      failure: unknown;
    try {
      this.mirror.receive(publication.document);
    } catch (error) {
      failed = true;
      failure = error;
    }
    this.publication = copy(publication);
    reconcile(
      this.selection,
      selected.length
        ? patch(publication.selection, selected)
        : publication.selection,
    );
    reconcile(
      this.workspaceValue,
      workspaceChanges.length
        ? patch(publication.workspace, workspaceChanges)
        : publication.workspace,
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
    if (this.channel.pending) this.channel.retry();
    let ops;
    try {
      ops = this.mirror.pending();
    } catch (error) {
      this.mirror.accept(this.publication.document);
      throw error;
    }
    if (ops.length) {
      this.unacknowledgedDocument = this.mirror.snapshot();
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
  private on(name: string, listener: (value: any) => void): () => void {
    const listeners = this.listeners.get(name) ?? new Set();
    listeners.add(listener);
    this.listeners.set(name, listeners);
    return () => {
      listeners.delete(listener);
    };
  }
  private emit(name: string, value: any): void {
    for (const listener of [...(this.listeners.get(name) ?? [])])
      listener(value);
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
    this.canvases.add(canvas);
    return canvas;
  }
  private renderQuery(method: string, args: any[]): any {
    this.flush();
    const value = this.options.host.query(method, {
      args,
      revision: this.publication.revision,
      realm: this.options.id,
      generation: this.options.generation,
    });
    if (value?.kind !== "recorder-surface") return value;
    const surface = value as NativeSurface;
    let canvas = this.nativeCanvases.get(surface.surface);
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
      this.nativeCanvases.set(surface.surface, canvas);
      this.canvases.add(canvas);
    }
    return canvas;
  }
  private baseRaster = (...args: any[]): any =>
    this.renderQuery("render.raster", args);
  extension(
    id: string,
    apiVersion: 1 | 2 | 3 = 1,
  ): { api: Data; dispose(): void } {
    this.live();
    if (!id || this.extensions.has(id))
      throw new Error("Extension id is empty or already active");
    const owned: Disposable[] = [];
    let active = true;
    const own = (d: Disposable): Disposable => {
      owned.push(d);
      return d;
    };
    const check = () => {
      this.live();
      if (!active)
        throw new HostError("DisposedExtension", "Extension is disposed");
    };
    const guard = <T extends (...args: any[]) => any>(
      fn: T,
      label: string,
      afterDispose = false,
    ): T =>
      ((...args: any[]) => {
        if (!active && !afterDispose) return undefined;
        const fail = (error: unknown) => {
          this.options.host.reportError({ extensionId: id, label, error });
          return undefined;
        };
        try {
          const result = fn(...args);
          return result && typeof result.then === "function"
            ? Promise.resolve(result).catch(fail)
            : result;
        } catch (error) {
          return fail(error);
        }
      }) as T;
    const api: Data = {
      ...this.options.capabilities,
      id,
      manifest: this.options.capabilities?.manifest ?? {
        id,
        name: id,
        version: "0.0.0",
        apiVersion,
      },
      onDispose: (fn: () => void) => {
        check();
        own({ dispose: guard(fn, "onDispose handler", true) });
      },
      apiVersion, // Report the declared contract version; see N3 decision in compat-realm.md.
      host: { pm: this.PM },
      history: this.history,
      edit: this.edit,
      workspace: this.workspace,
      services: {
        get: <T>(name: string) => {
          this.live();
          return this.services.get<T>(name);
        },
        register: <T>(name: string, value: T) => {
          check();
          return own(this.services.register(name, value));
        },
      },
      commands: {
        register: (
          definition: Data & { id: string; run: (...args: any[]) => any },
        ) => {
          check();
          if (
            !definition ||
            !definition.id ||
            typeof definition.run !== "function"
          )
            throw new Error("commands.register requires id and run()");
          return own(
            this.keymap.commands.register(id, {
              ...definition,
              run: guard(definition.run, `command ${definition.id}`),
            }),
          );
        },
        run: (command: string, ...args: any[]) => {
          this.live();
          return this.keymap.run(command, args);
        },
        has: (command: string) => this.keymap.commands.has(command),
        list: () => this.keymap.commands.list(),
      },
      keybindings: {
        bind: (definition: Data) => {
          check();
          return own(this.keymap.bind(id, definition));
        },
        unbind: (key: string, all = false) => {
          check();
          this.keymap.unbind(id, key, all);
        },
      },
      events: {
        emit: (name: string, value: any) => this.emit(name, value),
        on: (name: string, fn: (value: any) => void) => {
          check();
          return own({ dispose: this.on(name, guard(fn, `event ${name}`)) });
        },
      },
      project: {
        get: () => this.PM.proj,
        latest: () => this.PM.proj,
        revision: () => Number(this.PM.proj.revision ?? 0),
        apply: (commands: EditCommand | EditCommand[], meta: EditMeta = {}) =>
          this.edit.apply(commands, { ...meta, origin: `ext:${id}` }),
        selection: () => copy(this.selection),
        undo: this.history.undo,
        redo: this.history.redo,
        snapshot: async (time?: number, maxWidth = 480) =>
          this.renderQuery("render.snapshot", [time, maxWidth]),
      },
      model: {
        P: this.PM.P,
        KF: this.PM.KF,
        mkLayer: this.PM.mkLayer,
        mkMask: this.PM.mkMask,
        mkProject: this.PM.mkProject,
        cloneLayer: this.PM.cloneLayer,
        normalizeFill: this.PM.normalizeFill,
        CH: this.PM.CH,
        BLENDS: this.PM.BLENDS,
        TYPE_META: this.PM.TYPE_META,
        MASK_SHAPES: this.PM.MASK_SHAPES,
        layer: this.PM.L,
        byName: this.PM.byName,
        curComp: this.PM.curComp,
      },
      render: {
        raster: (...args: any[]) => this.PM.raster(...args),
        renderFrameTo: (...args: any[]) =>
          this.renderQuery("render.renderFrameTo", args),
        snapshot: (...args: any[]) => this.renderQuery("render.snapshot", args),
        rasterAsync: async (...args: any[]) => this.PM.raster(...args),
        renderFrameToAsync: async (...args: any[]) =>
          this.renderQuery("render.renderFrameTo", args),
        snapshotAsync: async (...args: any[]) =>
          this.renderQuery("render.snapshot", args),
      },
    };
    // Preserve backend accessors and all namespace members that this adapter does not override.
    for (const [name, backend] of Object.entries(
      this.options.capabilities ?? {},
    )) {
      if (name === "PM") continue;
      if (
        backend &&
        typeof backend === "object" &&
        api[name] &&
        typeof api[name] === "object"
      ) {
        api[name] = Object.defineProperties(
          {},
          {
            ...Object.getOwnPropertyDescriptors(backend),
            ...Object.getOwnPropertyDescriptors(api[name]),
          },
        );
      }
    }
    const handle = {
      api,
      dispose: () => {
        if (!active) return;
        active = false;
        for (const d of owned.reverse()) {
          try {
            d.dispose();
          } catch (error) {
            this.options.host.reportError(error);
          }
        }
        this.extensions.delete(id); // External history closures/generation pins deliberately survive.
      },
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
    for (const canvas of this.canvases) canvas.dispose();
    this.domConnection?.dispose();
    this.services.clear();
    this.listeners.clear();
    this.disposed = true;
  }
}
export { EditGesture };
