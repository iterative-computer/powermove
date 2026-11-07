import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
/** Packaging must never silently ship a missing/stale plugin, even when the
 * packager is invoked directly rather than through the desktop build command. */
export default function beforePack() {
  const plugin = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../packages/figma-plugin');
  execFileSync('bun', ['run', '--cwd', plugin, 'build'], { stdio: 'inherit' });
}
