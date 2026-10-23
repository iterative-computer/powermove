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
/** `_electron.launch({ ..., args: [..., '.'] })` or `spawn(electron, ['.', ...])`:
    the app's own main entry, not a harness main.js. MCP stdio mode exits
    before the app main runs, so it is not a launch. */
const APP_LAUNCH = /_electron\.launch\(\{[^}]*?args:\s*\[[^\]]*'\.'|spawn\(electron,\s*\['\.'(?!,\s*'--powermove-mcp')/;

function sources(): string[] {
  return roots.flatMap(root => readdirSync(path.join(repo, root), { recursive: true, encoding: 'utf8' })
    .filter(file => /\.(m?[jt]s)$/.test(file) && !file.includes('node_modules'))
    .map(file => path.join(root, file))
    .filter(file => file !== path.relative(repo, __filename)));
}

/** Launchers the pattern can't see: frame-bridge-measure preloads its probe
    with `electron -r app-main.js <desktop>` for its app rows. */
const INDIRECT = ['apps/desktop/scripts/frame-bridge-measure.mjs'];
const launchers = [...sources().filter(file => APP_LAUNCH.test(readFileSync(path.join(repo, file), 'utf8'))), ...INDIRECT];

describe('app launchers in test and measurement harnesses', () => {
  it('finds the known launchers', () => {
    for (const known of ['apps/desktop/e2e/helpers/app.ts', 'apps/desktop/scripts/gate-engine-switch-corpus.mjs',
      'engine/corpus/src/electron-bridge.mts']) {
      expect(launchers).toContain(known);
    }
  });

  it.each(launchers)('%s disables automatic runtime downloads', (file) => {
    expect(readFileSync(path.join(repo, DELEGATED[file] ?? file), 'utf8')).toMatch(SWITCH);
  });
});
