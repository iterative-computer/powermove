import {
  HostClient,
  type HostRequest,
  type HostAck,
  type Json,
} from "../../../src";
import type {
  InspectorModel,
  InspectorSnapshot,
  Intent,
  ModelOperation,
  NativeRequest,
  ReadoutClock,
  Row,
} from "../contracts";
import { InspectorSession } from "../session";

export const binding = (handle: string) => ({ handle, channels: [handle] });
export const numberRow = (id = "opacity", label = "Opacity"): Row => ({
  id,
  label,
  animation: "static",
  control: {
    kind: "number",
    axes: [{ binding: binding(id), value: 100, unit: "%", step: 1 }],
  },
});
export function snapshot(): InspectorSnapshot {
  return {
    target: {
      kind: "layers",
      key: "a",
      count: 1,
      first: {
        id: "a",
        name: "Headline",
        type: "text",
        typeLabel: "Text",
        locked: false,
        audio: false,
        acceptsEffects: true,
      },
      sharedType: "Text",
    },
    sections: [
      {
        id: "transform",
        title: "Transform",
        collapsed: false,
        rows: [numberRow()],
      },
    ],
    playing: false,
    time: 0,
  };
}
/** Synthetic model double only. Real authored edits are tested by v4_adapter.rs
 * against pm-panel-inspector and an actual KernelActor. */
export class ModelDouble implements InspectorModel {
  readonly updates: [string, Json][] = [];
  readonly intents: Intent[] = [];
  operation: ModelOperation | null = {
    type: "command",
    command: "test.intent",
    args: null,
  };
  constructor(public value = snapshot()) {}
  update(query: string, publication: Json) {
    this.updates.push([query, publication]);
  }
  inspect() {
    return this.value;
  }
  plan(intent: Intent) {
    this.intents.push(structuredClone(intent));
    return this.operation;
  }
}
export class ClockDouble implements ReadoutClock {
  callbacks = new Set<() => void>();
  schedule(callback: () => void) {
    this.callbacks.add(callback);
    return () => {
      this.callbacks.delete(callback);
    };
  }
  frame() {
    const callbacks = [...this.callbacks];
    this.callbacks.clear();
    callbacks.forEach((callback) => callback());
  }
}
export function fixture(
  value = snapshot(),
  transport?: (request: HostRequest) => Promise<HostAck>,
  generation = 1,
) {
  const requests: HostRequest[] = [],
    native: NativeRequest[] = [],
    errors: unknown[] = [];
  const model = new ModelDouble(value),
    clock = new ClockDouble();
  const host = new HostClient(
    { realm: "inspector-test", generation, project: "synthetic" },
    async (request) => {
      requests.push(structuredClone(request));
      return transport
        ? transport(request)
        : {
            ...request,
            revision: request.expectedRevision,
            accepted: true,
            value: null,
          };
    },
  );
  const session = new InspectorSession(
    host,
    model,
    {
      request: async (request) => {
        native.push(request);
      },
    },
    clock,
    (error) => errors.push(error),
  );
  let sequence = 0;
  return {
    model,
    host,
    session,
    requests,
    native,
    errors,
    clock,
    publish(
      query: "project" | "selection" | "time" | "evaluated" = "evaluated",
    ) {
      return host.publish({
        ...host.identity,
        version: 1,
        subscription: `inspector:${query}`,
        revision: 0,
        sequence: ++sequence,
        value: { tick: sequence },
      });
    },
  };
}
