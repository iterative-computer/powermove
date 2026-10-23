import { execFileSync } from 'node:child_process';
import { cp, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { build } from 'esbuild';
import { expect, it } from 'vitest';

const appRoot = path.resolve(import.meta.dirname, '..');
const appRequire = createRequire(path.join(appRoot, 'package.json'));
const builderRequire = createRequire(appRequire.resolve('electron-builder/package.json'));
const builderRoot = path.dirname(builderRequire.resolve('app-builder-lib/package.json'));

async function configuration() {
  const { load } = builderRequire('js-yaml') as { load(source: string): any };
  return load(await readFile(path.join(appRoot, 'electron-builder.yml'), 'utf8'));
}

it('excludes the SDK’s duplicate native executable while retaining its SDK, license, and copied runtime', async () => {
  const config = await configuration();
  const { getNodeModuleFileMatcher } = appRequire(path.join(builderRoot, 'out/fileMatcher.js'));
  const matcher = getNodeModuleFileMatcher(appRoot, '/unused/app', (value: string) => value, {}, {
    config, debugLogger: { isEnabled: false }
  });
  const filter = matcher.createFilter();
  const sdk = appRequire.resolve('@anthropic-ai/claude-agent-sdk');
  const info = await stat(sdk);
  // Use the same destination metadata electron-builder attaches when it
  // applies app-wide filters to hoisted or nested dependency files.
  const includes = (destination: string) => filter(sdk, Object.assign(info, { moduleFullFilePath: destination }));
  expect(includes('node_modules/@anthropic-ai/claude-agent-sdk-darwin-arm64/claude')).toBe(false);
  expect(includes('node_modules/other/node_modules/@anthropic-ai/claude-agent-sdk-darwin-arm64/claude')).toBe(false);
  expect(includes('node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs')).toBe(true);
  expect(includes('node_modules/@anthropic-ai/claude-agent-sdk/LICENSE.md')).toBe(true);
  expect(includes('node_modules/@anthropic-ai/claude-code/LICENSE.md')).toBe(true);
  expect(config.extraResources).toContainEqual({
    from: 'node_modules/@anthropic-ai/claude-code-darwin-arm64/claude', to: 'claude/bin/claude'
  });
});

it('discovers models using the real SDK and copied packaged CLI without any optional native SDK package', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-packaged-claude-'));
  try {
    const resources = path.join(directory, 'resources');
    const app = path.join(resources, 'app.asar');
    const sdkRoot = path.dirname(appRequire.resolve('@anthropic-ai/claude-agent-sdk'));
    const sdkTarget = path.join(app, 'node_modules/@anthropic-ai/claude-agent-sdk');
    await cp(sdkRoot, sdkTarget, { recursive: true, filter: (source) => path.basename(source) !== 'node_modules' });
    const cli = path.join(resources, 'claude/bin/claude');
    const home = path.join(directory, 'profile/claude-runtime');
    await mkdir(path.dirname(cli), { recursive: true });
    await mkdir(home, { recursive: true });
    await writeFile(cli, `#!${process.execPath}
      const fs = require('node:fs');
      const path = require('node:path');
      const readline = require('node:readline');
      fs.writeFileSync(path.join(process.env.CLAUDE_CONFIG_DIR, 'packaged-cli-ran'), 'yes');
      readline.createInterface({ input: process.stdin }).on('line', line => {
        const request = JSON.parse(line);
        if (request.type !== 'control_request') return;
        const response = request.request.subtype === 'initialize' ? {
          commands: [], agents: [], models: [{ value: 'fixture', displayName: 'Packaged fixture', description: '', supportedEffortLevels: ['low', 'high'] }]
        } : {};
        process.stdout.write(JSON.stringify({ type: 'control_response', response: {
          subtype: 'success', request_id: request.request_id, response
        } }) + '\\n');
      });
    `, { mode: 0o755 });
    const accountModule = path.join(app, 'account.cjs');
    await build({
      entryPoints: [path.join(appRoot, 'src/main/claude/account.ts')], outfile: accountModule,
      bundle: true, platform: 'node', format: 'cjs', external: ['@anthropic-ai/claude-agent-sdk']
    });
    const result = JSON.parse(execFileSync(process.execPath, ['-e', `
      const fs = require('node:fs');
      require('node:http').request = require('node:https').request = () => { throw new Error('Network is disabled in this fixture'); };
      global.fetch = () => { throw new Error('Network is disabled in this fixture'); };
      process.resourcesPath = ${JSON.stringify(resources)};
      const { ClaudeAccountClient } = require(${JSON.stringify(accountModule)});
      (async () => {
        const client = new ClaudeAccountClient({ userData: ${JSON.stringify(path.join(directory, 'profile'))}, claudeBinaryPref: () => null }, {
          prepareHome: async () => ${JSON.stringify(home)}
        });
        try { console.log(JSON.stringify(await client.models())); }
        finally { await client.shutdown(); }
      })().catch(error => { console.error(error); process.exit(1); });
    `], {
      cwd: app, encoding: 'utf8', timeout: 10_000,
      env: { PATH: process.env.PATH, DISABLE_TELEMETRY: '1', CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1' }
    }));
    expect(result).toEqual([{ id: 'fixture', label: 'Packaged fixture', reasoningEfforts: ['low', 'high'] }]);
    expect(await readFile(path.join(home, 'packaged-cli-ran'), 'utf8')).toBe('yes');
    expect(await stat(path.join(app, 'node_modules/@anthropic-ai/claude-agent-sdk-darwin-arm64')).catch(() => null)).toBeNull();
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
