import { json, type Json } from "./protocol";

export const HOST_API_VERSION = 1 as const;
export interface HostIdentity {
  realm: string;
  generation: number;
  project: string;
}
export type HostOperation =
  | { type: "binding.register"; handle: string; channels: string[] }
  | { type: "binding.dispose"; handle: string }
  | {
      type: "query";
      query: "selection" | "time" | "evaluated";
      channels?: string[];
      time?: number;
    }
  | {
      type: "subscribe";
      subscription: string;
      query: "selection" | "time" | "evaluated";
      channels?: string[];
    }
  | { type: "unsubscribe"; subscription: string }
  | { type: "command"; command: string; args: Json }
  | { type: "settings.get"; key: string }
  | { type: "settings.set"; key: string; value: Json };
export interface HostRequest extends HostIdentity {
  version: typeof HOST_API_VERSION;
  requestId: number;
  expectedRevision: number;
  expectedStateRevision?: number;
  operation: HostOperation;
}
export interface HostAck extends HostIdentity {
  version: typeof HOST_API_VERSION;
  requestId: number;
  revision: number;
  accepted: boolean;
  stateRevision?: number;
  value?: Json;
  error?: string;
}
export interface HostPublication extends HostIdentity {
  version: typeof HOST_API_VERSION;
  subscription: string;
  revision: number;
  sequence: number;
  value: Json;
}
/** Worker-only async queue. Native adapter must route to pm-kernel, never a JS document mirror. */
export type HostTransport = (request: HostRequest) => Promise<HostAck>;
export class HostClient {
  private requestId = 0;
  private revision: number;
  private stateRevision = 0;
  private retired = false;
  private fenced = false;
  private subscriptions = new Map<
    string,
    { revision: number; sequence: number; callback: (value: Json) => void }
  >();
  constructor(
    readonly identity: HostIdentity,
    private readonly transport: HostTransport,
    revision = 0,
  ) {
    this.identity = Object.freeze({ ...identity });
    if (
      !identity.realm ||
      !identity.project ||
      !Number.isSafeInteger(identity.generation) ||
      identity.generation < 1 ||
      !Number.isSafeInteger(revision) ||
      revision < 0
    )
      throw new TypeError("Invalid host identity/revision");
    this.revision = revision;
  }
  async request(operation: HostOperation): Promise<HostAck> {
    if (this.retired) throw new Error("Host generation retired");
    if (this.fenced)
      throw new Error("Host acknowledgement ambiguous; reset generation");
    const operationData = json(operation) as unknown as HostOperation;
    if (!Number.isSafeInteger(this.requestId + 1))
      throw new RangeError("Host request IDs exhausted");
    const request: HostRequest = {
      ...this.identity,
      version: HOST_API_VERSION,
      requestId: ++this.requestId,
      expectedRevision: this.revision,
      operation: operationData,
      ...(operationData.type.startsWith("settings.")
        ? { expectedStateRevision: this.stateRevision }
        : {}),
    };
    json(request);
    let ack: HostAck;
    try {
      ack = json(await this.transport(request)) as unknown as HostAck;
    } catch (error) {
      this.fenced = true;
      throw error;
    }
    if (
      this.retired ||
      typeof ack.accepted !== "boolean" ||
      !this.matches(ack) ||
      ack.requestId !== request.requestId ||
      !Number.isSafeInteger(ack.revision) ||
      ack.revision < request.expectedRevision
    ) {
      this.fenced = true;
      throw new Error("Stale or mismatched host acknowledgement");
    }
    if (!ack.accepted) throw new Error(ack.error ?? "Host request rejected");
    if (request.expectedStateRevision !== undefined) {
      if (
        !Number.isSafeInteger(ack.stateRevision) ||
        ack.stateRevision! < request.expectedStateRevision
      ) {
        this.fenced = true;
        throw new Error("Stale or missing settings revision");
      }
      this.stateRevision = Math.max(this.stateRevision, ack.stateRevision!);
    }
    if (ack.value !== undefined) json(ack.value);
    this.revision = Math.max(this.revision, ack.revision);
    return structuredClone(ack);
  }
  registerBinding(handle: string, channels: string[]) {
    return this.request({ type: "binding.register", handle, channels });
  }
  disposeBinding(handle: string) {
    return this.request({ type: "binding.dispose", handle });
  }
  query(
    query: "selection" | "time" | "evaluated",
    channels?: string[],
    time?: number,
  ) {
    return this.request({ type: "query", query, channels, time });
  }
  command(command: string, args: Json) {
    return this.request({ type: "command", command, args });
  }
  getSetting(key: string) {
    return this.request({ type: "settings.get", key });
  }
  setSetting(key: string, value: Json) {
    return this.request({ type: "settings.set", key, value });
  }
  async subscribe(
    subscription: string,
    query: "selection" | "time" | "evaluated",
    callback: (value: Json) => void,
    channels?: string[],
  ) {
    if (this.subscriptions.has(subscription))
      throw new Error("Duplicate subscription");
    const entry = { revision: this.revision, sequence: -1, callback };
    this.subscriptions.set(subscription, entry);
    try {
      await this.request({ type: "subscribe", subscription, query, channels });
    } catch (error) {
      this.subscriptions.delete(subscription);
      throw error;
    }
    return async () => {
      await this.request({ type: "unsubscribe", subscription });
      this.subscriptions.delete(subscription);
    };
  }
  publish(publication: HostPublication): boolean {
    const entry = this.subscriptions.get(publication.subscription);
    if (
      this.retired ||
      !entry ||
      !this.matches(publication) ||
      !Number.isSafeInteger(publication.revision) ||
      publication.revision < entry.revision ||
      !Number.isSafeInteger(publication.sequence) ||
      publication.sequence <= entry.sequence
    )
      return false;
    const value = json(publication.value);
    entry.revision = publication.revision;
    entry.sequence = publication.sequence;
    this.revision = Math.max(this.revision, publication.revision);
    entry.callback(value);
    return true;
  }
  /** Native realm retirement releases bindings/subscriptions independently of a tree commit. */
  retire() {
    this.retired = true;
    this.subscriptions.clear();
  }
  private matches(value: HostIdentity & { version: number }) {
    return (
      value.version === HOST_API_VERSION &&
      value.realm === this.identity.realm &&
      value.project === this.identity.project &&
      value.generation === this.identity.generation
    );
  }
}
