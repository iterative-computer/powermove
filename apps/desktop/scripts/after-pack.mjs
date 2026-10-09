import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { stat } from 'node:fs/promises';

import { flipFuses, FuseVersion, FuseV1Options } from '@electron/fuses';

const execFileAsync = promisify(execFile);

export default async function afterPack(context) {
  const windows = context.electronPlatformName === 'win32';
  if (windows) {
    const locale = path.join(context.appOutDir, 'locales', 'en-US.pak');
    const localeInfo = await stat(locale).catch(() => null);
    if (!localeInfo?.isFile() || localeInfo.size === 0) throw new Error('Missing Windows language resources: locales/en-US.pak');
    const resources = path.join(context.appOutDir, 'resources');
    for (const file of ['encoder/ffmpeg.exe', 'codex/bin/codex.exe', 'codex/bin/codex-code-mode-host.exe', 'claude/bin/claude.exe']) {
      const binary = path.join(resources, file);
      if (!(await stat(binary)).isFile()) throw new Error(`Missing Windows runtime: ${file}`);
      if (process.platform === 'win32' && !file.includes('code-mode-host')) await execFileAsync(binary, [file.startsWith('encoder/') ? '-version' : '--version'], { timeout: 15_000, windowsHide: true });
    }
  }
  if (!windows && context.electronPlatformName !== 'darwin') return;

  const appPath = path.join(
    context.appOutDir,
    `${context.packager.appInfo.productFilename}.${windows ? 'exe' : 'app'}`
  );

  // File Provider provenance copied from dependencies makes macOS reject even
  // an ad-hoc signature. The packaged bundle is generated output, so remove
  // extended attributes before fuses refresh its signature.
  if (!windows) await execFileAsync('/usr/bin/xattr', ['-cr', appPath]);

  await flipFuses(appPath, {
    version: FuseVersion.V1,
    // Fuses modify the Electron executable before electron-builder's final
    // ad-hoc or Developer ID signing pass, so discard the stale inner signature.
    resetAdHocDarwinSignature: !windows,
    [FuseV1Options.RunAsNode]: false,
    [FuseV1Options.EnableNodeOptionsEnvironmentVariable]: false,
    [FuseV1Options.EnableNodeCliInspectArguments]: false,
    [FuseV1Options.EnableEmbeddedAsarIntegrityValidation]: true,
    [FuseV1Options.OnlyLoadAppFromAsar]: true
  });

  console.log(`[fuses] Applied production fuse policy to ${appPath}`);
}
