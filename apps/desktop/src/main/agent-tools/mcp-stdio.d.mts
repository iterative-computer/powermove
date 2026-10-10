import type { Readable, Writable } from 'node:stream';
export function startMcpStdio(options: {
  listTools(): Promise<unknown[]>;
  callTool(name: string, args: Record<string, unknown>, id: string | number): Promise<unknown>;
  close?(): Promise<void>;
  instructions?: string;
  input?: Readable;
  output?: Writable;
}): Promise<unknown>;
