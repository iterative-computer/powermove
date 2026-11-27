/** Linux kernel-channel double. Not shipped or claimed to implement pm-kernel. */
import { copy, diff, equal, patch } from "../src/mirror";
import type {
  Data,
  KernelHost,
  Patch,
  Publication,
  Reply,
  Request,
} from "../src/types";
interface Entry {
  id: string;
  label: string;
  before?: Data;
  after?: Data;
  handle?: string;
  cleanup?: boolean;
  selectionBefore?: Data;
  selectionAfter?: Data;
}
export class FakeHost implements KernelHost {
  state: Publication;
  requests: Request[] = [];
  errors: unknown[] = [];
  pins = new Set<string>();
  queue: (() => void)[] = [];
  entries: Entry[] = [];
  index = -1;
  depth = 0;
  pending?: { before: Data; label: string; scoped: boolean };
  live = false;
  liveCommands = 0;
  failAfterApply = false;
  rejectNextWrite = false;
  private counter = 0;
  private mutationLeases = new Map<
    string,
    { before: Data; label: string; depth: number }
  >();
  private acks = new Map<string, Reply>();
  private retired: string[] = [];
  private continuations = new Map<string, boolean>();
  constructor(
    document: Data = {
      id: "synthetic",
      name: "Synthetic trace fixture",
      layers: [],
      comps: {},
      revision: 0,
      edits: [],
    },
    workspace: Data = {
      id: "synthetic-workspace",
      name: "Synthetic workspace",
      builtIn: false,
    },
  ) {
    this.state = {
      revision: 0,
      document: copy(document),
      selection: { layers: [], keys: [], chan: null },
      workspace: copy(workspace),
      facts: {
        historyBusy: false,
        canUndo: false,
        canRedo: false,
        editLive: false,
      },
    };
  }
  publication(): Publication {
    this.state.facts = {
      historyBusy: !!this.pending,
      canUndo: this.index >= 0,
      canRedo: this.index < this.entries.length - 1,
      editLive: this.live,
      label: this.entries[this.index]?.label ?? null,
    };
    return copy(this.state);
  }
  query(method: string, _args: Data): any {
    throw new Error("Fake host query unavailable: " + method);
  }
  scheduleFlush(flush: () => void): void {
    this.queue.push(flush);
  }
  drain(): void {
    for (const flush of this.queue.splice(0)) flush();
  }
  reportError(error: unknown): void {
    this.errors.push(error);
  }
  pinGeneration(handle: string): void {
    this.pins.add(handle);
  }
  unpinGeneration(handle: string): void {
    this.pins.delete(handle);
  }
  call(request: Request): Reply {
    const key = `${request.realm}:${request.generation}:${request.txid}`;
    if (this.acks.has(key)) return copy(this.acks.get(key)!);
    this.requests.push(copy(request));
    let reply: Reply;
    try {
      reply = this.run(request.method, request.args);
    } catch (error) {
      reply = {
        error: {
          code: "Rejected",
          message: String(error instanceof Error ? error.message : error),
        },
      };
    }
    reply.publication = this.publication();
    if (this.retired.length) {
      reply.retired = this.retired.splice(0);
    }
    this.acks.set(key, copy(reply));
    if (this.failAfterApply) {
      this.failAfterApply = false;
      throw new Error("Synthetic lost acknowledgement");
    }
    return reply;
  }
  private begin(label: string, scoped = false): string | null {
    if (this.depth++ > 0)
      return this.pending?.scoped ? null : JSON.stringify(this.pending?.before);
    this.pending = { before: copy(this.state.document), label, scoped };
    return scoped ? null : JSON.stringify(this.pending.before);
  }
  private push(entry: Omit<Entry, "id">): string {
    for (const entry of this.entries.splice(this.index + 1))
      if (entry.handle) this.retired.push(entry.handle);
    const id = `history-${++this.counter}`;
    this.entries.push({ ...entry, id });
    this.index = this.entries.length - 1;
    return id;
  }
  private commit(label?: string): boolean {
    if (this.depth <= 0 || --this.depth > 0 || !this.pending) return false;
    const pending = this.pending;
    this.pending = undefined;
    if (equal(pending.before, this.state.document)) return false;
    this.push({
      label: label || pending.label,
      before: pending.before,
      after: copy(this.state.document),
    });
    return true;
  }
  private cancel(): void {
    this.pending = undefined;
    this.depth = 0;
  }
  private rollback(): boolean {
    if (!this.pending) return false;
    this.state.document = copy(this.pending.before);
    this.state.revision++;
    this.cancel();
    return true;
  }
  private run(method: string, args: Data): Reply {
    const pass = (message: string, data?: Data) => ({
      value: { ok: true, message, ...(data ? { data } : {}) },
    });
    const fail = (message: string) => ({ value: { ok: false, message } });
    switch (method) {
      case "document.directWrite":
        if (this.rejectNextWrite) {
          this.rejectNextWrite = false;
          throw new Error("Synthetic validation rejection");
        }
        this.state.document = patch(this.state.document, args.ops);
        this.state.revision++;
        return {};
      case "document.touch":
        return {};
      case "selection.set":
        this.state.selection = copy(args.selection);
        return {};
      case "history.begin":
        return { value: this.begin(args.label) };
      case "history.commit":
        return { value: this.commit(args.label) };
      case "history.cancel":
        this.cancel();
        return {};
      case "history.rollback":
        return { value: this.rollback() };
      case "history.busy":
        return { value: !!this.pending };
      case "history.canUndo":
        return { value: this.index >= 0 };
      case "history.canRedo":
        return { value: this.index < this.entries.length - 1 };
      case "history.label":
        return { value: this.entries[this.index]?.label ?? null };
      case "history.external":
        return {
          value: this.push({
            label: args.label || "Interface change",
            handle: args.handle,
            cleanup: args.cleanup,
          }),
        };
      case "history.selection":
        if (
          this.pending ||
          this.index < this.entries.length - 1 ||
          equal(args.before, args.after)
        )
          return { value: null };
        return {
          value: this.push({
            label: "Selection",
            selectionBefore: args.before,
            selectionAfter: args.after,
          }),
        };
      case "history.undoIfTop":
        if (this.entries[this.index]?.id !== args.id) return { value: false };
        return this.run("history.undo", {});
      case "history.undo":
      case "history.redo": {
        const undo = method === "history.undo";
        if (undo ? this.index < 0 : this.index >= this.entries.length - 1)
          return { value: false };
        const entry = undo
          ? this.entries[this.index--]
          : this.entries[++this.index];
        if (entry.handle) {
          const token = `continuation-${++this.counter}`;
          this.continuations.set(token, true);
          return {
            continuation: {
              handle: entry.handle,
              phase: undo ? "undo" : "redo",
              token,
            },
          };
        }
        if (entry.selectionBefore)
          this.state.selection = copy(
            undo ? entry.selectionBefore : entry.selectionAfter!,
          );
        if (entry.before) {
          this.state.document = copy(undo ? entry.before : entry.after!);
          this.state.revision++;
        }
        return { value: true };
      }
      case "history.externalDone":
        if (!this.continuations.delete(args.token))
          throw new Error("Stale continuation");
        return { value: true };
      case "history.clear": {
        for (const entry of this.entries)
          if (entry.handle) this.retired.push(entry.handle);
        this.entries = [];
        this.index = -1;
        return {};
      }
      case "edit.admitMutation":
        return this.live
          ? fail("Finish the active source edit before changing structure")
          : { value: { ok: true } };
      case "edit.begin":
        if (this.live) throw new Error("A source edit is already active");
        this.begin(args.label || "Edit source", true);
        this.live = true;
        this.liveCommands = 0;
        return {};
      case "edit.cancel": {
        if (!this.live) return { value: false };
        this.live = false;
        if (
          this.liveCommands ||
          !equal(this.pending?.before, this.state.document)
        )
          this.rollback();
        else this.cancel();
        return { value: true };
      }
      case "edit.commit":
        if (!this.live) return fail("No source edit is active");
        this.live = false;
        if (
          !this.liveCommands ||
          equal(this.pending?.before, this.state.document)
        ) {
          this.cancel();
          return pass("No source changes");
        }
        this.record();
        this.commit(args.label);
        return pass(args.label || "Edit source", {
          revision: this.state.document.revision,
        });
      case "edit.apply":
      case "edit.dispatch": {
        const meta = args.meta ?? {};
        const list = Array.isArray(args.commands)
          ? args.commands
          : [args.commands];
        if (!list.length) return fail("No source edits supplied");
        if (
          meta.baseRevision != null &&
          Number(meta.baseRevision) !== this.state.document.revision
        )
          return fail(
            `Project changed: expected revision ${meta.baseRevision}, found ${this.state.document.revision}`,
          );
        const checkpoint = copy(this.state.document);
        const live = this.live;
        if (!live) this.begin(meta.label || "Edit source", true);
        try {
          const results = list.map((command) => {
            if (command.type !== "add_layer")
              throw new Error("Unsupported fake command: " + command.type);
            const layer = {
              id: command.id || `layer-${++this.counter}`,
              type: command.layerType || "shape",
              name: command.name || "Shape",
            };
            this.state.document.layers.push(layer);
            return { command, data: { id: layer.id } };
          });
          this.state.revision++;
          this.liveCommands += list.length;
          if (!live) {
            this.record();
            this.commit(meta.label);
          }
          return pass("Source updated", {
            results,
            revision: this.state.document.revision || 0,
          });
        } catch (error) {
          this.state.document = checkpoint;
          if (!live) this.cancel();
          return fail((error as Error).message);
        }
      }
      case "edit.mutateBegin": {
        if (this.live)
          return fail(
            "Finish the active source edit before changing structure",
          );
        const lease = `mutation-${++this.counter}`;
        this.mutationLeases.set(lease, {
          before: copy(this.state.document),
          label: args.label || "Edit source",
          depth: this.depth,
        });
        this.begin(args.label || "Edit source");
        return pass("Mutation started", { lease });
      }
      case "edit.mutateFinish": {
        const lease = this.mutationLeases.get(args.lease);
        if (!lease) throw new Error("Stale mutation lease");
        const previous = lease.before,
          next = patch(this.state.document, args.ops);
        const withoutTree = (doc: Data) =>
          Object.fromEntries(
            Object.entries(doc).filter(
              ([k]) => !["layers", "comps", "revision", "edits"].includes(k),
            ),
          );
        this.mutationLeases.delete(args.lease);
        if (!equal(withoutTree(previous), withoutTree(next))) {
          this.state.document = copy(previous);
          this.state.revision++;
          this.cancel();
          return fail(
            "Structural edits may only change the project layer tree",
          );
        }
        this.state.document = next;
        if (equal(previous, next)) {
          this.commit();
          return pass("No source changes", {
            revision: previous.revision || 0,
          });
        }
        this.record();
        this.commit(args.label);
        return pass(args.label || "Source updated", {
          revision: this.state.document.revision,
        });
      }
      case "edit.mutateAbort": {
        const lease = this.mutationLeases.get(args.lease);
        if (!lease) throw new Error("Stale mutation lease");
        this.mutationLeases.delete(args.lease);
        this.state.document = copy(lease.before);
        this.state.revision++;
        this.cancel();
        return {};
      }
      case "workspace.replace":
        this.state.workspace = copy(args.next);
        return {};
      case "workspace.prepare":
        if (this.state.workspace.builtIn && !args.options?.inPlace)
          this.state.workspace = {
            ...this.state.workspace,
            id: "workspace-copy",
            builtIn: false,
          };
        return {};
      case "workspace.mutate":
        this.state.workspace = copy(args.next);
        return { value: copy(this.state.workspace) };
      case "realm.failed":
        if (this.pending) {
          this.depth = 1;
          this.commit();
        }
        this.live = false;
        return {};
      default:
        throw new Error("Unsupported fake method: " + method);
    }
  }
  private record(): void {
    this.state.document.revision =
      Number(this.state.document.revision || 0) + 1;
    this.state.document.edits ??= [];
    this.state.revision++;
  }
}
