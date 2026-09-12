import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

import { flipFuses, FuseVersion, FuseV1Options } from '@electron/fuses';

const execFileAsync = promisify(execFile);

export default async function afterPack(context) {
  if (context.electronPlatformName !== 'darwin') return;

  const appPath = path.join(
    context.appOutDir,
    `${context.packager.appInfo.productFilename}.app`
  );

  // File Provider provenance copied from dependencies makes macOS reject even
  // an ad-hoc signature. The packaged bundle is generated output, so remove
  // extended attributes before fuses refresh its signature.
  await execFileAsync('/usr/bin/xattr', ['-cr', appPath]);

  await flipFuses(appPath, {
    version: FuseVersion.V1,
    // Fuses modify the Electron executable before electron-builder's final
    // ad-hoc or Developer ID signing pass, so discard the stale inner signature.
    resetAdHocDarwinSignature: true,
    [FuseV1Options.RunAsNode]: false,
    [FuseV1Options.EnableNodeOptionsEnvironmentVariable]: false,
    [FuseV1Options.EnableNodeCliInspectArguments]: false,
    [FuseV1Options.EnableEmbeddedAsarIntegrityValidation]: true,
    [FuseV1Options.OnlyLoadAppFromAsar]: true
  });

  console.log(`[fuses] Applied production fuse policy to ${appPath}`);
}
