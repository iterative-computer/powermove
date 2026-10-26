export function connectMcpHost(userData: string, options?: { onHostClosed?: () => void }): Promise<{
  listTools(): Promise<unknown[]>;
  callTool(name: string, args: Record<string, unknown>): Promise<unknown>;
  close(): Promise<void>;
}>;
