import type { Data } from "../../types";
export function createRpc(
  port: any,
  handlers: Data,
  timeout?: number,
  limits?: Data,
): Data;
export function serializeRpcError(error: unknown): Data;
