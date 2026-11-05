import { UnknownReadiness } from "./readiness";
import { HostClient, json, type Json } from "../../src";
import type {
  Action,
  Input,
  ItemId,
  Projection,
  Reply,
  Stamp,
} from "./contract";

/** The application supplies its registered command ID and publication decoder.
 * No command is registered, no raw project JSON is rewritten by the view. */
export interface ProjectServiceOptions {
  command: string;
  subscription: string;
  projectStamp: (publication: Json) => Stamp | null;
  diagnostic?: (error: unknown) => void;
  /** Enqueue a native AX announcement; never synchronously enter the UI. */
  announce?: (message: string, sequence: number) => void;
  now?: () => number;
}
const validInteger = (n: unknown): n is number =>
  Number.isSafeInteger(n) && (n as number) >= 0;
export function assertStamp(value: unknown): asserts value is Stamp {
  const s = value as Stamp;
  if (
    !s ||
    typeof s.project !== "string" ||
    !s.project ||
    !validInteger(s.document) ||
    !validInteger(s.revision)
  )
    throw new TypeError("Invalid media owner stamp");
}
function availability(value: unknown): boolean {
  if (["Unknown", "Loading", "Ready", "Offline"].includes(value as string))
    return true;
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  if (Object.keys(v).length !== 1) return false;
  if (typeof v.Failed === "string") return true;
  if (
    v.Changed &&
    typeof v.Changed === "object" &&
    typeof (v.Changed as { source: unknown }).source === "string"
  )
    return true;
  const cloud = v.Cloud as Record<string, unknown> | undefined;
  if (!cloud || typeof cloud.provider !== "string") return false;
  return (
    cloud.download === "Stored" ||
    cloud.download === "Downloading" ||
    (!!cloud.download &&
      typeof cloud.download === "object" &&
      typeof (cloud.download as Record<string, unknown>).Failed === "string")
  );
}
const nullableString = (v: unknown) => v === null || typeof v === "string";
const nullableNumber = (v: unknown) =>
  v === null || (typeof v === "number" && Number.isFinite(v));
