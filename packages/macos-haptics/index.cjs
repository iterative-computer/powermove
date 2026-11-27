'use strict';

const path = require('node:path');

let native = null;
if (process.platform === 'darwin') {
  native = require(path.join(__dirname, 'build', 'Release', 'macos_haptics.node'));
}

module.exports = {
  fileVersions(operation, path, contents = '') { return native?.fileVersions?.(operation, path, contents) ?? Promise.resolve(null); },
  cloneFile(source, destination) { return native?.fileOperation?.('clone', source, destination) ?? Promise.resolve(false); },
  swapFiles(source, destination) { return native?.fileOperation?.('swap', source, destination) ?? Promise.resolve(false); },
  installFile(source, destination) { return native?.fileOperation?.('install', source, destination) ?? Promise.resolve(false); },
  fullSync(fd) { return native?.fileOperation?.('sync', fd) ?? Promise.resolve(false); },
  cloudFileState(path, download = false) { return native?.cloudFileState?.(path, download) ?? Promise.resolve("unknown"); },
  fontFamilies() { return native?.fontFamilies?.() ?? null; },
  fontRegistryVersion() { return native?.fontRegistryVersion?.() ?? null; },
  sandboxDeniesLookup(pids, name) { return native?.sandboxDeniesLookup?.(pids, name) ?? null; },
  triggerAlignment() {
    native?.triggerAlignment();
  }
};
