import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
/** Packaging must never silently ship a missing/stale plugin, even when the
 * packager is invoked directly rather than through the desktop build command. */
export default function beforePack(context) {
  const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
  if (context.electronPlatformName === 'darwin') {
    // Direct electron-builder invocations must build the same release helper
    // and relocatable libraries as `bun run build`, never ship stale output.
    execFileSync(process.execPath, [path.join(desktop, 'scripts/build-studio-native.mjs')], { stdio: 'inherit' });
  }
  const plugin = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../packages/figma-plugin');
  execFileSync('bun', ['run', '--cwd', plugin, 'build'], { stdio: 'inherit' });
}
