/** Enforce Project access with the bundled Codex sandbox for native Windows
 * commands, including Claude. Initialization failure fails the command; it
 * never retries with broader access. `codex sandbox` makes no model request. */
export function windowsSandboxArgs(root: string, command: readonly string[], writableRoots: readonly string[] = []): string[] {
  return ['sandbox', '--cd', root,
    '--config', 'sandbox_mode="workspace-write"',
    '--config', 'sandbox_workspace_write.network_access=true',
    '--config', `sandbox_workspace_write.writable_roots=${JSON.stringify(writableRoots)}`,
    '--', ...command];
}
