import { execFile } from 'node:child_process';
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);
const repository = path.resolve(import.meta.dirname, '..');

function releaseCredentials(environment) {
  const missing = [];
  if (!environment.CSC_LINK) missing.push('CSC_LINK');
  if (!environment.CSC_KEY_PASSWORD) missing.push('CSC_KEY_PASSWORD');
  const apiKey = environment.APPLE_API_KEY && environment.APPLE_API_KEY_ID && environment.APPLE_API_ISSUER;
  const appleId = environment.APPLE_ID && environment.APPLE_APP_SPECIFIC_PASSWORD && environment.APPLE_TEAM_ID;
  if (!apiKey && !appleId) {
    missing.push('APPLE_API_KEY + APPLE_API_KEY_ID + APPLE_API_ISSUER (or Apple ID notarization credentials)');
  }
  return missing;
}

async function run(command, args) {
  try {
    const { stdout, stderr } = await execFileAsync(command, args, {
      cwd: repository,
      env: process.env,
      maxBuffer: 20 * 1024 * 1024,
    });
    if (stdout) process.stdout.write(stdout);
    if (stderr) process.stderr.write(stderr);
  } catch (error) {
    if (error?.stdout) process.stdout.write(error.stdout);
    if (error?.stderr) process.stderr.write(error.stderr);
    throw error;
  }
}

async function findPackagedApp() {
  const dist = path.join(repository, 'dist');
  for (const entry of await readdir(dist, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.startsWith('mac')) continue;
    const candidate = path.join(dist, entry.name, 'Powermove.app');
    try {
      await readdir(candidate);
      return candidate;
    } catch {
      // Keep looking for electron-builder's architecture-specific folder.
    }
  }
  throw new Error('The signed Powermove.app was not found in dist/.');
}

async function main() {
  const missing = releaseCredentials(process.env);
  if (missing.length) {
    throw new Error(`Production release credentials are incomplete: ${missing.join(', ')}.`);
  }
  if (process.argv.includes('--check')) {
    process.stdout.write('Production signing and notarization credentials are present.\n');
    return;
  }

  // electron-builder names the update manifest after publish.channel and
  // defaults to latest-mac.yml. electron-updater asks for beta-mac.yml on
  // prerelease versions (falling back to latest), so keep the two in step.
  const { version } = JSON.parse(await readFile(path.join(repository, 'package.json'), 'utf8'));
  const channel = /^\d+\.\d+\.\d+-([a-z]+)\./i.exec(version)?.[1]?.toLowerCase() ?? 'latest';

  await run('bun', ['run', 'build']);
  await run(path.join(repository, 'node_modules/.bin/electron-builder'), [
    '--mac', '--arm64', '--publish', 'never',
    '--config', 'electron-builder.yml',
    `--config.publish.channel=${channel}`,
    '--config.mac.identity=Developer ID Application',
    '--config.mac.hardenedRuntime=true',
    '--config.mac.notarize=true',
  ]);

  await run('node', [path.join(repository, 'scripts/create-dmg.mjs')]);

  const app = await findPackagedApp();
  await run('/usr/bin/codesign', ['--verify', '--deep', '--strict', '--verbose=2', app]);
  await run('/usr/sbin/spctl', ['--assess', '--type', 'execute', '--verbose=4', app]);
  await run('/usr/bin/xcrun', ['stapler', 'validate', app]);

  const dist = path.join(repository, 'dist');
  for (const entry of await readdir(dist, { withFileTypes: true })) {
    if (!entry.isFile() || !entry.name.endsWith('.dmg')) continue;
    const artifact = path.join(dist, entry.name);
    await run('/usr/bin/hdiutil', ['verify', artifact]);
    // electron-builder notarizes/staples the app before creating its archives.
    // The enclosing DMG has no separate notarization ticket.
  }
  process.stdout.write(`Verified signed and notarized release: ${app}\n`);
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 1;
});
