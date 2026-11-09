/**
 * The shipped source of each built-in panel as a v4 extension tree, for
 * `fork_builtin_extension`. Imports of the shared runtime become the published
 * `powermove/*` modules a fork links against; everything else is the panel's
 * own source, unchanged. Usage: bun scripts/builtin-sources.ts <out.json>
 */
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { version } from "../package.json";
import panels from "../app/panels.json";

const ROOT = resolve(import.meta.dir, "..");
const SKIP =
  /(?:\.test\.tsx?$|(?:^|\/)(?:test|tests|__snapshots__|conformance|native|wire|snapshots)\/|^bench\.tsx$|^nodefree|^native-test\.ts$|^tsconfig\.json$|\.rs$|\.sh$|^bunfig\.toml$|^package\.json$)/;
/** Runtime source module to the specifier a fork imports it by. */
const MODULES: Record<string, string> = {
  "src/index.ts": "powermove/runtime",
  "src/protocol.ts": "powermove/protocol",
  "src/host-api.ts": "powermove/host-api",
  "src/components/index.ts": "powermove/components",
  "src/memo.ts": "powermove/memo",
  "src/renderer.ts": "powermove/renderer",
  "src/jsx-runtime.ts": "powermove/jsx-runtime",
  "src/builtin.ts": "powermove/builtin",
  "src/ui.ts": "powermove/ui",
  "src/hooks.ts": "powermove/hooks",
};
function files(dir: string, prefix = ""): string[] {
  return readdirSync(dir)
    .sort()
    .flatMap((name) => {
      const path = prefix ? `${prefix}/${name}` : name;
      if (SKIP.test(path)) return [];
      return statSync(join(dir, name)).isDirectory()
        ? files(join(dir, name), path)
        : /\.(tsx?|md)$/.test(name)
          ? [path]
          : [];
    });
}
function runtimeModule(from: string, specifier: string): string | undefined {
  const target = relative(ROOT, resolve(dirname(from), specifier));
  for (const candidate of [
    target,
    `${target}.ts`,
    `${target}.tsx`,
    `${target}/index.ts`,
  ])
    if (MODULES[candidate]) return MODULES[candidate];
  return undefined;
}
const output: Record<string, Record<string, string>> = {};
for (const { id, folder, title: name } of panels) {
  const base = join(ROOT, "panels", folder);
  const tree: Record<string, string> = {};
  for (const path of files(base)) {
    const file = join(base, path);
    tree[path] = readFileSync(file, "utf8").replace(
      /(\bfrom\s*|\bimport\s*\(\s*)"(\.[^"]*)"/g,
      (match, keyword: string, specifier: string) => {
        const inside = resolve(dirname(file), specifier).startsWith(base);
        if (inside) return match;
        const module = runtimeModule(file, specifier);
        if (!module)
          throw new Error(`${folder}/${path} imports ${specifier} outside its panel`);
        return `${keyword}"${module}"`;
      },
    );
  }
  const hash = createHash("sha256");
  for (const [path, text] of Object.entries(tree)) hash.update(`${path}\0${text}\0`);
  // A changed panel ships a new version, so stage_fork_rebase sees it.
  const patch = parseInt(hash.digest("hex").slice(0, 8), 16) % 999_999 + 1;
  const [major, minor] = version.split(".");
  tree["manifest.json"] = `${JSON.stringify(
    {
      id,
      name,
      version: `${major}.${minor}.${patch}`,
      apiVersion: 4,
      entry: "mount.tsx",
      author: "powermove",
      description: `The built-in ${name} panel.`,
    },
    null,
    2,
  )}\n`;
  output[id] = tree;
}
writeFileSync(process.argv[2]!, JSON.stringify(output));
