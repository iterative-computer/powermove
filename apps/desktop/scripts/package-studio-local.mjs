/** Hardened, ad-hoc package for local acceptance. Never notarizes or publishes. */
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { homedir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { verifyStudioRuntime } from './verify-studio-runtime.mjs';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const env = { ...process.env, CARGO_TARGET_DIR: process.env.CARGO_TARGET_DIR ?? path.join(homedir(), 'Developer/pm-target'), CSC_IDENTITY_AUTO_DISCOVERY: 'false' };
delete env.ELECTRON_RUN_AS_NODE;
const run = (executable, args) => execFileSync('/usr/bin/nice', ['-n', '10', executable, ...args], { cwd: desktop, env, stdio: 'inherit' });
if (process.platform !== 'darwin') throw new Error('Local Studio packaging requires macOS.');
run('bun', ['run', 'build']);
run(path.join(desktop, 'node_modules/.bin/electron-builder'), [
  '--dir', '--mac', '--arm64', '--publish', 'never',
  '--config.mac.identity=-', '--config.mac.hardenedRuntime=true', '--config.mac.notarize=false',
  '--config.mac.entitlements=resources/entitlements.mac.adhoc.plist',
  '--config.mac.entitlementsInherit=resources/entitlements.mac.adhoc.plist',
]);
run('/usr/bin/codesign', ['--verify', '--deep', '--strict', path.join(desktop, 'dist/mac-arm64/Powermove.app')]);
await verifyStudioRuntime(path.join(desktop, 'dist/mac-arm64/Powermove.app'), { adHoc: true });
