/** Test-only bridge to the real Rust HostRouter, data service, model and actor. */
import {
  HostClient,
  type HostAck,
  type HostRequest,
  type Json,
} from "../../../src";
import { EffectsController } from "../controller";
import { platform, buildNative } from "./helpers";
export class ModelHost {
  readonly child;
  private reader;
  private buffer = "";
  readonly kit = platform();
  readonly requests: HostRequest[] = [];
  readonly client: HostClient;
  readonly controller: EffectsController;
  constructor() {
    const path = `${buildNative()}/debug/effect-controls-model-wire`;
    this.child = Bun.spawn([path], {
      stdin: "pipe",
      stdout: "pipe",
      stderr: "inherit",
    });
    this.reader = this.child.stdout.getReader();
    this.client = new HostClient(
      { realm: "effects", generation: 1, project: "P" },
      async (request) => {
        this.requests.push(request);
        const result = await this.line({ request });
        if (result.error) throw new Error(result.error);
        return result.ack as HostAck;
      },
    );
    this.controller = new EffectsController(
      this.client,
      this.kit.api,
      "effects",
    );
  }
  async line(value: unknown): Promise<any> {
    this.child.stdin.write(JSON.stringify(value) + "\n");
    await this.child.stdin.flush();
    while (!this.buffer.includes("\n")) {
      const result = await this.reader.read();
      if (result.done)
        throw new Error(`Model fixture exited: ${await this.child.exited}`);
      this.buffer += new TextDecoder().decode(result.value);
    }
    const end = this.buffer.indexOf("\n");
    const line = this.buffer.slice(0, end);
    this.buffer = this.buffer.slice(end + 1);
    return JSON.parse(line);
  }
  async publish() {
    const { publications } = await this.line({ publish: true });
    for (const item of publications) this.client.publish(item);
    // An explicit query fences adoption/binding registrations before returning.
    await this.client.query("evaluated", ["panel:layer-effects"]);
    await this.controller.settled();
  }
  async start(layer = "a") {
    await this.line({ selectLayer: layer });
    await this.controller.start();
  }
  async select(layer: string) {
    await this.line({ selectLayer: layer });
    await this.publish();
  }
  async close() {
    await this.controller.dispose();
    this.client.retire();
    this.child.stdin.end();
    await this.child.exited;
  }
}
