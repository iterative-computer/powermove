// Development evaluation host: Refresh must be installed before creating any root.
import "powermove-v4/refresh";
import { Evaluation } from "./harness";
import { exerciseOwner } from "./scenarios";
import { createRoot } from "powermove-v4";
import { tasks } from "./tasks";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

/** Consultation log: task-specific public reference sections/types used, including cached reads.
 * Counts are this coding-agent attempt, not human usability statistics or independent sessions.
 */
export const consultations: Record<string, string[]> = {
  "builtin-01": ["v4-api.md / powermove/ui", "public types:Row,NumField"],
  "builtin-02": ["v4-api.md / protocol/IDs", "public types:JSX"],
  "builtin-03": ["v4-api.md / primitives/resources"],
  "builtin-04": ["v4-api.md / primitives/resources", "public types:JSX"],
  "builtin-05": ["public types:Disclosure", "public types:dispatch/EventPlan"],
  "builtin-06": ["v4-api.md / controlled text", "public types:SurfaceEvent"],
  "builtin-07": ["public types:ToggleField,SelectField,Section"],
  "builtin-08": [
    "public types:TextField,NumField,Section",
    "v4-api.md / unknown props",
  ],
  "builtin-09": ["public types:Menu", "v4-api.md / declared defaults"],
  "builtin-10": ["public types:ToolbarItem,Glass"],
  "wild-01": ["v4-api.md / styles"],
  "wild-02": ["v4-api.md / primitives/anchored", "public types:JSX"],
  "wild-03": ["v4-api.md / primitives/resources"],
  "wild-04": ["v4-api.md / exact text", "public types:JSX"],
  "wild-05": ["v4-api.md / virtual-list", "public types:JSX"],
  "wild-06": ["public types:Menu", "v4-api.md / anchored"],
  "wild-07": ["v4-api.md / focus scopes", "public types:JSX"],
  "wild-08": ["public types:RampField,PointField,FillField"],
  "wild-09": ["v4-api.md / webview opt-in"],
  "wild-10": ["v4-api.md / selectors", "public types:StyleRule"],
  "held-out-01": ["v4-api.md / protocol/IDs"],
  "held-out-02": [
    "v4-api.md / stale events",
    "public types:dispatch/EventPlan",
  ],
  "held-out-03": [
    "v4-api.md / declared defaults",
    "public types:ListenerFlags",
  ],
  "held-out-04": ["v4-api.md / null/JSON semantics"],
  "held-out-05": ["public types:Fragment"],
  "held-out-06": ["public types:hooks"],
  "held-out-07": [
    "v4-api.md / Refresh compiler responsibilities",
    "public types:refresh",
  ],
  "held-out-08": ["public types:Row,TextField,createContext"],
  "held-out-09": ["v4-api.md / finite JSON protocol"],
  "held-out-10": [
    "v4-api.md / atomic rejection/fencing",
    "public types:Commit",
  ],
};
export const friction: Record<string, string[]> = {
  "builtin-01": ["A1", "A2"],
  "builtin-02": ["A1"],
  "builtin-03": ["A3"],
  "builtin-04": ["A3"],
  "builtin-05": ["A2"],
  "builtin-06": ["A4"],
  "builtin-07": ["A1", "A2"],
  "builtin-08": ["A1", "A3"],
  "builtin-09": ["A2"],
  "builtin-10": ["A2"],
  "wild-01": [],
  "wild-02": ["A3"],
  "wild-03": ["A3"],
  "wild-04": ["A3"],
  "wild-05": ["A3"],
  "wild-06": ["A2", "A3"],
  "wild-07": ["A3"],
  "wild-08": ["A1", "A2"],
  "wild-09": ["A3"],
  "wild-10": ["A7"],
  "held-out-01": [],
  "held-out-02": ["A4"],
  "held-out-03": ["A2", "A4"],
  "held-out-04": [],
  "held-out-05": [],
  "held-out-06": [],
  "held-out-07": ["A5"],
  "held-out-08": ["A2"],
  "held-out-09": [],
  "held-out-10": ["A6"],
};
export async function runExtension(id: string, wire?: string) {
  const task = tasks.find((task) => task.id === id);
  if (!task) throw Error(`Unknown task ${id}`);
  if (task.status === "pending-owner") return { id, status: "pending-owner" };
  if (task.group === "owner") return runOwner(id, wire);
  const path = new URL(`../examples/tasks/${id}.tsx`, import.meta.url);
  const source = await readFile(path, "utf8");
  const lines = source
    .split("\n")
    .filter((line) => line.trim() && !line.trim().startsWith("//")).length;
  const e = new Evaluation(wire);
  const start = performance.now();
  const metrics = () => ({
    id,
    lines,
    docLookups: consultations[id].length,
    references: consultations[id],
    friction: friction[id],
    commits: e.commits.length,
    ops: e.commits.reduce((count, commit) => count + commit.ops.length, 0),
    renderWithWireMs: e.timings,
    ms: performance.now() - start,
    wire: !!wire,
    diagnostics: e.wireResults.flatMap((row) => row.diagnostics ?? []),
  });
  try {
    const { evaluate } = await import(fileURLToPath(path));
    await evaluate(e);
    e.compareNative();
    return { ...metrics(), status: "pass" };
  } catch (error) {
    return { ...metrics(), status: "fail", error: String(error) };
  } finally {
    await e.dispose();
  }
}

/** Full owner acceptance fails on documented API gaps even when the synthetic
 * behavior fixture passes. These are authored service seams, not independent
 * extension ports or measured evidence that the missing services work.
 */
export async function runOwner(
  id: string,
  wire?: string,
  factory = createRoot,
) {
  const task = tasks.find((task) => task.id === id && task.group === "owner");
  if (!task?.behavior) throw Error(`Unknown owner specification ${id}`);
  const e = new Evaluation(wire, factory);
  const metrics = () => ({
    id,
    scope: "synthetic owner behavior fixture; full host integration required",
    wire: !!wire,
    commits: e.commits.length,
    ops: e.commits.reduce((count, commit) => count + commit.ops.length, 0),
  });
  try {
    await exerciseOwner(Number(id.slice(-2)), e);
    e.compareNative();
    const blockedBy = task.behavior.apiGaps;
    return {
      ...metrics(),
      status: blockedBy.length ? "fail" : "pass",
      fixtureStatus: "pass",
      failureKind: blockedBy.length ? "api-gap" : undefined,
      blockedBy,
    };
  } catch (error) {
    return {
      ...metrics(),
      status: "fail",
      fixtureStatus: "fail",
      failureKind: "fixture",
      error: String(error),
    };
  } finally {
    await e.dispose();
  }
}