export function decodeReply(value: unknown, owner: Stamp): Reply {
  const r = json(value) as unknown as Reply;
  const p = r?.view?.page;
  assertStamp(p?.stamp);
  if (
    p.stamp.project !== owner.project ||
    p.stamp.document !== owner.document ||
    p.stamp.revision < owner.revision ||
    !validInteger(p.total) ||
    !validInteger(p.offset) ||
    p.offset > p.total ||
    !Array.isArray(p.rows) ||
    p.rows.length > 4096 ||
    p.offset + p.rows.length > p.total ||
    !validInteger(r.view.selected_count) ||
    r.view.selected_count > p.total ||
    !nullableString(r.view.folder) ||
    !nullableString(r.view.renaming) ||
    typeof r.view.external_drag !== "boolean" ||
    (r.view.drop_target !== null &&
      (!r.view.drop_target || !nullableString(r.view.drop_target.folder))) ||
    (p.next !== null &&
      (!validInteger(p.next) ||
        p.next !== p.offset + p.rows.length ||
        p.next >= p.total)) ||
    !validInteger(r.view.composition_count) ||
    !Array.isArray(r.view.crumbs)
  )
    throw new TypeError("Invalid media projection");
  const ids = new Set<string>();
  for (const row of p.rows) {
    const entries = Object.entries(row.id ?? {});
    if (
      entries.length !== 1 ||
      !["Asset", "Folder", "Composition"].includes(entries[0][0]) ||
      typeof entries[0][1] !== "string" ||
      !entries[0][1] ||
      typeof row.selected !== "boolean" ||
      typeof row.focused !== "boolean" ||
      typeof row.tab_stop !== "boolean" ||
      !row.data ||
      Object.keys(row.data).length !== 1
    )
      throw new TypeError("Invalid media row");
    const [kind, id] = entries[0];
    const data =
      kind === "Asset" && "Asset" in row.data
        ? row.data.Asset[0]
        : kind === "Folder" && "Folder" in row.data
          ? row.data.Folder
          : kind === "Composition" && "Composition" in row.data
            ? row.data.Composition
            : null;
    if (
      !data ||
      data.id !== id ||
      typeof data.name !== "string" ||
      ids.has(JSON.stringify(row.id))
    )
      throw new TypeError("Invalid media row data");
    if ("Asset" in row.data) {
      const [asset, runtime] = row.data.Asset;
      if (
        !asset ||
        !runtime ||
        !availability(runtime.availability) ||
        !nullableString(runtime.poster) ||
        (runtime.progress != null &&
          (!validInteger(runtime.progress.completed) ||
            !validInteger(runtime.progress.total) ||
            runtime.progress.total < 1 ||
            runtime.progress.completed > runtime.progress.total)) ||
        typeof asset.kind !== "string" ||
        !validInteger(asset.references) ||
        ![
          asset.width,
          asset.height,
          asset.duration,
          asset.bytes,
          asset.triangles,
          asset.vertices,
        ].every(nullableNumber) ||
        ![asset.folder, asset.format, asset.source_path].every(nullableString)
      )
        throw new TypeError("Invalid media asset/runtime");
    } else if ("Folder" in row.data) {
      if (
        !validInteger(row.data.Folder.items) ||
        !nullableString(row.data.Folder.parent)
      )
        throw new TypeError("Invalid media folder");
    } else {
      const comp = row.data.Composition;
      if (
        typeof comp.open !== "boolean" ||
        typeof comp.can_nest !== "boolean" ||
        !validInteger(comp.uses) ||
        ![comp.width, comp.height, comp.duration, comp.fps].every(
          nullableNumber,
        )
      )
        throw new TypeError("Invalid media composition");
    }
    if (
      row.folder_path !== undefined &&
      (!Array.isArray(row.folder_path) ||
        !row.folder_path.every((part) => typeof part === "string"))
    )
      throw new TypeError("Invalid folder path");
    ids.add(JSON.stringify(row.id));
  }
  return r;
}
/** Worker-side bridge to pm-panel-media. Documents change only after kernel ack. */
export class ProjectService {
  view: Projection | null = null;
  error: unknown = null;
  errorSequence = 0;
  callbackError: unknown = null;
  readonly readiness = new UnknownReadiness();
  private readinessTimer?: ReturnType<typeof setTimeout>;
  now() {
    return (this.options.now ?? (() => performance.now()))();
  }
  private armReadiness() {
    clearTimeout(this.readinessTimer);
    this.readiness.observe(this.view, this.now());
    const delay = this.view ? this.readiness.next(this.view, this.now()) : null;
    if (delay !== null && !this.stopped)
      this.readinessTimer = setTimeout(() => {
        this.changed();
        this.armReadiness();
      }, delay);
  }
  private owner: Stamp | null = null;
  private epoch = 0;
  private stopped = false;
  private started = false;
  private stopSubscription?: () => Promise<void>;
  private listeners = new Set<() => void>();
  private window = { offset: 0, limit: 0 };
  private queue: Promise<unknown> = Promise.resolve();
  private pending = new Set<Promise<unknown>>();
  constructor(
    readonly host: HostClient,
    readonly options: ProjectServiceOptions,
  ) {}
  listen(listener: () => void) {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }
  private changed() {
    for (const listener of this.listeners) listener();
  }
  /** The native viewport supplies its bounded window, including approved overscan. */
  async page(offset: number, limit: number, expected?: Stamp) {
    if (expected && !this.owns(expected)) return null;
    if (!validInteger(offset) || !validInteger(limit) || limit > 4096)
      throw new RangeError("Invalid media page window");
    this.window = { offset, limit };
    return this.send({ Page: this.window }, expected);
  }
  async start() {
    if (this.started || this.stopped)
      throw new Error("Media service already started or disposed");
    this.started = true;
    const stop = await this.host.subscribe(
      this.options.subscription,
      "project",
      (publication) => {
        const owner = this.options.projectStamp(publication);
        if (owner) {
          assertStamp(owner);
          if (owner.project !== this.host.identity.project)
            throw new Error("Foreign media project");
        }
        const changedDocument = this.owner?.document !== owner?.document;
        if (
          owner &&
          this.owner &&
          !changedDocument &&
          owner.revision < this.owner.revision
        )
          return;
        this.owner = owner ? { ...owner } : null;
        if (changedDocument || !owner) {
          this.epoch++;
          this.view = null;
          this.error = null;
          this.armReadiness();
          this.window.offset = 0;
          this.changed();
        }
        if (owner) this.run(this.send({ Page: this.window }));
      },
    );
    if (this.stopped) await stop();
    else this.stopSubscription = stop;
    await this.idle();
  }
  owns(stamp: Stamp) {
    return (
      !this.stopped &&
      !!this.owner &&
      stamp.project === this.owner.project &&
      stamp.document === this.owner.document &&
      stamp.revision === this.owner.revision
    );
  }
  input(input: Input, expected?: Stamp) {
    return this.send({ Input: input }, expected);
  }
  menu(id: ItemId | null, expected?: Stamp) {
    return this.send({ Menu: id }, expected);
  }
  send(action: Action, expected?: Stamp): Promise<Reply | null> {
    if (this.error !== null && !("Page" in action))
      return Promise.resolve(null);
    const captured = json(action) as unknown as Action;
    const origin = this.owner ? { ...this.owner } : null;
    const guard = expected ? { ...expected } : undefined;
    const work = this.queue.then(() => this.perform(captured, origin, guard));
    this.queue = work.catch(() => undefined);
    return work;
  }
  private async perform(
    action: Action,
    origin: Stamp | null,
    expected?: Stamp,
  ): Promise<Reply | null> {
    if (
      this.stopped ||
      !this.owner ||
      !origin ||
      origin.project !== this.owner.project ||
      origin.document !== this.owner.document ||
      (expected && !this.owns(expected))
    )
      return null;
    if (this.error !== null && !("Page" in action)) return null;
    const owner = { ...this.owner };
    const epoch = this.epoch;
    try {
      const ack = await this.host.command(
        this.options.command,
        json({ owner, action }),
      );
      if (this.stopped || epoch !== this.epoch) return null;
      const reply = decodeReply(ack.value, owner);
      if (this.owner && reply.view.page.stamp.revision < this.owner.revision)
        return null;
      this.owner = { ...reply.view.page.stamp };
      this.view = reply.view;
      this.error = null;
      this.armReadiness();
      this.changed();
      return reply;
    } catch (error) {
      this.recordFailure(error, epoch);
      throw error;
    }
  }
  private recordFailure(error: unknown, epoch: number) {
    if (this.stopped || epoch !== this.epoch || this.error === error) return;
    const first = this.error === null;
    if (first) this.errorSequence++;
    this.error = error;
    try {
      this.options.diagnostic?.(error);
      if (first)
        this.options.announce?.(
          "Project media isn’t updating right now.",
          this.errorSequence,
        );
    } catch (callbackError) {
      this.callbackError = callbackError;
    }
    this.changed();
  }

  run(work: Promise<unknown>) {
    const epoch = this.epoch;
    const tracked = work.catch((error) => {
      this.recordFailure(error, epoch);
    });
    this.pending.add(tracked);
    void tracked.finally(() => this.pending.delete(tracked));
  }
  async idle() {
    while (this.pending.size) await Promise.all([...this.pending]);
  }
  async dispose() {
    this.stopped = true;
    this.epoch++;
    this.view = null;
    clearTimeout(this.readinessTimer);
    this.readiness.observe(null, this.now());
    this.changed();
    await this.stopSubscription?.();
    this.listeners.clear();
  }
}
