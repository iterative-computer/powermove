// Source evidence only. Identical Electron files are not native acceptance.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { REFERENCES } from "./references.mjs";
const root = fileURLToPath(new URL("../../", import.meta.url));
export const CARRY = {
  A08: [
    "apps/desktop/src/renderer/src/kernel/theme-state.ts",
    "apps/desktop/src/renderer/src/kernel/workspace-appearance.ts",
    "apps/desktop/src/main/extensions/rebase.ts",
    "apps/desktop/src/main/extensions/update-fork.ts",
  ],
  A09: [
    "apps/desktop/src/renderer/src/legacy/assistant/panel-tools.ts",
    "apps/desktop/src/main/agent-tools/external-session.ts",
  ],
  comparison: [
    "apps/desktop/src/main/extensions/compiler.ts",
    "apps/desktop/src/renderer/src/legacy/core/model.ts",
  ],
};
export function audit() {
  const rows = {};
  for (const [area, files] of Object.entries(CARRY)) {
    rows[area] = files.map((file) => {
      const bytes = (revision) =>
        execFileSync("git", ["-C", root, "show", `${revision}:${file}`]);
      const current = bytes(REFERENCES.current),
        legacy = bytes(REFERENCES.legacy);
      return {
        file,
        equal: current.equals(legacy),
        current_sha256: createHash("sha256").update(current).digest("hex"),
        legacy_sha256: createHash("sha256").update(legacy).digest("hex"),
      };
    });
  }
  return {
    evidence: "source-only",
    native_acceptance: false,
    references: REFERENCES,
    rows,
  };
}
if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
)
  console.log(JSON.stringify(audit(), null, 2));
