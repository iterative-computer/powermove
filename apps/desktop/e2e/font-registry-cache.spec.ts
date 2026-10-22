import { execFileSync } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, repoRoot, test } from './helpers/app';

test.skip(process.platform !== 'darwin', 'Core Text notifications are macOS-specific');
const nativeRoot = path.resolve(repoRoot, '../../packages/macos-haptics');
const family = 'Powermove Font Cache Fixture';

test('native font registration and removal refresh the cached editor font list', async ({ session }) => {
  await session.openEditor();
  await session.page.waitForFunction(() => (window as any).PM.Fonts.system.length > 0);
  const before = await session.app.evaluate(async ({ app }, input) => {
    const request = process.getBuiltinModule('node:module').createRequire(app.getAppPath() + '/package.json');
    const fs = process.getBuiltinModule('node:fs'), path = process.getBuiltinModule('node:path');
    const dependencies = process.getBuiltinModule('node:module').createRequire(path.join(fs.realpathSync(path.join(app.getAppPath(), 'node_modules/transcribe-cpp')), 'package.json'));
    const koffi = dependencies('koffi'), native = request(input.native);
    const version = native.fontRegistryVersion();
    const { Worker } = process.getBuiltinModule('node:worker_threads');
    const workerRevision = await new Promise((resolve, reject) => {
      const worker = new Worker(`const {parentPort}=require('node:worker_threads'); parentPort.postMessage(require(${JSON.stringify(input.native)}).fontRegistryVersion());`, { eval: true });
      let value: unknown;
      worker.on('message', message => { value = message; });
      worker.on('error', reject);
      worker.on('exit', code => code === 0 ? resolve(value) : reject(new Error('Font registry worker failed')));
    });
    const core = koffi.load('/System/Library/Frameworks/CoreText.framework/CoreText');
    const foundation = koffi.load('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation');
    const makeUrl = foundation.func('void *CFURLCreateFromFileSystemRepresentation(void *, const uint8_t *, long, bool)');
    const file = Buffer.from(input.file), url = makeUrl(null, file, file.length, false);
    const state = { native, core, foundation, url, registered: false,
      register: core.func('bool CTFontManagerRegisterFontsForURL(void *, unsigned int, void **)'),
      unregister: core.func('bool CTFontManagerUnregisterFontsForURL(void *, unsigned int, void **)'),
      release: foundation.func('void CFRelease(void *)') };
    (globalThis as any).__fontRegistryProof = state;
    // Process scope changes only this disposable app's registry.
    state.registered = state.register(url, 1, null);
    if (!state.registered) throw new Error('Could not register the synthetic process-scoped font');
    return { version, workerRevision };
  }, { native: path.join(nativeRoot, 'index.cjs'), file: path.join(repoRoot, 'e2e/fixtures/font-registry.ttf') });
  try {
    expect(before.workerRevision).toBeNull();
    await expect.poll(() => session.app.evaluate(() => (globalThis as any).__fontRegistryProof.native.fontRegistryVersion())).toBeGreaterThan(before.version!);
    await expect.poll(() => session.page.evaluate(family => (window as any).PM.Fonts.system.includes(family), family), { timeout: 10_000 }).toBe(true);
    const added = await session.app.evaluate(() => {
      const state = (globalThis as any).__fontRegistryProof, version = state.native.fontRegistryVersion();
      if (!state.unregister(state.url, 1, null)) throw new Error('Could not unregister the synthetic font');
      state.registered = false;
      return version;
    });
    await expect.poll(() => session.app.evaluate(() => (globalThis as any).__fontRegistryProof.native.fontRegistryVersion())).toBeGreaterThan(added!);
    await expect.poll(() => session.page.evaluate(family => (window as any).PM.Fonts.system.includes(family), family), { timeout: 10_000 }).toBe(false);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.app.evaluate(() => {
      const state = (globalThis as any).__fontRegistryProof;
      if (state.registered) state.unregister(state.url, 1, null);
      state.release(state.url); delete (globalThis as any).__fontRegistryProof;
    });
  }
});

test('native font invalidation receives cross-process notifications on the Electron run loop', async ({ session }) => {
  test.setTimeout(60_000);
  const notification = 'com.powermove.font-registry-test.' + path.basename(session.userData);
  const source = path.join(session.userData, 'font-notifications.mm'), binary = path.join(session.userData, 'font-notifications.node');
  const original = await readFile(path.join(nativeRoot, 'src/addon.mm'), 'utf8');
  // Exercise the production observer/lifetime code with an isolated name, so
  // the cross-process test never broadcasts a real system font change.
  const isolated = original.replaceAll('kCTFontManagerRegisteredFontsChangedNotification', 'regressionNotificationName')
    .replace('static std::atomic<uint32_t> fontRegistryVersion', `static CFStringRef regressionNotificationName = CFSTR(${JSON.stringify(notification)});\nstatic std::atomic<uint32_t> fontRegistryVersion`);
  await writeFile(source, isolated);
  const makefile = await readFile(path.join(nativeRoot, 'build/macos_haptics.target.mk'), 'utf8');
  const include = makefile.match(/-I([^\n]+\/include\/node)\s*\\/u)?.[1];
  if (!include) throw new Error('The built native addon must provide Node headers for its notification regression test');
  execFileSync('xcrun', ['clang++', '-std=c++17', '-shared', '-undefined', 'dynamic_lookup', '-I', include,
    '-framework', 'AppKit', '-framework', 'CoreText', source, '-o', binary], { timeout: 30_000 });
  const before = await session.app.evaluate(({ app }, binary) => {
    const request = process.getBuiltinModule('node:module').createRequire(app.getAppPath() + '/package.json');
    const native = request(binary);
    (globalThis as any).__isolatedFontNotifications = native;
    return native.fontRegistryVersion();
  }, binary);
  const emitter = path.join(session.userData, 'font-notification-emitter.cjs');
  await writeFile(emitter, `
    const {createRequire}=require('node:module'),fs=require('node:fs'),path=require('node:path');
    const request=createRequire(path.join(fs.realpathSync(${JSON.stringify(path.join(repoRoot, 'node_modules/transcribe-cpp'))}),'package.json'));
    const koffi=request('koffi'),cf=koffi.load('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation');
    const center=cf.func('void *CFNotificationCenterGetDistributedCenter()')();
    const make=cf.func('void *CFStringCreateWithCString(void *, const char *, unsigned int)');
    const name=make(null,${JSON.stringify(notification)},0x08000100);
    cf.func('void CFNotificationCenterPostNotification(void *, void *, void *, void *, bool)')(center,name,null,null,true);
    cf.func('void CFRelease(void *)')(name);
  `);
  execFileSync(process.execPath, [emitter], { timeout: 10_000 });
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__isolatedFontNotifications.fontRegistryVersion()), { timeout: 10_000 }).toBeGreaterThan(before!);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
