import type { Data } from "../../types";
export function createSandboxAPI(
  rpc: Data,
  init: Data,
  mode?: "runtime" | "view",
): Data;
export function sandboxControl(api: Data): Data;
export function sandboxReporter(
  rpc: Data,
): (kind: string, member: string) => void;
