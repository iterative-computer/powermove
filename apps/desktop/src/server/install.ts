/*
 * `powermove install`: keep the host running as a user service, so a box
 * serves Powermove across reboots and logouts. systemd user units on Linux,
 * a launchd agent on macOS. `uninstall`, `status` and `logs` complete the
 * set. The unit runs the same `serve` entry with the flags given here.
 */
import { execFile } from 'node:child_process';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

const run = promisify(execFile);

export interface ServiceSpec {
  /** Absolute path to bin/powermove.mjs. */
  entry: string;
  node: string;
  /** Flags after `serve`, e.g. ['--port', '4747']. */
  args: string[];
  /** Profile directory; the log lives here. */
  userData: string;
  home?: string;
  platform?: NodeJS.Platform;
  /** Installed version, for status. */
  version?: string;
}

export const SERVICE_NAME = 'powermove';
export const LAUNCHD_LABEL = 'com.zellzoi.powermove.serve';

const quote = (value: string): string => (/^[A-Za-z0-9_@%+=:,./-]+$/.test(value) ? value : `"${value.replace(/(["\\$`])/g, '\\$1')}"`);
const xml = (value: string): string => value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

export function logPath(userData: string): string { return path.join(userData, 'serve.log'); }

export function systemdUnit(spec: ServiceSpec): string {
  const command = [spec.node, spec.entry, 'serve', ...spec.args].map(quote).join(' ');
  return `[Unit]
Description=Powermove host
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${command}
Environment=POWERMOVE_USER_DATA=${quote(spec.userData)}
Restart=always
RestartSec=3
StandardOutput=append:${logPath(spec.userData)}
StandardError=append:${logPath(spec.userData)}

[Install]
WantedBy=default.target
`;
}

export function launchdPlist(spec: ServiceSpec): string {
  const program = [spec.node, spec.entry, 'serve', ...spec.args].map((item) => `    <string>${xml(item)}</string>`).join('\n');
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>${LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
${program}
  </array>
  <key>EnvironmentVariables</key>
  <dict><key>POWERMOVE_USER_DATA</key><string>${xml(spec.userData)}</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>${xml(logPath(spec.userData))}</string>
  <key>StandardErrorPath</key><string>${xml(logPath(spec.userData))}</string>
</dict>
</plist>
`;
}

export function unitPath(spec: ServiceSpec): string {
  const home = spec.home ?? homedir();
  return (spec.platform ?? process.platform) === 'darwin'
    ? path.join(home, 'Library', 'LaunchAgents', `${LAUNCHD_LABEL}.plist`)
    : path.join(home, '.config', 'systemd', 'user', `${SERVICE_NAME}.service`);
}

export function unitText(spec: ServiceSpec): string {
  return (spec.platform ?? process.platform) === 'darwin' ? launchdPlist(spec) : systemdUnit(spec);
}

async function launchctlDomain(): Promise<string> {
  const { stdout } = await run('id', ['-u']);
  return `gui/${stdout.trim()}`;
}

export async function install(spec: ServiceSpec, log: (line: string) => void): Promise<void> {
  const platform = spec.platform ?? process.platform;
  if (platform !== 'darwin' && platform !== 'linux') throw new Error('Install is supported on Linux (systemd) and macOS (launchd).');
  const file = unitPath(spec);
  await mkdir(path.dirname(file), { recursive: true });
  await mkdir(spec.userData, { recursive: true });
  await writeFile(file, unitText(spec));
  log(`wrote ${file}`);
  if (platform === 'darwin') {
    const domain = await launchctlDomain();
    await run('launchctl', ['bootout', `${domain}/${LAUNCHD_LABEL}`]).catch(() => undefined);
    await run('launchctl', ['bootstrap', domain, file]);
    log(`started ${LAUNCHD_LABEL} (launchd). It restarts with your login.`);
  } else {
    await run('systemctl', ['--user', 'daemon-reload']);
    await run('systemctl', ['--user', 'enable', '--now', `${SERVICE_NAME}.service`]);
    log(`started ${SERVICE_NAME}.service (systemd --user).`);
    log('To keep it running while you are logged out: loginctl enable-linger $USER');
  }
  log(`logs: ${logPath(spec.userData)}  (powermove logs)`);
}

export async function uninstall(spec: ServiceSpec, log: (line: string) => void): Promise<void> {
  const platform = spec.platform ?? process.platform;
  const file = unitPath(spec);
  if (platform === 'darwin') {
    const domain = await launchctlDomain();
    await run('launchctl', ['bootout', `${domain}/${LAUNCHD_LABEL}`]).catch(() => undefined);
  } else if (platform === 'linux') {
    await run('systemctl', ['--user', 'disable', '--now', `${SERVICE_NAME}.service`]).catch(() => undefined);
    await run('systemctl', ['--user', 'daemon-reload']).catch(() => undefined);
  }
  await rm(file, { force: true });
  log(`removed ${file}`);
}

export async function status(spec: ServiceSpec, log: (line: string) => void): Promise<void> {
  const platform = spec.platform ?? process.platform;
  try {
    if (platform === 'darwin') {
      const { stdout } = await run('launchctl', ['print', `${await launchctlDomain()}/${LAUNCHD_LABEL}`]);
      const state = /state = (\w+)/.exec(stdout)?.[1] ?? 'unknown';
      const pid = /pid = (\d+)/.exec(stdout)?.[1];
      log(`${LAUNCHD_LABEL}: ${state}${pid ? ` (pid ${pid})` : ''}`);
    } else {
      const { stdout } = await run('systemctl', ['--user', 'is-active', `${SERVICE_NAME}.service`]).catch((error: { stdout?: string }) => ({ stdout: error.stdout ?? 'inactive' }));
      log(`${SERVICE_NAME}.service: ${stdout.trim()}`);
    }
  } catch {
    log('not installed (powermove install)');
  }
  try {
    const text = await readFile(logPath(spec.userData), 'utf8');
    const url = [...text.matchAll(/https?:\/\/\S+\?token=\S+/g)].at(-1)?.[0];
    if (url) log(`last address: ${url}`);
  } catch { /* no log yet */ }
  try {
    const response = await fetch('https://registry.npmjs.org/powermove-cli/latest', { signal: AbortSignal.timeout(5000) });
    const latest = ((await response.json()) as { version?: string }).version;
    if (latest) log(`latest on npm: ${latest}${spec.version && latest !== spec.version ? `  (this install: ${spec.version}; update with npx powermove-cli@latest install)` : ''}`);
  } catch { /* offline */ }
}

export async function logs(spec: ServiceSpec, lines: number, log: (line: string) => void): Promise<void> {
  try {
    const text = await readFile(logPath(spec.userData), 'utf8');
    for (const line of text.trimEnd().split('\n').slice(-lines)) log(line);
  } catch {
    log(`no log at ${logPath(spec.userData)} yet`);
  }
}
