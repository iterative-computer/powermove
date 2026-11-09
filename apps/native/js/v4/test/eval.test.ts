import { test, expect } from "bun:test";
import { tasks } from "../eval/tasks";
import { runTask } from "../eval/runner";
const wire = process.env.PM_SURFACE_WIRE;
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
