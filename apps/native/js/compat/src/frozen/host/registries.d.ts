import type { Data } from "../../types";
export function createKernel(): Data;
export function runKernelCommand(
  kernel: Data,
  id: string,
  args?: unknown[],
): unknown;
export function settledWhen(ask: () => boolean | Promise<boolean>): {
  when(): boolean;
  check: { last(): boolean; ask(): boolean | Promise<boolean> };
};
export const ASYNC_CONTRIBUTOR: unique symbol;
export function whenCheck(definition: unknown): unknown;
export function withWhenCheck<T extends object>(
  definition: T,
  check: unknown,
): T;
