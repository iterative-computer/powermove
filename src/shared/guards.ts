/* Runtime narrowing helpers used by main-process IPC handlers. */

export const isRecord = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

export const isString = (v: unknown, max = Infinity): v is string => typeof v === 'string' && v.length <= max;

export const isBytes = (v: unknown, max = Infinity): v is Uint8Array =>
  v instanceof Uint8Array && v.byteLength <= max;

export const isOneOf = <T extends readonly string[]>(v: unknown, options: T): v is T[number] =>
  typeof v === 'string' && (options as readonly string[]).includes(v);

export const isArrayOf = <T>(v: unknown, max: number, item: (x: unknown) => x is T): v is T[] =>
  Array.isArray(v) && v.length <= max && v.every(item);

export class IpcValidationError extends Error {
  constructor(channel: string, detail: string) {
    super(`${channel}: ${detail}`);
    this.name = 'IpcValidationError';
  }
}
