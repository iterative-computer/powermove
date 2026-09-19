/*
 * Entry for `npx powermove serve`. Resolves where the renderer, resources and
 * agent runtimes live for this install, then starts the host.
 */
import { createRequire } from 'node:module';
import { existsSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';

import { serve, type ServeOptions } from './index';

const require = createRequire(import.meta.url);

export interface CliLayout {
  /** Directory holding renderer/ and resources/. */
  distDir: string;
  version: string;
}

const HELP = `powermove serve — run the Powermove host on this machine and use it from a browser.

Usage:
  powermove serve [options]

Options:
  --port <n>        Port to listen on (default 4747)
  --host <addr>     Interface to bind (default 0.0.0.0; use 127.0.0.1 to stay local)
  --user-data <dir> Profile directory (default ~/.powermove)
  --exports <dir>   Where Save… writes on this machine (default ~/Powermove)
  --token <value>   Access token (default: generated once, kept in the profile)
  -h, --help        Show this help

Open the printed URL in a browser. Over Tailscale, use the 100.x address.
Sign in to ChatGPT or Claude on this machine first (\`codex login\`, \`claude auth login\`)
or use Settings › Agents in the browser; the sign-in link opens on your side.
`;

function parseArgs(argv: string[]): Partial<ServeOptions> & { help?: boolean; command?: string } {
  const out: Partial<ServeOptions> & { help?: boolean; command?: string } = {};
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
      case '-h': case '--help': out.help = true; break;
      default: throw new Error(`Unknown option ${arg}`);
    }
  }
  return out;
}

function platformPackage(prefix: string, suffixes: Record<string, string>): string | null {
  const key = `${process.platform}-${process.arch}`;
  const suffix = suffixes[key];
  if (!suffix) return null;
  try { return path.dirname(require.resolve(`${prefix}${suffix}/package.json`)); } catch { return null; }
}

/** The Codex binary shipped by @openai/codex's platform package, if installed. */
export function bundledCodexBinary(): string | null {
  const triples: Record<string, string> = {
    'linux-x64': 'x86_64-unknown-linux-musl', 'linux-arm64': 'aarch64-unknown-linux-musl',
    'darwin-x64': 'x86_64-apple-darwin', 'darwin-arm64': 'aarch64-apple-darwin',
    'win32-x64': 'x86_64-pc-windows-msvc', 'win32-arm64': 'aarch64-pc-windows-msvc'
  };
  const dir = platformPackage('@openai/codex-', { 'linux-x64': 'linux-x64', 'linux-arm64': 'linux-arm64', 'darwin-x64': 'darwin-x64', 'darwin-arm64': 'darwin-arm64', 'win32-x64': 'win32-x64', 'win32-arm64': 'win32-arm64' });
  const triple = triples[`${process.platform}-${process.arch}`];
  if (!dir || !triple) return null;
  const binary = path.join(dir, 'vendor', triple, 'bin', process.platform === 'win32' ? 'codex.exe' : 'codex');
  return existsSync(binary) ? binary : null;
}

/** The Claude binary shipped by @anthropic-ai/claude-code's platform package, if installed. */
export function bundledClaudeBinary(): string | null {
  const dir = platformPackage('@anthropic-ai/claude-code-', { 'linux-x64': 'linux-x64', 'linux-arm64': 'linux-arm64', 'darwin-x64': 'darwin-x64', 'darwin-arm64': 'darwin-arm64', 'win32-x64': 'win32-x64', 'win32-arm64': 'win32-arm64' });
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
  if (parsed.command && parsed.command !== 'serve') { console.error(`Unknown command '${parsed.command}'.\n\n${HELP}`); process.exit(2); }

  const running = await serve({
    host: parsed.host ?? '0.0.0.0',
    port: Number.isInteger(parsed.port) ? parsed.port! : 4747,
    userData: parsed.userData ?? process.env['POWERMOVE_USER_DATA'] ?? path.join(homedir(), '.powermove'),
    exportsDir: parsed.exportsDir ?? path.join(homedir(), 'Powermove'),
    rendererDir: path.join(layout.distDir, 'renderer'),
    resourcesDir: path.join(layout.distDir, 'resources'),
    appPath: ffmpegAppPath(),
    version: layout.version,
    codexBinary: bundledCodexBinary(),
    claudeBinary: bundledClaudeBinary(),
    ...(parsed.token ? { token: parsed.token } : {})
  });

  console.log(`\nPowermove ${layout.version} is serving. Open one of these in a browser:\n`);
  for (const url of running.urls) console.log(`  ${url}`);
  console.log('\nThe token is remembered by the browser; the plain address works after the first visit.\nPress Ctrl+C to stop.\n');

  let stopping = false;
  const stop = async () => {
    if (stopping) return;
    stopping = true;
    console.log('\nStopping…');
    await running.close().catch((error) => console.error(error));
    process.exit(0);
  };
  process.on('SIGINT', () => { void stop(); });
  process.on('SIGTERM', () => { void stop(); });
}
