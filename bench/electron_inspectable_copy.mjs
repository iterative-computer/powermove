// Playwright's Electron launcher needs Node's inspector, which the production build disables with the
// EnableNodeCliInspectArguments fuse. This copies the packaged app and flips only that fuse (ad-hoc re-signed).
// The app code, asar and every other fuse are unchanged.
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
const [src, dst, pkg] = process.argv.slice(2);
const { flipFuses, FuseVersion, FuseV1Options } = createRequire(pkg)('@electron/fuses');
execFileSync('rm', ['-rf', dst]); execFileSync('ditto', [src, dst]);
await flipFuses(dst, { version: FuseVersion.V1, resetAdHocDarwinSignature: true, [FuseV1Options.EnableNodeCliInspectArguments]: true });
console.log('ok', dst);
