/*
 * Builds the native engine's pm-node addon (engine/crates/pm-node-addon, a
 * cdylib over pm-node, so the release profile's thin LTO applies:
 * docs/engine/release-build.md) with the pinned toolchain (engine/env.sh)
 * and installs it as native/pm_node.node,
 * where development runs and electron-builder (asarUnpack: native/**) find
 * it. Then loads it once and checks that it speaks the same protocol build
 * as main's generated types: draft contracts interoperate only between
 * equal builds, so a stale addon would fail every handshake.
 *
 * Also builds main's pm-bridge addon (engine/crates/pm-bridge-addon: the
 * frame bridge and the font registry) into native/pm_bridge.node for
 * packaging and
 * engine/target/pm-bridge/pm_bridge.node, where development runs load it.
 *
 *   node scripts/build-engine-addon.mjs [--debug] [--test-hooks]
 */
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readFileSync, renameSync, rmSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';

const desktop = path.resolve(import.meta.dirname, '..');
const engine = path.resolve(desktop, '../../engine');
const debug = process.argv.includes('--debug');
// e2e builds only: env-gated test hooks (the proxy disk guard's reserve
// override). Release builds never pass it.
const testHooks = process.argv.includes('--test-hooks');
const profile = debug ? 'debug' : 'release';

// A loaded Mach-O must retain its inode and code signature. Overwriting it
// in place can kill a running engine or the verification load on macOS.
function installAddon(source, target) {
  mkdirSync(path.dirname(target), { recursive: true });
  const staged = `${target}.${process.pid}.tmp`;
  try {
    copyFileSync(source, staged);
    renameSync(staged, target);
  } finally {
    rmSync(staged, { force: true });
  }
}

// Two invocations: in one, feature unification would compile pm-bridge's
// napi glue into pm_node.node too.
const cargo = `. ./env.sh && cargo build ${debug ? '' : '--release '}--locked -p pm-node-addon --features node${testHooks ? ',test-hooks' : ''} && exec cargo build ${debug ? '' : '--release '}--locked -p pm-bridge-addon --features node`;
execFileSync('/bin/sh', ['-c', cargo], { cwd: engine, stdio: 'inherit' });

const bridgeBuilt = path.join(engine, 'target', profile, 'libpm_bridge_addon.dylib');
for (const bridgeTarget of [path.join(desktop, 'native', 'pm_bridge.node'), path.join(engine, 'target', 'pm-bridge', 'pm_bridge.node')]) {
  installAddon(bridgeBuilt, bridgeTarget);
}

const built = path.join(engine, 'target', profile, 'libpm_node_addon.dylib');
const target = path.join(desktop, 'native', 'pm_node.node');
installAddon(built, target);

const expected = /PROTOCOL_BUILD = "([^"]+)"/.exec(readFileSync(path.join(desktop, 'src/shared/engine-protocol/index.ts'), 'utf8'))?.[1];
const addon = createRequire(import.meta.url)(target);
const actual = addon.buildId();
if (!expected || actual !== expected) {
  throw new Error(`pm_node.node speaks ${actual}, but main's generated types are ${expected}. Regenerate with pm-protocol-gen and rebuild.`);
}
const bridge = createRequire(import.meta.url)(path.join(desktop, 'native', 'pm_bridge.node'));
if (typeof bridge.BridgeHost !== 'function') throw new Error('pm_bridge.node does not export BridgeHost');
process.stdout.write(`pm_bridge.node (${profile}, ${(statSync(bridgeBuilt).size / 1e6).toFixed(1)} MB) loads\n`);
process.stdout.write(`pm_node.node (${profile}, ${(statSync(target).size / 1e6).toFixed(1)} MB) speaks ${actual}\n`);
