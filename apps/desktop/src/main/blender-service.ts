import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import {
  access,
  mkdtemp,
  writeFile,
  readFile,
  rm,
  stat,
} from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { randomUUID } from "node:crypto";
import {
  recipeSchema,
  render3DSchema,
  blenderMaterialSchema,
  type BlenderJob,
  type BlenderOutput,
  type BlenderStatus,
} from "../shared/blender";
import { BLENDER_WORKER } from "./blender-worker";

const safeId = /^[a-zA-Z][a-zA-Z0-9_-]{0,159}$/;
export function validateBlenderJob(raw: BlenderJob): BlenderJob {
  if (
    !raw ||
    !safeId.test(raw.id) ||
    !["generate", "import", "render"].includes(raw.operation) ||
    !Array.isArray(raw.assets) ||
    raw.assets.length > 512
  )
    throw new Error("Invalid Blender request");
  if (
    Object.keys(raw).some(
      (k) =>
        ![
          "id",
          "operation",
          "recipe",
          "sourceAssetId",
          "snapshot",
          "assets",
        ].includes(k),
    )
  )
    throw new Error("Unknown Blender request field");
  let bytes = 0;
  const ids = new Set<string>();
  for (const asset of raw.assets) {
    if (
      !safeId.test(asset.id) ||
      ids.has(asset.id) ||
      typeof asset.name !== "string" ||
      asset.name.length > 240 ||
      !(asset.data instanceof Uint8Array)
    )
      throw new Error("Invalid Blender asset");
    ids.add(asset.id);
    bytes += asset.data.byteLength;
  }
  if (bytes > 256 * 1024 * 1024)
    throw new Error("Blender inputs exceed 256 MB");
  if (raw.operation === "generate")
    raw = { ...raw, recipe: recipeSchema.parse(raw.recipe) };
  if (
    raw.operation === "import" &&
    (!raw.sourceAssetId || !ids.has(raw.sourceAssetId))
  )
    throw new Error("Missing Blender source");
  if (raw.operation === "render") {
    const s = structuredClone(raw.snapshot) as any;
    raw = { ...raw, snapshot: s };
    if (
      !s ||
      !Number.isInteger(s.width) ||
      !Number.isInteger(s.height) ||
      s.width < 2 ||
      s.height < 2 ||
      s.width > 8192 ||
      s.height > 8192 ||
      !Array.isArray(s.objects) ||
      s.objects.length > 512 ||
      !Array.isArray(s.lights) ||
      s.lights.length > 16
    )
      throw new Error("Invalid Blender render snapshot");
    s.settings = render3DSchema.parse(s.settings);
    let vertices = 0;
    const numeric = (list: any, max: number) =>
      Array.isArray(list) &&
      list.length <= max &&
      list.every(
        (v: any) =>
          typeof v === "number" && Number.isFinite(v) && Math.abs(v) <= 1e8,
      );
    if (
      !s.camera ||
      !numeric(s.camera.matrix, 16) ||
      s.camera.matrix.length !== 16 ||
      !Number.isFinite(s.camera.fov) ||
      s.camera.fov <= 0 ||
      s.camera.fov >= 180 ||
      !Number.isFinite(s.camera.zoom) ||
      s.camera.zoom <= 0 ||
      !Number.isFinite(s.camera.near) ||
      !Number.isFinite(s.camera.far) ||
      s.camera.near <= 0 ||
      s.camera.far <= s.camera.near
    )
      throw new Error("Invalid render camera");
    const bounded = (value: any, min: number, max: number) =>
      typeof value === "number" &&
      Number.isFinite(value) &&
      value >= min &&
      value <= max;
    const rgb = (value: any) =>
      typeof value === "string" && /^#[0-9a-f]{6}$/i.test(value);
    if (
      s.camera.orthographic &&
      !bounded(s.camera.orthoHeight, 0.0001, 100000000)
    )
      throw new Error("Invalid orthographic camera");
    for (const light of s.lights) {
      if (
        !safeId.test(light.id) ||
        !["sun", "point", "spot", "area"].includes(light.type) ||
        !numeric(light.position, 3) ||
        light.position.length !== 3 ||
        !numeric(light.target, 3) ||
        light.target.length !== 3 ||
        !rgb(light.color) ||
        !bounded(light.intensity, 0, 10000) ||
        (light.distance !== undefined && !bounded(light.distance, 0, 100000))
      )
        throw new Error("Invalid render light");
    }
    if (
      s.environment &&
      (!bounded(s.environment.ambient, 0, 100) ||
        !rgb(s.environment.ambientColor) ||
        !bounded(s.environment.exposure, 0.01, 20))
    )
      throw new Error("Invalid render environment");
    if (s.background != null && !rgb(s.background))
      throw new Error("Invalid render background");
    const objectIds = new Set<string>();
    for (const object of s.objects) {
      if (object.generated) {
        object.generated.recipe = recipeSchema.parse(object.generated.recipe);
        if (
          typeof object.generated.part !== "string" ||
          object.generated.part.length > 160
        )
          throw new Error("Invalid generated part");
      }
      if (
        !safeId.test(object.id) ||
        objectIds.has(object.id) ||
        !Array.isArray(object.meshes) ||
        object.meshes.length > 1024
      )
        throw new Error("Invalid render object");
      objectIds.add(object.id);
      for (const mesh of object.meshes) {
        if (
          mesh.matrix &&
          (!numeric(mesh.matrix, 16) || mesh.matrix.length !== 16)
        )
          throw new Error("Invalid model matrix");
        if (
          mesh.normals &&
          (!numeric(mesh.normals, 3000000) ||
            mesh.normals.length !== mesh.positions.length)
        )
          throw new Error("Invalid model normals");
        if (
          !numeric(mesh.positions, 3000000) ||
          mesh.positions.length % 9 ||
          !numeric(mesh.uvs, 2000000) ||
          mesh.uvs.length !== (mesh.positions.length / 3) * 2
        )
          throw new Error("Invalid render geometry");
        vertices += mesh.positions.length / 3;
        mesh.material ||= {};
        if (mesh.material.shader) {
          mesh.material.shader = blenderMaterialSchema.parse(
            mesh.material.shader,
          );
          const shader = mesh.material.shader;
          for (const id of [
            shader.source?.assetId,
            ...(shader.graph?.nodes || []).map((node: any) => node.image),
          ].filter(Boolean))
            if (!ids.has(id)) throw new Error("Missing shader asset");
        }
        for (const id of Object.values(mesh.material?.maps || {}))
          if (typeof id !== "string" || !ids.has(id))
            throw new Error("Missing texture asset");
      }
    }
    if (vertices > 1500000)
      throw new Error("Render exceeds 1.5 million vertices");
    if (JSON.stringify(s).length > 64 * 1024 * 1024)
      throw new Error("Render snapshot is too large");
  }
  return raw;
}

