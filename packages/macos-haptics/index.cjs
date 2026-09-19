'use strict';

const path = require('node:path');

let native = null;
if (process.platform === 'darwin') {
  native = require(path.join(__dirname, 'build', 'Release', 'macos_haptics.node'));
}

module.exports = {
  fontFamilies() { return native?.fontFamilies?.() ?? null; },
  triggerAlignment() {
    native?.triggerAlignment();
  }
};
