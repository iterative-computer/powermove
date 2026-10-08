import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const cli = require.resolve('electron-vite/bin/electron-vite.js');
const child = spawn(process.execPath, [cli, 'dev'], { stdio: 'inherit', env: { ...process.env, POWERMOVE_REGISTRY_URL: 'http://localhost:8787', POWERMOVE_DEV_PROTOCOL: '1' } });
child.on('error', error => { console.error(error.message); process.exitCode = 1; });
child.on('exit', code => { process.exitCode = code ?? 1; });
