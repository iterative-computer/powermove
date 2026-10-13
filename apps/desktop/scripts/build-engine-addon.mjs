/*
 * Builds the native engine's pm-node addon (engine/crates/pm-node) with the
 * pinned toolchain (engine/env.sh) and installs it as native/pm_node.node,
 * where development runs and electron-builder (asarUnpack: native/**) find
 * it. Then loads it once and checks that it speaks the same protocol build
 * as main's generated types: draft contracts interoperate only between
 * equal builds, so a stale addon would fail every handshake.
 *
 *   node scripts/build-engine-addon.mjs [--debug]
 */
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readFileSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';

const desktop = path.resolve(import.meta.dirname, '..');
const engine = path.resolve(desktop, '../../engine');
const debug = process.argv.includes('--debug');
const profile = debug ? 'debug' : 'release';

const cargo = `. ./env.sh && exec cargo build ${debug ? '' : '--release '}--locked -p pm-node --features node`;
execFileSync('/bin/sh', ['-c', cargo], { cwd: engine, stdio: 'inherit' });

const built = path.join(engine, 'target', profile, 'libpm_node.dylib');
const target = path.join(desktop, 'native', 'pm_node.node');
mkdirSync(path.dirname(target), { recursive: true });
copyFileSync(built, target);

const expected = /PROTOCOL_BUILD = "([^"]+)"/.exec(readFileSync(path.join(desktop, 'src/shared/engine-protocol/index.ts'), 'utf8'))?.[1];
const addon = createRequire(import.meta.url)(target);
const actual = addon.buildId();
if (!expected || actual !== expected) {
  throw new Error(`pm_node.node speaks ${actual}, but main's generated types are ${expected}. Regenerate with pm-protocol-gen and rebuild.`);
}
process.stdout.write(`pm_node.node (${profile}, ${(statSync(target).size / 1e6).toFixed(1)} MB) speaks ${actual}\n`);
