import path from 'node:path';
import type { ServiceSpec } from './install';
import { logPath } from './install';
import { powershellPath, windowsScript } from '../main/windows-system';

export const WINDOWS_TASK = 'Powermove Host';
const xml = (value: string) => value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const literal = (value: string) => `'${value.replaceAll("'", "''")}'`;

/** A normal-user logon task. The hidden PowerShell parent remains alive so
 * Task Scheduler can stop the host and its descendants on uninstall. */
export function windowsTask(spec: ServiceSpec): string {
  const supervisor = path.win32.join(spec.userData, 'serve-service.cjs');
  const script = `& ${literal(spec.node)} ${literal(supervisor)}; exit $LASTEXITCODE`;
  const args = `-NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -EncodedCommand ${Buffer.from(script, 'utf16le').toString('base64')}`;
  return `<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>Powermove browser host</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Hidden>true</Hidden><AllowHardTerminate>true</AllowHardTerminate></Settings>
  <Actions Context="Author"><Exec><Command>${xml(powershellPath())}</Command><Arguments>${xml(args)}</Arguments><WorkingDirectory>${xml(spec.userData)}</WorkingDirectory></Exec></Actions>
</Task>
`;
}

export function windowsSupervisor(spec: ServiceSpec): string {
  // All inputs are JS data, with no shell or command-string interpolation.
  return `const { spawn } = require('node:child_process');
const { openSync, writeFileSync } = require('node:fs');
const fd = openSync(${JSON.stringify(logPath(spec.userData, 'win32'))}, 'a');
writeFileSync(${JSON.stringify(path.win32.join(spec.userData, 'serve-service.pid'))}, String(process.pid));
let child, stopping = false;
function start() {
  child = spawn(${JSON.stringify(spec.node)}, ${JSON.stringify([spec.entry, 'serve', ...spec.args])}, { env: { ...process.env, POWERMOVE_USER_DATA: ${JSON.stringify(spec.userData)} }, windowsHide: true, stdio: ['ignore', fd, fd] });
  child.once('error', error => { require('node:fs').writeSync(fd, error.message + '\\n'); });
  child.once('close', () => { if (!stopping) setTimeout(start, 3000); });
}
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => { stopping = true; child?.kill(); });
start();
`;
}

export async function installWindowsTask(spec: ServiceSpec): Promise<void> {
  await removeWindowsTask(spec);
  await windowsScript('Register-ScheduledTask -TaskName $data.name -Xml $data.xml -Force | Out-Null; Start-ScheduledTask -TaskName $data.name', { name: WINDOWS_TASK, xml: windowsTask(spec) });
}

export async function removeWindowsTask(spec: ServiceSpec): Promise<void> {
  // Task Scheduler can stop the PowerShell action while leaving its Node child
  // alive. Verify the supervisor command before stopping its entire tree.
  await windowsScript('$task = Get-ScheduledTask -TaskName $data.name -ErrorAction SilentlyContinue; if ($task) { Stop-ScheduledTask -TaskName $data.name }; if (Test-Path -LiteralPath $data.pidFile) { $supervisorPid = [int](Get-Content -LiteralPath $data.pidFile -Raw); $process = Get-CimInstance Win32_Process -Filter ("ProcessId=" + $supervisorPid); if ($process -and $process.ExecutablePath -eq $data.node -and $process.CommandLine.Contains($data.supervisor)) { & taskkill.exe /PID $supervisorPid /T /F | Out-Null; if ($LASTEXITCODE -ne 0 -and (Get-Process -Id $supervisorPid -ErrorAction SilentlyContinue)) { throw "Could not stop the Powermove host" } }; Remove-Item -LiteralPath $data.pidFile -Force }; if ($task) { Unregister-ScheduledTask -TaskName $data.name -Confirm:$false }', {
    name: WINDOWS_TASK, node: spec.node,
    supervisor: path.win32.join(spec.userData, 'serve-service.cjs'),
    pidFile: path.win32.join(spec.userData, 'serve-service.pid')
  });
}

export async function windowsTaskStatus(): Promise<string> {
  return windowsScript('$task = Get-ScheduledTask -TaskName $data -ErrorAction SilentlyContinue; if ($task) { $task.State.ToString() } else { "not installed" }', WINDOWS_TASK);
}
