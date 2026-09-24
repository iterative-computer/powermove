import { chmod, mkdir } from 'node:fs/promises';
import path from 'node:path';

export const ISOLATED_CLAUDE_HOME_NAME = 'claude-runtime';

export function isolatedClaudeHome(userData: string): string {
  return path.join(userData, ISOLATED_CLAUDE_HOME_NAME);
}

export async function prepareIsolatedClaudeHome(userData: string): Promise<string> {
  const directory = isolatedClaudeHome(userData);
  await mkdir(directory, { recursive: true, mode: 0o700 });
  await chmod(directory, 0o700);
  return directory;
}

/** Claude Code owns every credential inside this directory. Powermove never
 * reads or copies OAuth material, and logout cannot affect a terminal login. */
export function isolatedClaudeEnvironment(configDirectory: string): NodeJS.ProcessEnv {
  // Safe mode suppresses MCP servers, including the run-scoped Powermove tools.
  // Keep the isolated config directory and let --strict-mcp-config select only
  // the server supplied for this run.
  const environment = { ...process.env };
  delete environment.CLAUDE_CODE_SAFE_MODE;
  return {
    ...environment,
    CLAUDE_CONFIG_DIR: configDirectory
  };
}
