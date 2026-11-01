import { copy } from "./mirror";
import type { Data, KernelHost, Publication, Reply, Request } from "./types";
export class HostError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "HostError";
  }
}
/** A lost acknowledgement retries the same immutable request, never a new txid. */
export class HostChannel {
  private sequence = 0;
  private outstanding?: Request;
  constructor(
    readonly host: KernelHost,
    readonly realm: string,
    readonly generation: number,
    private publish: (publication: Publication, reply: Reply) => void,
    private resume: (reply: Reply) => any,
  ) {}
  call(method: string, args: Data = {}): any {
    if (this.outstanding)
      throw new HostError(
        "AckPending",
        "Resolve the pending host acknowledgement before another call",
      );
    const clean = (value: any): any => {
      if (Array.isArray(value)) return value.map(clean);
      if (value && typeof value === "object")
        return Object.fromEntries(
          Object.entries(value)
            .filter(([, v]) => v !== undefined)
            .map(([k, v]) => [k, clean(v)]),
        );
      return value;
    };
    const request = {
      realm: this.realm,
      generation: this.generation,
      txid: ++this.sequence,
      method,
      args: copy(clean(args)),
    };
    this.outstanding = request;
    return this.send(request);
  }
  retry(): any {
    return this.outstanding ? this.send(this.outstanding) : undefined;
  }
  get pending(): boolean {
    return !!this.outstanding;
  }
  private send(request: Request): any {
    const reply = this.host.call(request); // Throw retains request for deduplicated retry.
    this.outstanding = undefined;
    if (reply.publication) this.publish(reply.publication, reply);
    if (reply.error) throw new HostError(reply.error.code, reply.error.message);
    if (reply.continuation) return this.resume(reply);
    return reply.value;
  }
}
