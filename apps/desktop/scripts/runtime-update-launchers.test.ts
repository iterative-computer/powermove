import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

/* Test and measurement harnesses launch the real app with a fresh profile but
   inherit the developer's agent accounts. Without the switch, the automatic
   runtime refresh (codex/index.ts refreshRuntime, 20 s after boot) downloads
   the ~122 MB Claude Code runtime, and killing the launch mid-download leaves
   its scratch directory behind. Every launcher of the app's main entry must
   set POWERMOVE_DISABLE_RUNTIME_UPDATES=1 (or go through e2e/helpers/app.ts). */

const desktop = path.resolve(__dirname, '..');
const repo = path.resolve(desktop, '../..');
const roots = ['apps/desktop/e2e', 'apps/desktop/scripts', 'engine/corpus/src'];
const SWITCH = /POWERMOVE_DISABLE_RUNTIME_UPDATES:\s*'1'/;
/** Launchers that receive their env from another file, which must set the switch. */
const DELEGATED: Record<string, string> = {
  // The corpus bridge launches with the env engine/corpus/src/app.ts sends it.
  'engine/corpus/src/electron-bridge.mts': 'engine/corpus/src/app.ts'
};
/** Inspect arguments independently of the spawning function's name, so wrappers
    such as start(electron, ['.']) count too. Harness main.js files and tool-only
    entry modes never start the editor or its automatic runtime refresh. */
const LAUNCH_ARGS = /\belectron(?:\s+as\s+(?:unknown|string))*\s*,\s*\[([^\]]*)\]|_electron\.launch\(\{[^}]*?args:\s*\[([^\]]*)\]/g;
const APP_ENTRY = /['"]\.['"]|\b(?:desktop(?:Root|Dir)?|repoRoot|appDir)\b/;
const TOOL_ONLY = /['"]--powermove-(?:mcp|agent-tools)['"]/;

function launchesApp(source: string): boolean {
  return [...source.matchAll(LAUNCH_ARGS)].some(match => {
    const args = match[1] ?? match[2] ?? '';
    return APP_ENTRY.test(args) && !TOOL_ONLY.test(args);
  });
}

function auditLauncher(file: string, source: string, envSource = source): void {
  if (launchesApp(source) || INDIRECT.includes(file)) {
    expect(envSource, `${file} must disable automatic runtime downloads`).toMatch(SWITCH);
  }
}

function sources(): string[] {
  return roots.flatMap(root => readdirSync(path.join(repo, root), { recursive: true, encoding: 'utf8' })
    .filter(file => /\.(m?[jt]s)$/.test(file) && !file.includes('node_modules'))
    .map(file => path.join(root, file))
    .filter(file => file !== path.relative(repo, __filename)));
}

/** Launchers the pattern can't see: frame-bridge-measure preloads its probe
    with `electron -r app-main.js <desktop>` for its app rows. */
const INDIRECT = ['apps/desktop/scripts/frame-bridge-measure.mjs'];
const launchers = [...new Set([...sources().filter(file => launchesApp(readFileSync(path.join(repo, file), 'utf8'))), ...INDIRECT])];

describe('app launchers in test and measurement harnesses', () => {
  it('finds the known launchers', () => {
    for (const known of ['apps/desktop/e2e/helpers/app.ts', 'apps/desktop/scripts/gate-engine-switch-corpus.mjs',
      'apps/desktop/scripts/test-native-hidden-parity.mjs', 'engine/corpus/src/electron-bridge.mts']) {
      expect(launchers).toContain(known);
    }
  });

  it.each([
    "start(electron, ['.'], { env: appEnv })",
    'start(electron, ["."], { env: appEnv })',
    'start(electron, [desktop], { env: appEnv })',
    'start(electron, [repoRoot], { env: appEnv })',
    'spawn(electron as unknown as string, [repoRoot], { env: appEnv })',
    '_electron.launch({ args: [repoRoot], env: appEnv })'
  ])('rejects an unswitched app-entry launch through %s', (launch) => {
    const source = `const appEnv = { ...process.env }; ${launch};`;
    expect(launchesApp(source)).toBe(true);
    expect(() => auditLauncher('wrapper.mjs', source)).toThrow('must disable automatic runtime downloads');
    expect(() => auditLauncher('wrapper.mjs', `const appEnv = { POWERMOVE_DISABLE_RUNTIME_UPDATES: '1' }; ${launch};`)).not.toThrow();
  });

  it.each([
    "start(electron, ['.', '--powermove-mcp'], { env })",
    "start(electron, [repoRoot, '--powermove-agent-tools'], { env })",
    "spawn(electron, [path.join(out, 'main.js')], { env })"
  ])('excludes tool-only entries and standalone harnesses: %s', (source) => {
    expect(launchesApp(source)).toBe(false);
  });

  it.each(launchers)('%s disables automatic runtime downloads', (file) => {
    auditLauncher(file, readFileSync(path.join(repo, file), 'utf8'),
      readFileSync(path.join(repo, DELEGATED[file] ?? file), 'utf8'));
  });
});
