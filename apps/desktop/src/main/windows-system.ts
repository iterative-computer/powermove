import { execFile } from 'node:child_process';
import path from 'node:path';
import { promisify } from 'node:util';

const run = promisify(execFile);
export function powershellPath(environment: NodeJS.ProcessEnv = process.env): string {
  return path.win32.join(environment.SystemRoot ?? environment.SYSTEMROOT ?? 'C:\\Windows', 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe');
}

/** Scripts are application code. Data crosses through one encoded JSON value,
 * never through interpolated PowerShell or cmd.exe command text. */
export async function windowsScript(script: string, data: unknown = null): Promise<string> {
  const source = "$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'; [Console]::OutputEncoding = [Text.UTF8Encoding]::new(); "
    + "$data = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($env:POWERMOVE_SCRIPT_DATA)) | ConvertFrom-Json; " + script;
  const { stdout } = await run(powershellPath(), ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(source, 'utf16le').toString('base64')], {
    encoding: 'utf8', timeout: 60_000, maxBuffer: 16 * 1024 * 1024, windowsHide: true,
    env: { ...process.env, POWERMOVE_SCRIPT_DATA: Buffer.from(JSON.stringify(data)).toString('base64') }
  });
  return stdout.trim();
}

export async function windowsFontFamilies(): Promise<string[]> {
  const result: unknown = JSON.parse(await windowsScript('Add-Type -AssemblyName System.Drawing; $fonts = New-Object System.Drawing.Text.InstalledFontCollection; try { ConvertTo-Json -Compress -InputObject @($fonts.Families | ForEach-Object { $_.Name } | Sort-Object -Unique) } finally { $fonts.Dispose() }'));
  if (!Array.isArray(result) || !result.every(value => typeof value === 'string')) throw new Error('Windows returned an invalid font list.');
  return result;
}

export async function windowsCloudFileState(file: string): Promise<'local' | 'cloud' | 'missing'> {
  const value = await windowsScript('if (-not (Test-Path -LiteralPath $data -PathType Leaf)) { "missing" } else { $attributes = [int](Get-Item -LiteralPath $data -Force).Attributes; if (($attributes -band 0x1000) -or ($attributes -band 0x40000) -or ($attributes -band 0x400000)) { "cloud" } else { "local" } }', file);
  if (value !== 'local' && value !== 'cloud' && value !== 'missing') throw new Error('Windows returned an invalid cloud file state.');
  return value;
}
