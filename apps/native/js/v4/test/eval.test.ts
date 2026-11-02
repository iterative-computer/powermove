import { test, expect } from "bun:test";
import { tasks } from "../eval/tasks";
import { runTask } from "../eval/runner";
const wire = process.env.PM_SURFACE_WIRE;
test("eval registry has four ten-task groups and complete owner behavior specs", async () => {
  expect(tasks).toHaveLength(40);
  for (const group of ["builtin", "wild", "held-out", "owner"])
    expect(tasks.filter((t) => t.group === group)).toHaveLength(10);
  for (const task of tasks.filter((task) => task.group === "owner")) {
    expect(task.status).toBe("ready");
    expect(task.checks.length).toBeGreaterThan(0);
    expect(task.behavior).toBeDefined();
    for (const field of [
      "inputs",
      "interactions",
      "reads",
      "writes",
      "edgeCases",
      "apiGaps",
    ] as const)
      expect(task.behavior![field].length).toBeGreaterThan(0);
  }
});
for (const task of tasks.filter((t) => t.status === "ready"))
  test(`${task.id}: ${task.brief}`, async () => {
    const result = await runTask(task.id, wire);
    expect(result).toMatchObject(
      task.group === "owner"
        ? {
            id: task.id,
            status: "pass",
            fixtureStatus: "pass",
            serviceStatus: "pass",
            productionReady: false,
          }
        : { id: task.id, status: "pass" },
    );
  });
