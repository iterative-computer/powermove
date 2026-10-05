import { readFile, writeFile, mkdir } from "node:fs/promises";
import path from "node:path";
import { app, dialog, BrowserWindow, type IpcMain } from "electron";
import { BlenderWorker, discoverBlender } from "./blender-service";
import type { BlenderJob } from "../shared/blender";
import { IPC } from "../shared/ipc";
export function registerBlenderIpc(
  ipc: IpcMain,
  ctx: { isTrustedSender: (event: any) => boolean },
) {
  const workers = new Map<number, BlenderWorker>();
  let configured: string | undefined,
    status: Awaited<ReturnType<typeof discoverBlender>> | undefined;
  const configuration = path.join(app.getPath("userData"), "blender-path.json");
  const ready = readFile(configuration, "utf8")
    .then((raw) => {
      const parsed = JSON.parse(raw);
      if (typeof parsed.path === "string" && path.isAbsolute(parsed.path))
        configured = parsed.path;
    })
    .catch(() => {});
  const trusted = (e: any) => {
    if (!ctx.isTrustedSender(e))
      throw new Error("Unauthorized Blender request");
    return e.sender.id as number;
  };
  const stop = (owner: number) => {
    workers.get(owner)?.dispose();
    workers.delete(owner);
  };
  ipc.handle(IPC.blenderStatus, async (e) => {
    trusted(e);
    await ready;
    return (status ||= await discoverBlender(configured));
  });
  ipc.handle(IPC.blenderChoose, async (e) => {
    trusted(e);
    await ready;
    const win = BrowserWindow.fromWebContents(e.sender);
    if (!win) return null;
    const result = await dialog.showOpenDialog(win, {
      title: "Choose Blender",
      properties: ["openFile"],
      ...(process.platform === "darwin"
        ? { defaultPath: "/Applications" }
        : {}),
    });
    if (result.canceled || !result.filePaths[0]) return null;
    let executable = result.filePaths[0];
    if (executable.endsWith(".app")) executable += "/Contents/MacOS/Blender";
    const next = await discoverBlender(executable);
    if (!next.available) throw new Error(next.error);
    for (const owner of workers.keys()) stop(owner);
    configured = executable;
    status = next;
    await mkdir(path.dirname(configuration), { recursive: true });
    await writeFile(configuration, JSON.stringify({ path: executable }), {
      mode: 0o600,
    });
    return next;
  });
  ipc.handle(IPC.blenderRun, async (e, job: BlenderJob) => {
    const owner = trusted(e);
    await ready;
    status ||= await discoverBlender(configured);
    if (!status.available || !status.path)
      throw new Error(status.error || "Blender is unavailable");
    let worker = workers.get(owner);
    if (!worker) {
      worker = new BlenderWorker(status.path, app.getPath("temp"));
      workers.set(owner, worker);
    }
    return worker.run(job);
  });
  ipc.handle(IPC.blenderCancel, (e) => stop(trusted(e)));
  app.on("web-contents-created", (_e, contents) =>
    contents.once("destroyed", () => stop(contents.id)),
  );
  app.on("will-quit", () => {
    for (const owner of workers.keys()) stop(owner);
  });
}
