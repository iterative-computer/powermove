import { chmodSync, constants, copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { randomUUID } from 'node:crypto';

type JsonObject = Record<string, unknown>;
const object = (value: unknown): value is JsonObject => !!value && typeof value === 'object' && !Array.isArray(value);
function fileExists(file: string): boolean {
  try { lstatSync(file); return true; } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false;
    throw error;
  }
}
const readObject = (file: string): JsonObject => {
  const stat = lstatSync(file);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 4 * 1024 * 1024) throw new Error('Unsupported configuration');
  const value: unknown = JSON.parse(readFileSync(file, 'utf8'));
  if (!object(value)) throw new Error('Invalid configuration');
  return value;
};

function writeObject(file: string, value: JsonObject): void {
  const temporary = `${file}.${randomUUID()}.tmp`;
  try {
    writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600, flag: 'wx' });
    renameSync(temporary, file);
  } finally {
    if (existsSync(temporary)) unlinkSync(temporary);
  }
}

export type ClaudeDesktopRegistration = 'registered' | 'ready' | 'unavailable' | 'customized' | 'removed' | 'failed';

/** One-time setup for the installed Claude Mac app. Never accesses Claude authentication,
 * replaces a custom server, or reconnects after someone removes the connection. */
export function registerClaudeDesktopMcp(options: {
  homeDir: string;
  userDataDir: string;
  executablePath: string;
  claudeInstalled: boolean;
}): ClaudeDesktopRegistration {
  const directory = path.join(options.homeDir, 'Library', 'Application Support', 'Claude');
  const configFile = path.join(directory, 'claude_desktop_config.json');
  const receiptDirectory = path.join(options.userDataDir, 'mcp');
  const receiptFile = path.join(receiptDirectory, 'claude-desktop-registration.json');
  try {
    if (!fileExists(directory)) {
      if (!options.claudeInstalled) return 'unavailable';
      mkdirSync(directory, { recursive: true, mode: 0o700 });
    }
    if (!lstatSync(directory).isDirectory() || lstatSync(directory).isSymbolicLink()
      || !path.isAbsolute(options.executablePath)) return 'failed';
    if (fileExists(receiptDirectory) && (!lstatSync(receiptDirectory).isDirectory()
      || lstatSync(receiptDirectory).isSymbolicLink())) return 'failed';
    const receipt = fileExists(receiptFile) ? readObject(receiptFile) : null;
    const config = fileExists(configFile) ? readObject(configFile) : {};
    if (config.mcpServers !== undefined && !object(config.mcpServers)) return 'failed';
    const servers: JsonObject = object(config.mcpServers) ? config.mcpServers : {};
    const existing = servers.powermove;
    // A removed entry is a deliberate disconnect, even across app updates.
    if (receipt?.registered === true && existing === undefined) return 'removed';
    if (existing !== undefined && (!object(existing)
      || !Array.isArray(existing.args) || existing.args.length !== 1 || existing.args[0] !== '--powermove-mcp'
      || typeof existing.command !== 'string'
      || !existing.command.endsWith('/Powermove.app/Contents/MacOS/Powermove')
      || Object.keys(existing).some(key => key !== 'command' && key !== 'args'))) return 'customized';

    const changed = existing === undefined || (existing as JsonObject).command !== options.executablePath;
    if (changed) {
      // Keep one private recovery copy; it may contain another server's secrets.
      const backup = `${configFile}.before-powermove`;
      if (fileExists(configFile) && !fileExists(backup)) {
        copyFileSync(configFile, backup, constants.COPYFILE_EXCL);
        // copyFile retains the source's permissions, so restrict the backup too.
        chmodSync(backup, 0o600);
      }
      servers.powermove = { command: options.executablePath, args: ['--powermove-mcp'] };
      config.mcpServers = servers;
      writeObject(configFile, config);
    }
    if (!receipt) {
      mkdirSync(receiptDirectory, { recursive: true, mode: 0o700 });
      writeObject(receiptFile, { registered: true });
    }
    return changed ? 'registered' : 'ready';
  } catch {
    // A broken/custom configuration must never prevent the editor opening.
    return 'failed';
  }
}
