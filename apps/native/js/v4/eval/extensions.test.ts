import { fileURLToPath } from "node:url";
import { test, expect } from "bun:test";
import { runExtension, consultations } from "./extensions";
import { tasks } from "./tasks";
import { Evaluation } from "./harness";
const wire = process.env.PM_SURFACE_WIRE;
const wireTest = test.skipIf(!wire);
for (const task of tasks.filter((task) => task.group !== "owner")) {
  wireTest(`independent extension ${task.id}`, async () => {
    const result = await runExtension(task.id, wire);
    if (task.status === "pending-owner") {
      expect(result.status).toBe("pending-owner");
      expect("lines" in result).toBe(false);
    } else expect(result.status, JSON.stringify(result)).toBe("pass");
  });
}
wireTest("unknown spec cannot become a fabricated pass", async () => {
  expect(runExtension("unknown", wire)).rejects.toThrow("Unknown task");
});
wireTest(
  "all independent public briefs have explicit public reference consultations",
  () => {
    expect(Object.keys(consultations).sort()).toEqual(
      tasks
        .filter((task) => task.status === "ready" && task.group !== "owner")
        .map((task) => task.id)
        .sort(),
    );
  },
);
wireTest(
  "atomic invalid tail keeps the full Rust tree and revision unchanged",
  async () => {
    const e = new Evaluation(wire);
    try {
      await e.render("accepted");
      await e.rejectTail();
      e.compareNative();
    } finally {
      await e.dispose();
    }
  },
);
wireTest(
  "negative protocol checks build and use the real validator without an explicit wire",
  async () => {
    const result = await runExtension("held-out-10");
    expect(result.status, JSON.stringify(result)).toBe("pass");
  },
);
wireTest(
  "extensions import package exports, never implementation files or scaffold",
  async () => {
    const files = new Bun.Glob("../examples/**/*.tsx");
    let checked = 0;
    for await (const path of files.scan({
      cwd: import.meta.dir,
      absolute: true,
    })) {
      checked++;
      const source = await Bun.file(path).text();
      expect(source).not.toMatch(
        /(?:from|import\()\s*["'][^"']*(?:\/src|scenarios|\/host|\/renderer)["']/,
      );
      expect(source).not.toContain(".host");
      expect(source).not.toContain("window.");
      expect(source).not.toContain("document.");
    }
    expect(checked).toBe(33);
  },
);
wireTest(
  "Refresh evaluation initializes the public dev entry before roots",
  async () => {
    const child = Bun.spawnSync(
      [
        process.execPath,
        "-e",
        `import "powermove-v4/refresh"; import {runExtension} from "./eval/extensions"; const r=await runExtension("held-out-07",${JSON.stringify(wire)}); console.log(JSON.stringify(r)); process.exitCode=r.status==="pass"?0:1;`,
      ],
      { cwd: fileURLToPath(new URL("..", import.meta.url)) },
    );
    expect(child.exitCode, new TextDecoder().decode(child.stderr)).toBe(0);
    expect(JSON.parse(new TextDecoder().decode(child.stdout)).status).toBe(
      "pass",
    );
  },
);

for (const task of tasks.filter((task) => task.group === "owner")) {
  test(`owner default evaluation ${task.id} exercises data service and keeps production gates visible`, async () => {
    const result = await runExtension(task.id);
    expect(result).toMatchObject({
      id: task.id,
      status: "pass",
      fixtureStatus: "pass",
      serviceStatus: "pass",
      productionReady: false,
      wire: false,
    });
    expect("integrationGates" in result && result.integrationGates).toEqual(
      task.behavior!.apiGaps,
    );
    expect("commits" in result && result.commits).toBeGreaterThan(0);
  });
}
