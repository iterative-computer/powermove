import { bundlePlayerEntry, entries } from '@powermove/player/build';

/** A browser-only module, emitted as text so an export owns its exact runtime. */
export function playerBundlePlugin() {
  return {
    name: 'powermove-player-bundle',
    resolveId(id: string) { if (id === 'virtual:powermove-player') return '\0' + id; },
    async load(this: { addWatchFile(file: string): void }, id: string) {
      if (id !== '\0virtual:powermove-player') return;
      const { text, inputs } = await bundlePlayerEntry(entries.player);
      for (const file of inputs) this.addWatchFile(file);
      return `export default ${JSON.stringify(text)};`;
    },
  };
}
