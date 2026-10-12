import { access, constants } from 'node:fs/promises';
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

  // Powermove's FFmpeg (export, proxies, transcription, agent media tools).
  // A bundle without it would install and then fail on first export.
  const ffmpeg = path.join(appPath, 'Contents/Resources/ffmpeg');
  for (const file of ['bin/ffmpeg', 'bin/ffprobe', 'lib/libavcodec.63.dylib', 'NOTICE.txt', 'share/licenses/ffmpeg']) {
    try { await access(path.join(ffmpeg, file), file.startsWith('bin/') ? constants.X_OK : constants.R_OK); }
    catch { throw new Error(`The packaged app is missing Resources/ffmpeg/${file}. Run engine/third_party/ffmpeg/package.sh (bun run stage:ffmpeg) before electron-builder.`); }
  }

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