export async function discoverBlender(
  configured?: string,
): Promise<BlenderStatus> {
  const candidates = configured
    ? [configured]
    : [
        process.env["POWERMOVE_BLENDER_PATH"],
        ...(process.platform === "darwin"
          ? [
              "/Applications/Blender.app/Contents/MacOS/Blender",
              path.join(
                os.homedir(),
                "Applications/Blender.app/Contents/MacOS/Blender",
              ),
            ]
          : process.platform === "win32"
            ? [
                "C:\\Program Files\\Blender Foundation\\Blender 5.2\\blender.exe",
                "C:\\Program Files\\Blender Foundation\\Blender 4.5\\blender.exe",
              ]
            : ["/usr/bin/blender", "/usr/local/bin/blender"]),
        ...(process.env["PATH"] || "")
          .split(path.delimiter)
          .filter(Boolean)
          .map((p) =>
            path.join(
              p,
              process.platform === "win32" ? "blender.exe" : "blender",
            ),
          ),
      ].filter((s): s is string => !!s);
  for (const executable of candidates) {
    if (!path.isAbsolute(executable)) continue;
    if (
      !(await access(executable).then(
        () => true,
        () => false,
      ))
    )
      continue;
    try {
      const version = await new Promise<string>((resolve, reject) => {
        const p = spawn(executable, ["--version"]);
        let out = "";
        const timer = setTimeout(() => {
          p.kill("SIGKILL");
          reject(new Error("Blender detection timed out"));
        }, 10000);
        p.stdout.on("data", (b) => {
          out = (out + b).slice(0, 4000);
        });
        p.on("error", (e) => {
          clearTimeout(timer);
          reject(e);
        });
        p.on("close", (code) => {
          clearTimeout(timer);
          code === 0
            ? resolve(out.split("\n")[0] || "Blender")
            : reject(new Error("Blender could not start"));
        });
      });
      const match = /^Blender (\d+)\.(\d+)/.exec(version);
      if (
        !match ||
        Number(match[1]) < 4 ||
        (Number(match[1]) === 4 && Number(match[2]) < 5)
      )
        return {
          available: false,
          error: "Blender 4.5 or newer is required",
          version,
          path: executable,
        };
      return { available: true, path: executable, version };
    } catch {}
  }
  return {
    available: false,
    error:
      "Install Blender or choose its executable to enable modeling and rendered previews.",
  };
}

