/* Vite's `?raw` imports as text for the esbuild bundles of the engine
   harnesses (main imports the Blender worker script that way,
   src/main/engine/blender.ts); electron-vite bundles them natively. */
import fs from 'node:fs';
import path from 'node:path';

export const rawText = {
  name: 'raw-text',
  setup(b) {
    b.onResolve({ filter: /\?raw$/ }, (args) => ({ path: path.resolve(args.resolveDir, args.path.slice(0, -'?raw'.length)), namespace: 'raw-text' }));
    b.onLoad({ filter: /.*/, namespace: 'raw-text' }, (args) => ({ contents: fs.readFileSync(args.path, 'utf8'), loader: 'text' }));
  }
};
