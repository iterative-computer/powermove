'use strict';

if (process.platform !== 'darwin') process.exit(0);

const { spawnSync } = require('node:child_process');

const result = spawnSync('node-gyp', ['rebuild'], {
  cwd: require('node:path').resolve(__dirname, '..'),
  stdio: 'inherit',
  shell: process.platform === 'win32'
});

if (result.error) throw result.error;
process.exit(result.status ?? 1);
