/*
 * Entry for `npx powermove serve`. Resolves where the renderer, resources and
 * agent runtimes live for this install, then starts the host.
 */
import { createRequire } from 'node:module';
import { existsSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';

import { serve, type ServeOptions } from './index';
import { install, logs, status, uninstall, unitPath, unitText, type ServiceSpec } from './install';
import { detectInstallKind, PACKAGE } from './updates';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const run = promisify(execFile);

/** A service must point at a durable install, not an npx cache: install globally first when needed. */
async function durableEntry(entry: string, log: (line: string) => void): Promise<string> {
  if (detectInstallKind(entry) !== 'npx') return entry;
  log(`installing ${PACKAGE} globally so the service has a fixed path…`);
  await run('npm', ['install', '-g', `${PACKAGE}@latest`], { maxBuffer: 16 * 1024 * 1024 });
  const { stdout } = await run('npm', ['root', '-g']);
  const installed = path.join(stdout.trim(), PACKAGE, 'bin', 'powermove.mjs');
  if (!existsSync(installed)) throw new Error(`Global install did not land at ${installed}.`);
  return installed;
}

const require = createRequire(import.meta.url);

export interface CliLayout {
  /** Directory holding renderer/ and resources/. */
  distDir: string;
  /** bin/powermove.mjs, what a service runs. */
  entry: string;
  version: string;
}

const HELP = `powermove — run the Powermove host on this machine and use it from a browser.

Usage:
  powermove serve [options]      Run the host in this terminal (try it: npx powermove@latest serve)
  powermove install [options]    Keep it running as a user service (systemd on Linux, launchd on macOS)
  powermove uninstall            Stop and remove the service
  powermove status               Is the service running, and at which address
  powermove logs [-n 200]        Tail the service log

Options:
  --port <n>        Port to listen on (default 4747)
  --host <addr>     Interface to bind (default 0.0.0.0; use 127.0.0.1 to stay local)
  --user-data <dir> Profile directory (default ~/.powermove)
  --exports <dir>   Where Save… writes on this machine (default ~/Powermove)
  --token <value>   Access token (default: generated once, kept in the profile)
  --http            Plain http instead of self-signed https (only behind a TLS proxy such as tailscale serve)
  --dry-run         (install) print the service file instead of installing it
  -n <lines>        (logs) how many lines to show
  -h, --help        Show this help

Open the printed URL in a browser. Over Tailscale, use the 100.x address. The certificate
is self-signed, so the browser asks once whether to proceed; for a trusted one run
\`tailscale serve --bg https+insecure://localhost:4747\` and use the ts.net URL it prints.
Sign in to ChatGPT or Claude on this machine first (\`codex login\`, \`claude auth login\`)
or use Settings › Agents in the browser; the sign-in link opens on your side.
`;

type Parsed = Partial<ServeOptions> & { help?: boolean; command?: string; dryRun?: boolean; lines?: number };
function parseArgs(argv: string[]): Parsed {
  const out: Parsed = {};
  const rest = [...argv];
  if (rest[0] && !rest[0].startsWith('-')) out.command = rest.shift();
  while (rest.length) {
    const arg = rest.shift()!;
    const value = () => { const next = rest.shift(); if (next === undefined) throw new Error(`${arg} needs a value`); return next; };
    switch (arg) {
      case '--port': out.port = Number(value()); break;
      case '--host': out.host = value(); break;
      case '--user-data': out.userData = value(); break;
      case '--exports': out.exportsDir = value(); break;
      case '--token': out.token = value(); break;
      case '--http': out.insecure = true; break;
      case '--dry-run': out.dryRun = true; break;
      case '-n': out.lines = Number(value()); break;
      case '-h': case '--help': out.help = true; break;
      default: throw new Error(`Unknown option ${arg}`);
    }
  }
  return out;
}

const PLATFORM_SUFFIX: Record<string, string> = {
  'linux-x64': 'linux-x64', 'linux-arm64': 'linux-arm64', 'darwin-x64': 'darwin-x64', 'darwin-arm64': 'darwin-arm64',
  'win32-x64': 'win32-x64', 'win32-arm64': 'win32-arm64'
};

/** A platform package is an optional dependency of its parent, so resolve it from where the parent was installed. */
function platformPackage(parent: string, prefix: string): string | null {
  const suffix = PLATFORM_SUFFIX[`${process.platform}-${process.arch}`];
  if (!suffix) return null;
  try {
    const parentDir = path.dirname(require.resolve(`${parent}/package.json`));
    const fromParent = createRequire(path.join(parentDir, 'package.json'));
    return path.dirname(fromParent.resolve(`${prefix}${suffix}/package.json`));
  } catch { return null; }
}

/** The Codex binary shipped by @openai/codex's platform package, if installed. */
export function bundledCodexBinary(): string | null {
  const triples: Record<string, string> = {
    'linux-x64': 'x86_64-unknown-linux-musl', 'linux-arm64': 'aarch64-unknown-linux-musl',
    'darwin-x64': 'x86_64-apple-darwin', 'darwin-arm64': 'aarch64-apple-darwin',
    'win32-x64': 'x86_64-pc-windows-msvc', 'win32-arm64': 'aarch64-pc-windows-msvc'
  };
  const dir = platformPackage('@openai/codex', '@openai/codex-');
  const triple = triples[`${process.platform}-${process.arch}`];
  if (!dir || !triple) return null;
  const binary = path.join(dir, 'vendor', triple, 'bin', process.platform === 'win32' ? 'codex.exe' : 'codex');
  return existsSync(binary) ? binary : null;
}

/** The Claude binary shipped by @anthropic-ai/claude-code's platform package, if installed. */
export function bundledClaudeBinary(): string | null {
  const dir = platformPackage('@anthropic-ai/claude-code', '@anthropic-ai/claude-code-');
  if (!dir) return null;
  const binary = path.join(dir, process.platform === 'win32' ? 'claude.exe' : 'claude');
  return existsSync(binary) ? binary : null;
}

/** ffmpeg-static's package root, so render-encoder's `<appPath>/node_modules/ffmpeg-static/ffmpeg` resolves. */
function ffmpegAppPath(): string {
  const binary = require('ffmpeg-static') as string;
  return path.resolve(path.dirname(binary), '..', '..');
}

export async function main(argv: string[], layout: CliLayout): Promise<void> {
  let parsed;
  try { parsed = parseArgs(argv); } catch (error) { console.error(error instanceof Error ? error.message : String(error)); process.exit(2); }
  if (parsed.help || parsed.command === 'help') { process.stdout.write(HELP); return; }
  const userData = parsed.userData ?? process.env['POWERMOVE_USER_DATA'] ?? path.join(homedir(), '.powermove');
  if (parsed.command && parsed.command !== 'serve') {
    const flags: string[] = [];
    if (parsed.port !== undefined) flags.push('--port', String(parsed.port));
    if (parsed.host) flags.push('--host', parsed.host);
    if (parsed.exportsDir) flags.push('--exports', parsed.exportsDir);
    if (parsed.token) flags.push('--token', parsed.token);
    if (parsed.insecure) flags.push('--http');
    const spec: ServiceSpec = { entry: layout.entry, node: process.execPath, args: flags, userData, version: layout.version };
    const say = (line: string) => console.log(line);
    try {
      switch (parsed.command) {
        case 'install':
          if (parsed.dryRun) { console.log(`# ${unitPath(spec)}\n${unitText(spec)}`); return; }
          await install({ ...spec, entry: await durableEntry(spec.entry, say) }, say); return;
        case 'uninstall': await uninstall(spec, say); return;
        case 'status': await status(spec, say); return;
        case 'logs': await logs(spec, Number.isInteger(parsed.lines) ? parsed.lines! : 200, say); return;
        default: console.error(`Unknown command '${parsed.command}'.\n\n${HELP}`); process.exit(2);
      }
    } catch (error) {
      console.error(error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  }

  const running = await serve({
    host: parsed.host ?? '0.0.0.0',
    port: Number.isInteger(parsed.port) ? parsed.port! : 4747,
    userData,
    exportsDir: parsed.exportsDir ?? path.join(homedir(), 'Powermove'),
    rendererDir: path.join(layout.distDir, 'renderer'),
    resourcesDir: path.join(layout.distDir, 'resources'),
    appPath: ffmpegAppPath(),
    version: layout.version,
    codexBinary: bundledCodexBinary(),
    claudeBinary: bundledClaudeBinary(),
    engineScript: existsSync(path.join(layout.distDir, 'engine', 'engine.mjs')) ? path.join(layout.distDir, 'engine', 'engine.mjs') : null,
    installKind: detectInstallKind(layout.entry),
    ...(parsed.token ? { token: parsed.token } : {}),
    ...(parsed.insecure ? { insecure: true } : {})
  });

  console.log(`\nPowermove ${layout.version} is serving. Open one of these in a browser:\n`);
  for (const url of running.urls) console.log(`  ${url}`);
  console.log('\nThe token is remembered by the browser; the plain address works after the first visit.');
  if (!parsed.insecure) console.log('The certificate is self-signed: the browser will ask once whether to proceed.');
  console.log('Press Ctrl+C to stop.\n');

  let stopping = false;
  const stop = async () => {
    if (stopping) return;
    stopping = true;
    console.log('\nStopping…');
    // Agent shutdowns and ffmpeg cleanup get a few seconds; nothing here is
    // worth holding the terminal for longer than that.
    const deadline = setTimeout(() => { console.error('Stop timed out; exiting.'); process.exit(1); }, 5000);
    deadline.unref();
    await running.close().catch((error) => console.error(error));
    process.exit(0);
  };
  process.on('SIGINT', () => { void stop(); });
  process.on('SIGTERM', () => { void stop(); });
}
