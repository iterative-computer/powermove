import { build } from 'esbuild';
import path from 'node:path';

/** A browser-only module, emitted as text so an export owns its exact runtime. */
export function playerBundlePlugin() {
  return {
    name: 'powermove-player-bundle',
    resolveId(id: string) { if (id === 'virtual:powermove-player') return '\0' + id; },
    async load(this: { addWatchFile(file: string): void }, id: string) {
      if (id !== '\0virtual:powermove-player') return;
      const result = await build({
        entryPoints: [path.resolve('src/renderer/src/player/player.ts')],
        bundle: true, write: false, format: 'esm', platform: 'browser', target: 'es2022',
        minify: true, legalComments: 'inline', define: { 'import.meta.hot': 'false' },
        metafile: true,
      });
      for (const file of Object.keys(result.metafile!.inputs)) this.addWatchFile(path.resolve(file));
      return `export default ${JSON.stringify(result.outputFiles[0]!.text)};`;
    },
  };
}
