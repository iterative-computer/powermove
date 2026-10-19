/** The desktop's browser/driver probe namespace, shared by native CLI hosts.
 * Unknown installations get a separate namespace and must use ANGLE. */
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import os from 'node:os';

const FILE = 'desktop-environment.json';
export function persistShaderEnvironment(directory: string, environment: string): string {
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const file = path.join(directory, FILE), temporary = `${file}.${process.pid}.tmp`;
  writeFileSync(temporary, JSON.stringify({ v: 1, environment }), { mode: 0o600 });
  renameSync(temporary, file);
  return environment;
}
export function readShaderEnvironment(directory: string): string {
  try {
    const record = JSON.parse(readFileSync(path.join(directory, FILE), 'utf8'));
    if (record.v === 1 && typeof record.environment === 'string' && record.environment.length > 0 && record.environment.length <= 16_384) return record.environment;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw new Error(`shader-probe-store-unreadable: ${String(error)}`);
  }
  return JSON.stringify({ host: 'native-without-desktop-probes', os: os.version() });
}

/** A hidden profile may supply its own desktop probe copy; otherwise it reads
 * this machine's desktop store. An explicit cache is useful for isolated hosts. */
export function shaderCacheDirectory(userData: string, env: NodeJS.ProcessEnv = process.env,
  desktopProfile = path.join(os.homedir(), 'Library', 'Application Support', 'Powermove')): string {
  if (env['POWERMOVE_ENGINE_SHADER_CACHE']) return path.resolve(env['POWERMOVE_ENGINE_SHADER_CACHE']);
  const local = path.join(userData, 'Engine Cache', 'Shader Cache');
  if (existsSync(path.join(local, FILE))) return local;
  const desktop = path.join(desktopProfile, 'Engine Cache', 'Shader Cache');
  return existsSync(path.join(desktop, FILE)) ? desktop : local;
}
