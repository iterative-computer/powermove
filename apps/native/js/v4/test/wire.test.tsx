import { test, expect } from "bun:test";
import { createRoot, type Commit } from "../src";
const wire = process.env.PM_SURFACE_WIRE;
// Build the Linux harness first and set PM_SURFACE_WIRE; no macOS-dependent crate is built.
(wire ? test : test.skip)(
  "real JSX commits validate in Rust and an invalid tail keeps the entire last tree",
  async () => {
    const commits: Commit[] = [];
    const root = createRoot(async (c) => {
      commits.push(c);
      return { accepted: true, revision: c.baseRevision + 1 };
    });
    await root.render(
      <div>
        <input checked={true} value="original" />
        <button>Run</button>
      </div>,
    );
    const input = root.host.root.firstChild!.children[0];
    await root.render(
      <div>
        <input checked={false} value="" />
        <button>Run</button>
      </div>,
    );
    await root.render(
      <div>
        <input value="original" />
        <button>Run</button>
      </div>,
    );
    const invalid: Commit = {
      baseRevision: 3,
      ops: [
        { op: "setProps", id: input.id, patch: { value: "partial" } },
        { op: "remove", id: 999999 },
      ],
    };
    const child = Bun.spawnSync([wire!], {
      stdin: Buffer.from(
        [...commits, invalid, { baseRevision: 3, ops: [] }]
          .map((c) => JSON.stringify(c))
          .join("\n") + "\n",
      ),
    });
    expect(child.exitCode).toBe(0);
    const rows = child.stdout
      .toString()
      .trim()
      .split("\n")
      .map((l) => JSON.parse(l));
    expect(rows.map((r) => r.accepted)).toEqual([
      true,
      true,
      true,
      false,
      true,
    ]);
    expect(rows[3].nodes).toEqual(rows[2].nodes);
    expect(rows[4].nodes[input.id].input.displayed).toBe("original");
  },
);
