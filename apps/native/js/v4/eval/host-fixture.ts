import { deepStrictEqual, ok } from "node:assert/strict";
import {
  HostClient,
  type HostAck,
  type HostPublication,
  type HostQuery,
  type Json,
} from "powermove-v4";
import { nativeWire } from "./native-wire";

/** Test-only native worker/KernelData double, never a JS document backend. Every
 * request crosses the public client -> Rust router -> concrete data service. */
export class OwnerHost {
  readonly client: HostClient;
  readonly requests: Json[] = [];
  private history: Json[] = [];
  private initialized = false;
  revision = 0;
  constructor() {
    this.client = new HostClient(
      { realm: "owner-fixture", generation: 1, project: "synthetic" },
      async (request) => {
        this.requests.push(structuredClone(request) as unknown as Json);
        const result = this.send({ request: request as unknown as Json });
        ok(result.ack, "actual native host acknowledgement required");
        this.initialized = true;
        const ack = result.ack as unknown as HostAck;
        this.revision = ack.revision;
        return ack;
      },
    );
  }
  private send(input: Json): Record<string, Json> {
    this.history.push(input);
    const result = Bun.spawnSync([nativeWire().host], {
      stdin: new TextEncoder().encode(
        this.history.map((row) => JSON.stringify(row)).join("\n") + "\n",
      ),
    });
    deepStrictEqual(
      result.exitCode,
      0,
      new TextDecoder().decode(result.stderr),
    );
    const rows = new TextDecoder()
      .decode(result.stdout)
      .trim()
      .split("\n")
      .map((row) => JSON.parse(row));
    deepStrictEqual(rows.length, this.history.length);
    const row = rows.at(-1)!;
    if (row.error) throw new Error(String(row.error));
    return row;
  }
  fixture(query: HostQuery, value: Json) {
    this.send({ fixture: { query, value, revision: this.revision } });
  }
  rejectNextEdit() {
    this.send({ rejectNextEdit: true });
  }
  publish() {
    const row = this.send({ publish: true });
    return (row.publications as unknown as HostPublication[]).map((p) =>
      this.client.publish(p),
    );
  }
  async retire() {
    this.client.retire();
    if (this.initialized) this.send({ retire: true });
  }
}

/** Executable checks replace static claims that public data services are absent.
 * Native resource capabilities are still integration gates, not fabricated here. */
export async function exerciseHostData(host: OwnerHost) {
  const project = { annotations: [], dimensions: [320, 180], unknown: false };
  host.fixture("project", project);
  host.fixture("selection", ["synthetic-layer"]);
  host.fixture("time", 1.25);
  host.fixture("evaluated", {
    bounds: [0, 0, 40, 20],
    transform: [0, 2, -3, 0],
  });
  for (const [query, value] of [
    ["project", project],
    ["selection", ["synthetic-layer"]],
    ["time", 1.25],
    ["evaluated", { bounds: [0, 0, 40, 20], transform: [0, 2, -3, 0] }],
  ] as const)
    deepStrictEqual((await host.client.query(query)).value, value);
  const seen: Json[] = [];
  const stop = await host.client.subscribe("selection", "selection", (value) =>
    seen.push(value),
  );
  deepStrictEqual(
    seen,
    [["synthetic-layer"]],
    "subscribe returns an initial kernel snapshot",
  );
  host.fixture("selection", ["replacement"]);
  deepStrictEqual(host.publish(), [true]);
  deepStrictEqual(host.publish(), []);
  await stop();
  host.fixture("selection", []);
  deepStrictEqual(host.publish(), []);
  deepStrictEqual(seen, [["synthetic-layer"], ["replacement"]]);
  const before = host.revision;
  const ack = await host.client.edit("Synthetic transaction", [
    { command: "fixture.edit", args: { value: "日本🙂", exact: false } },
  ]);
  ok(ack.accepted && ack.revision === before + 1 && ack.value);
  host.rejectNextEdit();
  await host.client
    .edit("Declined transaction", [{ command: "fixture.edit", args: {} }])
    .then(
      () => {
        throw new Error("negative actor ack accepted");
      },
      (error) => ok(String(error).includes("declined")),
    );
  deepStrictEqual(
    host.revision,
    before + 1,
    "declined edit consumes no document revision",
  );
  await host.client.setSetting("fixture", { unknown: [false, null, ""] });
  deepStrictEqual((await host.client.getSetting("fixture")).value, {
    unknown: [false, null, ""],
  });
}
