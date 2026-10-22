import { angleService } from './native/angle';
import { cliProbeEnvironment } from '../main/engine/shader-environment';
import { profileDefinitionsPath } from '../main/engine/profile-definitions';
import { homedir } from 'node:os';
/** The user-facing native renderer. The user chooses both paths, so unlike
 * agent exports this command deliberately has no output-directory restriction. */
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { constants } from 'node:os';
import path from 'node:path';

const profileOf = (env: NodeJS.ProcessEnv): string => env['POWERMOVE_USER_DATA'] ?? path.join(homedir(), 'Library', 'Application Support', 'Powermove');

/** Without `--definitions`, a render draws the definitions the app (or hidden
 *  host) of this profile last published: its user extensions, overrides by id
 *  and forks of built-ins (main/engine/profile-definitions.ts). */
function withProfileDefinitions(options: string[], env: NodeJS.ProcessEnv): string[] {
  if (options.some(option => option === '--definitions' || option.startsWith('--definitions='))) return options;
  const file = profileDefinitionsPath(profileOf(env));
  return existsSync(file) ? [...options, '--definitions', file] : options;
}

export function renderInvocation(argv: string[], distDir: string, env: NodeJS.ProcessEnv = process.env): { file: string; args: string[] } {
  const [project, output, ...options] = argv;
  if (!project || !output || project.startsWith('-') || output.startsWith('-')) throw new Error('Usage: powermove render <project.pmv|project.json> <output path> [pm-cli render options]');
  // The wrapper owns -o: a second destination would silently redirect work.
  if (options.some(option => option === '-o' || option === '--output' || option.startsWith('--output='))) throw new Error('The output path is the second argument; do not pass -o or --output.');
  const file = env['POWERMOVE_PM_CLI'] || path.join(distDir, 'native', `${process.platform}-${process.arch}`, 'pm-cli');
  if (!existsSync(file)) throw new Error(`native-renderer-unavailable: The native renderer is missing at ${file}. Install a Powermove CLI build for ${process.platform}-${process.arch}, or set POWERMOVE_PM_CLI to your pm-cli binary.`);
  return { file, args: ['render', path.resolve(project), '-o', path.resolve(output), ...withProfileDefinitions(options, env)] };
}

export async function renderProject(argv: string[], distDir: string, env: NodeJS.ProcessEnv = process.env): Promise<number> {
  const invocation = renderInvocation(argv, distDir, env);
  const userData = profileOf(env);
  const cache = path.join(userData, 'Engine Cache', 'Shader Cache');
  const probes = cliProbeEnvironment(cache, env);
  const runner = await angleService(distDir, env);
  const child = spawn(invocation.file, invocation.args, { stdio: 'inherit', windowsHide: true, env: { ...env, ...runner.env, ...probes } });
  const forward = (signal: NodeJS.Signals) => { if (child.exitCode === null && child.signalCode === null) child.kill(signal); };
  const interrupt = () => forward('SIGINT'), terminate = () => forward('SIGTERM');
  process.on('SIGINT', interrupt); process.on('SIGTERM', terminate);
  try {
    return await new Promise<number>((resolve, reject) => {
      child.once('error', error => reject(new Error(`native-renderer-unavailable: ${error.message}`)));
      child.once('exit', (code, signal) => resolve(code ?? 128 + (constants.signals[signal ?? 'SIGTERM'] ?? 15)));
    });
  } finally { process.off('SIGINT', interrupt); process.off('SIGTERM', terminate); await runner.close(); }
}
