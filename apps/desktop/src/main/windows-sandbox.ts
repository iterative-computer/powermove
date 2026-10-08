/** Enforce Project access with the bundled Codex sandbox for native Windows
 * commands, including Claude. Initialization failure fails the command; it
 * never retries with broader access. `codex sandbox` makes no model request. */
export function windowsSandboxArgs(root: string, command: readonly string[], writableRoots: readonly string[] = []): string[] {
  const filesystem = ['\":root\" = \"read\"', ...[root, ...writableRoots].map(value => `${JSON.stringify(value)} = \"write\"`)].join(', ');
  return ['sandbox', '--permission-profile', 'powermove-project', '--cd', root, '--include-managed-config',
    '--config', `permissions.powermove-project={filesystem={${filesystem}},network={enabled=true}}`,
    '--', ...command];
}
