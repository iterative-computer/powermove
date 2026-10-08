import { readFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';
import { parse } from 'smol-toml';

/** Enforce Project access with the bundled Codex sandbox for native Windows
 * commands, including Claude. Initialization failure fails the command; it
 * never retries with broader access. `codex sandbox` makes no model request. */
export function windowsSandboxArgs(root: string, command: readonly string[], writableRoots: readonly string[] = [], defaults: readonly string[] = []): string[] {
  const filesystem = ['":root" = "read"', ...[root, ...writableRoots].map(value => `${JSON.stringify(value)} = "write"`)].join(', ');
  return ['sandbox', '--permission-profile', 'powermove-project', '--cd', root, '--include-managed-config', ...defaults,
    '--config', `permissions.powermove-project={filesystem={${filesystem}},network={enabled=true}}`,
    '--', ...command];
}
/** Fresh installations can sandbox without an administrator setup prompt.
 * Explicit native or legacy sandbox selections remain under user control. */
export function needsWindowsSandboxDefault(config: Record<string, unknown>): boolean {
  const windows = config.windows as { sandbox?: unknown } | undefined;
  const features = config.features as Record<string, unknown> | undefined;
  return windows?.sandbox === undefined && !['experimental_windows_sandbox', 'enable_experimental_windows_sandbox', 'elevated_windows_sandbox'].some(key => features?.[key] === true);
}

export async function windowsSandboxDefaultArgs(): Promise<string[]> {
  const home = process.env.CODEX_HOME?.trim() || path.join(homedir(), '.codex');
  let config: Record<string, unknown> = {};
  try { config = parse(await readFile(path.join(home, 'config.toml'), 'utf8')); }
  catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error; }
  return needsWindowsSandboxDefault(config) ? ['--config', 'windows.sandbox="unelevated"'] : [];
}
