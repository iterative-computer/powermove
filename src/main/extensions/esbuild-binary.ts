import { existsSync } from 'node:fs';
import path from 'node:path';

type EsbuildModule = typeof import('esbuild');

let esbuildPromise: Promise<EsbuildModule> | undefined;

/**
 * Electron can read native files through app.asar, but child_process cannot
 * execute them there. electron-builder mirrors esbuild's platform binary into
 * app.asar.unpacked, so packaged builds must point the JavaScript API at that
 * executable before importing esbuild.
 */
export function resolvePackagedEsbuildBinary(
  resourcesPath: string | undefined = process.resourcesPath,
  platform = process.platform,
  arch = process.arch
): string | undefined {
  if (!resourcesPath) return undefined;

  const executable = platform === 'win32' ? 'esbuild.exe' : 'esbuild';
  const candidate = path.join(resourcesPath, 'app.asar.unpacked', 'node_modules', '@esbuild', `${platform}-${arch}`, 'bin', executable);
  return existsSync(candidate) ? candidate : undefined;
}

export function loadEsbuild(): Promise<EsbuildModule> {
  if (!esbuildPromise) {
    const binaryPath = resolvePackagedEsbuildBinary();
    if (binaryPath) process.env.ESBUILD_BINARY_PATH = binaryPath;
    esbuildPromise = import('esbuild');
  }
  return esbuildPromise;
}