type Pending = {
  resolve: (value: any) => void;
  reject: (error: Error) => void;
};
/** One serialized bpy process. It never runs project-provided scripts or shell commands. */
export class BlenderWorker {
  private process: ChildProcessWithoutNullStreams | null = null;
  private root = "";
  private pending = new Map<string, Pending>();
  private tail: Promise<unknown> = Promise.resolve();
  private generation = 0;
  private queued = new Set<string>();
  private queuedBytes = 0;
  constructor(
    readonly executable: string,
    readonly tempRoot = os.tmpdir(),
    readonly timeoutMs = 10 * 60 * 1000,
  ) {}
  private async start() {
    if (this.process) return;
    this.root = await mkdtemp(path.join(this.tempRoot, "powermove-blender-"));
    const script = path.join(this.root, "worker.py");
    await writeFile(script, BLENDER_WORKER, { mode: 0o600 });
    const proc = spawn(
      this.executable,
      [
        "--background",
        "--factory-startup",
        "--disable-autoexec",
        "--python-exit-code",
        "1",
        "--python",
        script,
      ],
      {
        stdio: ["pipe", "pipe", "pipe"],
        env: { ...process.env, PYTHONUNBUFFERED: "1" },
      },
    );
    this.process = proc;
    let stdout = "",
      errors = "";
    proc.stdin.on("error", () => {});
    proc.stderr.on("data", (b) => {
      errors = (errors + b).slice(-8000);
    });
    proc.stdout.on("data", (b) => {
      stdout += b;
      let index;
      while ((index = stdout.indexOf("\n")) >= 0) {
        const line = stdout.slice(0, index);
        stdout = stdout.slice(index + 1);
        if (!line.startsWith("PM_BLENDER:")) continue;
        try {
          const msg = JSON.parse(line.slice(11)),
            job = this.pending.get(msg.id);
          if (job) {
            this.pending.delete(msg.id);
            msg.ok
              ? job.resolve(msg)
              : job.reject(new Error(msg.error || "Blender operation failed"));
          }
        } catch {}
      }
      if (stdout.length > 1e6) stdout = stdout.slice(-8000);
    });
    const failed = (message: string) => {
      if (this.process !== proc) return;
      this.process = null;
      this.generation++;
      for (const p of this.pending.values()) p.reject(new Error(message));
      this.pending.clear();
      const root = this.root;
      this.root = "";
      if (root) void rm(root, { recursive: true, force: true }).catch(() => {});
    };
    proc.on("error", (e) => failed(e.message));
    proc.on("close", (code) =>
      failed(
        `Blender stopped (${code ?? "cancelled"}). ${errors.slice(-2000)}`,
      ),
    );
  }
  run(raw: BlenderJob): Promise<BlenderOutput> {
    const job = validateBlenderJob(raw),
      weight =
        job.assets.reduce((sum, a) => sum + a.data.byteLength, 0) +
        (job.snapshot ? JSON.stringify(job.snapshot).length : 0);
    if (
      this.queuedBytes + weight > 512 * 1024 * 1024 ||
      this.queued.size >= 16 ||
      this.queued.has(job.id)
    )
      return Promise.reject(
        new Error("Blender queue is full or request already exists"),
      );
    this.queued.add(job.id);
    this.queuedBytes += weight;
    const generation = this.generation;
    const work = this.tail
      .catch(() => {})
      .then(async () => {
        if (generation !== this.generation)
          throw new Error("Blender request cancelled");
        await this.start();
        const dir = await mkdtemp(path.join(this.root, "job-"));
        try {
          const assets: Record<string, string> = {};
          for (const asset of job.assets) {
            const extension = path.extname(asset.name).toLowerCase();
            if (
              ![
                ".blend",
                ".glb",
                ".gltf",
                ".obj",
                ".png",
                ".jpg",
                ".jpeg",
                ".webp",
                ".exr",
                ".hdr",
              ].includes(extension)
            )
              throw new Error("Unsupported Blender asset format");
            const file = path.join(dir, asset.id + extension);
            await writeFile(file, asset.data);
            assets[asset.id] = file;
          }
          const request = { ...job, assets, dir };
          const requestPath = path.join(dir, "request.json");
          await writeFile(requestPath, JSON.stringify(request));
          await new Promise<void>((resolve, reject) => {
            const timer = setTimeout(() => {
              this.cancel();
              reject(new Error("Blender operation timed out"));
            }, this.timeoutMs);
            this.pending.set(job.id, {
              resolve: () => {
                clearTimeout(timer);
                resolve();
              },
              reject: (e) => {
                clearTimeout(timer);
                reject(e);
              },
            });
            this.process!.stdin.write(
              JSON.stringify({ id: job.id, path: requestPath }) + "\n",
              (e) => {
                if (e) {
                  clearTimeout(timer);
                  this.pending.delete(job.id);
                  reject(e);
                }
              },
            );
          });
          const manifest = JSON.parse(
            await readFile(path.join(dir, "manifest.json"), "utf8"),
          );
          const files: BlenderOutput["files"] = [];
          let bytes = 0;
          for (const file of manifest.files || []) {
            if (
              !safeId.test(file.id) ||
              typeof file.name !== "string" ||
              path.basename(file.name) !== file.name ||
              file.name.length > 240
            )
              throw new Error("Invalid Blender output");
            const filename = path.join(dir, file.name);
            const size = (await stat(filename)).size;
            bytes += size;
            if (bytes > 512 * 1024 * 1024)
              throw new Error("Blender output exceeds 512 MB");
            files.push({
              id: file.id,
              name: file.name,
              data: new Uint8Array(await readFile(filename)),
            });
          }
          delete manifest.files;
          return { files, manifest };
        } finally {
          await rm(dir, { recursive: true, force: true });
        }
      });
    this.tail = work;
    return work.finally(() => {
      this.queued.delete(job.id);
      this.queuedBytes -= weight;
    });
  }
  cancel() {
    this.generation++;
    const proc = this.process;
    this.process = null;
    proc?.stdin.destroy();
    proc?.kill("SIGKILL");
    for (const p of this.pending.values())
      p.reject(new Error("Blender request cancelled"));
    this.pending.clear();
    const root = this.root;
    this.root = "";
    if (root) void rm(root, { recursive: true, force: true }).catch(() => {});
  }
  dispose() {
    this.cancel();
  }
}
