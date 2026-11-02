import type { HostClient } from "../../src/host-api";
import { json } from "../../src/protocol";
import type { Action, AgentModel, Publication } from "./contract";
import { publication } from "./validation";

/** Application registers these command ids; no speculative built-in id is sent. */
export interface AgentRegistration {
  read: string;
  dispatch: string;
  /** Native coordinator forwards run/model publications, also at unchanged document revisions. */
  subscribe(publish: (publication: Publication) => void): () => void;
}
/** Worker-only facade. HostClient enforces realm/project/generation/ack fencing. */
export class HostAgentModel implements AgentModel {
  private current: Publication | undefined;
  private listeners = new Set<() => void>();
  private stop: (() => void) | undefined;
  private retired = false;
  private error: unknown;
  private queue: Promise<unknown> = Promise.resolve();
  private constructor(
    private host: HostClient,
    private registration: AgentRegistration,
  ) {}
  static async connect(host: HostClient, registration: AgentRegistration) {
    if (!registration.read || !registration.dispatch)
      throw new Error("Agent command registration required");
    const model = new HostAgentModel(host, registration);
    model.stop = registration.subscribe((value) => {
      try {
        model.publish(value);
      } catch (error) {
        model.error = error;
        for (const callback of model.listeners) callback();
      }
    });
    try {
      const ack = await host.command(registration.read, {});
      model.publish(ack.value as unknown as Publication);
      if (model.error) throw model.error;
      model.inspect();
      return model;
    } catch (error) {
      model.dispose();
      throw error;
    }
  }
  private publish(input: Publication) {
    if (this.retired) return;
    const value = publication(
      json(input),
      this.host.identity.project,
      this.host.identity.generation,
    );
    if (this.current && value.sequence <= this.current.sequence) return;
    this.current = structuredClone(value);
    for (const callback of this.listeners) callback();
  }
  inspect() {
    if (this.error) throw this.error;
    if (this.retired || !this.current)
      throw new Error("Agent publication unavailable");
    return structuredClone(this.current);
  }
  subscribe(callback: () => void) {
    if (this.retired) throw new Error("Agent model retired");
    this.listeners.add(callback);
    return () => {
      this.listeners.delete(callback);
    };
  }
  async dispatch(action: Action) {
    const data = json(action);
    // Capture at invocation, not when a queued operation happens to execute.
    const { sequence, snapshot: s } = this.inspect();
    const target = { project: s.projectId, thread: s.threadId, run: s.runId };
    const operation = this.queue.then(async () => {
      this.inspect();
      await this.host.command(this.registration.dispatch, {
        sequence,
        target,
        action: data,
      });
      // Only native publications may change UI facts. An ack is not a snapshot.
    });
    this.queue = operation.catch(() => undefined);
    return operation;
  }
  /** Worker/test drain; never called by a native main-thread handler. */
  async flush() {
    await this.queue;
  }
  dispose() {
    if (this.retired) return;
    this.retired = true;
    this.stop?.();
    this.listeners.clear();
  }
}
