import { pathToFileURL } from "node:url";
import { createRoot, type Commit } from "../src";
import { tasks } from "./tasks";
import { exercise } from "./scenarios";
import { runExtension, runOwner } from "./extensions";
import { nativeWire } from "./native-wire";
export async function runTask(
  id: string,
  wire?: string,
  factory: typeof createRoot = createRoot,
) {
  const task = tasks.find((t) => t.id === id);
  if (!task) throw Error(`Unknown task ${id}`);
  if (task.status !== "ready") return { id, status: "pending-owner" };
  if (task.group === "owner") return runOwner(id, wire, factory);
  const commits: Commit[] = [];
  const root = factory(async (c) => {
    commits.push(c);
    return { accepted: true, revision: c.baseRevision + 1 };
  });
  const start = performance.now();
  try {
    await exercise(id, root, factory);
    if (wire) {
      const child = Bun.spawnSync([wire], {
        stdin: Buffer.from(
          commits.map((c) => JSON.stringify(c)).join("\n") + "\n",
        ),
        env: { ...process.env, PM_SURFACE_WEBVIEW: "1" },
      });
      if (child.exitCode !== 0) throw Error(child.stderr.toString());
      const rows = child.stdout
        .toString()
        .trim()
        .split("\n")
        .filter(Boolean)
        .map((s) => JSON.parse(s));
      if (rows.length !== commits.length || rows.some((r) => !r.accepted))
        throw Error(
          `Rust rejected: ${JSON.stringify(rows.find((r) => !r.accepted)?.error)}`,
        );
      const actual = rows.at(-1)?.nodes;
      if (actual)
        for (const node of root.host.nodes.values()) {
          if (
            !actual[node.id] ||
            actual[node.id].type !== node.type ||
            actual[node.id].children.join() !==
              node.children.map((c) => c.id).join()
          )
            throw Error("Native tree differs");
        }
    }
    return {
      id,
      status: "pass",
      commits: commits.length,
      ops: commits.reduce((n, c) => n + c.ops.length, 0),
      ms: performance.now() - start,
    };
  } catch (error) {
    return { id, status: "fail", error: String(error) };
  }
}
if (import.meta.main) {
  const args = Bun.argv.slice(2),
    group = args.find((a) => a.startsWith("--group="))?.split("=")[1];
  const selected = tasks.filter(
    (t) =>
      (!group || t.group === group) &&
      (t.group !== "held-out" ||
        args.includes("--include-held-out") ||
        group === "held-out"),
  );
  if (args.includes("--list")) {
    console.log(JSON.stringify(selected, null, 2));
  } else {
    const wire =
      args.find((a) => a.startsWith("--wire="))?.slice(7) ??
      (args.includes("--wire") ? nativeWire().surface : undefined);
    const candidate = args.find((a) => a.startsWith("--candidate="))?.slice(12);
    const factory: typeof createRoot = candidate
      ? (
          await import(
            new URL(candidate, pathToFileURL(`${process.cwd()}/`)).href
          )
        ).createRoot
      : createRoot;
    if (typeof factory !== "function")
      throw new Error("Candidate must export createRoot");
    const results = [];
    for (const task of selected)
      results.push(
        args.includes("--scaffold") || candidate
          ? await runTask(task.id, wire, factory)
          : await runExtension(task.id, wire),
      );
    console.log(
      JSON.stringify(
        {
          scope:
            args.includes("--scaffold") || candidate
              ? "original scaffold plus authored owner fixtures; headless protocol subset; not visual or macOS acceptance"
              : "independent public briefs plus authored owner fixtures; headless behavior/wire subset; not visual or macOS acceptance",
          mode:
            args.includes("--scaffold") || candidate
              ? "scaffold"
              : "extensions",
          results,
          passed: results.filter((r) => r.status === "pass").length,
          failed: results.filter((r) => r.status === "fail").length,
          pending: results.filter((r) => r.status === "pending-owner").length,
          ownerProductionBlocked: results.filter(
            (r) => "productionReady" in r && r.productionReady === false,
          ).length,
        },
        null,
        2,
      ),
    );
    if (results.some((r) => r.status === "fail")) process.exitCode = 1;
  }
}
